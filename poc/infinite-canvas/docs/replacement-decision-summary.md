# Replacement Decision Summary

## Recommendation

Do not replace the production DOM/SVG scene canvas in this branch.

The POC has enough evidence to plan the replacement, but not enough evidence to mount the new engine as the default production editor surface or delete the current path.

## Evidence Now Available

- Isolated POC workspace under `poc/infinite-canvas/`.
- Rust/WASM package builds with `wasm-pack` and mounts in the web harness.
- Retained scene model renders group frames, cards, text snippets, and edges.
- Small and 1,000+ object fixtures exercise pan/zoom/fit, selection, group tag attach/filtering, comments, product export preview, group drag, card drag, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge creation/deletion, DOM text edit overlay, and scripted benchmark stats.
- Rust core exposes a Canvas2D debug path, detached WebGPU readiness probe, and visible `wgpu` canvas primitive renderer with shared style token colors, selected group/card/edge outlines, segmented cubic edges, edge labels, `unicode-segmentation`/`unicode-width` backed bitmap title/summary/edge-label glyph quads with CJK/fallback glyph metrics, compact render patch ingestion, fixed-slot dirty buffer writes for move/edit/select patches, order-preserving rebuilds for z-order patches, spare-slot group create/delete writes, group-slot capacity growth, group-slot compaction after delete-heavy churn, spare-slot card create/delete writes, card-slot capacity growth, card-slot compaction after delete-heavy churn, spare-slot edge create/delete writes, edge-slot capacity growth, edge-slot compaction after delete-heavy churn, and card/text/port/edge/group hit testing.
- App/render boundary excludes comments, artifacts, export state, confidence/evidence refs, and MCP/proposal semantics.
- The render snapshot adapter now lives in `src/shared/renderScene.ts`, so production source has a testable conversion module without mounting the new renderer.
- `GET /api/scene/render-snapshot` exposes that conversion as a read-only internal comparison route without mounting the new renderer.
- Shape app scene fixture projects through the render adapter, applies group tag filtering before snapshot projection, updates selected-group tags, adds comments, generates product export previews in the app shell, and translates group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge/select patches back into app `ScenePatch` semantics in memory.
- POC harness can optionally load `/api/scene` through the Vite proxy, apply local group tag filtering, send selected-group tag updates through `/api/groups/:id/tags`, send comments through `/api/comments`, send exports through `/api/groups/:id/export`, and send group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge app patches through the real `/api/scene` `PATCH` route when the backend is running.
- The POC API proxy was verified against a temporary backend: create group, load scene through the POC server, patch a node position, patch node text, create/delete a group through `PATCH /api/scene`, create an edge, delete that edge, reload the scene, and observe persisted scene version/selection/state.
- The render snapshot comparison route was verified through the POC proxy after the same backend mutations; render card/edge keys still exclude business-only fields such as `confidence`.
- Export compatibility is covered by tests proving deterministic graph/export helpers still read app `Scene`, not render snapshots.

## Not Ready Yet

- Product-quality text shaping/cache, real font fallback, and Korean glyph raster quality are not implemented in Rust/wgpu. The bitmap path now wraps by grapheme/display width and keeps CJK/Korean content visible through fallback glyph quads, but it still cannot shape or rasterize real Korean glyphs.
- Visible WebGPU now accepts compact render patches, writes dirty card/edge/group slots for move/edit/select patches, uses spare group/card/edge slots for create/delete patches, grows/compacts group/card/edge slot capacity, and hit-tests cards, text, ports, cubic edges, and groups, but richer product card/edge styling, DOM overlay alignment/IME behavior, and browser interaction proof are still POC-level.
- The POC API-backed real scene path is terminal-verified through the proxy for drag/text/edge-equivalent patches, but not rendered browser-interaction verified in this run.
- MCP-driven semantics and production-polished app shell affordances still live only in the current app path.
- Rendered visual QA screenshots are not captured in this run because localhost Browser validation is blocked in the current host by a crash-tab/data-URL navigation policy; this was rechecked after starting both POC and backend servers. Visual approval must happen before production replacement.

## Migration Scope

1. Harden the POC renderer: text shaping/cache and real font fallback beyond grapheme-aware bitmap fallback text, richer card/edge styling, broader style/layout dirty-range updates, and browser-verified DOM overlay alignment/IME behavior.
2. Browser-verify the API-backed real scene rendered load/save path with the backend running.
3. Browser-verify the render snapshot route and POC `Real scene` mode against the same persisted backend scene.
4. Port remaining production interactions in order: MCP/API-driven shell semantics and production app-shell polish after the rendered POC path is browser-verified.
5. Remove the DOM/SVG path only after visual QA, export/e2e coverage, and persisted workflow parity pass on the same real scene fixtures.

## Fallback Policy

The current DOM/SVG path should remain only as a temporary comparison and rollback path during migration. It should not become a long-lived alternate rendering mode after the new engine passes production parity.

## Recommended Next Task

Run the POC against the real backend in a working browser surface and verify `Real scene` rendered load/save interactions for group tag attach/filtering, comments, product export preview, group drag, card drag, text edit, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge create, and edge delete. After that, harden Rust/wgpu real text shaping/font fallback and visible WebGPU hit/overlay ownership.
