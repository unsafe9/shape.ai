# Source Architecture Deep Dive

## Conclusion

The POC should keep the Graphite/Figma lesson of owning the canvas engine boundary, while avoiding their full editor complexity. Vello/wgpu are useful references for scene/device/cache lifecycle, but the current POC keeps the app boundary narrow and does not adopt Vello or CanvasKit as the primary renderer. HTML-in-Canvas remains explicitly out of foundation scope because it is still experimental.

## Applied Patterns

- Graphite-style wrapper boundary: one WASM wrapper exposes callable backend functions and sends compact messages/events back to the web shell.
- Web shell owns browser APIs: canvas element lifecycle, ResizeObserver, pointer events, DOM edit overlay, inspector/chrome, and benchmark UI.
- Retained scene contract: stable ids, camera, renderable group/card/edge/text objects, and app-owned patches.
- Cache boundaries: edge routes, text wrapping, spatial index, and visible object queries are explicit POC subsystems.
- Text editing compromise: rendered text is canvas content; active editing is a DOM textarea overlay.

## Avoided Patterns

- Do not move comments, export/proposal state, MCP semantics, confidence/evidence business logic, or tag registry rules into Rust.
- Do not adopt Graphite-level editor backend complexity.
- Do not adopt CanvasKit/Skia as the product engine by default; keep it as quality/benchmark reference.
- Do not make HTML-in-Canvas the foundation.
- Do not make semantic LOD the core UX.

## Source Notes

1. Graphite root structure shows a large editor split across `frontend/`, `editor/`, and GPU-related packages: https://github.com/GraphiteEditor/Graphite
2. Graphite wrapper README documents a Rust WASM wrapper that exposes JS-callable backend functions and routes frontend messages back to JS: https://github.com/GraphiteEditor/Graphite/blob/master/frontend/wrapper/README.md
3. Graphite frontend README separates Svelte UI, browser API managers, stores, and subscriptions router from the Rust editor backend: https://github.com/GraphiteEditor/Graphite/blob/master/frontend/src/README.md
4. Graphite `wgpu-executor` source shows a GPU executor with wgpu context, texture cache, Vello renderer, resampling, and background compositor: https://github.com/GraphiteEditor/Graphite/blob/master/node-graph/libraries/wgpu-executor/src/lib.rs
5. Vello README describes a Rust GPU compute-centric 2D renderer using `wgpu`, with large-scene interactive goals and alpha caveats: https://github.com/linebender/vello
6. wgpu README describes a Rust graphics API based on WebGPU, with native backends and WebGPU/WebGL2 on wasm: https://github.com/gfx-rs/wgpu
7. Lyon README frames Lyon as path tessellation for GPU-based 2D rendering, not a full SVG renderer: https://github.com/nical/lyon
8. Kurbo README frames Kurbo as Rust 2D curves/path data structures and algorithms: https://github.com/linebender/kurbo
9. Peniko README frames Peniko as Rust 2D graphics style/brush/color data types layered on Kurbo/color: https://github.com/linebender/peniko
10. Swash README frames Swash as font introspection, shaping, and glyph rendering while leaving layout/composition to the application: https://github.com/dfrg/swash
11. Cosmic Text README frames Cosmic Text as pure Rust multi-line shaping, layout, rendering, fallback, and editing support: https://github.com/pop-os/cosmic-text
12. ThorVG README frames ThorVG as a lightweight vector graphics engine with retainable scene graph, shapes, text, images, effects, and multiple deployment targets: https://github.com/thorvg/thorvg
13. Pathfinder README frames Pathfinder as a GPU rasterizer for fonts/vector graphics and explicitly notes heavy development/incomplete areas: https://github.com/servo/pathfinder
14. Figma's WebGPU article describes a migration from WebGL to WebGPU, interface updates, compute shader opportunities, and render bundle opportunities: https://www.figma.com/blog/figma-rendering-powered-by-webgpu/
15. Figma's original web design tool article explains why HTML/SVG/2D canvas were insufficient for its retained infinite canvas goals and why it built a custom WebGL renderer: https://www.figma.com/blog/building-a-professional-design-tool-on-the-web/
16. Figma's WebAssembly article documents C++/WebAssembly loading and renderer context for a large browser design tool: https://www.figma.com/blog/webassembly-cut-figmas-load-time-by-3x/
17. CanvasKit docs describe Skia's WebAssembly build, WebGL-backed surface, and path/text API availability: https://docs.skia.org/docs/user/modules/canvaskit/
18. Chrome's HTML-in-Canvas origin trial article says the API is experimental in Chrome 148-150 and explains layout subtree, draw/update phases, and limitations: https://developer.chrome.com/blog/html-in-canvas-origin-trial
19. WICG HTML-in-Canvas explainer states it is a living proposal behind a Chromium flag and describes `layoutsubtree`, `drawElementImage`, WebGL/WebGPU equivalents, and synchronization: https://github.com/WICG/html-in-canvas
20. wasm-pack book documents the Rust/WASM package build tool used by the POC command: https://rustwasm.github.io/docs/wasm-pack/
21. wasm-bindgen guide documents Rust and JavaScript interop used by the POC Rust core: https://wasm-bindgen.github.io/wasm-bindgen/
22. MDN `devicePixelContentBoxSize` documents device-pixel canvas sizing and notes limited availability: https://developer.mozilla.org/en-US/docs/Web/API/ResizeObserverEntry/devicePixelContentBoxSize
23. web.dev's device-pixel-content-box article explains CSS pixels, DPR, and pixel-perfect canvas sizing: https://web.dev/articles/device-pixel-content-box
24. MDN Pointer Events documents pointer capture and the event model used by the POC input bridge: https://developer.mozilla.org/en-US/docs/Web/API/Pointer_events

## POC Decision

The implemented POC now uses Rust/WASM WebGPU as its only scene rendering path. The TypeScript web harness owns browser input, DOM overlay editing, app-scene controls, metrics, and benchmark orchestration, but it no longer carries a TypeScript Canvas2D scene renderer fallback. The Rust core exposes both a wgpu/WebGPU readiness probe and a visible WebGPU primitive renderer that uploads retained group/card primitives plus segmented cubic edge vertices and edge labels, consumes shared style token colors, builds padded viewport draw ranges, renders grapheme-aware bitmap title/summary/edge-label glyphs through `unicode-segmentation` and `unicode-width`, caches text line/wrap layout results, reports visible object/drawn vertex/draw range/CJK/fallback glyph/text-cache counts, applies compact render patches, writes fixed card/edge/group vertex slots for group translate/card move/edit/select dirty ranges, rebuilds card draw order for z-order patches, uses spare group slots for group create/delete dirty writes, grows and compacts group slots, uses spare card slots for card create/delete dirty writes, grows and compacts card slots, uses spare edge slots for edge create/delete dirty writes, grows exhausted edge slots, compacts delete-heavy edge/free-slot churn, performs card/text/port/cubic-edge/group hit testing, submits render passes, and presents frames. This is enough to prove the app boundary, retained scene contract, fixture scale, pointer batching, Rust-owned viewport culling, DOM edit overlay, benchmark path, group tag attach/filtering, comments/product export shell, copy/paste/duplicate/z-order app patch parity, WebGPU lifecycle boundary, visible WebGPU canvas ownership, and renderer-owned text snippet/style-token/hit-test/patch path without touching production canvas source. The remaining renderer-specific hardening is product-quality Rust/wgpu drawing: real text shaping and font fallback beyond the bitmap atlas, richer card/edge styling fidelity, real Korean glyph rendering, broader dirty-range updates, and browser-verified overlay behavior behind the same API.
