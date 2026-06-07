# shape.ai

Visual group canvas for humans and AI agents — an infinite, local-first canvas
that agents can review and extend through MCP.

## Technical Background

shape.ai is an infinite canvas and design system built in Rust on top of `wgpu`.
The canvas is the product, so everything performance-sensitive about it — drawing,
navigation, hit testing, layout, LOD — lives in the Rust core.

The web (WASM + WebGPU) is the only target shipping today, but multi-platform
reach — macOS via Metal, and mobile/iOS — is a deliberate design constraint, not a
someday-nice-to-have. `wgpu` already abstracts those native backends, so the core
is meant to compile for them without forking the scene model.

The architecture turns on one rule: **keep canvas logic in the Rust core and keep
every platform layer thin.** A platform layer — the web today, a native app
later — carries only what the OS forces it to: essential product UI plus OS
integration such as input, IME, clipboard, and the GPU surface. Everything else
belongs in the core. This guards against two equal-and-opposite failure modes:
pushing canvas behavior up into UI/component state, and leaking platform
assumptions down into the core. Either one erodes the portability the boundary
exists to protect.

That boundary runs core → platform adapter → app shell. The adapter is a thin
per-target seam; the app shell owns product UI and orchestration and talks to the
canvas only through a narrow imperative handle plus an event stream, never by
reaching into its internals. The web shell is **Svelte** (the project has fully
cut over from React); a future macOS shell would be SwiftUI, changing only the
language and the adapter.

## Stack and Layout

The project has decommissioned its original Node/Fastify/sql.js server: there is
no Node backend, no `tsx`/`concurrency` toolchain, and no `src/server/*.ts`
runtime beyond `local.ts` (a TS export helper still imported by surviving
renderer/export tests). Everything server-side is now one Rust workspace; the
client is a Svelte shell over WASM.

Rust workspace (`Cargo.toml` members):

- `crates/scene-core` (`shape_scene_core`) — the pure canvas core: model, op
  enum, op-apply, per-property LWW, fractional index, templates, wire serde,
  command catalog. Native + `wasm32`, no time/rng/thread/IO. The web client
  builds it to WASM (`--features wasm`) and uses it as the client op-apply.
- `src/storage/core` (`shape_storage_core`) — store-neutral persistence:
  `StorageAdapter` trait, spatial region index, portable sharded bundle format,
  Memory/File/SQLite adapters. SQLite + the on-disk format are native-only;
  `wasm32` keeps model + trait + `MemoryAdapter`.
- `crates/coordination` (`shape_coordination`) — scale-out seam: `Coordinator`
  trait for single-writer canvas leases, pub/sub, and presence; in-memory
  (single-process) and file (single-host multi-process) impls. No canvas logic.
- `crates/server` (`shape_server`) — the native tokio/axum platform layer. It
  orchestrates transport, persistence, and fan-out only and never reimplements
  canvas logic. Key files: `app.rs` (router: `/api/*`, `/ws`, `/mcp`, static
  SPA), `canvas_actor.rs` (one actor per canvas), `sync.rs` (dedup + LWW +
  fractional keys), `scene_store.rs` (per-object region-indexed records),
  `registry.rs` (canvasId→actor, leases, eviction, routing, graceful shutdown),
  `ws.rs` (two-channel WS transport), `mcp.rs` (rmcp Streamable HTTP tools),
  `local_export.rs`/`group_seed.rs`/`scene_api.rs` (ported legacy scene routes).

The renderer core `src/renderer/core` (`shape_canvas_core`) is `exclude`d from
the workspace (its pinned wgpu lock would pollute the workspace lock) and builds
standalone to WASM for the web canvas.

Client (`src/client`) is the Svelte shell: it owns product UI and orchestration
and reaches the canvas through the imperative handle + event stream, the server
over `/ws` and `/api/*`, and uses scene-core-WASM for optimistic op-apply.

## Conventions

- **Pointer-width-agnostic core.** scene-core/storage-core deny width-narrowing
  and raw-pointer casts (the workspace lint table in root `Cargo.toml`); use
  checked `try_from`, or a reasoned `#[allow]` for intentional truncation. Intent:
  a future Wasm 3.0 Memory64 port stays a target-triple flip, not a rewrite.

## Running

Build the client (Rust → WASM, then Vite), then run the native server:

```bash
npm install
npm run build              # scene:wasm:build + renderer:wasm:build + vite build -> dist/client
cargo run -p shape_server # serves dist/client + /ws + /mcp on :8787
```

For iterative client work, run Vite (`npm run dev`, :5173) against a running
`cargo run -p shape_server` (:8787). There is no `npm start`/`npm run dev:server`
anymore. Scene data persists per-object in `.local/shape.sqlite`; exports under
`.local/exports/`. `.local/` is gitignored.

Always invoke cargo/wasm-pack through `scripts/renderer-toolchain.sh` so the
pinned renderer toolchain is on `PATH`.

## Operating Notes

- scene-core WASM is the SINGLE runtime op-apply in both the browser and
  Node/vitest. The loader (`src/client/scene/sceneCoreWasm.ts`) keeps one
  `--target web` artifact and picks its init per environment: the browser awaits
  the async `default` (which fetches the `.wasm`), while Node/vitest reads the
  sibling `.wasm` from disk and inits synchronously via `initSync` (no `fetch`).
  `applyRenderPatchSync` is the synchronous op-apply the sync engine drives;
  `ensureSceneCore()` must resolve first (the wasm instance must be initialized),
  which `SceneClient.connect` awaits before the engine can author. `syncEngine.ts`
  and the shell (`App.svelte`) call only the WASM apply; there is no runtime TS
  op-apply. Because vitest now drives the WASM op-apply, `pretest:unit` builds the
  scene wasm before the suite runs.
- `src/shared/renderPatch.ts` (`applyRenderPatchToShapeScene`,
  `updateShapeSceneGroupTags`, `addShapeSceneComment`) and its `operation.ts`
  envelope helper are retained as TEST-ONLY golden-oracle tooling: the golden
  generator (`crates/scene-core/tests/golden/generate.ts`) and the equivalence/
  unit suites import them to prove the Rust port matches the canonical TS. The
  oracle must stay independent — do NOT regenerate goldens from the Rust-derived
  WASM. These files are no longer in the client op-apply runtime path; only their
  `RenderScenePatch` TYPE and the render projection
  `shapeSceneToFilteredRenderSnapshot` are still imported by the client.
- Template lowering still runs through TS `applyTemplate` in the shell
  (`src/shared/templates/contract.ts` uses `applyRenderPatchToShapeScene`); the
  WASM template contract is mirrored but not yet routed through. This is a
  separate, deliberate deviation from the op-apply cutover above.
- Verification gates: `cargo test --workspace`, the two `wasm32` `cargo check`s
  (scene-core, storage-core), both wasm-pack builds, `npm run test:unit`
  (vitest), and `npm run build` (vite).

