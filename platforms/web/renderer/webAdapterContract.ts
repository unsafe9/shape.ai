/**
 * T1.2 — Web Adapter Contract
 *
 * Declares the seven owned surfaces of the web (WebGPU/WASM) adapter as a
 * shell-neutral TypeScript contract.  No behavior is added; this file only
 * names and documents the boundary so that:
 *
 *  - the adapter layer (`engine.ts`, `wasmLoader.ts`, `RendererCanvasHost.tsx`)
 *    can be validated against a single source of truth, and
 *  - a future Svelte shell can drive the adapter through the same narrow API
 *    without touching React or re-implementing clipboard against business objects.
 *
 * Real symbols are imported from `engine.ts`, `wasmLoader.ts`, and `scene.ts`
 * so that TypeScript checks the contract is consistent with the implementation.
 * No new behavior is introduced and no existing exports are modified.
 *
 * ## The seven owned surfaces
 *
 *  1. WASM loading + capability probe           → {@link WebAdapterLoadContract}
 *  2. WebGPU surface lifecycle                  → {@link WebAdapterSurfaceContract}
 *  3. Pointer / keyboard input bridge           → {@link WebAdapterInputContract}
 *  4. DOM text overlay (IME-capable)            → {@link WebAdapterOverlayContract}
 *  5. Browser clipboard + canvas keyboard       → {@link WebAdapterClipboardContract}
 *  6. Diagnostics bridge                        → {@link WebAdapterDiagnosticsContract}
 *  7. Shell integration API                     → {@link WebAdapterShellContract}
 *
 * ## Boundary invariants (binding Verify guards)
 *
 *  - The adapter does **not** define product templates.
 *    It speaks the object render view + object ops only (`RenderObjectScene`;
 *    the object substrate carries no business fields).
 *  - The adapter does **not** define canonical persistence rules.
 *    It relays ops via `{ type: "patch" }` events; the server (`storage.ts`) is
 *    the system of record.  T2.5 op metadata rides opaquely through the same
 *    JSON envelope — the adapter gains no new responsibility.
 *  - Shell-neutrality: the adapter imports no UI framework.
 *    `RendererCanvasHost.tsx` is a thin React *binding* over `ShapeCanvasEngine`;
 *    a Svelte binding instantiates `ShapeCanvasEngine` identically.
 */

// ─── real symbols ─────────────────────────────────────────────────────────────

import type {
  ShapeCanvasEngine,
  ShapeCanvasEngineOptions,
  EngineEvent,
  FocusBoundsOptions
} from "./engine";

import type {
  RustCoreStatus,
  RustWebGpuProbeReport,
  RustWebGpuRenderer,
  RustCanvasInputEvent,
  RustInputBatchResult,
  RustDebugSnapshot
} from "../bridge/wasmLoader";

import type {
  RenderObjectScene,
  SceneSelection,
  HitResult,
  DomOverlayRequest,
  FrameStats,
  CameraState,
  WorldPoint,
  WorldRect
} from "./scene";

import type { ObjectOp } from "../shared/object";

// ─── re-exports (convenience) ─────────────────────────────────────────────────

export type {
  ShapeCanvasEngine,
  ShapeCanvasEngineOptions,
  EngineEvent,
  FocusBoundsOptions,
  RustCoreStatus,
  RustWebGpuProbeReport,
  RustWebGpuRenderer,
  RustCanvasInputEvent,
  RustInputBatchResult,
  RustDebugSnapshot,
  RenderObjectScene,
  ObjectOp,
  SceneSelection,
  HitResult,
  DomOverlayRequest,
  FrameStats,
  CameraState,
  WorldPoint,
  WorldRect
};

// ─── Surface 1 — WASM loading + capability probe ──────────────────────────────

/**
 * Surface 1: WASM loading + capability probe.
 *
 * The adapter owns the dynamic `import()` of `shape_canvas_core.js`, awaiting
 * the `default()` init, resolving exports, and classifying capability/health.
 *
 * Implemented by: `loadRustCore()` in `wasmLoader.ts`.
 *
 * Contract rules:
 *  - `renderer_backend()` is treated as an opaque string; never branched on for
 *    behavior.
 *  - `probeWebGpu(canvas, w, h, dpr)` returns a read-only probe report; the
 *    adapter owns *when* to probe.
 *  - `ShapeWebGpuRenderer.create(canvas, w, h, dpr)` is the sole construction
 *    path for the live renderer handle.
 */
