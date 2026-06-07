# shape_storage_core — storage architecture

This is the durable design background for the storage layer: the decisions that
shaped it and the contracts downstream milestones (the MG-2 server and beyond)
build on. For the day-to-day adapter authoring rules — memory discipline, the
byte-stable invariant, how to add an adapter — read [`CLAUDE.md`](./CLAUDE.md)
alongside this file. The exact shapes live in the code: `adapter.rs` (trait),
`format.rs` (bundle), `record.rs` (data model), `spatial.rs` (region query).

## D1 — exchange bundle vs at-rest store are different things

The crate has two distinct persistence concerns and deliberately does **not**
conflate them:

- **The exchange bundle** is the one portable, byte-stable, parallel-friendly
  on-disk format defined in `format.rs` (the `*.shapestore` sharded directory).
  Its job is *interchange and archival*: dump a store from any backend, hand the
  bytes to another machine or another backend, and load them back losslessly. It
  is the lingua franca that lets memory, file, and sqlite stores round-trip
  through each other. Identical logical contents always serialize to identical
  bytes (see D-byte-stability below).
- **The at-rest store** is whatever a given adapter natively keeps its data in: a
  `BTreeMap` in RAM, a `.shapestore` directory on disk, a sqlite database file,
  or (later) postgres/s3/a remote server. Each at-rest format is the adapter's
  own business and may differ wildly from the bundle.

The bridge between the two is the `StorageAdapter` streaming pair (`records` /
`ingest`): `export` walks the at-rest store as an id-sorted cursor straight into
an exchange bundle, and `import` streams an exchange bundle one record at a time
into the at-rest store. Because the bridge is record-at-a-time, the two formats
never need to be the same and neither side is ever fully materialized in RAM.

One consequence to remember: **spatial data is part of the at-rest store, not the
exchange bundle.** A `Record` is only `{id, kind, version, payload}`, and that is
all the bundle carries. A `RegionKey` lives in the adapter's side index. So a
round-trip through a bundle drops the region index; re-index with `save_indexed`
after an import if the destination needs region queries.

## D2 — the data model stays domain-neutral

A `Record` is `{id, kind, version, payload: Vec<u8>}` and nothing else. The
storage core has **no dependency on scene-core or any domain type**, and must
keep it that way. The canvas domain (cards, edges, groups, geometry) is encoded
by the caller into `payload` bytes plus a `kind` tag; the store moves opaque
blobs. Even the spatial capability respects this: geometry lives in the side
index (`RegionKey`), never in the payload. This neutrality is what lets one
storage crate serve the canvas today and anything else later without forking.

## D5 — the adapter seam

`StorageAdapter` (in `adapter.rs`) is the single seam every backend implements.
It carries three concerns:

1. **In-store I/O** — `save` / `load` / `delete` / `list`, per-record ops against
   the backend's native at-rest format.
2. **Streaming** — `records` (a lazy, id-sorted cursor) and `ingest` (a
   one-at-a-time sink). These are the memory-safe bridge to the exchange bundle.
3. **Portability** — `snapshot` / `restore` for the small/in-memory convenience
   path, plus `export` / `import`, which are provided once on top of the
   streaming pair so every adapter gets memory-bounded interchange for free.

A new backend only has to provide a correct streaming cursor and a bounded write
sink; the trait's default `export` / `import` then work unchanged. Override
`import` only for a genuinely faster bulk path that stays bounded (the sqlite
adapter does this to wrap the upserts in one transaction).

`SpatialStore` (in `spatial.rs`) is an **optional second capability** bolted onto
the seam, not part of `StorageAdapter`. Only backends that maintain a region
index implement it. A caller that wants region queries must therefore hold a
concrete type that implements `SpatialStore` (or `&dyn SpatialStore`), not just a
`&dyn StorageAdapter`.

## Adapter matrix

| Adapter        | At-rest store              | Targets                         | `SpatialStore` | Notes |
|----------------|----------------------------|---------------------------------|----------------|-------|
| `MemoryAdapter`| `BTreeMap` in RAM          | all (incl. wasm32)              | yes            | default in-process store; backs tests |
| `FileAdapter`  | a `*.shapestore` bundle dir| native only                     | no             | the at-rest store *is* an exchange bundle; no in-RAM mirror |
| `SqliteAdapter`| native rusqlite db (file or `:memory:`) | native + `sqlite` feature | yes  | `records` table + `region_index` side table |
| `PostgresAdapter` / `S3Adapter` / `RemoteServerAdapter` | (driver unavailable offline) | native only | no | real adapter *shapes*; per-record I/O returns `Unsupported` |

