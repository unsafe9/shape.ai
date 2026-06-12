# scene-core — the canonical scene substrate

Pure: no ambient time, randomness, threads, or IO. Every such seam is an
injected parameter — `now: &str`, an explicit operation id, a `local_seq`. If a
new function wants ambient state, lift it into the signature; never reach for a
global clock or counter.

**One apply path, with an inverse-op contract.** Every mutation goes through
`apply_object_op`, which RETURNS the op that reverses what it just applied. Undo
is reverse-op authoring, not state rollback: the host re-applies the returned
inverse through the same path, and gets the re-inverse back for redo. Never add a
snapshot/restore shortcut around this.

Adding an op (do every step, in order):
1. Define the variant in `object/kernel/op.rs`.
2. Apply + emit its inverse in `object/kernel/apply.rs`.
3. Validate it in `object/kernel/validate.rs`.
4. Wire it through authoring/binding/catalog as the feature needs.
5. Regenerate the TS wire types (`npm run types:gen`) and commit
   `platforms/web/shared/generated/`. The types are ts-rs-derived from this serde
   surface (behind the dev-only `ts-gen` feature); never hand-edit the TS mirror.
   `npm run types:check` fails the unit gate on drift.
6. Add a golden test driving the real core (no second apply, ever).

Tier dependency direction is one-way: `kernel <- {authoring, binding, catalog}`.
The kernel (model/op/apply/undo/validate) knows nothing of the outer tiers.

`wasm_api.rs` is a JSON bridge only. Logic appearing there is in the wrong layer
— push it down into a tier and re-export.
