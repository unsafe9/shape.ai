//! The single portable on-disk export format.
//!
//! Every adapter exports to — and imports from — exactly this format, so a
//! store dumped from one backend (memory, file, and eventually
//! sqlite/postgres/s3/remote) can be loaded into any other.
//!
//! # Why a sharded directory rather than one file
//!
//! `export` is meant to run *intensively and in parallel*. The format is a
//! directory (a "bundle") shaped so each shard is an independent unit:
//!
//! ```text
//! bundle.shapestore/
//!   manifest.json        # bundle metadata + per-shard index
//!   shard-00000.bin      # self-contained, framed record stream
//!   shard-00001.bin
//!   ...
//! ```
//!
//! Records are assigned to shards by a stable hash of their id, so:
//! * shards are written concurrently (one rayon task per shard),
//! * shards are read + parsed concurrently on import,
//! * the same logical store always produces the same shard layout and the
//!   same bytes (stability), which the manifest pins with a per-shard CRC.
//!
//! # Shard binary layout
//!
//! ```text
//! magic   : b"SHPSHARD"           (8 bytes)
//! version : u16 LE                (= FORMAT_VERSION)
//! count   : u32 LE                (records in this shard)
//! repeat count times:
//!   id_len      : u32 LE   id_bytes     (utf-8)
//!   kind_len    : u32 LE   kind_bytes   (utf-8)
//!   version     : u64 LE
//!   payload_len : u32 LE   payload_bytes
//! ```
//!
//! Records within a shard are written in id-sorted order, giving byte-stable
//! shards.

use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// On-disk format version. Bump on any incompatible framing change.
pub const FORMAT_VERSION: u16 = 1;

const SHARD_MAGIC: &[u8; 8] = b"SHPSHARD";

/// Default number of shards when the caller does not pick one.
pub const DEFAULT_SHARD_COUNT: u32 = 8;

/// Per-shard entry in the manifest: lets import discover and verify shards
/// without scanning the directory.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShardEntry {
    /// Shard index (0-based). File name is `shard-{index:05}.bin`.
    pub index: u32,
    /// Records contained in this shard.
    pub records: u32,
    /// CRC-32 of the shard file's full bytes; pins stability/integrity.
    pub crc32: u32,
}

/// Bundle manifest written as `manifest.json` at the bundle root.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    /// Format version this bundle was written with.
    pub format_version: u16,
    /// Total records across all shards.
    pub total_records: u64,
    /// Number of shard files.
    pub shard_count: u32,
    /// Per-shard index, ordered by shard index.
    pub shards: Vec<ShardEntry>,
}

/// File name a bundle uses for its manifest.
pub const MANIFEST_NAME: &str = "manifest.json";

fn shard_file_name(index: u32) -> String {
    format!("shard-{index:05}.bin")
}

/// Stable FNV-1a hash of an id, used to assign records to shards. A fixed,
/// inlined hash (rather than the std `Hasher`, whose output is not guaranteed
/// stable across builds) keeps shard assignment reproducible.
fn shard_of(id: &str, shard_count: u32) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    (hash % shard_count as u64) as u32
}

/// CRC-32 (IEEE) over a byte slice — dependency-free, used for shard integrity.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn put_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// A bounds-checked little-endian reader over a shard's bytes.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| StorageError::Format("length overflow".into()))?;
        if end > self.data.len() {
            return Err(StorageError::Format("unexpected end of shard".into()));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self, n: usize) -> Result<String> {
        let bytes = self.take(n)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|e| StorageError::Format(format!("invalid utf-8: {e}")))
    }
}

/// Encode one shard's records (already id-sorted) into framed bytes.
fn encode_shard(records: &[&Record]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(SHARD_MAGIC);
    put_u16(&mut buf, FORMAT_VERSION);
    put_u32(&mut buf, records.len() as u32);
    for r in records {
        let id = r.id.as_bytes();
        let kind = r.kind.as_bytes();
        put_u32(&mut buf, id.len() as u32);
        buf.extend_from_slice(id);
        put_u32(&mut buf, kind.len() as u32);
        buf.extend_from_slice(kind);
        put_u64(&mut buf, r.version);
        put_u32(&mut buf, r.payload.len() as u32);
        buf.extend_from_slice(&r.payload);
    }
    buf
}

/// Decode one shard's framed bytes back into records.
fn decode_shard(bytes: &[u8]) -> Result<Vec<Record>> {
    let mut cur = Cursor::new(bytes);
    let magic = cur.take(8)?;
    if magic != SHARD_MAGIC {
        return Err(StorageError::Format("bad shard magic".into()));
    }
    let version = cur.u16()?;
    if version != FORMAT_VERSION {
        return Err(StorageError::Format(format!(
            "unsupported shard version {version} (expected {FORMAT_VERSION})"
        )));
    }
    let count = cur.u32()? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id_len = cur.u32()? as usize;
        let id = cur.string(id_len)?;
        let kind_len = cur.u32()? as usize;
        let kind = cur.string(kind_len)?;
        let version = cur.u64()?;
        let payload_len = cur.u32()? as usize;
        let payload = cur.take(payload_len)?.to_vec();
        out.push(Record {
            id,
            kind,
            version,
            payload,
        });
    }
    if cur.pos != bytes.len() {
        return Err(StorageError::Format("trailing bytes in shard".into()));
    }
    Ok(out)
}