Target notes:

- **wasm32 today = core + memory.** The exchange-bundle format (`format.rs`) and
  every adapter that depends on it lean on `std::fs` + rayon, neither of which
  exists on `wasm32-unknown-unknown`. So wasm compiles only the data model, the
  `StorageAdapter` trait, and `MemoryAdapter` (which still implements
  `SpatialStore`). There is no on-disk format and no `export`/`import` on wasm.
  `cargo check -p shape_storage_core --target wasm32-unknown-unknown` enforces
  this.
- **wasm-sqlite (OPFS) is deferred to MG4.3 (DEFERRAL).** A persistent sqlite
  backend in the browser would run on the Origin Private File System via a JS
  sqlite bridge (the wasm sqlite build talking to OPFS). That is **not** built in
  this crate today and is intentionally deferred to client integration at MG4.3:
  the browser shell will supply the JS sqlite/OPFS bridge and a thin wasm adapter
  over it. Until then, the browser uses `MemoryAdapter` for in-session state and
  the exchange bundle (produced server-side) for persistence/transfer.

## Bundle atomicity (MG1.4) and the plaintext file adapter (MG1.5)

`FileAdapter` keeps the `*.shapestore` bundle as its sole source of truth (no
in-memory mirror), so the file adapter *is* the plaintext at-rest store (MG1.5).
Whole-bundle rewrites (`import`, `reshard`) are made crash-safe by the
**write-then-swap** discipline in `file.rs`: the new bundle is written to a
sibling temp directory and only `rename`d over the live bundle once it is fully
written and verified. A failure partway (corrupt incoming bundle, disk full)
leaves the live bundle byte-intact and removes the partial temp directory rather
than leaking it beside the store. This is covered by
`integrity::file_import_failure_leaves_existing_bundle_intact` in `lib.rs`.

(Note the narrower seam: per-record `save` / `delete` rewrite a single shard in
place and are *not* whole-bundle-atomic — they touch only the one shard an id
hashes to. The atomicity contract above is about whole-bundle operations.)

## D-byte-stability — identical contents, identical bytes

The exchange bundle is byte-stable: the same logical contents always produce the
same bytes, no matter which adapter wrote them or how many times. This is a hard
contract, not an optimization, and the constants that pin it must not change —
FNV-1a shard hash (`0xcbf29ce484222325` / `0x100000001b3`), CRC-32/IEEE
(`0xedb88320`), little-endian framing, id-sorted records within each shard,
`DEFAULT_SHARD_COUNT = 8`, `FORMAT_VERSION = 1`, and a `serde_json::to_vec_pretty`
manifest. `integrity::all_adapters_export_byte_identical_and_reimport` proves all
three real adapters (Memory, File, Sqlite) emit byte-identical bundles for one
dataset and that each re-imports into a fresh store reproducing the snapshot.

## PC10 — the SpatialStore region-query contract

`SpatialStore` adds geometry-aware queries without touching the neutral data
model. Its contract (enforced by the shared test in `spatial.rs`, run against
both Memory and Sqlite):

- `save_indexed(record, Some(key))` upserts the record **and** its region row in
  one step. `save_indexed(record, None)` upserts the record but **clears** its
  region row — the record still loads, it just no longer appears in
  `query_region`.
- `query_region(canvas_id, bbox)` filters by `canvas_id` first, then by `bbox`.
  `bbox = None` means the whole canvas (every indexed record on it). Overlap is
  an **inclusive** AABB intersect — edge-touching counts:
  `min_x <= qmaxx && max_x >= qminx && min_y <= qmaxy && max_y >= qminy`.
- Results are always **id-sorted** and **streamed** — bounded memory, one page /
  one record resident at a time, exactly like `records`.
- `delete` removes a record **and** its region row (the sqlite adapter relies on
  a `region_index → records` foreign key with `ON DELETE CASCADE`, with
  `PRAGMA foreign_keys = ON` set per connection). `restore` wipes the region
  index. Plain `StorageAdapter::save` does **not** touch the region index — only
  `save_indexed` does — so a record may legitimately exist without a region row.
