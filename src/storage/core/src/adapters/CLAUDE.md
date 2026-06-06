# Storage adapters — guidance

Every concrete `StorageAdapter` lives here; the trait, bundle format, data model,
and errors live one level up. Read those files for the exact shapes — this doc is
only the intent you can't recover from the code.

## Memory discipline (the whole point of the rework)

**Export / import / save must never hold the entire store in memory.** This is a
hard contract, not an optimization.

- Stream records with bounded buffers — roughly one shard or one batch resident
  at a time, regardless of total size.
- Never snapshot the whole store to serialize or write it. `snapshot` / `restore`
  exist for small / in-memory convenience only and are not on the streaming path.
- The on-disk backend keeps the bundle as its source of truth with no full
  in-memory mirror: reads merge across shards, per-record ops touch only the one
  shard an id maps to, and import streams a merge of old and incoming records
  into a fresh bundle.

## Byte-stable format

Identical logical contents always produce identical bytes. The shard layout is
deterministic so that repeated exports, two different adapters holding the same
data, and an export→import→re-export round-trip all yield byte-identical bundles.
Preserve this property in any format or adapter change.

## Adding a new adapter

A correct streaming cursor (lazy, deterministic id order, one record at a time)
plus a bounded write sink gives you a memory-safe, byte-stable, portable bundle
for free — the trait's streaming core builds export/import on top of them. Only
override `import` if the backend has a genuinely faster bulk path, and only if it
stays bounded.