/// Write `snapshot` into a portable bundle directory at `root`, fanning the
/// shard writes out across rayon worker threads.
///
/// `shard_count` must be >= 1; pass [`DEFAULT_SHARD_COUNT`] for the default.
/// The directory is created if missing and any stale `shard-*.bin` files are
/// removed first so re-exports are clean.
pub fn export_bundle(snapshot: &StoreSnapshot, root: &Path, shard_count: u32) -> Result<Manifest> {
    let shard_count = shard_count.max(1);
    fs::create_dir_all(root)?;
    clear_stale_shards(root)?;

    // Partition records into shards by stable id hash; sort each for stability.
    let mut buckets: Vec<Vec<&Record>> = vec![Vec::new(); shard_count as usize];
    for record in snapshot.records() {
        let idx = shard_of(&record.id, shard_count) as usize;
        buckets[idx].push(record);
    }
    for bucket in &mut buckets {
        bucket.sort_by(|a, b| a.id.cmp(&b.id));
    }

    // Encode + write each shard in parallel. Each task is independent: it owns
    // one file, so there is no cross-task contention.
    let entries: Vec<Result<ShardEntry>> = buckets
        .par_iter()
        .enumerate()
        .map(|(index, bucket)| {
            let bytes = encode_shard(bucket);
            let crc = crc32(&bytes);
            let path = root.join(shard_file_name(index as u32));
            fs::write(&path, &bytes)?;
            Ok(ShardEntry {
                index: index as u32,
                records: bucket.len() as u32,
                crc32: crc,
            })
        })
        .collect();

    let mut shards = Vec::with_capacity(entries.len());
    for e in entries {
        shards.push(e?);
    }
    shards.sort_by_key(|s| s.index);

    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        total_records: snapshot.len() as u64,
        shard_count,
        shards,
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(root.join(MANIFEST_NAME), json)?;
    Ok(manifest)
}

/// Read a portable bundle directory at `root` back into a [`StoreSnapshot`],
/// reading + decoding shards in parallel and verifying each shard CRC.
pub fn import_bundle(root: &Path) -> Result<StoreSnapshot> {
    let manifest_path = root.join(MANIFEST_NAME);
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|e| StorageError::Io(format!("reading {}: {e}", manifest_path.display())))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.format_version != FORMAT_VERSION {
        return Err(StorageError::Format(format!(
            "unsupported bundle version {} (expected {FORMAT_VERSION})",
            manifest.format_version
        )));
    }

    // Read + verify + decode each shard concurrently.
    let shard_results: Vec<Result<Vec<Record>>> = manifest
        .shards
        .par_iter()
        .map(|entry| {
            let path = root.join(shard_file_name(entry.index));
            let bytes = fs::read(&path)
                .map_err(|e| StorageError::Io(format!("reading {}: {e}", path.display())))?;
            if crc32(&bytes) != entry.crc32 {
                return Err(StorageError::Format(format!(
                    "shard {} crc mismatch (corrupt or tampered)",
                    entry.index
                )));
            }
            let records = decode_shard(&bytes)?;
            if records.len() as u32 != entry.records {
                return Err(StorageError::Format(format!(
                    "shard {} record count mismatch: manifest {} vs decoded {}",
                    entry.index,
                    entry.records,
                    records.len()
                )));
            }
            Ok(records)
        })
        .collect();

    let mut snapshot = StoreSnapshot::new();
    for shard in shard_results {
        for record in shard? {
            snapshot.insert(record);
        }
    }
    if snapshot.len() as u64 != manifest.total_records {
        return Err(StorageError::Format(format!(
            "total record count mismatch: manifest {} vs imported {}",
            manifest.total_records,
            snapshot.len()
        )));
    }
    Ok(snapshot)
}

/// Remove any pre-existing `shard-*.bin` files so a re-export starts clean and
/// stale shards from a larger previous run cannot leak into a smaller one.
fn clear_stale_shards(root: &Path) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path: PathBuf = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("shard-") && name.ends_with(".bin") {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        // CRC-32/IEEE of "123456789" is the canonical 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn shard_encode_decode_roundtrip() {
        let recs = vec![
            Record::new("a", "card", b"alpha".to_vec()),
            Record::new("b", "edge", vec![0u8, 255, 1, 254]),
        ];
        let refs: Vec<&Record> = recs.iter().collect();
        let bytes = encode_shard(&refs);
        let back = decode_shard(&bytes).unwrap();
        assert_eq!(recs, back);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let err = decode_shard(b"NOTSHARDxxxx").unwrap_err();
        assert!(matches!(err, StorageError::Format(_)));
    }

    #[test]
    fn shard_assignment_is_stable() {
        // Same id + shard count must always land on the same shard.
        for id in ["card-1", "edge-42", "group-z", ""] {
            let a = shard_of(id, DEFAULT_SHARD_COUNT);
            let b = shard_of(id, DEFAULT_SHARD_COUNT);
            assert_eq!(a, b);
            assert!(a < DEFAULT_SHARD_COUNT);
        }
    }
}
