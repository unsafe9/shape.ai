use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shape_coordination::{Clock, Coordinator, FileCoordinator, InMemoryCoordinator};

struct FakeClock(AtomicU64);

impl FakeClock {
    fn new() -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(1_000_000)))
    }
    fn advance(&self, by: Duration) {
        self.0.fetch_add(by.as_millis() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

fn ttl() -> Duration {
    Duration::from_secs(30)
}

#[tokio::test]
async fn inmem_acquire_grants() {
    let c = InMemoryCoordinator::with_clock(FakeClock::new());
    let lease = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    assert_eq!(lease.canvas_id, "cv");
    assert_eq!(lease.owner, "alice");
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn inmem_second_owner_denied_while_held() {
    let c = InMemoryCoordinator::with_clock(FakeClock::new());
    c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    let err = c.acquire_lease("cv", "bob", ttl()).await;
    assert!(err.is_err(), "bob must not steal a live lease");
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn inmem_same_owner_reacquire_refreshes() {
    let c = InMemoryCoordinator::with_clock(FakeClock::new());
    c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    let again = c.acquire_lease("cv", "alice", ttl()).await;
    assert!(again.is_ok());
}

#[tokio::test]
async fn inmem_steal_after_expiry() {
    let clock = FakeClock::new();
    let c = InMemoryCoordinator::with_clock(clock.clone());
    c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(31));
    assert_eq!(c.find_owner("cv").await.unwrap(), None);
    let lease = c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert_eq!(lease.owner, "bob");
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("bob".to_string()));
}

#[tokio::test]
async fn inmem_renew_extends() {
    let clock = FakeClock::new();
    let c = InMemoryCoordinator::with_clock(clock.clone());
    let lease = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(20));
    c.renew(&lease, ttl()).await.unwrap();
    clock.advance(Duration::from_secs(20));
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn inmem_renew_fails_after_steal() {
    let clock = FakeClock::new();
    let c = InMemoryCoordinator::with_clock(clock.clone());
    let alice = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(31));
    c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert!(c.renew(&alice, ttl()).await.is_err());
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("bob".to_string()));
}

#[tokio::test]
async fn inmem_release_frees() {
    let c = InMemoryCoordinator::with_clock(FakeClock::new());
    let lease = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    c.release(lease).await.unwrap();
    assert_eq!(c.find_owner("cv").await.unwrap(), None);
    c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("bob".to_string()));
}

#[tokio::test]
async fn inmem_stale_release_is_noop() {
    let clock = FakeClock::new();
    let c = InMemoryCoordinator::with_clock(clock.clone());
    let alice = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(31));
    let bob = c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    c.release(alice).await.unwrap();
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("bob".to_string()));
    drop(bob);
}

#[tokio::test]
async fn inmem_pubsub_delivers() {
    let c = InMemoryCoordinator::new();
    let mut rx = c.subscribe("cv");
    c.publish("cv", b"hello".to_vec()).await.unwrap();
    let got = rx.recv().await.unwrap();
    assert_eq!(got, b"hello");
}

#[tokio::test]
async fn inmem_pubsub_isolated_per_canvas() {
    let c = InMemoryCoordinator::new();
    let mut rx_a = c.subscribe("a");
    let mut rx_b = c.subscribe("b");
    c.publish("a", b"to-a".to_vec()).await.unwrap();
    assert_eq!(rx_a.recv().await.unwrap(), b"to-a");
    c.publish("b", b"to-b".to_vec()).await.unwrap();
    assert_eq!(rx_b.recv().await.unwrap(), b"to-b");
}

#[tokio::test]
async fn inmem_presence_put_get_with_expiry() {
    let clock = FakeClock::new();
    let c = InMemoryCoordinator::with_clock(clock.clone());
    c.presence_put("cv", "alice", b"cursor:1".to_vec(), ttl())
        .await
        .unwrap();
    c.presence_put("cv", "bob", b"cursor:2".to_vec(), Duration::from_secs(5))
        .await
        .unwrap();

    let mut got = c.presence_get("cv").await.unwrap();
    got.sort();
    assert_eq!(
        got,
        vec![
            ("alice".to_string(), b"cursor:1".to_vec()),
            ("bob".to_string(), b"cursor:2".to_vec()),
        ]
    );

    clock.advance(Duration::from_secs(6));
    let got = c.presence_get("cv").await.unwrap();
    assert_eq!(got, vec![("alice".to_string(), b"cursor:1".to_vec())]);
}

