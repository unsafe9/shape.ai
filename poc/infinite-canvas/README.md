# Infinite Canvas POC

This directory contains the isolated proof-of-concept for the infinite canvas engine task graph.

## Scope

- `core/`: Rust/WASM scene core scaffold. It owns retained scene parsing, camera state, frame stats, hit testing, compact render patch ingestion, a minimal browser canvas debug frame API, a feature-gated wgpu/WebGPU readiness probe, and a visible WebGPU primitive renderer for group/card geometry, selected group/card/edge outlines, segmented cubic edge geometry, edge labels, shared style token colors, `unicode-segmentation`/`unicode-width` based bitmap title/summary/edge-label wrapping with CJK/fallback glyph metrics, fixed-slot dirty buffer writes for group translate/card move/edit/select patches, order-preserving rebuilds for z-order patches, group-slot buffer growth/compaction for group create/delete churn, card-slot buffer growth/compaction for card create/delete churn, edge-slot buffer growth when spare slots are exhausted, edge-slot compaction after delete-heavy churn, and card/text/port/edge/group hit testing. It intentionally does not own shape.ai business rules.
- `web/`: Vite React harness that mounts Canvas2D and WebGPU canvases, renders group frames/cards/text/edges, exercises pan/zoom/group tag attach/filtering/comments/product export preview/group drag/card drag/selection/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge creation/deletion, and uses a DOM textarea overlay for active text editing.
- `docs/`: task evidence, benchmark criteria, architecture notes, parity notes, and replacement recommendations.
- `fixtures/`: committed fixture metadata. Large generated benchmark output should stay out of git unless intentionally promoted.

Core module layout:

- `core/src/lib.rs`: wasm exports, root re-exports, panic hook, serialization helper, feature-disabled WebGPU probe fallback.
- `core/src/model.rs`: renderer scene contract and compact patch contract.
- `core/src/stats.rs`: frame/hit/probe report payloads returned to TypeScript.
- `core/src/debug_canvas.rs`: Canvas2D debug renderer path.
- `core/src/webgpu.rs`: visible `wgpu` renderer, WebGPU probe, buffer slots, primitive/text/edge helpers.

## Commands

From the repository root:

```bash
npm run poc:dev
npm run poc:typecheck
npm run poc:test
npm run poc:build
npm run poc:verify
```

To load and save a real app scene from the POC harness, run the normal backend on its default `http://127.0.0.1:8787` and use the `Real scene` control in the POC. The POC Vite server proxies `/api` to that backend by default. Override the target with `SHAPE_AI_API_TARGET` if needed.

The Rust/WASM package is generated only when the local Rust toolchain and `wasm-pack` are available:

```bash
npm run poc:wasm:build
```

Generated WASM glue is written to `poc/infinite-canvas/web/src/wasm/` and is gitignored. If the WASM package is missing, the web harness reports `TS fallback` and still runs the browser interaction and benchmark harness so the app boundary remains testable. If the WASM package is present and WebGPU is available, the toolbar can switch between the TypeScript Canvas2D renderer, the Rust/WASM Canvas2D debug draw path, and the visible Rust/wgpu primitive renderer.

## Current Backend Boundary

The current POC has two layers:

- Rust/WASM scene core scaffold: retained scene parsing, camera state, frame stats, hit testing API shape, compact render patch ingestion, `mount`, `resize`, optional debug canvas frame rendering, wgpu/WebGPU adapter/device/surface/render-pass probe on a detached canvas, visible WebGPU group/card/cubic-edge/edge-label/text-snippet primitive rendering, grapheme-aware bitmap text wrapping via `unicode-segmentation`/`unicode-width`, CJK/fallback glyph counting, selected group/card/edge outline rendering, fixed-slot dirty card/edge/group buffer writes for group translate/card move/edit/select patches, z-order buffer rebuilds, spare-slot dirty group create/delete writes, dynamic group-slot growth, group-slot compaction after delete-heavy churn, spare-slot dirty card create/delete writes, dynamic card-slot growth, card-slot compaction after delete-heavy churn, spare-slot dirty edge create/delete writes, dynamic edge-slot growth before full buffer rebuild fallback, edge-slot compaction after delete-heavy churn, and Rust-owned WebGPU hit testing for cards/text/ports/edges/groups.
- TypeScript browser renderer harness: canvas mount, DPR resize, retained draw loop, spatial culling, caches, pointer batching, group tag attach/filtering, comments, product export preview, group/node create/delete/duplicate/copy/paste/z-order controls, DOM edit overlay, scripted benchmark, Rust/TS/WebGPU draw backend toggle, WebGPU readiness readout, GPU vertex count metric, GPU glyph/fallback-glyph/CJK-glyph metrics, style token count metric, GPU patch/dirty-write/rebuild/group-slot/group-grow/group-compact/card-slot/card-grow/card-compact/edge-slot/edge-grow/edge-compact metrics, app-scene fixture loading, and optional API-backed real scene load/save through app `ScenePatch`.

This keeps production source untouched while making the eventual Rust/wgpu renderer boundary concrete. The next renderer hardening step is to move from grapheme-aware bitmap fallback text, primitive geometry, selected-outline dirty writes, and group/card/edge slot churn handling to product-quality Rust/wgpu drawing: real text shaping/cache, font fallback, Korean glyph raster quality, richer card/edge styling, broader style/layout dirty updates, and browser-verified overlay behavior behind the same `mount`, `resize`, `loadScene`, `applyPatch`, `hitTest`, `beginTextEdit`, and `renderFrame` API.