export type WebAdapterLoadContract = {
  /**
   * Load the Rust/WASM package and probe WASM availability.
   * Returns `RustCoreStatus` handed to the shell as `RendererHealth`.
   * Must be awaited once before any renderer call is made.
   */
  loadRustCore: () => Promise<RustCoreStatus>;

  /**
   * Optional pre-flight capability probe before allocating a renderer.
   * Provided by `RustCoreStatus.probeWebGpu` when WASM loaded successfully.
   * The `RustWebGpuProbeReport` is read-only evidence; the adapter never
   * branches on `format` or `presentMode` for behavior decisions.
   */
  probeWebGpu: RustCoreStatus["probeWebGpu"];
};

// ─── Surface 2 — WebGPU surface lifecycle ─────────────────────────────────────

/**
 * Surface 2: WebGPU surface lifecycle.
 *
 * The adapter owns the two canvas elements, `devicePixelRatio`, the
 * `ResizeObserver`, and create/resize/teardown of `ShapeWebGpuRenderer`.
 * The shell supplies only the mount container and CSS box.
 *
 * Implemented by: `RendererCanvasHost.tsx` + `ShapeCanvasEngine.resize()`.
 *
 * Health classification (adapter-owned):
 *  - `wasm-unavailable`    — import/init failed
 *  - `webgpu-unavailable`  — WASM ok but `ShapeWebGpuRenderer.create` rejected
 *  - `ready`               — renderer live
 *
 * The shell receives a single `RendererHealth` and never computes device pixels
 * or touches a `GPUCanvasContext`.
 *
 * Canvas identity:
 *  - `renderer-webgpu-canvas`  — GPU surface (WebGPU)
 *  - `renderer-input-canvas`   — pointer event target (adapter implementation
 *    detail; Metal adapter will not have it — the shell-neutral API stays
 *    agnostic to canvas count)
 */
export type WebAdapterSurfaceContract = {
  /** Current renderer health; emitted to the shell via `onHealthChange`. */
  readonly health: RendererHealth;

  /**
   * Notify the adapter of a new CSS layout size and DPR.
   * Drives `ShapeWebGpuRenderer.resize(width, height, dpr)` and keeps the
   * overlay `<textarea>` aligned via `updateOverlayPosition()`.
   * Implemented by `ShapeCanvasEngine.resize()`.
   */
  resize(width: number, height: number, dpr?: number): void;
};

/**
 * Adapter-owned health classification.
 * Identical in shape to `RendererHealth` in `RendererCanvasHost.tsx`;
 * restated here shell-neutrally.
 */
export type RendererHealth = {
  /** Lifecycle state. The adapter is the only layer that distinguishes them. */
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  /** Human-readable detail for diagnostics. */
  detail: string;
  /** True when the WASM package loaded and `ShapeWebGpuRenderer` is present. */
  rustAvailable: boolean;
  /** Opaque string from `renderer_backend()`; never branched on. */
  rustBackend: string;
  /** True when a live `RustWebGpuRenderer` handle exists. */
  webGpuRendererAvailable: boolean;
};

// ─── Surface 3 — Pointer / keyboard input bridge ──────────────────────────────

/**
 * Surface 3: Pointer / keyboard input bridge.
 *
 * The adapter normalises DOM `pointer`/`mouse`/`wheel`/`dblclick`/key events
 * to `RustCanvasInputEvent[]`, converts screen coordinates, manages pointer
 * capture, batches events, and provides a mouse fallback.
 *
 * Implemented by: `ShapeCanvasEngine.bindInput()` / event handlers +
 * `sendInputBatch()` → `RustWebGpuRenderer.inputBatch(eventsJson)`.
 *
 * Contract rules:
 *  - The core receives only `RustCanvasInputEvent[]` serialised as JSON; no DOM
 *    types cross the WASM boundary.
 *  - The `gesture` event (`EngineEvent { type: "gesture" }`) signals active
 *    drag to the shell so React/Svelte can suppress scene re-sync mid-drag
 *    (see commit `5d3dc31`).
 *  - Mouse fallback (`window mousemove/mouseup`) handles browsers where
 *    `PointerEvent` capture is unreliable; it is an adapter implementation
 *    detail invisible to the shell.
 */
