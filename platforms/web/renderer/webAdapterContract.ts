// The seven owned surfaces of the web (WebGPU/WASM) adapter as a shell-neutral TS contract. Real
// symbols are imported so TypeScript checks the contract against the implementation; no behavior is added.
//
// Boundary invariants: the adapter defines no product templates (it speaks `RenderObjectScene` + object
// ops only) and no persistence rules (it relays ops; the server is the system of record). It imports no
// UI framework — a shell binding instantiates `ShapeCanvasEngine` identically across React/Svelte.

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
  RustInputBatchResult
} from "../bridge/wasmLoader";

import type {
  RenderObjectScene,
  HitResult,
  DomOverlayRequest,
  FrameStats,
  CameraState,
  WorldPoint,
  WorldRect
} from "./scene";

import type { ObjectOp } from "../shared/object";

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
  RenderObjectScene,
  ObjectOp,
  HitResult,
  DomOverlayRequest,
  FrameStats,
  CameraState,
  WorldPoint,
  WorldRect
};

// Surface 1 — WASM loading + capability probe. Implemented by `loadRustCore()`. `renderer_backend()`
// is an opaque string, never branched on; `ShapeWebGpuRenderer.create` is the sole construction path.
export type WebAdapterLoadContract = {
  // Load the Rust/WASM package; returns `RustCoreStatus`. Must be awaited once before any renderer call.
  loadRustCore: () => Promise<RustCoreStatus>;
  // Optional pre-flight probe; read-only evidence, never branched on for behavior.
  probeWebGpu: RustCoreStatus["probeWebGpu"];
};

// Surface 2 — WebGPU surface lifecycle. The adapter owns the canvas elements, DPR, ResizeObserver,
// and renderer create/resize/teardown; the shell supplies only the mount container and receives a
// single `RendererHealth`, never touching device pixels or a `GPUCanvasContext`.
export type WebAdapterSurfaceContract = {
  readonly health: RendererHealth;
  // New CSS layout size + DPR; drives `ShapeWebGpuRenderer.resize` and keeps the overlay aligned.
  resize(width: number, height: number, dpr?: number): void;
};

export type RendererHealth = {
  // `wasm-unavailable` = import/init failed; `webgpu-unavailable` = WASM ok but create rejected; `ready` = live.
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  detail: string;
  rustAvailable: boolean;
  // Opaque string from `renderer_backend()`; never branched on.
  rustBackend: string;
  webGpuRendererAvailable: boolean;
};

// Surface 3 — Pointer / keyboard input bridge. The adapter normalises DOM events to
// `RustCanvasInputEvent[]` (no DOM types cross the WASM boundary), batches them, and provides a mouse
// fallback for unreliable PointerEvent capture. The `gesture` event lets the shell suppress re-sync mid-drag.
export type WebAdapterInputContract = {
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;
  fitScene(): void;
};

// Surface 4 — DOM text overlay (IME-capable). The adapter mounts/positions/styles a `<textarea>` from
// a `DomOverlayRequest`, guards IME composition (Enter/Escape only outside a composition sequence), and
// commits/cancels by emitting an op. Never mounted when the renderer is null.
export type WebAdapterOverlayContract = {
  // Returns the initial `DomOverlayRequest`, or null if the hit is not a text field / renderer unavailable.
  beginTextEdit(hit: HitResult): DomOverlayRequest | null;
  commitTextEdit(): void;
};

// Surface 5 — Browser clipboard + canvas keyboard. Split rule: acts on a canvas primitive via an op →
// adapter; acts on business semantics or shell chrome → shell. The clipboard payload carries only render
// primitives, never business fields. Markdown export, shell shortcuts, and non-canvas clipboard stay in the shell.
export type WebAdapterClipboardContract = {
  copySelection(): Promise<void>;
  pasteFromClipboard(): Promise<void>;
  deleteSelection(): void;
  duplicateSelection(): void;
};

// The adapter's clipboard wire format — only canvas objects, no business fields cross the clipboard.
export type CanvasClipboardPayload = {
  kind: "objects";
  objects: import("../shared/object").Object[];
};

// Surface 6 — Diagnostics bridge. The adapter produces a plain `FrameStats` snapshot per frame (imports
// no UI framework); `rust*`-prefix fields are copied verbatim from the wasm stats, never interpreted.
export type WebAdapterDiagnosticsContract = {
  readonly lastFrameStats: FrameStats | null;
};

// Surface 7 — Shell integration API (shell-neutral). The narrow imperative API the shell calls in, plus
// the typed `onEvent` stream the adapter emits out — the only coupling between the two layers. No shell
// reactivity reaches canvas state. The `gesture` event lets the shell suppress scene re-sync during a drag.
export type WebAdapterShellContract = {
  // `activeTagIds` controls which objects are visible.
  loadObjectScene(scene: RenderObjectScene, activeTagIds: string[]): void;
  setCamera(camera: CameraState): void;
  fitScene(): void;
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;
  // Typed adapter→shell event stream: "stats" (FrameStats), "selection" (HitResult|null), "patch"
  // (ObjectOp + errors, persisted by the shell), "overlay" (DomOverlayRequest|null), "gesture" (active
  // bool, suppresses re-sync), "status" (message string).
  readonly onEvent: (event: EngineEvent) => void;
};

// The complete adapter↔core call boundary, the only surface the shell never touches (typed as
// `RustWebGpuRenderer` in wasmLoader.ts; restated here for cross-reference).
export type AdapterCoreCallBoundary = RustWebGpuRenderer;

// The adapter is the serialisation boundary between TS op contracts and the WASM core: it passes the
// object render view (never the full app `Scene`), invents no op kinds (edits are `ObjectOp`s applied
// by the scene-core wasm), and relays op metadata (operationId, actorId, …) opaquely inside the envelope.
export type AdapterSceneTransportContract = {
  loadObjectScene(scene: RenderObjectScene): void;
  applyObjectOps(ops: ObjectOp[]): string[];
  sendInputBatch(events: RustCanvasInputEvent[]): RustInputBatchResult | null;
};
