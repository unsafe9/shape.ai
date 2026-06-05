# Phase P1: Portable Canvas Core Boundary

> Reviewer-facing phase summary. Detail lives in the task fragments under `tasks/`; this doc surfaces the contracts, audits the phase verifies, and frames the review gate. Design-only: no production code is written, moved, or scaffolded in P1.

## Goal / Why now

Make the canvas engine architecture portable across web, macOS, and future iOS by drawing a stable core/adapter/shell boundary now. The in-flight Rust/WebGPU replacement must not hard-code web-only assumptions into the core scene model before that boundary is agreed.

## Decisions at a glance

The four tasks converge on one shared vocabulary and one boundary. Deduped key contracts:

- **The split is core / web-adapter / Metal-adapter / app-shell.** Performance-sensitive canvas behavior is core; browser/native I/O is the platform adapter; product/UI/persistence is the shell. (`tasks/T1.1.md`, `tasks/T1.2.md`, `tasks/T1.3.md`, `tasks/T1.4.md`)
- **The core already exists** as the Rust crate `shape_canvas_core`; P1 fixes its *contract*, not its code. The web-coupling is narrow and locatable (`HtmlCanvasElement`, `Backends::BROWSER_WEBGPU`, `SurfaceTarget::Canvas`), so the boundary is a *promotion* of the shipping `ShapeWebGpuRenderer` surface, not a parallel model. (`tasks/T1.1.md` §1, `tasks/T1.3.md` §0)
- **Shared payloads (the wire vocabulary).** `SceneSnapshot` (load), `RenderScenePatch` (op application), `CanvasInputEvent` (input), `CoreHitResult` (hit testing), `CoreOverlayRequest`/`CoreOverlayStyle` (active-edit overlay geometry+typography), `WebGpuFrameStats`/`WebGpuDebugSnapshot` (stats). All are plain serde types with no DOM/`JsValue` coupling — the structural guarantee that web and Metal share the same scene model. (`tasks/T1.1.md` §5, `tasks/T1.3.md` §0)
- **The 8 core capabilities** the breakdown enumerates each map 1:1 to an existing `ShapeWebGpuRenderer` method: `load_scene`, `apply_op_batch`, `set_camera`, `hit_test`, `selection_geometry`, `render_frame`, `frame_stats`/`debug_snapshot`, plus `input_batch`/`overlay_request`/`resize` lifecycle. (`tasks/T1.1.md` §2-§3)
- **The hard line (책임 경계).** Core owns the *Canvas scene* half: id, bounds/transform/z-order, style key, text runs, ports, edge route, hit regions, selection geometry, render-cache keys, camera math, culling, LOD thresholding, text layout. The shell keeps the *Business document* half: node business meaning, status/confidence, comments rules, export/proposal/MCP workflow. `node_type`/`status` stay opaque `String` the core feeds only into `style_key`. (`tasks/T1.1.md` §1, all tasks' Stop-or-ask)
- **One bridge, two directions.** Shell↔canvas traffic flows only through an imperative handle in (`RendererCanvasHostHandle`-shaped: `loadScene`/`applyPatch`/`setCamera`/`focusBounds`/`fitScene`/`getSnapshot`) and an event stream out (`EngineEvent`: `stats`/`selection`/`patch`/`overlay`/`gesture`/`status`). No back-channel through component reactivity. This pair is already framework-neutral, so the Svelte migration wraps the same adapter. (`tasks/T1.2.md` §7, `tasks/T1.4.md` §2)
- **Document vs ephemeral split.** Document = `SceneSnapshot` geometry/text/style/z-order/selection, mutated only via the typed op vocabulary. Ephemeral = camera/viewport, hover, in-flight gesture, follow/companion animation, render caches — owned transiently by core/adapter/shell, never persisted. `CameraState` already lives outside the persisted `Scene` schema. (`tasks/T1.1.md` §4, `tasks/T1.4.md` §1)
- **T2.5 op metadata is an envelope, not a core change.** `operationId`/`actorId`/`targetIds`/`timestamp`/`baseRevision` ride *around* the op vocabulary; core and adapter relay them opaquely and never read them, keeping the core MCP/actor-free. `actor_marker` (P5) is an adapter/shell overlay, not core scene state. (`tasks/T1.1.md` §5, `tasks/T1.2.md` §4)

## Per-task deliverable summaries

**T1.1 — Platform-neutral core contract.** Lifts the contract out of the WASM-only binding layer so it reads as a host-neutral `CanvasCore` trait that Metal can also implement. Core contract = scene object identity (`id`-keyed, never positional), geometry/bounds (`WorldRect` on every object), camera math, hit testing, typed op application, selection geometry, culled render-primitive stream, debug stats. The WASM `ShapeWebGpuRenderer` is this trait + wasm-bindgen glue; `model.rs`/`stats.rs` are already pure serde types. **Core contract:** no `web-sys`/`HtmlCanvasElement`/`JsValue` in the trait or model, and never absorbs MCP/API/export/business semantics. (`tasks/T1.1.md`)

**T1.2 — Web adapter contract.** Names the seven browser-host surfaces the existing `src/client/renderer/` already implements: (1) WASM load + capability probe, (2) WebGPU surface lifecycle (canvases, DPR, resize, context-loss), (3) pointer/keyboard→`inputBatch` bridge, (4) DOM `<textarea>` overlay with browser IME, (5) browser clipboard + canvas-scoped keyboard, (6) diagnostics data bridge (shell renders the drawer), (7) shell-neutral imperative+event API. **Core contract:** the adapter is the JSON serialization boundary over the *primitive* render snapshot + op union; it owns no product templates and no canonical persistence. One boundary correction — canvas-scoped clipboard/keyboard moves conceptually from `App.tsx` (which acts on business `SceneNode`s) into the adapter; the template-flavored `formatNodeMarkdown` stays in the shell. (`tasks/T1.2.md`)

**T1.3 — Metal adapter contract.** Splits the single `ShapeWebGpuRenderer` struct into a portable `SceneRenderer` (scene/camera/text/stats — reused verbatim) plus a thin per-platform surface/input/text bridge. Metal retargets only: surface (`SurfaceTarget::Canvas`→`CAMetalLayer`/`MTKView`), backend (`BROWSER_WEBGPU`→`METAL`), input (`NSEvent`/`UITouch`→`CanvasInputEvent`), native text overlay (`NSTextView`/`UITextView` styled from the same `CoreOverlayStyle`), display scale (`backingScaleFactor`/`nativeScale`→ same `resize` path). wgpu already abstracts Metal, so the WGSL shader and draw loop are reused unchanged. **Core contract:** the same `SceneSnapshot`/`RenderScenePatch`/`CanvasInputEvent` drive Metal with no fork of the semantic scene model; the Metal adapter is a *sibling* of the web adapter over a shared core, not a second renderer. (`tasks/T1.3.md`)

**T1.4 — App shell boundaries.** Separates product-shell responsibility (UI composition + event routing + API/MCP/persistence orchestration) from canvas-engine responsibility, and fixes the web shell as a **Svelte shell** (React/React-DOM removed, no dual/compat shell). One contract describes the current React shell, the target Svelte shell, and a future macOS/SwiftUI shell — only language + adapter change. Shell owns: account/session, MCP connection state, persistence/`lib/api.ts`, panels/inspector/template-picker/export-drawer/companion-dock/spectator-controls, diagnostics host, overlay host. **Core contract:** the shell never draws a frame or owns geometry/hit/LOD/text/cache as source of truth; it commands via the handle and reads via `EngineEvent`. The React↔canvas seam is already narrow and imperative, so Svelte wraps the same adapter rather than rewriting the engine. (`tasks/T1.4.md`)

## Phase verify-or-evaluate audit

| Phase "Verify or evaluate" bullet | Status | Evidence |
| --- | --- | --- |
| Core has no browser DOM assumption. | **PARTIAL** | The render/input/hit/overlay/stats *logic and payloads* carry no DOM assumption: `model.rs`/`stats.rs` are pure serde types and the only `web_sys` import sits in the wasm-bindgen shell (`tasks/T1.1.md` §3 MET; `tasks/T1.2.md` Stop-or-ask CLEAR). But the core crate still *physically* links `web_sys`/`HtmlCanvasElement` into `ShapeWebGpuRenderer` via the `wgpu-probe` feature, so the design *names* the exact extraction (portable `SceneRenderer` vs per-platform adapter, gated by a `metal-adapter` feature) rather than *proving* DOM-freedom in code — that extraction is owned by P6/T6.3 (`tasks/T1.3.md` Verify: PARTIAL). The gap is execution-only; the contract is fully specified. |
| Web and Metal adapters can share scene snapshots, operations, hit testing contracts, and LOD policy. | **MET** | Shared `SceneSnapshot`, `RenderScenePatch`/`CanvasInputEvent` op+input vocabulary, `CoreHitResult` hit contract, and `SceneStyleToken.states.compact` LOD seed — all plain serde, no web coupling (`tasks/T1.1.md` §2/§5 MET; `tasks/T1.3.md` §0/§4/§7 MET, Metal inherits T3.1 LOD with no adapter branch; `tasks/T1.2.md` §4 JSON transport). |
| Product shell retains API/MCP/business state. | **MET** | The 책임 경계 line holds: business-document fields, persistence (`lib/api.ts` + server), and MCP/export/proposal workflow stay in the shell; `node_type`/`status` stay opaque in core; T2.5 op metadata is an envelope the core never reads (`tasks/T1.1.md` §1/§4 MET; `tasks/T1.4.md` §1/§5 + Stop-or-ask CLEAR; `tasks/T1.3.md` §8). |

Net: 2 MET, 1 PARTIAL. The single PARTIAL is a deliberate design/execution boundary — P1 is a design-only pass, the DOM-free core is fully specified, and physically extracting `web_sys` out of the crate is sequenced to P6/T6.3. No verify is a GAP.

## Review gate

`human-decision`: **approve the core/adapter split before production migration starts.**

Recommended boundary, as specified across T1.1–T1.4:

- **Core (`shape_canvas_core`, Rust):** scene object identity + geometry/bounds, camera math, hit testing, typed op application, selection/hull geometry, LOD thresholding, culling/spatial index/render caches, text layout/shaping, render-primitive generation, frame/debug stats. I/O is plain serde (`SceneSnapshot`, `RenderScenePatch`, `CanvasInputEvent`, `CoreHitResult`, `CoreOverlayRequest`, `WebGpuFrameStats`). No `web-sys`, no MCP/business semantics.
- **Web adapter (`src/client/renderer/`, TS):** WASM load/probe, WebGPU surface lifecycle, pointer/keyboard→`inputBatch`, DOM text overlay + browser IME, browser clipboard + canvas keyboard, JSON serialization boundary, diagnostics snapshot, shell-neutral handle/event API. No templates, no canonical persistence.
- **Metal adapter (native, behind a `metal-adapter` cargo feature):** `CAMetalLayer`/`MTKView` surface, `Backends::METAL`, native text overlay (`NSTextView`/`UITextView`), `NSEvent`/`UITouch`→`CanvasInputEvent`, native backing-scale, Metal capability probe + native telemetry. A sibling of the web adapter over the shared `SceneRenderer`, not a second renderer.
- **App shell (web = Svelte target, future macOS = SwiftUI):** account/session, MCP connection state, persistence/API orchestration, panels/inspector/template-picker/export-drawer/companion-dock/spectator-controls, diagnostics host, overlay host. Commands the canvas only through the imperative handle; reads only through `EngineEvent`. React/React-DOM removed; no dual shell.

What the gate is really approving: that this split (a) lets the app shell be replaced per platform while core stays stable, (b) lets web and Metal adapters be siblings over one core, and (c) keeps performance-sensitive canvas work in Rust/core rather than leaking into Svelte component state. Approval unblocks P2/P3/P5 (which depend on these contracts) and the P6 migration that physically extracts the DOM-free core.

## Stop-conditions encountered

**None triggered.** Every task's stop-or-ask condition is CLEAR:

- T1.1 — core never starts absorbing MCP/API/export/business semantics (bounded to the Canvas-scene half; op metadata is an envelope).
- T1.2 — no web-only behavior becomes required by the core scene model (all browser specifics stay in the adapter, converted to core-neutral inputs).
- T1.3 — no native feature forces a fork of the semantic scene model (every native need maps onto an existing core payload; Metal-only telemetry is additive adapter data).
- T1.4 — no business logic moves into adapters, and no performance-sensitive canvas behavior moves into Svelte component state.

Cross-cutting note carried by every task (not a stop-condition, but a logged risk): the upstream T0.2/T0.3 fragments (package boundaries, target layout) do not yet exist in-repo — only `tasks/T0.1.md` does. All four P1 tasks are therefore deliberately *directory-agnostic*, specifying responsibility boundaries rather than file paths and deferring physical placement to T0.2/T0.3. This is consistent with each task's mandated "independent of its final directory" framing and does not block the boundary decision.
