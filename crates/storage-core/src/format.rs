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
//!
//! # Memory discipline
//!
//! Export and import are **streaming**: neither holds the whole store in RAM.
//!
//! * [`export_stream`] consumes a lazy, id-sorted record cursor and routes each
//!   record straight to its shard's on-disk body as it arrives — peak buffering
//!   is one record plus small writer buffers, never `O(total)`. The CRC and the
//!   final framed shard files are produced by a bounded, chunked finalize pass
//!   (parallel across shards via rayon).
//! * [`import_stream`] reads one shard at a time, decoding it frame-by-frame
//!   straight into the caller's sink — peak is one record, never `O(total)`.
//!
//! [`export_bundle`] / [`import_bundle`] are thin snapshot-shaped wrappers kept
//! for the in-memory convenience path; they too go through the streaming core.

use crate::error::{Result, StorageError};
use crate::record::{Record, StoreSnapshot};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

/// On-disk format version. Bump on any incompatible framing change.
pub const FORMAT_VERSION: u16 = 1;

const SHARD_MAGIC: &[u8; 8] = b"SHPSHARD";

/// Default number of shards when the caller does not pick one.
pub const DEFAULT_SHARD_COUNT: u32 = 8;

/// Bytes copied per chunk when streaming shard bodies during finalize/import.
/// Bounds the resident buffer to a small fixed size regardless of shard size.
const COPY_CHUNK: usize = 16 * 1024;

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

fn shard_body_tmp_name(index: u32) -> String {
    format!("shard-{index:05}.bin.body")
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
    // The modulus is a u32, so `hash % shard_count` always fits in u32.
    #[allow(clippy::cast_possible_truncation, reason = "modulo a u32 result fits u32")]
    let shard = (hash % u64::from(shard_count)) as u32;
    shard
}

/// Incremental CRC-32 (IEEE), dependency-free. Fed left-to-right so streaming
/// over the final byte order produces the same value as a one-shot CRC of the
/// assembled file.
struct Crc32 {
    state: u32,
}

impl Crc32 {
    fn new() -> Self {
        Crc32 { state: 0xffff_ffff }
    }

    fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.state;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        self.state = crc;
    }

    fn finish(self) -> u32 {
        !self.state
    }

    /// The CRC value so far without consuming the accumulator.
    fn value(&self) -> u32 {
        !self.state
    }
}

/// CRC-32 (IEEE) over a byte slice — convenience for tests.
#[cfg(test)]
fn crc32(bytes: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(bytes);
    c.finish()
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}
fn put_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// A length/count as the format's `u32`. Every field is framed with a u32
/// length; a field at or beyond the 4 GiB cap is a programming error, not a
/// runtime condition, so this asserts the cap rather than silently truncating.
fn field_len(n: usize) -> u32 {
    u32::try_from(n).expect("bundle field length exceeds u32 (4 GiB cap)")
}

/// Append one record's frame to `buf` (no header; body framing only).
fn encode_record_frame(buf: &mut Vec<u8>, r: &Record) {
    let id = r.id.as_bytes();
    let kind = r.kind.as_bytes();
    put_u32(buf, field_len(id.len()));
    buf.extend_from_slice(id);
    put_u32(buf, field_len(kind.len()));
    buf.extend_from_slice(kind);
    put_u64(buf, r.version);
    put_u32(buf, field_len(r.payload.len()));
    buf.extend_from_slice(&r.payload);
}