export type WebAdapterInputContract = {
  /**
   * Dispatch an externally synthesised wheel event (e.g. from a minimap).
   * Implemented by `ShapeCanvasEngine.wheelAtScreen()`.
   */
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;

  /**
   * Jump the camera to fit the given world bounds.
   * Implemented by `ShapeCanvasEngine.focusBounds()`.
   */
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;

  /**
   * Fit all scene objects into view.
   * Implemented by `ShapeCanvasEngine.fitScene()`.
   */
  fitScene(): void;
};

// ─── Surface 4 — DOM text overlay (IME-capable) ───────────────────────────────

/**
 * Surface 4: DOM text overlay (IME-capable).
 *
 * The adapter mounts, positions, and styles a `<textarea>` from a
 * `DomOverlayRequest`, handles browser IME composition, and commits or cancels
 * the edit by emitting an `edit-card-text` op.
 *
 * Implemented by: `ShapeCanvasEngine.mountOverlay()` / `removeOverlay()` /
 * `updateOverlayPosition()` / `commitTextEdit()`.
 *
 * Contract rules:
 *  - `overlayRequest(cardId, field)` is called each frame to keep the
 *    `<textarea>` glued to the moving card.
 *  - IME composition events (`event.isComposing`) are guarded; Enter/Escape are
 *    only processed outside a composition sequence.
 *  - When the overlay is committed, the adapter emits a `set-text` `ObjectOp`
 *    via the shell integration event stream.
 *  - The overlay is never mounted when `webGpuRenderer` is null; a
 *    `{ type: "overlay", request: null }` event is emitted on cancel.
 */
export type WebAdapterOverlayContract = {
  /**
   * Begin a text-edit session for the hit result's field.
   * Returns the initial `DomOverlayRequest` or `null` if the hit is not
   * a text field or the renderer is unavailable.
   */
  beginTextEdit(hit: HitResult): DomOverlayRequest | null;

  /**
   * Commit the current overlay value as an `edit-card-text` op and remove
   * the `<textarea>` from the DOM.
   */
  commitTextEdit(): void;
};

// ─── Surface 5 — Browser clipboard + canvas keyboard ─────────────────────────

/**
 * Surface 5: Browser clipboard + canvas keyboard (the corrected boundary).
 *
 * The split rule: **acts on a canvas primitive via an op → adapter; acts on
 * business semantics or shell chrome → shell.**
 *
 * Canvas-scoped behaviours claimed by the adapter:
 *  - Copy / paste / duplicate of the selected primitive → `create-card` /
 *    `create-edge` ops; clipboard payload is the render primitive, not the
 *    decision-graph `SceneNode`.
 *  - Delete of the selected primitive → `delete-card` / `delete-edge` /
 *    `delete-group` op.
 *  - Enter-to-edit / Escape / Cmd-Enter commit → opens / commits overlay →
 *    `edit-card-text` op.
 *  - IME composition during text edit → native `<textarea>` composition;
 *    never split per primitive type.
 *  - OS clipboard transport (`navigator.clipboard`) for canvas-scoped read /
 *    write.
 *
 * Behaviours that remain in the shell / template layer (NOT adapter):
 *  - `formatNodeMarkdown` / business markdown export — template-flavoured
 *    serialiser keyed off `nodeType`/`confidence`/`evidenceRefs`.
 *  - Shell shortcuts (toggle diagnostics, panels, export drawer).
 *  - Non-canvas clipboard content.
 *
 * Code ownership note: the actual code move is owned by T6.2 (React→Svelte) /
 * T6.3 (Rust boundary).  This contract fixes the *ownership boundary* so the
 * Svelte migration does not re-implement clipboard against the business
 * `SceneNode` shape.
 *
 * The `CanvasClipboardPayload` below is the adapter's clipboard type — it
 * carries only render primitives, never business fields.
 */
