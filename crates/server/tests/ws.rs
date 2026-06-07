//! MG-3 WebSocket transport integration tests.
//!
//! These bind the full router (`build_router_with_mcp`) on an ephemeral port and
//! drive it with a real WebSocket client (`tokio-tungstenite`) over the loopback
//! socket, exercising the actual upgrade + frame path rather than an in-process
//! `oneshot`. Each test gets its own in-memory registry, so canvases never leak
//! state between tests.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use shape_server::{build_router_with_mcp, CanvasRegistry, ClientRegistry, Config};
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

fn test_config() -> Config {
    Config {
        host: "127.0.0.1".to_string(),
        port: 0,
        client_dir: std::env::temp_dir().join("shape_server_ws_test_no_client_dir"),
    }
}

/// Bind the full router on an ephemeral port and serve it on a background task.
/// Returns the bound address so the test can connect a WS client.
async fn spawn_server() -> SocketAddr {
    let canvases = CanvasRegistry::open_in_memory().unwrap();
    let clients = ClientRegistry::new();
    let router = build_router_with_mcp(&test_config(), canvases, clients);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Open a WS client to `/ws` on `addr`.
async fn connect(addr: SocketAddr) -> WsStream {
    let url = format!("ws://{addr}/ws");
    let (ws, _resp) = connect_async(url).await.expect("ws upgrade");
    ws
}

/// Send one JSON value as a text frame.
async fn send_json(ws: &mut WsStream, v: Value) {
    ws.send(Message::Text(v.to_string())).await.unwrap();
}

/// Receive the next text frame and parse it as JSON, skipping any control frames.
async fn recv_json(ws: &mut WsStream) -> Value {
    loop {
        match ws.next().await.expect("socket open").expect("frame ok") {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected non-text frame: {other:?}"),
        }
    }
}

/// Receive frames until one of the given `type` is seen, returning it. Since
/// MG-6.1 the server no longer echoes a connection's own applied ops back as a
/// `patch` (author self-skip), but a connection can still see peers' patches
/// interleaved with its own `ack`/`welcome`; this drains past those.
async fn recv_json_of(ws: &mut WsStream, ty: &str) -> Value {
    loop {
        let v = recv_json(ws).await;
        if v["type"] == ty {
            return v;
        }
    }
}

fn create_group(id: &str) -> Value {
    json!({
        "kind": "create-group",
        "group": {
            "id": id,
            "title": "G",
            "bounds": { "x": 0.0, "y": 0.0, "width": 400.0, "height": 300.0 }
        }
    })
}

fn create_card(id: &str, group_id: &str) -> Value {
    json!({
        "kind": "create-card",
        "card": {
            "id": id,
            "groupId": group_id,
            "title": "C",
            "bounds": { "x": 10.0, "y": 10.0, "width": 120.0, "height": 80.0 }
        }
    })
}

fn create_group_at(id: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({
        "kind": "create-group",
        "group": {
            "id": id,
            "title": "G",
            "bounds": { "x": x, "y": y, "width": w, "height": h }
        }
    })
}

fn create_card_at(id: &str, group_id: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({
        "kind": "create-card",
        "card": {
            "id": id,
            "groupId": group_id,
            "title": "C",
            "bounds": { "x": x, "y": y, "width": w, "height": h }
        }
    })
}

/// A region/window message body: a `region` with a `canvasId` + optional `bbox`.
fn region(canvas_id: &str, bbox: Option<(f64, f64, f64, f64)>) -> Value {
    match bbox {
        None => json!({ "canvasId": canvas_id }),
        Some((x, y, w, h)) => json!({
            "canvasId": canvas_id,
            "bbox": { "x": x, "y": y, "width": w, "height": h }
        }),
    }
}

/// Wrap a render patch in an MG-4 op envelope with the given opId coordinates.
fn envelope(client_id: &str, local_seq: i64, base_revision: i64, patch: Value) -> Value {
    json!({
        "opId": { "clientId": client_id, "localSeq": local_seq },
        "baseRevision": base_revision,
        "ts": "1970-01-01T00:00:00Z",
        "patch": patch,
    })
}

#[tokio::test]
async fn hello_yields_welcome_then_ops_ack_with_increasing_seq() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    // hello -> welcome{ scene, seq, revision }.
    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-ws", "lastAckSeq": 0 })).await;
    let welcome = recv_json(&mut ws).await;
    assert_eq!(welcome["type"], "welcome");
    assert!(welcome["scene"].is_object(), "welcome carries a scene");
    assert_eq!(welcome["scene"]["groups"].as_array().unwrap().len(), 0);
    assert_eq!(welcome["revision"], json!(0));

    // ops{ create-group } -> ack{ opIds, seq: 1 }.
    send_json(
        &mut ws,
        json!({
            "type": "ops",
            "ops": [envelope("user-1", 1, 0, create_group("g1"))],
        }),
    )
    .await;
    let ack1 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack1["seq"], json!(1), "first apply is seq 1");
    assert_eq!(
        ack1["opIds"][0],
        json!({ "clientId": "user-1", "localSeq": 1 }),
        "ack echoes the op's id"
    );

    // ops{ create-card } -> ack{ seq: 2 } (increasing).
    send_json(
        &mut ws,
        json!({
            "type": "ops",
            "ops": [envelope("user-1", 2, 1, create_card("n1", "g1"))],
        }),
    )
    .await;
    let ack2 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack2["seq"], json!(2), "second apply is seq 2, increasing");
}