/// The fixed shard header bytes for a given record count.
fn shard_header(count: u32) -> [u8; 14] {
    let mut h = [0u8; 14];
    h[..8].copy_from_slice(SHARD_MAGIC);
    h[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    h[10..14].copy_from_slice(&count.to_le_bytes());
    h
}

// ---------------------------------------------------------------------------
// Streaming export
// ---------------------------------------------------------------------------

/// Write records yielded by `records` (a lazy, **id-sorted** cursor) into a
/// portable bundle directory at `root`, in bounded memory.
///
/// Memory discipline: each record is routed straight to its shard's on-disk
/// body file as it is produced — at most one record plus small writer buffers
/// is resident, never the whole store. A second, chunked finalize pass prepends
/// each shard's header and computes its CRC by streaming the body back through
/// in `COPY_CHUNK` slices; finalize runs in parallel across shards (rayon), so
/// peak memory is `O(COPY_CHUNK * parallelism)`, independent of total size.
///
/// `shard_count` must be >= 1; pass [`DEFAULT_SHARD_COUNT`] for the default.
/// The directory is created if missing and any stale shard/body files are
/// removed first so re-exports are clean and byte-stable.
pub fn export_stream<I>(records: I, root: &Path, shard_count: u32) -> Result<Manifest>
where
    I: IntoIterator<Item = Result<Record>>,
{
    let shard_count = shard_count.max(1);
    fs::create_dir_all(root)?;
    // NOTE: export is not atomic — stale shards are cleared and new ones written
    // in place, so a failure partway (disk full/crash) can leave `root` partial.
    // Export to a fresh path when the existing bundle must survive failure.
    clear_stale_shards(root)?;

    // One open, buffered body writer per shard. Records arrive id-sorted, so
    // filtering them per shard preserves id order within each shard, matching
    // the snapshot-based layout byte-for-byte.
    let mut bodies: Vec<BufWriter<File>> = Vec::with_capacity(shard_count as usize);
    for index in 0..shard_count {
        let path = root.join(shard_body_tmp_name(index));
        bodies.push(BufWriter::new(File::create(&path)?));
    }
    let mut counts: Vec<u32> = vec![0; shard_count as usize];

    let mut frame = Vec::new();
    let mut total: u64 = 0;
    for record in records {
        let record = record?;
        let idx = shard_of(&record.id, shard_count) as usize;
        frame.clear();
        encode_record_frame(&mut frame, &record);
        bodies[idx].write_all(&frame)?;
        // The shard header stores the count as u32; turn a >u32::MAX overflow into
        // a clean error instead of a silently truncated, CRC-sealed corrupt shard.
        counts[idx] = counts[idx]
            .checked_add(1)
            .ok_or_else(|| StorageError::Format(format!("shard {idx} record count exceeds u32::MAX")))?;
        total += 1;
    }
    for w in &mut bodies {
        w.flush()?;
    }
    drop(bodies);

    // Finalize: header + body -> final shard file, with a streaming CRC. Each
    // task owns one shard and streams its body in bounded chunks.
    let entries: Vec<Result<ShardEntry>> = (0..shard_count)
        .into_par_iter()
        .map(|index| finalize_shard(root, index, counts[index as usize]))
        .collect();

    let mut shards = Vec::with_capacity(entries.len());
    for e in entries {
        shards.push(e?);
    }
    shards.sort_by_key(|s| s.index);

    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        total_records: total,
        shard_count,
        shards,
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(root.join(MANIFEST_NAME), json)?;
    Ok(manifest)
}

/// Assemble one final shard file from its body tmp: write the header, then
/// stream-copy the body in bounded chunks while folding both into the CRC.
fn finalize_shard(root: &Path, index: u32, count: u32) -> Result<ShardEntry> {
    let body_path = root.join(shard_body_tmp_name(index));
    let final_path = root.join(shard_file_name(index));

    let header = shard_header(count);
    let mut crc = Crc32::new();
    crc.update(&header);

    let mut out = BufWriter::new(File::create(&final_path)?);
    out.write_all(&header)?;

    let mut body = BufReader::new(File::open(&body_path)?);
    let mut buf = vec![0u8; COPY_CHUNK];
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        crc.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    drop(out);
    drop(body);
    fs::remove_file(&body_path)?;

    Ok(ShardEntry {
        index,
        records: count,
        crc32: crc.finish(),
    })
}

/// Snapshot-shaped convenience wrapper over [`export_stream`]. Kept for the
/// in-memory path and tests; it streams the snapshot's id-sorted records and so
/// produces byte-identical bundles to the cursor-driven export.
pub fn export_bundle(snapshot: &StoreSnapshot, root: &Path, shard_count: u32) -> Result<Manifest> {
    export_stream(snapshot.records().cloned().map(Ok), root, shard_count)
}

// ---------------------------------------------------------------------------
// Streaming import
// ---------------------------------------------------------------------------

/// Read a portable bundle at `root` and feed every record to `sink`, one shard
/// at a time, in bounded memory.
///
/// Memory discipline: shards are processed independently and each is decoded
/// **frame-by-frame** straight from a buffered reader into `sink` — at most one
/// record is resident at a time, never the whole shard or store. Each shard's
/// CRC is verified incrementally as its bytes stream through.
///
/// Returns the bundle's manifest after a full integrity check (per-shard CRC,
/// per-shard record count, and total record count).
pub fn import_stream<F>(root: &Path, mut sink: F) -> Result<Manifest>
where
    F: FnMut(Record) -> Result<()>,
{
    let manifest = read_manifest(root)?;

    let mut total: u64 = 0;
    for entry in &manifest.shards {
        let decoded = stream_shard(root, entry, &mut sink)?;
        if decoded != entry.records {
            return Err(StorageError::Format(format!(
                "shard {} record count mismatch: manifest {} vs decoded {}",
                entry.index, entry.records, decoded
            )));
        }
        total += decoded as u64;
    }
    if total != manifest.total_records {
        return Err(StorageError::Format(format!(
            "total record count mismatch: manifest {} vs imported {}",
            manifest.total_records, total
        )));
    }
    Ok(manifest)
}

/// Read and validate `manifest.json`, rejecting an unsupported version.
fn read_manifest(root: &Path) -> Result<Manifest> {
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
    Ok(manifest)
}

/// Stream one shard file: verify its header, decode each record frame straight
/// into `sink`, and verify the per-shard CRC. Peak memory is one record.
fn stream_shard<F>(root: &Path, entry: &ShardEntry, sink: &mut F) -> Result<u32>
where
    F: FnMut(Record) -> Result<()>,
{
    let path = root.join(shard_file_name(entry.index));
    let file =
        File::open(&path).map_err(|e| StorageError::Io(format!("reading {}: {e}", path.display())))?;
    let mut reader = ShardReader::new(BufReader::new(file));

    let magic = reader.take(8)?;
    if magic != SHARD_MAGIC {
        return Err(StorageError::Format("bad shard magic".into()));
    }
    let version = reader.u16()?;
    if version != FORMAT_VERSION {
        return Err(StorageError::Format(format!(
            "unsupported shard version {version} (expected {FORMAT_VERSION})"
        )));
    }
    let count = reader.u32()?;
    for _ in 0..count {
        let record = reader.record()?;
        sink(record)?;
    }
    reader.expect_eof()?;
    let crc = reader.crc_value();
    if crc != entry.crc32 {
        return Err(StorageError::Format(format!(
            "shard {} crc mismatch (corrupt or tampered)",
            entry.index
        )));
    }
    Ok(count)
}

/// Snapshot-shaped convenience wrapper over [`import_stream`]. Kept for the
/// in-memory path and tests; collects the streamed records into a snapshot.
pub fn import_bundle(root: &Path) -> Result<StoreSnapshot> {
    let mut snapshot = StoreSnapshot::new();
    import_stream(root, |record| {
        snapshot.insert(record);
        Ok(())
    })?;
    Ok(snapshot)
}

// ---------------------------------------------------------------------------
// Per-shard helpers (single-shard I/O for the file adapter)
// ---------------------------------------------------------------------------

/// Which shard an id maps to for a given shard count. Public, stable mapping so
/// adapters can target a single shard for per-record I/O.
pub fn shard_index_of(id: &str, shard_count: u32) -> u32 {
    shard_of(id, shard_count.max(1))
}

/// Read and validate a bundle's manifest.
pub fn manifest_of(root: &Path) -> Result<Manifest> {
    read_manifest(root)
}

/// Decode a single shard file fully into its (id-sorted) records, verifying its
/// CRC. Bounded by one shard's size, never the whole store.
pub fn read_shard_records(root: &Path, index: u32) -> Result<Vec<Record>> {
    let manifest = read_manifest(root)?;
    let entry = manifest
        .shards
        .iter()
        .find(|e| e.index == index)
        .ok_or_else(|| StorageError::Format(format!("shard {index} missing from manifest")))?;
    let mut out = Vec::with_capacity(entry.records as usize);
    stream_shard(root, entry, &mut |r| {
        out.push(r);
        Ok(())
    })?;
    Ok(out)
}

/// Read one shard, let `mutate` edit its (id-sorted) record vec in place, then
/// rewrite just that shard file and patch the manifest. Bounded by one shard's
/// size. The vec is re-sorted before writing so byte-stability holds.
pub fn rewrite_shard<F>(root: &Path, shard_count: u32, index: u32, mutate: F) -> Result<()>
where
    F: FnOnce(&mut Vec<Record>),
{
    let shard_count = shard_count.max(1);
    let mut manifest = read_manifest(root)?;
    let mut records = read_shard_records(root, index)?;
    let before = records.len();
    mutate(&mut records);
    records.sort_by(|a, b| a.id.cmp(&b.id));

    // Re-frame and rewrite just this shard, with a streaming CRC.
    let mut crc = Crc32::new();
    let header = shard_header(field_len(records.len()));
    crc.update(&header);
    let mut out = BufWriter::new(File::create(root.join(shard_file_name(index)))?);
    out.write_all(&header)?;
    let mut frame = Vec::new();
    for r in &records {
        frame.clear();
        encode_record_frame(&mut frame, r);
        crc.update(&frame);
        out.write_all(&frame)?;
    }
    out.flush()?;
    drop(out);

    // Patch the manifest entry + total.
    let after = records.len();
    let entry = manifest
        .shards
        .iter_mut()
        .find(|e| e.index == index)
        .ok_or_else(|| StorageError::Format(format!("shard {index} missing from manifest")))?;
    entry.records = field_len(after);
    entry.crc32 = crc.finish();
    manifest.shard_count = shard_count;
    manifest.total_records = manifest.total_records + after as u64 - before as u64;
    let json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(root.join(MANIFEST_NAME), json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Lazy, globally id-sorted bundle cursor (k-way merge across shards)
// ---------------------------------------------------------------------------

/// A lazy cursor over a whole bundle's records in **global id-sorted order**,
/// produced by a streaming k-way merge across the id-sorted shards. Peak
/// resident: one open reader and one peeked record per shard (`O(shard_count)`),
/// never the whole store. The per-shard CRC is verified as each shard drains.
pub fn stream_bundle(root: &Path) -> Result<BundleCursor> {
    let manifest = read_manifest(root)?;
    let mut shards = Vec::with_capacity(manifest.shards.len());
    for entry in &manifest.shards {
        shards.push(ShardStream::open(root, entry.clone())?);
    }
    let mut cursor = BundleCursor {
        shards,
        fronts: Vec::new(),
    };
    cursor.prime()?;
    Ok(cursor)
}

/// A single shard opened for streaming reads, carrying its declared count and
/// the CRC entry it must match when drained.
struct ShardStream {
    reader: ShardReader<BufReader<File>>,
    entry: ShardEntry,
    remaining: u32,
}

impl ShardStream {
    fn open(root: &Path, entry: ShardEntry) -> Result<Self> {
        let path = root.join(shard_file_name(entry.index));
        let file = File::open(&path)
            .map_err(|e| StorageError::Io(format!("reading {}: {e}", path.display())))?;
        let mut reader = ShardReader::new(BufReader::new(file));
        let magic = reader.take(8)?.to_vec();
        if magic.as_slice() != SHARD_MAGIC {
            return Err(StorageError::Format("bad shard magic".into()));
        }
        let version = reader.u16()?;
        if version != FORMAT_VERSION {
            return Err(StorageError::Format(format!(
                "unsupported shard version {version} (expected {FORMAT_VERSION})"
            )));
        }
        let count = reader.u32()?;
        Ok(ShardStream {
            reader,
            remaining: count,
            entry,
        })
    }

    /// Pull the next record, or `None` at the shard's end (after verifying CRC).
    fn next_record(&mut self) -> Result<Option<Record>> {
        if self.remaining == 0 {
            self.reader.expect_eof()?;
            let crc = self.reader.crc_value();
            if crc != self.entry.crc32 {
                return Err(StorageError::Format(format!(
                    "shard {} crc mismatch (corrupt or tampered)",
                    self.entry.index
                )));
            }
            return Ok(None);
        }
        let r = self.reader.record()?;
        self.remaining -= 1;
        Ok(Some(r))
    }
}

/// Streaming k-way merge cursor across a bundle's shards.
pub struct BundleCursor {
    shards: Vec<ShardStream>,
    /// Current front record for each shard (`None` once that shard is drained).
    fronts: Vec<Option<Record>>,
}

impl BundleCursor {
    /// Load each shard's first record.
    fn prime(&mut self) -> Result<()> {
        self.fronts = Vec::with_capacity(self.shards.len());
        for shard in &mut self.shards {
            self.fronts.push(shard.next_record()?);
        }
        Ok(())
    }
}

impl Iterator for BundleCursor {
    type Item = Result<Record>;

    fn next(&mut self) -> Option<Self::Item> {
        // Pick the shard whose front record has the smallest id.
        let mut best: Option<usize> = None;
        for (i, front) in self.fronts.iter().enumerate() {
            if let Some(rec) = front {
                let take = match best {
                    None => true,
                    Some(b) => rec.id < self.fronts[b].as_ref().unwrap().id,
                };
                if take {
                    best = Some(i);
                }
            }
        }
        let idx = best?;
        let record = self.fronts[idx].take().unwrap();
        // Advance that shard's front.
        match self.shards[idx].next_record() {
            Ok(next) => self.fronts[idx] = next,
            Err(e) => return Some(Err(e)),
        }
        Some(Ok(record))
    }
}

// ---------------------------------------------------------------------------
// Streaming shard reader
// ---------------------------------------------------------------------------

/// A bounded, CRC-folding reader over a shard file. Reads frames one at a time
/// from the underlying `Read`; never materializes the whole shard.
struct ShardReader<R: Read> {
    inner: R,
    crc: Crc32,
    scratch: Vec<u8>,
}

impl<R: Read> ShardReader<R> {
    fn new(inner: R) -> Self {
        ShardReader {
            inner,
            crc: Crc32::new(),
            scratch: Vec::new(),
        }
    }

    /// Read exactly `n` bytes into the shared scratch buffer, folding the CRC.
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        self.scratch.resize(n, 0);
        self.inner
            .read_exact(&mut self.scratch)
            .map_err(|_| StorageError::Format("unexpected end of shard".into()))?;
        self.crc.update(&self.scratch);
        Ok(&self.scratch)
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

    /// Decode one full record frame.
    fn record(&mut self) -> Result<Record> {
        let id_len = self.u32()? as usize;
        let id = self.string(id_len)?;
        let kind_len = self.u32()? as usize;
        let kind = self.string(kind_len)?;
        let version = self.u64()?;
        let payload_len = self.u32()? as usize;
        let payload = self.take(payload_len)?.to_vec();
        Ok(Record {
            id,
            kind,
            version,
            payload,
        })
    }

    /// Ensure no trailing bytes remain after the declared records.
    fn expect_eof(&mut self) -> Result<()> {
        let mut byte = [0u8; 1];
        match self.inner.read(&mut byte) {
            Ok(0) => Ok(()),
            Ok(_) => Err(StorageError::Format("trailing bytes in shard".into())),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    /// The CRC of bytes consumed so far (does not consume the reader).
    fn crc_value(&self) -> u32 {
        self.crc.value()
    }
}

/// Remove any pre-existing `shard-*.bin` and stale `*.bin.body` files so a
/// re-export starts clean and stale shards from a larger previous run cannot
/// leak into a smaller one.
fn clear_stale_shards(root: &Path) -> Result<()> {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries {
        let entry = entry?;
        let path: PathBuf = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("shard-") && (name.ends_with(".bin") || name.ends_with(".bin.body"))
            {
                fs::remove_file(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "test fixtures intentionally truncate to byte/u32 values"
)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        // CRC-32/IEEE of "123456789" is the canonical 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn crc32_incremental_matches_oneshot() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 256) as u8).collect();
        let one = crc32(&data);
        let mut inc = Crc32::new();
        for chunk in data.chunks(7) {
            inc.update(chunk);
        }
        assert_eq!(one, inc.finish());
    }

    #[test]
    fn shard_encode_decode_roundtrip() {
        // Build a one-shard body in memory and stream-decode it back.
        let recs = vec![
            Record::new("a", "card", b"alpha".to_vec()),
            Record::new("b", "edge", vec![0u8, 255, 1, 254]),
        ];
        let mut bytes = shard_header(recs.len() as u32).to_vec();
        for r in &recs {
            encode_record_frame(&mut bytes, r);
        }
        let entry = ShardEntry {
            index: 0,
            records: recs.len() as u32,
            crc32: crc32(&bytes),
        };

        let tmp = std::env::temp_dir().join(format!(
            "shape_fmt_roundtrip_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join(shard_file_name(0)), &bytes).unwrap();

        let mut back = Vec::new();
        stream_shard(&tmp, &entry, &mut |r| {
            back.push(r);
            Ok(())
        })
        .unwrap();
        let _ = fs::remove_dir_all(&tmp);
        assert_eq!(recs, back);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let tmp = std::env::temp_dir().join(format!(
            "shape_fmt_badmagic_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&tmp).unwrap();
        let bytes = b"NOTSHARDxxxx".to_vec();
        fs::write(tmp.join(shard_file_name(0)), &bytes).unwrap();
        let entry = ShardEntry {
            index: 0,
            records: 0,
            crc32: crc32(&bytes),
        };
        let err = stream_shard(&tmp, &entry, &mut |_| Ok(())).unwrap_err();
        let _ = fs::remove_dir_all(&tmp);
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
