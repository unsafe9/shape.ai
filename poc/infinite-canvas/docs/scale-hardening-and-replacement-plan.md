# Scale Hardening And Replacement Plan

## Implemented Scale Hooks

- Spatial index: `buildSpatialIndex()` and `querySpatialIndex()`.
- Viewport culling: renderer queries visible card ids from the spatial index and filters by padded viewport.
- Edge culling: edges render only when both endpoint cards are visible.
- Text cache: wrapped title/summary lines are cached by text, width, line count, and font.
- Edge cache: cubic edge routes are cached by edge id.
- Boundary stats: scene load, resize, camera updates, patches, and benchmark calls increment `boundaryCalls`.
- Rust boundary stats: generated WASM core load, resize, camera updates, scene sync, compact WebGPU patch sync, and debug frame calls increment `rustBoundaryCalls`.
- WebGPU readiness probe: generated WASM can check browser WebGPU support on a detached probe canvas, create a wgpu canvas surface, request an adapter/device, configure the surface, run a clear render pass, submit it, and present it.
- Visible WebGPU primitive renderer: generated WASM can own a visible WebGPU canvas, upload group/card primitives plus segmented cubic edge vertices, edge labels, and bitmap text glyph quads, consume shared style token colors, wrap title/summary/edge-label text by grapheme/display width through `unicode-segmentation` and `unicode-width`, report CJK/fallback glyph counts, draw selected group/card/edge outlines, apply compact render patches without re-sending the whole scene snapshot, write fixed card/edge/group vertex slots for group translate/card move/edit/select dirty ranges, rebuild card draw order for z-order patches, use spare group slots for group create/delete dirty writes, grow group slot capacity by inserting transparent slots before edge/card segments, compact delete-heavy group/free-slot churn by copying used group slots and shifting edge/card segments into a smaller GPU buffer, use spare card slots for card create/delete dirty writes, grow card slot capacity by appending transparent slots, compact delete-heavy card/free-slot churn by copying used card slots into a smaller suffix, use spare edge slots for edge create/delete dirty writes, grow edge slot capacity by inserting transparent slots before the card draw segment, compact delete-heavy edge/free-slot churn by copying used edge and card segments into a smaller GPU buffer, perform Rust-owned card/text/port/cubic-edge/group hit testing, submit render passes, present frames, and report GPU vertex/glyph/fallback-glyph/CJK-glyph/style-token/patch/dirty-write/rebuild/group-slot/group-grow/group-compact/card-slot/card-grow/card-compact/edge-slot/edge-grow/edge-compact counts.
- Memory stats: browser JS heap is reported when `performance.memory` exists.
- Visual fixture: small and 1k fixture toggles exercise interaction and scale modes.
- Shape scene parity fixture: the harness can load a deterministic app `Scene`, apply group tag filtering before render snapshot projection, update selected group tags, add comments, generate product export previews in the app shell, and translate group drag/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge/select patches back into app `ScenePatch` semantics in memory.
- Real scene API path: the harness can proxy `/api`, load a backend scene, apply local group tag filtering, send selected-group tag updates to `/api/groups/:id/tags`, send comments to `/api/comments`, send exports to `/api/groups/:id/export`, and send group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge app patches to the real `PATCH /api/scene` route when the backend is running.
- Proxy verification: a temporary backend round trip verified create group, proxy scene load, node-position patch persistence, text patch persistence, group create/delete patch persistence, edge create persistence, edge delete persistence, and render-snapshot conversion through the POC `/api` proxy.
- Production adapter module: `src/shared/renderScene.ts` converts app `Scene` data to the render snapshot contract without mounting the new renderer.
- Internal comparison route: `GET /api/scene/render-snapshot` returns the production render snapshot and is covered by a server test that confirms comments and confidence stay out of the renderer payload.

## Known Gaps

- The Rust core has a visible WebGPU primitive renderer that consumes style tokens and wraps bitmap text by grapheme/display width, but not the production-quality Rust/wgpu draw backend.
- Scene buffers, render passes, style-token colors, selected outlines, grapheme-aware bitmap text snippets and edge labels with CJK/fallback glyph metrics, cubic edge geometry, compact patch ingestion, fixed-slot group translate/card move/edit/select dirty writes, z-order rebuilds, group-slot create/delete dirty writes, dynamic group-slot growth/compaction, card-slot create/delete dirty writes, dynamic card-slot growth/compaction, spare-slot edge create/delete dirty writes, dynamic edge-slot growth, edge-slot compaction, and Rust WebGPU hit testing exist, but Swash/Cosmic Text-level shaping/cache, real Korean glyph rendering, richer product card/edge styling, broader style/layout dirty invalidation, DOM overlay alignment/IME verification, and browser interaction proof are not production-ready.
- Text shaping quality and real font fallback are not proven with Swash/Cosmic Text.
- Geometry tessellation is not proven with Lyon/Kurbo/Peniko.
- Cache invalidation and GPU buffer writes are POC-level; move/edit/select writes update fixed vertex slots, group/card/edge create/delete can use spare slots, exhausted group spare slots can grow by inserting slots before edge/card segments, exhausted card spare slots can grow by appending suffix slots, exhausted edge spare slots can grow capacity without rebuilding the whole primitive buffer, and delete-heavy group/card/edge free-slot churn can compact without recomputing all geometry.
- Visual regression screenshots are not committed; use Browser/Playwright captures during review.

## Replacement Readiness

Not ready for production source replacement yet.

Reasons:

- The current POC proves app/renderer contracts and interaction boundary, but not the production-quality Rust/wgpu rendering backend.
- The API-backed real scene path is terminal-verified through the proxy for drag/text/edge-equivalent patches, but still needs rendered browser interaction verification against a running backend before production migration.
- Existing DOM/SVG path still carries MCP-driven app semantics and production-polished app shell affordances.

The migration scope and replacement recommendation are summarized in `replacement-decision-summary.md`.

## Next Migration Cycle

1. Harden the visible Rust/wgpu primitive renderer into the product renderer: shaping/cache and real font fallback beyond the current grapheme-aware bitmap atlas, richer card/edge styling, and broader style/layout dirty-range updates.
2. Build generated WASM in `poc/infinite-canvas/web/src/wasm/`.
3. Re-run the same 1k fixture benchmark and record real GPU stats.
4. Browser-verify the POC `Real scene` rendered load/save path against a running backend.
5. Compare POC and production DOM/SVG path for pan/zoom/edit/select/connect/export.
6. Only then start production replacement tasks.

## Recommended First Production Replacement Task

After the Rust/wgpu backend passes the POC benchmark and browser interaction verification, mount the new renderer behind an internal comparison route or temporary flag. The render snapshot adapter and read-only comparison endpoint already exist, keeping the first renderer mount reversible before UI replacement begins.
