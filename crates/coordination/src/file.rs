//! File-backed [`Coordinator`] for single-host multi-process dev.
//!
//! Lease ops take an exclusive `fs2` advisory lock on a per-canvas `.lock`
//! sidecar, then read/apply/write the lease JSON under the lock, so racing
//! processes serialize and exactly one wins. Presence is guarded the same way.
//! Pub/sub is best-effort: `publish` appends a length-prefixed frame to a
//! per-canvas log; a poll task tails new frames into a local `broadcast`
//! channel. Cross-process delivery is bounded by the poll interval and is not
//! crash-guaranteed — use the in-memory coordinator when pub/sub must be reliable.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::inmem::{Clock, SystemClock};
use crate::{Coordinator, Lease, Result};

#[derive(Default, Serialize, Deserialize)]
struct LeaseFile {
    owner: String,
    token: String,
    expires_at: u64,
}

#[derive(Serialize, Deserialize)]
struct PresenceRecord {
    val: Vec<u8>,
    expires_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct PresenceFile {
    entries: HashMap<String, PresenceRecord>,
}

pub struct FileCoordinator {
    dir: PathBuf,
    counter: AtomicU64,
    clock: Arc<dyn Clock>,
    channel_capacity: usize,
    poll_interval: Duration,
    channels: StdMutex<HashMap<String, broadcast::Sender<Vec<u8>>>>,
}

impl FileCoordinator {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_clock(dir, Arc::new(SystemClock))
    }

