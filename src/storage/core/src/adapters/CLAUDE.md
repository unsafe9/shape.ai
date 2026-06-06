# Storage adapters — guidance

This directory holds every concrete `StorageAdapter` implementation. The trait
lives in `../adapter.rs`; the portable bundle format in `../format.rs`; the data
model in `../record.rs`; errors in `../error.rs`.

- `memory.rs` — `MemoryAdapter`, the in-RAM backend (default in-process store).
- `file.rs` — `FileAdapter`, the on-disk bundle store. **No full in-memory
  mirror**: it streams record-by-record from its sharded on-disk representation.
- `stubs.rs` — sqlite / postgres / s3 / remote-server stubs. Real adapter
  *shapes* that return `Unsupported` until their drivers are wired in.

## Memory discipline (MANDATORY)

The whole point of the rework: **export / import / save must never load the
entire store into memory.** This is a hard contract, not a nice-to-have.

- **Forbidden:** the "snapshot the whole store, then serialize / write" pattern
  for large data. Do **not** route `export`/`import` through `snapshot()` /
  `restore()` / a full `StoreSnapshot` for any streaming-capable adapter.
- **Required:** stream records with **bounded buffers** — at most ~one shard or
  one batch resident at a time. Large export/import/save must run in bounded
  memory (`O(shard/batch size * parallelism)`) with good throughput.
- Export consumes the lazy `records()` cursor and writes shards incrementally
  (`format::export_stream`); a chunked, rayon-parallel finalize pass prepends
  headers and computes CRCs by streaming each shard body back through in fixed
  `COPY_CHUNK` slices — peak is `O(COPY_CHUNK * parallelism)`.
- Import reads one shard at a time and decodes it **frame-by-frame** straight
  into the sink (`format::import_stream` → `ingest`) — peak is one record.
- `FileAdapter` keeps the on-disk bundle as the source of truth:
  - `records()` does a streaming k-way merge across the id-sorted shards
    (`format::stream_bundle`) — peak `O(shard_count)`.
  - `save`/`delete`/`load` touch only the single shard the id hashes to
    (`format::{rewrite_shard,read_shard_records,shard_index_of}`) — peak
    `O(one shard)`.
  - `import` is a bounded 2-way streaming merge of the on-disk records with the
    incoming bundle (incoming wins on equal id), re-sharded via the streaming
    exporter into a temp bundle that is then swapped in — peak `O(shard chunk *
    parallelism)`, never `O(total)`.
- `MemoryAdapter` is inherently in-RAM (the store itself is resident — that is
  acceptable for the memory backend), but its export/import must still stream
  shard-by-shard and must **not** duplicate the whole serialized bundle in
  memory on top of the store.

`snapshot`/`restore` remain for small / in-memory convenience only; they are
**not** on the streaming export/import path.

## Byte-stable deterministic format

The bundle format is byte-stable for identical logical contents:

- Records are partitioned into shards by a fixed FNV-1a hash of the id, and
  written **id-sorted within each shard**. The cursor yields records in global
  id-sorted order, so filtering it per shard preserves that order — streaming
  export produces bytes identical to a snapshot-based export.
- Stable shard framing (magic + version + count + length-prefixed fields) and a
  per-shard CRC-32 pinned in `manifest.json`.
- Consequences any change must preserve: repeated exports are byte-identical;
  two different adapter kinds holding identical logical data export to
  byte-identical bundles; export → import → re-export reproduces the original
  bundle bytes.

## Unit tests that MUST pass (integrity contract)

Run: `scripts/renderer-toolchain.sh cargo test --manifest-path
src/storage/core/Cargo.toml`. All must stay green.

Integrity / memory-safety suite (`../lib.rs` `mod integrity`), run for BOTH
`MemoryAdapter` and `FileAdapter`:

- `memory_all_methods_same_data`, `file_all_methods_same_data` — one fixed,
  varied dataset (mixed kinds/versions; empty, NUL, high-byte, binary, and
  utf-8 payloads) round-trips through every method: `load` (every id),
  `list` (deterministic id order + completeness), `records()` cursor (id
  order + completeness), `snapshot` equality, `delete` (returns true, gone
  after, false on repeat), missing-id `NotFound`.
- `repeated_export_is_byte_identical` — re-export is byte-for-byte identical.
- `different_adapter_kinds_export_byte_identical` — memory and file holding
  identical logical data export byte-identical manifests + shard bytes.
- `export_import_reexport_is_byte_identical` — export → import → re-export equals
  the original bundle bytes (via both file and memory).
- `import_into_memory_and_file_matches_source`,
  `cross_adapter_memory_file_memory_preserves_everything` — after import every
  id/kind/version/payload matches the source exactly, including memory↔file.
- `large_dataset_roundtrips_in_bounded_memory` — tens of thousands of records,
  multi-MB total. Exports from a lazy generator and imports through an
  instrumented sink. Asserts **peak simultaneously-resident records == 1** and,
  via a counting global allocator, that **peak heap during export/import stays
  far below `O(total)`** (the regression guard against snapshot-then-write).
- `large_file_adapter_import_export_roundtrip` — same scale through the file
  adapter's streaming import + re-export (byte-identical) + single-shard ops.

Format-level tests (`../format.rs`): CRC known vector, incremental==one-shot CRC,
shard encode/decode roundtrip, bad-magic rejection, stable shard assignment.

Pre-existing contract tests (`../lib.rs` `mod tests`): byte-stability across
exports, corrupted-shard detection (CRC mismatch → `Format` error), empty-store
roundtrip, configurable-and-lossless shard count, cross-adapter roundtrip,
in-memory snapshot roundtrip, adapter-kind vocabulary.

## Adding a new adapter

Implement `StorageAdapter` for the new backend and re-export it from `mod.rs`.
Required methods:

- `kind`, `save`, `load`, `delete`, `list` — per-record native I/O.
- `records(&self) -> Result<RecordCursor<'_>>` — a **lazy** cursor over all
  records in **deterministic id-sorted order**. Yield one record at a time
  (e.g. shard/page-by-page off the backend); never build the whole store.
- `ingest(&mut self, record)` — streaming write sink (defaults to `save`).
  Override only if a cheaper bulk path exists; keep it bounded.
- `snapshot`/`restore` — for small/in-memory convenience.

`export` / `export_with_shards` / `import` come for free from the trait via the
streaming core, so a correct streaming `records`/`ingest` gives you a
memory-safe, byte-stable, portable bundle automatically. If your backend can do
a faster bounded bulk import (like `FileAdapter`'s merge-and-reshard), override
`import` — but it must stay `O(shard/batch * parallelism)`, never `O(total)`.