fn file_coord(clock: Arc<FakeClock>) -> (FileCoordinator, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let c = FileCoordinator::open_with_clock(dir.path(), clock).unwrap();
    (c, dir)
}

#[tokio::test]
async fn file_acquire_grants() {
    let (c, _d) = file_coord(FakeClock::new());
    let lease = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    assert_eq!(lease.owner, "alice");
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn file_second_owner_denied_while_held() {
    let (c, _d) = file_coord(FakeClock::new());
    c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    assert!(c.acquire_lease("cv", "bob", ttl()).await.is_err());
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn file_steal_after_expiry() {
    let clock = FakeClock::new();
    let (c, _d) = file_coord(clock.clone());
    c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(31));
    assert_eq!(c.find_owner("cv").await.unwrap(), None);
    let lease = c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert_eq!(lease.owner, "bob");
}

#[tokio::test]
async fn file_renew_extends_and_blocks_stale() {
    let clock = FakeClock::new();
    let (c, _d) = file_coord(clock.clone());
    let alice = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    clock.advance(Duration::from_secs(20));
    c.renew(&alice, ttl()).await.unwrap();
    clock.advance(Duration::from_secs(20));
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("alice".to_string()));

    clock.advance(Duration::from_secs(31));
    c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert!(c.renew(&alice, ttl()).await.is_err());
}

#[tokio::test]
async fn file_release_frees() {
    let (c, _d) = file_coord(FakeClock::new());
    let lease = c.acquire_lease("cv", "alice", ttl()).await.unwrap();
    c.release(lease).await.unwrap();
    assert_eq!(c.find_owner("cv").await.unwrap(), None);
    c.acquire_lease("cv", "bob", ttl()).await.unwrap();
    assert_eq!(c.find_owner("cv").await.unwrap(), Some("bob".to_string()));
}

#[tokio::test]
async fn file_lease_visible_across_two_handles() {
    let clock = FakeClock::new();
    let dir = tempfile::tempdir().unwrap();
    let c1 = FileCoordinator::open_with_clock(dir.path(), clock.clone()).unwrap();
    let c2 = FileCoordinator::open_with_clock(dir.path(), clock.clone()).unwrap();
    c1.acquire_lease("cv", "alice", ttl()).await.unwrap();
    assert!(c2.acquire_lease("cv", "bob", ttl()).await.is_err());
    assert_eq!(c2.find_owner("cv").await.unwrap(), Some("alice".to_string()));
}

#[tokio::test]
async fn file_presence_put_get_with_expiry() {
    let clock = FakeClock::new();
    let (c, _d) = file_coord(clock.clone());
    c.presence_put("cv", "alice", b"cursor:1".to_vec(), ttl())
        .await
        .unwrap();
    c.presence_put("cv", "bob", b"cursor:2".to_vec(), Duration::from_secs(5))
        .await
        .unwrap();
    let mut got = c.presence_get("cv").await.unwrap();
    got.sort();
    assert_eq!(got.len(), 2);

    clock.advance(Duration::from_secs(6));
    let got = c.presence_get("cv").await.unwrap();
    assert_eq!(got, vec![("alice".to_string(), b"cursor:1".to_vec())]);
}

#[tokio::test]
async fn file_pubsub_delivers_in_process() {
    let (c, _d) = file_coord(FakeClock::new());
    let mut rx = c.subscribe("cv");
    c.publish("cv", b"hello".to_vec()).await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("pubsub frame should arrive within poll interval")
        .unwrap();
    assert_eq!(got, b"hello");
}

#[tokio::test]
async fn file_distinct_ids_sharing_a_stem_do_not_collide() {
    for (id_a, id_b) in [("canvas-1", "canvas_1"), ("a/b", "a-b")] {
        let (c, _d) = file_coord(FakeClock::new());

        c.acquire_lease(id_a, "alice", ttl()).await.unwrap();
        c.acquire_lease(id_b, "bob", ttl())
            .await
            .expect("distinct canvas must not contend for the same lease");
        assert_eq!(c.find_owner(id_a).await.unwrap(), Some("alice".to_string()));
        assert_eq!(c.find_owner(id_b).await.unwrap(), Some("bob".to_string()));

        let mut rx_b = c.subscribe(id_b);
        c.publish(id_a, b"to-a".to_vec()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(300), rx_b.recv())
                .await
                .is_err(),
            "{id_b} must not receive a frame published to {id_a}"
        );
        c.publish(id_b, b"to-b".to_vec()).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(2), rx_b.recv())
            .await
            .expect("id_b's own frame should arrive")
            .unwrap();
        assert_eq!(got, b"to-b");
    }
}