export type WebAdapterClipboardContract = {
  /**
   * Write the currently selected render primitive to the OS clipboard.
   * Uses `navigator.clipboard` or `execCommand` as a fallback.
   * The payload is a `CanvasClipboardPayload` — no `SceneNode` fields.
   */
  copySelection(): Promise<void>;

  /**
   * Read a `CanvasClipboardPayload` from the OS clipboard and emit the
   * appropriate `create-card` / `create-edge` op.
   */
  pasteFromClipboard(): Promise<void>;

  /**
   * Emit a `delete-card` / `delete-edge` / `delete-group` op for the current
   * selection.
   */
  deleteSelection(): void;

  /**
   * Duplicate the current selection: emit `create-card` / `create-edge` ops
   * with new ids, offset by a fixed delta.
   */
  duplicateSelection(): void;
};

/**
 * The adapter's clipboard wire format.
 * Carries only canvas objects (`src/shared/object.ts` `Object`); no business
 * fields (the object substrate has none) cross the clipboard.
 */
export type CanvasClipboardPayload = {
  kind: "objects";
  objects: import("../shared/object").Object[];
};

// ─── Surface 6 — Diagnostics bridge ──────────────────────────────────────────

/**
 * Surface 6: Diagnostics bridge.
 *
 * The adapter produces a plain data snapshot per frame; the shell renders the
 * diagnostics drawer (a leaf consumer with no callbacks that mutate canvas
 * state).
 *
 * Implemented by: `ShapeCanvasEngine.renderFrame()` / `debugSnapshot()` →
 * `EngineEvent { type: "stats" }`.
 *
 * Contract rules:
 *  - The adapter never imports a UI framework to surface diagnostics.
 *  - `FrameStats` and `RendererHealth` are the only types the adapter exposes
 *    for diagnostic purposes.
 *  - `RustWebGpuFrameStats` and `RustDebugSnapshot` fields are copied verbatim
 *    into `FrameStats` (the `rust*` prefix fields); the adapter never
 *    interprets them.
 *  - The Svelte diagnostics drawer will be a Svelte component over the
 *    identical `FrameStats` snapshot — no React dependency.
 */
export type WebAdapterDiagnosticsContract = {
  /**
   * Latest per-frame stats snapshot.  Emitted to the shell via
   * `EngineEvent { type: "stats" }` after every `renderFrame()` call.
   */
  readonly lastFrameStats: FrameStats | null;

  /**
   * Latest raw debug snapshot from the Rust core.
   * Null when `RustWebGpuRenderer` is unavailable.
   * Accessible via `ShapeCanvasEngine.debugSnapshot()`.
   */
  readonly lastDebugSnapshot: RustDebugSnapshot | null;
};

// ─── Surface 7 — Shell integration API ───────────────────────────────────────

/**
 * Surface 7: Shell integration API — shell-neutral, Svelte-ready.
 *
 * The narrow imperative API that the shell calls into the adapter (in), and the
 * typed event stream the adapter emits out.  No shell framework is imported by
 * the adapter; this API is the only coupling between the two layers.
 *
 * Implemented by: `ShapeCanvasEngine` (imperative methods) + `EngineEvent`
 * (event union emitted via `onEvent` callback).
 *
 * Rules:
 *  - All adapter→shell communication is plain data via `onEvent`; all
 *    shell→adapter via the imperative API.
 *  - No shell reactivity reaches into canvas state (performance-sensitive
 *    canvas state lives in Rust/core, not component state).
 *  - The `gesture` event exists so the shell suppresses scene re-sync during a
 *    drag (commit `5d3dc31`); this is an adapter responsibility.
 *
 * `RendererCanvasHostHandle` in `RendererCanvasHost.tsx` is the current React
 * binding of this surface; the Svelte binding will instantiate
 * `ShapeCanvasEngine` with `{ canvas, overlayRoot, onEvent }` identically.
 */
