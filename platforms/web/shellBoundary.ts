// The seam between the product shell and the canvas engine: types and JSDoc only, no runtime logic.
// Shell→canvas is the imperative `RendererCanvasHostHandle`; canvas→shell is the `EngineEvent` stream.
//
// Boundary invariants: (1) the shell never draws a frame nor holds geometry/hit/LOD/layout as a source
// of truth — it reads them as events/FrameStats and commands via the handle; (2) the canvas never calls
// product I/O (no HTTP/WS, no business semantics) — persistence stays in the WS transport client; (3) all
// traffic goes through the handle / event stream, no back-channel through reactivity or shared state.

// Re-export the real event union from its canonical location.
export type { EngineEvent, FocusBoundsOptions } from "./renderer/engine";

import type { EngineEvent } from "./renderer/engine";
import type { RenderObjectScene } from "./renderer/scene";
import type { CameraState, WorldPoint, WorldRect } from "./shared/geometry";
import type { FocusBoundsOptions } from "./renderer/engine";

// The shell→canvas imperative command surface; the canonical surface lives on the framework-neutral
// `ShapeCanvasHost` class, this structural type is the stable boundary-contract name.
export type RendererCanvasHostHandle = {
  fitScene: () => void;
  focusBounds: (bounds: WorldRect, options?: FocusBoundsOptions) => void;
  wheelAtScreen: (screen: { x: number; y: number }, deltaY: number) => void;
  setCamera: (camera: CameraState) => void;
  loadObjectScene: (scene: RenderObjectScene) => void;
};

// The framework-neutral bridge the shell instantiates over the canvas engine. The bridge is already
// batched inside the engine; callers MUST route through these methods and MUST NOT call them per-reactive-tick.
export interface ShapeCanvasHost {
  // Attach the engine to three shell-owned DOM nodes: `inputCanvas` (pointer/keyboard target), `webGpuCanvas`
  // (render target), `overlayRoot` (a positioned div the engine mounts DOM text overlays into). Resolves when ready for loadScene.
  mount(inputCanvas: HTMLCanvasElement, webGpuCanvas: HTMLCanvasElement, overlayRoot: HTMLElement): Promise<void>;

  // Load (or reload) the full scene; the renderer is authoritative for camera/hit/selection after this call.
  // Edits do NOT flow here — they are object ops applied by the scene-core wasm; the shell re-loads the result.
  loadObjectScene(scene: RenderObjectScene): void;

  setCamera(camera: CameraState): void;
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;
  // Fit all visible scene objects into the viewport.
  fitScene(): void;
  // Synthesise a wheel-zoom event at a screen-space point.
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;

  // Register the single callback for all canvas→shell events (subsequent calls replace it). The shell
  // routes each variant: stats → mirror camera/stats (derived), selection/patch → persist via the WS
  // client, overlay → mount/unmount the DOM textarea, gesture → gate deferred loads, status → status bar.
  onEvent(callback: (event: EngineEvent) => void): void;

  // Stop the render loop and remove all event listeners.
  destroy(): void;
}

// Document shell state — fields part of the persistent scene that MUST survive a shell swap or reload.
// They flow through the op model / WS persistence path and live in the shell + server, NOT in the core.
export interface DocumentShellState {
  // The full scene graph; loaded from the WS welcome snapshot, mutated through the WS transport client.
  scene: unknown; // typed as Scene in the shell — `unknown` here avoids a circular dep
  // Active tag filter, applied client-side over the WS-held scene; not a top-level scene field.
  activeTagIds: string[];
  // The current selection; broadcast as presence on every change (selection is not a document op).
  selection: unknown;
}

// Ephemeral shell state — UI/session-only fields, intentionally NOT persisted and never entering the op model.
export interface EphemeralShellState {
  // The group the user is "currently inside" for group-scoped operations.
  currentGroupId: string | undefined;
  // Whether a long-running server operation is in flight (drives busy indicators).
  busy: boolean;
  status: string;
  diagnosticsOpen: boolean;
  groupPanelOpen: boolean;
  templatePickerOpen: boolean;
  // The node currently open in inline edit mode (DOM overlay textarea active).
  editingNodeId: string | null;
  nodeMenu: { nodeId: string; x: number; y: number } | null;
  // The last node copied (paste); local clipboard only, not synced.
  copiedNode: unknown;
  exportPreview: unknown;
  // Camera mirror — a derived read of FrameStats; the engine is authoritative. MUST NOT be a write-source; use `setCamera`.
  camera: CameraState;
  // Read-only in the shell; never written back to the engine.
  rendererStats: unknown;
  rendererHealth: unknown;
  rendererStatus: string;

  // Net-new slots, not yet implemented.
  mcpClientPresence: unknown;
  companionDockState: unknown;
  // Follow drives camera through `setCamera`/`focusBounds`; never owns camera state itself.
  spectatorState: unknown;
}
