# Renderer Architecture Confirmation

## Decision

Primary production target remains custom Rust + wgpu/WebGPU. This POC does not change that decision.

## Current POC Implementation

Implemented:

- Isolated `poc/infinite-canvas/` workspace.
- Rust/WASM scene core scaffold in `core/`.
- Rust core code is split by responsibility: `lib.rs` keeps wasm exports/re-exports, `model.rs` owns scene and patch contracts, `stats.rs` owns report payloads, and `webgpu.rs` owns the visible WebGPU renderer/probe implementation.
- Rust `probeWebGpu` API that checks browser WebGPU support through `wgpu`, creates a canvas surface, requests an adapter/device, builds a default surface config, runs a clear render pass, submits it, and presents it. The web harness runs this against a detached probe canvas before creating the visible WebGPU renderer.
- Rust `ShapeWebGpuRenderer` API that owns a visible WebGPU canvas surface, uploads retained group/card primitives plus segmented cubic edge vertices, edge labels, and bitmap title/summary glyph quads, consumes shared style token colors, builds Rust-owned padded viewport draw ranges, wraps title/summary/edge-label text by grapheme/display width through `unicode-segmentation` and `unicode-width`, caches text line/wrap layout results, reports visible object, drawn vertex, draw range, CJK/fallback glyph, and text-cache counts, applies compact render patches, writes fixed card/edge/group vertex slots for group translate/card move/edit/select dirty ranges, rebuilds card draw order for z-order patches, draws selected group/card/edge outlines, uses spare group slots for dirty group create/delete updates, grows group slot capacity by inserting transparent slots before the edge/card segments, compacts delete-heavy group/free-slot churn by copying used groups and shifting edge/card segments into a smaller buffer, uses spare card slots for dirty card create/delete updates, grows card slot capacity by appending transparent slots, compacts delete-heavy card/free-slot churn by copying used cards into a smaller suffix, uses spare edge slots for dirty edge create/delete updates, grows edge slot capacity by copying GPU buffer segments and inserting new transparent edge slots before the card draw segment, compacts delete-heavy edge/free-slot churn by copying used edge and card segments into a smaller GPU buffer, performs card/text/port/cubic-edge/group hit testing, and submits render passes.
- Vite React web harness in `web/`.
- Canvas mount lifecycle with DPR resize.
- WebGPU-only scene rendering in the web harness when generated WASM and WebGPU are available.
- WebGPU readiness readout in the web harness contract panel.
- Retained scene snapshot with stable object ids.
- Group frame, card, text snippet, edge, and selection rendering.
- Camera pan/zoom/fit.
- Group and node creation/deletion controls through renderer patches.
- Rust-owned viewport culling, visible-object stats, and merged WebGPU draw ranges.
- WebGPU renderer metrics for vertices, glyphs, dirty writes, full rebuilds, and buffer slot churn.
- Scripted pan/zoom benchmark runner.
- DOM textarea edit overlay.
- Edge creation from source port to target port.

Not yet implemented:

- Product-quality Rust/wgpu draw backend.
- Production text shaping with Swash/Cosmic Text or an equivalent path. The bitmap path now wraps by grapheme/display width and caches line/wrap layout results, but still uses fallback bitmap glyphs for non-ASCII text.
- Richer product card styling beyond shared style-token colors, selected outlines, primitive filled rectangles, borders, and badges.
- Production-grade broader dirty-range invalidation beyond the fixed-slot move/edit/select, z-order rebuild, and group/card/edge slot growth/compaction POC paths.
- Rust-side font shaping, fallback, and glyph rasterization.

## Why This Is Still Useful

The highest-risk product boundary is not only the draw API. It is the split between app-owned business scene and renderer-owned retained scene, plus the input/patch/edit overlay flow. The POC makes that split executable and testable without modifying production source.

## Adopted Reference Patterns

- Graphite: WASM wrapper and web shell boundary.
- Figma: own the renderer interface and keep a retained, performance-aware graphics path.
- Vello/wgpu: treat GPU lifecycle, cache ownership, and browser target maturity as explicit risks.
- Lyon/Kurbo/Peniko/Swash/Cosmic Text: keep primitive/text building blocks modular instead of copying editor-wide complexity.

## Rejected Dependency Adoption

- Vello: reference only for now because Web target caveats and alpha status are still material.
- CanvasKit/Skia: benchmark/text/path quality reference only because it would shift ownership to a heavy non-Rust-first dependency.
- HTML-in-Canvas: not foundation because it is behind Chrome origin-trial/flag constraints and is still changing.

## Risks Carried Forward

- The current Rust/wgpu path is a style-token-aware primitive geometry, segmented cubic edge, grapheme-aware bitmap fallback text, and hit-test renderer, not product-quality final rendering.
- Bitmap text proves WebGPU-owned text snippets and edge labels, no-space/overlong wrapping, cacheable layout, and CJK visibility through fallback glyphs, but shaping, fallback fonts, kerning, and Korean glyph quality are not proven yet.
- GPU buffer updates use fixed slots for group translate/card move/edit/select dirty writes, rebuild card draw order for z-order patches, spare group slots for group create/delete dirty writes, dynamic group-slot growth/compaction, spare card slots for card create/delete dirty writes, dynamic card-slot growth/compaction, spare edge slots for edge create/delete dirty writes, dynamic edge-slot growth when spare slots are exhausted, and edge-slot compaction after delete-heavy churn.

## D1 Result

Proceed with the same scene/API shape, but keep production replacement blocked until browser real-scene verification and product-quality Rust/wgpu text/styling/buffer hardening pass the same fixture/benchmark/report loop.