export type WebAdapterShellContract = {
  /**
   * Load (or reload) the scene into the renderer. The shell projects its
   * canonical `ObjectScene` (D1) into the renderer-core `RenderObjectScene`
   * render view; `activeTagIds` controls which objects are visible.
   */
  loadObjectScene(scene: RenderObjectScene, activeTagIds: string[]): void;

  /**
   * Synchronise the selection highlight without re-loading the scene.
   * No-ops when the selection is already identical to the current scene.
   */
  syncSelection(selection: SceneSelection): void;

  /**
   * Teleport the camera to an exact state.
   * Implemented by `ShapeCanvasEngine.setCamera()` → `inputBatch`.
   */
  setCamera(camera: CameraState): void;

  /**
   * Fit all scene objects into view.
   */
  fitScene(): void;

  /**
   * Animate the camera to frame the given world rect.
   */
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;

  /**
   * Programmatic wheel event at a screen point (e.g. from a minimap overlay).
   */
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;

  /**
   * Typed event stream from adapter to shell.
   * Delivered via the `onEvent` callback passed to `ShapeCanvasEngineOptions`.
   *
   * | Event type   | Payload                          | Shell responsibility                         |
   * | ------------ | -------------------------------- | -------------------------------------------- |
   * | `"stats"`    | `FrameStats`                     | Diagnostics drawer + camera echo             |
   * | `"selection"`| `HitResult \| null`              | Shell selection state                        |
   * | `"patch"`    | `ObjectOp` + `errors`            | Persist via the WS transport client          |
   * | `"overlay"`  | `DomOverlayRequest \| null`      | Open/closed indicator                        |
   * | `"gesture"`  | `active: boolean`                | Suppress React/Svelte re-sync during drag    |
   * | `"status"`   | `message: string`                | Status bar / toast                           |
   */
  readonly onEvent: (event: EngineEvent) => void;
};

// ─── Adapter-core call boundary ───────────────────────────────────────────────

/**
 * The complete adapter↔core call boundary — the only surface the shell never
 * touches.  Already typed as `RustWebGpuRenderer` in `wasmLoader.ts`; restated
 * here for cross-reference and to document which adapter surface owns each call.
 *
 * | Method               | Called by surface |
 * | -------------------- | ----------------- |
 * | `resize`             | 2 — WebGPU surface lifecycle |
 * | `loadScene`          | 7 — shell integration (load) |
 * | `applyPatchBatch`    | 7 — shell integration (edit) |
 * | `renderFrame`        | 6 — diagnostics bridge       |
 * | `inputBatch`         | 3 — input bridge             |
 * | `overlayRequest`     | 4 — DOM text overlay         |
 * | `debugSnapshot`      | 6 — diagnostics bridge       |
 */
export type AdapterCoreCallBoundary = RustWebGpuRenderer;

// ─── Scene / op transport contract ───────────────────────────────────────────

/**
 * The adapter is the **serialisation boundary** between TS document/op contracts
 * and the WASM core.
 *
 * | Direction    | TS type                        | Wire        | Core entry          |
 * | ------------ | ------------------------------ | ----------- | ------------------- |
 * | Load scene   | `RenderObjectScene`            | JSON string | object draw entry   |
 * | Apply edit   | `ObjectOp` / `ObjectOp[]`      | JSON string | scene-core wasm     |
 * | Input        | `RustCanvasInputEvent[]`       | JSON string | `inputBatch`        |
 * | Overlay rect | `(objectId, field)`            | call args   | `overlayRequest`    |
 * | Frame stats  | —                              | struct      | `renderFrame`       |
 * | Debug        | —                              | struct      | `debugSnapshot`     |
 *
 * Contract rules:
 *  a. The adapter passes the **object render view** (`RenderObjectScene`, the
 *     object substrate has no business fields), never the full app `Scene`.
 *  b. The adapter does not invent op kinds — edits are `ObjectOp`s applied by the
 *     scene-core wasm (the single op-apply source of truth).
 *  c. Operation metadata (`operationId`, `actorId`, `targetIds`, `baseRevision`,
 *     …) rides **inside** the op/JSON envelope — the adapter relays it opaquely
 *     and gains no new responsibility.
 */
export type AdapterSceneTransportContract = {
  /** Serialise and forward a `RenderObjectScene` to the renderer object draw entry. */
  loadObjectScene(scene: RenderObjectScene): void;
  /** Serialise and forward one or more `ObjectOp`s to the scene-core wasm. */
  applyObjectOps(ops: ObjectOp[]): string[];
  /** Serialise and forward input events to the core. */
  sendInputBatch(events: RustCanvasInputEvent[]): RustInputBatchResult | null;
};