#[tokio::test]
async fn duplicate_op_id_is_idempotent_reacks_same_seq() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-dedup", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws).await["type"], "welcome");

    // First send of (user-1, 1) applies and acks seq 1.
    let dup = envelope("user-1", 1, 0, create_group("g1"));
    send_json(&mut ws, json!({ "type": "ops", "ops": [dup.clone()] })).await;
    let ack1 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack1["seq"], json!(1));

    // Re-sending the SAME opId is idempotent: re-acks seq 1, does NOT bump to 2.
    send_json(&mut ws, json!({ "type": "ops", "ops": [dup] })).await;
    let ack_dup = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack_dup["seq"], json!(1), "duplicate op re-acks the original seq");

    // A genuinely new op advances to seq 2, proving the duplicate did not.
    send_json(
        &mut ws,
        json!({ "type": "ops", "ops": [envelope("user-1", 2, 1, create_card("n1", "g1"))] }),
    )
    .await;
    let ack2 = recv_json_of(&mut ws, "ack").await;
    assert_eq!(ack2["seq"], json!(2), "new op is seq 2 (duplicate did not bump)");

    // The scene has exactly one group + one card (the duplicate did not mutate).
    send_json(&mut ws, json!({ "type": "resume", "canvasId": "c-dedup", "lastAckSeq": 0 })).await;
    let welcome = recv_json_of(&mut ws, "welcome").await;
    assert_eq!(welcome["scene"]["groups"].as_array().unwrap().len(), 1);
    assert_eq!(welcome["scene"]["nodes"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn rejected_ops_yield_rejected_message() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    send_json(&mut ws, json!({ "type": "hello", "canvasId": "c-reject", "lastAckSeq": 0 })).await;
    let welcome = recv_json(&mut ws).await;
    assert_eq!(welcome["type"], "welcome");

    // create-card against a missing group is rejected by scene-core.
    send_json(
        &mut ws,
        json!({
            "type": "ops",
            "ops": [envelope("user-1", 1, 0, create_card("n1", "missing-group"))],
        }),
    )
    .await;
    let reply = recv_json_of(&mut ws, "rejected").await;
    assert!(
        !reply["errors"].as_array().unwrap().is_empty(),
        "rejection carries errors"
    );
    assert_eq!(
        reply["opIds"][0],
        json!({ "clientId": "user-1", "localSeq": 1 }),
        "rejection echoes the op's id so the client can fail the outbox entry"
    );
}

#[tokio::test]
async fn second_client_receives_patch_broadcast_from_first() {
    let addr = spawn_server().await;

    // Client 1 connects and applies an op.
    let mut ws1 = connect(addr).await;
    send_json(&mut ws1, json!({ "type": "hello", "canvasId": "c-multi", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws1).await["type"], "welcome");

    // Client 2 connects to the SAME canvas and subscribes.
    let mut ws2 = connect(addr).await;
    send_json(&mut ws2, json!({ "type": "hello", "canvasId": "c-multi", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws2).await["type"], "welcome");

    // Client 1 applies a create-group; it should ack on ws1...
    send_json(
        &mut ws1,
        json!({
            "type": "ops",
            "ops": [envelope("user-1", 1, 0, create_group("g1"))],
        }),
    )
    .await;
    let ack = recv_json_of(&mut ws1, "ack").await;
    assert_eq!(ack["seq"], json!(1));

    // ...and fan out to client 2 as a patch.
    let patch = recv_json_of(&mut ws2, "patch").await;
    assert_eq!(patch["seq"], json!(1), "patch carries the applied seq");
    assert_eq!(patch["ops"][0]["kind"], json!("create-group"));
    assert_eq!(patch["ops"][0]["group"]["id"], json!("g1"));
}

#[tokio::test]
async fn presence_fans_out_to_other_client_best_effort() {
    let addr = spawn_server().await;

    let mut ws1 = connect(addr).await;
    send_json(&mut ws1, json!({ "type": "hello", "canvasId": "c-presence", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws1).await["type"], "welcome");

    let mut ws2 = connect(addr).await;
    send_json(&mut ws2, json!({ "type": "hello", "canvasId": "c-presence", "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut ws2).await["type"], "welcome");

    // Client 1 publishes presence; client 2 receives it on the ephemeral channel.
    send_json(
        &mut ws1,
        json!({
            "type": "presence",
            "canvasId": "c-presence",
            "payload": { "cursor": { "x": 12.0, "y": 34.0 }, "userId": "user-1" },
        }),
    )
    .await;

    let frame = recv_json(&mut ws2).await;
    assert_eq!(frame["type"], "presence", "got {frame}");
    assert_eq!(frame["payload"]["userId"], json!("user-1"));
    assert_eq!(frame["payload"]["cursor"]["x"], json!(12.0));

    // MG-6.2: the sender (client 1) does NOT receive its own presence back.
    assert!(
        try_recv_json_of(&mut ws1, "presence", 300).await.is_none(),
        "a connection does not receive its own presence frame"
    );
}

#[tokio::test]
async fn non_hello_first_message_errors() {
    let addr = spawn_server().await;
    let mut ws = connect(addr).await;

    // Sending ops before hello is a protocol error.
    send_json(&mut ws, json!({ "type": "ops", "ops": [] })).await;
    let reply = recv_json(&mut ws).await;
    assert_eq!(reply["type"], "error", "got {reply}");
}

// ---------------------------------------------------------------------------
// MG-9.5: region-scoped subscription / data-layer windowing over the wire.
// ---------------------------------------------------------------------------

use std::time::Duration;

/// Try to receive a frame of `ty` within `ms`; `None` if none arrives. Used to
/// assert that an out-of-region patch is NOT delivered (a negative over the wire
/// needs a bounded wait, not a blocking recv).
async fn try_recv_json_of(ws: &mut WsStream, ty: &str, ms: u64) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                if v["type"] == ty {
                    return Some(v);
                }
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(_))) | Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => return None, // timed out
        }
    }
}

/// Region-windowed clients: seed two far-apart clusters, then verify (1) a
/// region-A subscriber's welcome contains only A's objects, (2) an op applied in
/// region B is delivered to a region-B subscriber but NOT to the region-A
/// subscriber, (3) re-subscribing the A client to region B delivers B's snapshot,
/// and (4) a whole-canvas (None) subscriber sees everything.
#[tokio::test]
async fn region_windowing_filters_welcome_and_fanout() {
    let addr = spawn_server().await;
    let canvas = "c-windowing";

    // Seeder client (whole canvas): build region A near origin, region B far away.
    let mut seed = connect(addr).await;
    send_json(&mut seed, json!({ "type": "hello", "canvasId": canvas, "lastAckSeq": 0 })).await;
    assert_eq!(recv_json(&mut seed).await["type"], "welcome");

    let seed_ops = [
        create_group_at("gA", 0.0, 0.0, 400.0, 300.0),
        create_card_at("nA1", "gA", 10.0, 10.0, 120.0, 80.0),
        create_group_at("gB", 100_000.0, 100_000.0, 400.0, 300.0),
        create_card_at("nB1", "gB", 100_010.0, 100_010.0, 120.0, 80.0),
    ];
    for (i, op) in seed_ops.iter().enumerate() {
        let ls = (i + 1) as i64;
        send_json(
            &mut seed,
            json!({ "type": "ops", "ops": [envelope("seed", ls, ls - 1, op.clone())] }),
        )
        .await;
        let ack = recv_json_of(&mut seed, "ack").await;
        assert_eq!(ack["seq"], json!(ls), "seed op {ls} acked");
    }

    let win_a = Some((-50.0, -50.0, 600.0, 500.0));
    let win_b = Some((99_900.0, 99_900.0, 700.0, 600.0));

    // (1) Region-A subscriber: welcome holds only A's group + card.
    let mut a = connect(addr).await;
    send_json(
        &mut a,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, win_a), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_a = recv_json_of(&mut a, "welcome").await;
    let a_groups = welcome_a["scene"]["groups"].as_array().unwrap();
    let a_nodes = welcome_a["scene"]["nodes"].as_array().unwrap();
    assert_eq!(a_groups.len(), 1, "region-A welcome has one group");
    assert_eq!(a_groups[0]["id"], json!("gA"), "region-A welcome group is gA");
    assert_eq!(a_nodes.len(), 1, "region-A welcome has one card");
    assert_eq!(a_nodes[0]["id"], json!("nA1"), "region-A welcome card is nA1");

    // Region-B subscriber: welcome holds only B's group + card.
    let mut b = connect(addr).await;
    send_json(
        &mut b,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, win_b), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_b = recv_json_of(&mut b, "welcome").await;
    assert_eq!(welcome_b["scene"]["groups"][0]["id"], json!("gB"), "region-B welcome group is gB");
    assert_eq!(welcome_b["scene"]["nodes"][0]["id"], json!("nB1"), "region-B welcome card is nB1");

    // (2) Seeder applies an op in region B (a new card inside gB).
    send_json(
        &mut seed,
        json!({
            "type": "ops",
            "ops": [envelope("seed", 5, 4, create_card_at("nB2", "gB", 100_200.0, 100_010.0, 120.0, 80.0))],
        }),
    )
    .await;
    assert_eq!(recv_json_of(&mut seed, "ack").await["seq"], json!(5), "region-B op acked");

    // ...delivered to the region-B subscriber.
    let patch_b = try_recv_json_of(&mut b, "patch", 1000)
        .await
        .expect("region-B subscriber receives the in-region patch");
    assert_eq!(patch_b["ops"][0]["card"]["id"], json!("nB2"), "B sees the region-B card");

    // ...but NOT to the region-A subscriber.
    assert!(
        try_recv_json_of(&mut a, "patch", 300).await.is_none(),
        "region-A subscriber does not receive the region-B op"
    );

    // (3) Re-subscribe the A client to region B: it gets a region-B snapshot now.
    send_json(
        &mut a,
        json!({ "type": "subscribe", "canvasId": canvas, "region": region(canvas, win_b).clone() }),
    )
    .await;
    let resnap = recv_json_of(&mut a, "welcome").await;
    let groups: Vec<&str> = resnap["scene"]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["id"].as_str().unwrap())
        .collect();
    let nodes: Vec<&str> = resnap["scene"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(groups, vec!["gB"], "after re-subscribe, A's snapshot is region B's group");
    let mut nodes_sorted = nodes.clone();
    nodes_sorted.sort();
    assert_eq!(nodes_sorted, vec!["nB1", "nB2"], "after re-subscribe, A's snapshot has B's cards");

    // (4) Whole-canvas (None bbox) subscriber sees everything.
    let mut all = connect(addr).await;
    send_json(
        &mut all,
        json!({ "type": "hello", "canvasId": canvas, "region": region(canvas, None), "lastAckSeq": 0 }),
    )
    .await;
    let welcome_all = recv_json_of(&mut all, "welcome").await;
    assert_eq!(welcome_all["scene"]["groups"].as_array().unwrap().len(), 2, "None sees both groups");
    assert_eq!(welcome_all["scene"]["nodes"].as_array().unwrap().len(), 3, "None sees all three cards");
}

// ---------------------------------------------------------------------------
// MG-6.1: author self-skip — the originator is not echoed its own applied op;
// other connections still receive it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn author_does_not_receive_own_op_but_peer_does() {
    let addr = spawn_server().await;
    let canvas = "c-selfskip";

    // Author connection: identifies as user-A via hello `userId`.
    let mut author = connect(addr).await;
    send_json(
        &mut author,
        json!({ "type": "hello", "canvasId": canvas, "userId": "user-A", "lastAckSeq": 0 }),
    )
    .await;
    assert_eq!(recv_json(&mut author).await["type"], "welcome");

    // Peer connection (a different user) on the same canvas.
    let mut peer = connect(addr).await;
    send_json(
        &mut peer,
        json!({ "type": "hello", "canvasId": canvas, "userId": "user-B", "lastAckSeq": 0 }),
    )
    .await;
    assert_eq!(recv_json(&mut peer).await["type"], "welcome");

    // Author applies an op authored by user-A; it must ack on the author socket.
    send_json(
        &mut author,
        json!({ "type": "ops", "ops": [envelope("user-A", 1, 0, create_group("g1"))] }),
    )
    .await;
    let ack = recv_json_of(&mut author, "ack").await;
    assert_eq!(ack["seq"], json!(1), "author gets the ack");

    // The author does NOT receive its own op echoed back as a patch.
    assert!(
        try_recv_json_of(&mut author, "patch", 400).await.is_none(),
        "originator is not echoed its own applied op"
    );

    // The peer DOES receive the op as a patch.
    let patch = try_recv_json_of(&mut peer, "patch", 1000)
        .await
        .expect("peer receives the author's op");
    assert_eq!(patch["seq"], json!(1), "peer's patch carries the applied seq");
    assert_eq!(patch["ops"][0]["group"]["id"], json!("g1"), "peer sees the created group");
}