    pub fn open_with_clock(dir: impl AsRef<Path>, clock: Arc<dyn Clock>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            counter: AtomicU64::new(0),
            clock,
            channel_capacity: 256,
            poll_interval: Duration::from_millis(50),
            channels: StdMutex::new(HashMap::new()),
        })
    }

    fn next_token(&self, owner: &str) -> String {
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        format!("{owner}-{n}")
    }

    /// Filesystem-safe, collision-free stem: the sanitized prefix is lossy, so a
    /// `-{hex}` suffix from a deterministic FNV-1a hash of the full raw id keeps
    /// distinct ids (e.g. `canvas-1` vs `canvas_1`) on separate files.
    fn stem(canvas_id: &str) -> String {
        let prefix: String = canvas_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!("{prefix}-{:016x}", fnv1a64(canvas_id))
    }

    fn lock_path(&self, canvas_id: &str) -> PathBuf {
        self.dir.join(format!("{}.lock", Self::stem(canvas_id)))
    }
    fn lease_path(&self, canvas_id: &str) -> PathBuf {
        self.dir.join(format!("{}.lease.json", Self::stem(canvas_id)))
    }
    fn presence_path(&self, canvas_id: &str) -> PathBuf {
        self.dir
            .join(format!("{}.presence.json", Self::stem(canvas_id)))
    }
    fn log_path(&self, canvas_id: &str) -> PathBuf {
        self.dir.join(format!("{}.log", Self::stem(canvas_id)))
    }

    fn with_lock<T>(&self, canvas_id: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.lock_path(canvas_id))?;
        lock.lock_exclusive()?;
        let out = f();
        let _ = FileExt::unlock(&lock);
        out
    }

    fn read_lease(&self, canvas_id: &str) -> Result<Option<LeaseFile>> {
        let path = self.lease_path(canvas_id);
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => Ok(Some(serde_json::from_slice(&bytes)?)),
            Ok(_) => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn write_lease(&self, canvas_id: &str, lease: &LeaseFile) -> Result<()> {
        let bytes = serde_json::to_vec(lease)?;
        std::fs::write(self.lease_path(canvas_id), bytes)?;
        Ok(())
    }

    fn read_presence(&self, canvas_id: &str) -> Result<PresenceFile> {
        let path = self.presence_path(canvas_id);
        match std::fs::read(&path) {
            Ok(bytes) if !bytes.is_empty() => Ok(serde_json::from_slice(&bytes)?),
            _ => Ok(PresenceFile::default()),
        }
    }

    fn write_presence(&self, canvas_id: &str, presence: &PresenceFile) -> Result<()> {
        let bytes = serde_json::to_vec(presence)?;
        std::fs::write(self.presence_path(canvas_id), bytes)?;
        Ok(())
    }

    fn sender(&self, canvas_id: &str) -> broadcast::Sender<Vec<u8>> {
        let mut chans = self.channels.lock().expect("channels mutex poisoned");
        if let Some(tx) = chans.get(canvas_id) {
            return tx.clone();
        }
        let (tx, _rx) = broadcast::channel(self.channel_capacity);
        chans.insert(canvas_id.to_string(), tx.clone());
        drop(chans);
        self.spawn_tail(canvas_id.to_string(), tx.clone());
        tx
    }

    fn spawn_tail(&self, canvas_id: String, tx: broadcast::Sender<Vec<u8>>) {
        let log_path = self.log_path(&canvas_id);
        let interval = self.poll_interval;
        // Capture the start offset synchronously, before spawning: the task body
        // is first polled later and could otherwise skip a frame already appended.
        let mut offset: u64 = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
        tokio::spawn(async move {
            loop {
                if let Ok(mut f) = File::open(&log_path) {
                    let len = f.metadata().map(|m| m.len()).unwrap_or(offset);
                    if len > offset {
                        let _ = f.seek(SeekFrom::Start(offset));
                        let mut buf = Vec::new();
                        if f.read_to_end(&mut buf).is_ok() {
                            offset += buf.len() as u64;
                            for frame in decode_frames(&buf) {
                                let _ = tx.send(frame);
                            }
                        }
                    } else if len < offset {
                        offset = len;
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
    }
}

/// Deterministic FNV-1a 64-bit hash for disambiguating canvas-id file stems.
fn fnv1a64(s: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Append a length-prefixed (u32 LE) frame to the log file.
fn append_frame(log_path: &Path, msg: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let len = u32::try_from(msg.len())?;
    f.write_all(&len.to_le_bytes())?;
    f.write_all(msg)?;
    f.flush()?;
    Ok(())
}

/// Decode whole length-prefixed frames; a trailing partial frame (torn append
/// caught mid-write) is dropped and re-read from the same offset on the next poll.
fn decode_frames(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= buf.len() {
        let len = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]) as usize;
        if i + 4 + len > buf.len() {
            break;
        }
        out.push(buf[i + 4..i + 4 + len].to_vec());
        i += 4 + len;
    }
    out
}

#[async_trait]
impl Coordinator for FileCoordinator {
    async fn acquire_lease(&self, canvas_id: &str, owner: &str, ttl: Duration) -> Result<Lease> {
        let now = self.clock.now_ms();
        let token = self.next_token(owner);
        let expires_at = now + crate::millis_u64(ttl);
        self.with_lock(canvas_id, || {
            if let Some(existing) = self.read_lease(canvas_id)? {
                if existing.expires_at > now && existing.owner != owner {
                    anyhow::bail!(
                        "canvas '{canvas_id}' is leased by '{}' until {}",
                        existing.owner,
                        existing.expires_at
                    );
                }
            }
            self.write_lease(
                canvas_id,
                &LeaseFile {
                    owner: owner.to_string(),
                    token: token.clone(),
                    expires_at,
                },
            )?;
            Ok(())
        })?;
        Ok(Lease {
            canvas_id: canvas_id.to_string(),
            owner: owner.to_string(),
            token,
            expires_at,
        })
    }

    async fn renew(&self, lease: &Lease, ttl: Duration) -> Result<()> {
        let now = self.clock.now_ms();
        let new_expiry = now + crate::millis_u64(ttl);
        self.with_lock(&lease.canvas_id, || {
            match self.read_lease(&lease.canvas_id)? {
                Some(existing)
                    if existing.token == lease.token
                        && existing.owner == lease.owner
                        && existing.expires_at > now =>
                {
                    self.write_lease(
                        &lease.canvas_id,
                        &LeaseFile {
                            owner: lease.owner.clone(),
                            token: lease.token.clone(),
                            expires_at: new_expiry,
                        },
                    )
                }
                _ => anyhow::bail!(
                    "lease for '{}' (owner '{}') is no longer held",
                    lease.canvas_id,
                    lease.owner
                ),
            }
        })
    }

    async fn release(&self, lease: Lease) -> Result<()> {
        self.with_lock(&lease.canvas_id, || {
            if let Some(existing) = self.read_lease(&lease.canvas_id)? {
                if existing.token == lease.token && existing.owner == lease.owner {
                    let _ = std::fs::remove_file(self.lease_path(&lease.canvas_id));
                }
            }
            Ok(())
        })
    }

    async fn find_owner(&self, canvas_id: &str) -> Result<Option<String>> {
        let now = self.clock.now_ms();
        self.with_lock(canvas_id, || {
            Ok(self
                .read_lease(canvas_id)?
                .filter(|e| e.expires_at > now)
                .map(|e| e.owner))
        })
    }

    async fn publish(&self, canvas_id: &str, msg: Vec<u8>) -> Result<()> {
        // Ensure the local tail task exists so in-process subscribers see it.
        let _ = self.sender(canvas_id);
        append_frame(&self.log_path(canvas_id), &msg)
    }

    fn subscribe(&self, canvas_id: &str) -> broadcast::Receiver<Vec<u8>> {
        self.sender(canvas_id).subscribe()
    }

    async fn presence_put(
        &self,
        canvas_id: &str,
        key: &str,
        val: Vec<u8>,
        ttl: Duration,
    ) -> Result<()> {
        let now = self.clock.now_ms();
        let expires_at = now + crate::millis_u64(ttl);
        self.with_lock(canvas_id, || {
            let mut presence = self.read_presence(canvas_id)?;
            presence
                .entries
                .insert(key.to_string(), PresenceRecord { val, expires_at });
            self.write_presence(canvas_id, &presence)
        })
    }

    async fn presence_get(&self, canvas_id: &str) -> Result<Vec<(String, Vec<u8>)>> {
        let now = self.clock.now_ms();
        self.with_lock(canvas_id, || {
            let mut presence = self.read_presence(canvas_id)?;
            presence.entries.retain(|_, r| r.expires_at > now);
            let out = presence
                .entries
                .iter()
                .map(|(k, r)| (k.clone(), r.val.clone()))
                .collect::<Vec<_>>();
            self.write_presence(canvas_id, &presence)?;
            Ok(out)
        })
    }
}
