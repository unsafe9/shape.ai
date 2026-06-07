/**
 * T1.4 — App Shell Boundary Contract
 *
 * This file is the single source of truth for the seam between the product shell
 * and the canvas engine.  It contains NO runtime logic — types and JSDoc only.
 *
 * ## Two directions, one bridge
 *
 * Shell → canvas (imperative handle):  {@link RendererCanvasHostHandle}
 * Canvas → shell (event stream):       {@link EngineEvent}
 *
 * Both symbols already exist in their original files.  This module re-exports them
 * under the framework-neutral {@link ShapeCanvasHost} interface so that:
 *   - The current React wrapper (`RendererCanvasHost.tsx`) can implement it.
 *   - A future Svelte host (T6.2) can implement the same interface without
 *     touching the adapter or Rust core.
 *   - A macOS/SwiftUI shell (T1.3 adapter) can satisfy the same contract in
 *     its own language without a fork of the scene model.
 *
 * ## Boundary invariants (binding for all downstream tasks)
 *
 * 1. **Shell never draws a frame.**  The shell holds no object geometry,
 *    visible-object set, hit region, LOD decision, text layout, or render cache
 *    as a source of truth.  It reads them as {@link EngineEvent} / `FrameStats`
 *    and commands via the handle.
 *
 * 2. **Canvas never calls product I/O.**  The adapter/core never call HTTP/WS,
 *    never know MCP/export/proposal semantics.  Persistence stays in the WS
 *    transport client (`src/client/lib/sceneClient.ts`) plus the residual
 *    server-side ops (`src/client/lib/sceneServerApi.ts`); business semantics
 *    stay in the Svelte shell + server.
 *
 * 3. **One bridge, two directions.**  All shell→canvas traffic goes through the
 *    {@link RendererCanvasHostHandle}-shaped methods; all canvas→shell traffic
 *    goes through {@link EngineEvent}.  No back-channel through component
 *    reactivity or shared mutable state.
 *
 * 4. **Per-platform shell, shared core.**  Swapping React→Svelte, or adding a
 *    macOS shell, changes only the shell rows in the responsibility table; the
 *    core scene model, op vocabulary, and adapter contracts are untouched.
 *
 * ## Document vs ephemeral state partition
 *
 * See {@link DocumentShellState} and {@link EphemeralShellState} for the
 * classification of every state field currently in `App.tsx`.
 */

// Re-export the real event union from its canonical location.
export type { EngineEvent, FocusBoundsOptions } from "./renderer/engine";

import type { EngineEvent } from "./renderer/engine";
import type { RenderObjectScene } from "./renderer/scene";
import type { CameraState, WorldPoint, WorldRect } from "../shared/geometry";
import type { FocusBoundsOptions } from "./renderer/engine";

/**
 * The shell→canvas imperative command surface. Originally exported by the React
 * `RendererCanvasHost.tsx`; after the Svelte cutover the canonical command
 * surface lives on the framework-neutral `ShapeCanvasHost` class
 * (`lib/canvasHost.ts`). This structural type is kept here as the stable
 * boundary-contract name downstream tasks reference.
 */
export type RendererCanvasHostHandle = {
  fitScene: () => void;
  focusBounds: (bounds: WorldRect, options?: FocusBoundsOptions) => void;
  wheelAtScreen: (screen: { x: number; y: number }, deltaY: number) => void;
  setCamera: (camera: CameraState) => void;
  loadObjectScene: (scene: RenderObjectScene) => void;
};

// ---------------------------------------------------------------------------
// Framework-neutral host interface
// ---------------------------------------------------------------------------

/**
 * The framework-neutral bridge the shell instantiates over the canvas engine.
 *
 * This is the strategy doc's `ShapeCanvas` JS API made concrete from real code.
 * It carries exactly the same methods as {@link RendererCanvasHostHandle} plus
 * lifecycle helpers (`mount`, `destroy`) and the event subscription point
 * (`onEvent`).  The React host (`RendererCanvasHost.tsx`) satisfies this
 * contract today via `forwardRef` / `useImperativeHandle`; the Svelte
 * replacement (T6.2) will satisfy it without React glue.
 *
 * ### Batching contract
 * The bridge is already batched inside the engine (`sendInputBatch` /
 * `applyPatchBatch`).  Callers MUST route through the handle methods and MUST
 * NOT call them per-reactive-tick.  Svelte reactive statements that would
 * re-derive canvas inputs on every keystroke violate this contract; route
 * through debounced handlers or the batched patch path instead.
 */
export interface ShapeCanvasHost {
  /**
   * Attach the engine to three DOM nodes created by the shell:
   *   - `inputCanvas`  — the HTML canvas that receives pointer/keyboard events.
   *   - `webGpuCanvas` — the WebGPU render target canvas (may be the same element
   *     on platforms that share the surface, but is kept separate today).
   *   - `overlayRoot`  — a positioned `<div>` the engine mounts DOM text overlays
   *     into during `beginTextEdit` (see IME/clipboard seam below).
   *
   * The shell creates and owns these DOM nodes; the engine owns their content
   * while mounted.  Resolves when the engine is ready to receive `loadScene`.
   */
  mount(inputCanvas: HTMLCanvasElement, webGpuCanvas: HTMLCanvasElement, overlayRoot: HTMLElement): Promise<void>;

  /**
   * Load (or reload) the full scene.  The shell projects its canonical
   * `ObjectScene` (D1) into the renderer-core `RenderObjectScene` render view
   * before handing off to the renderer's object draw entry. The renderer is
   * authoritative for camera, hit, and selection geometry after this call.
   *
   * Edits do NOT flow through this path — they are object ops applied by the
   * scene-core wasm (via the WS transport client); the shell re-loads the
   * resulting `ObjectScene` here.
   */
  loadObjectScene(scene: RenderObjectScene): void;

  /** Overwrite the camera directly (e.g. from follow/spectator — T5.3). */
  setCamera(camera: CameraState): void;

  /**
   * Animate the camera to bring `bounds` into view.
   * Used by the shell for group selection, node focus, and companion
   * `focusBounds` calls (T5.1 dock).
   */
  focusBounds(bounds: WorldRect, options?: FocusBoundsOptions): void;

  /** Fit all visible scene objects into the viewport. */
  fitScene(): void;

  /**
   * Synthesise a wheel-zoom event at a screen-space point.
   * Used by the shell zoom-in/zoom-out buttons (`App.tsx#zoomAtCanvasCenter`).
   */
  wheelAtScreen(screen: WorldPoint, deltaY: number): void;

  /**
   * Register the single callback that receives all canvas→shell events.
   * Only one callback is active at a time; subsequent calls replace the previous.
   *
   * The shell routes each {@link EngineEvent} variant as follows:
   *   - `stats`     → update `rendererStats` / `camera` mirror (derived, not authoritative).
   *   - `selection` → call `handleRendererSelection`, persist via the WS client.
   *   - `patch`     → call `handleRendererPatch`, save via the WS client (coalesced).
   *   - `overlay`   → mount/unmount the DOM `<textarea>` at coordinates the
   *                   engine computed (IME/clipboard seam — shell mounts the
   *                   DOM node, adapter computes target + coordinates).
   *   - `gesture`   → gate deferred scene loads and patch-save flushes.
   *   - `status`    → route to status bar / health indicator.
   */
  onEvent(callback: (event: EngineEvent) => void): void;

  /** Stop the render loop and remove all event listeners. */
  destroy(): void;
}

// ---------------------------------------------------------------------------
// Document vs ephemeral state partition
// ---------------------------------------------------------------------------

/**
 * Document shell state — fields that are part of the persistent scene and MUST
 * survive a shell swap (React → Svelte) or a page reload.
 *
 * These fields flow through the op model (T2.5) and/or the persistence path
 * (the WS transport client → `ops` envelope → per-canvas actor).  They must
 * NOT be inlined into Rust core or the canvas adapter; they live in the
 * Svelte shell + server.
 *
 * Derived from `App.tsx` state at T1.4 time; extended by downstream tasks
 * (P4 template, P5 MCP/companion) as their fields become document state.
 */
export interface DocumentShellState {
  /**
   * The full scene graph — groups, nodes, edges, tags, artifacts, comments,
   * and the persisted selection.  Loaded from the WS welcome snapshot and
   * mutated through the WS transport client (`sceneClient.ts`).
   *
   * @see src/shared/schema.ts `Scene`
   */
  scene: unknown; // typed as Scene in the shell — kept `unknown` here to avoid a circular dep

  /**
   * Active tag filter.  Applied client-side over the WS-held object scene to
   * pick which objects are visible on canvas; not a top-level scene field.
   */
  activeTagIds: string[];

  /**
   * Which scene item (canvas / group / node / edge) is currently selected.
   * Broadcast as presence on every change via the WS client (selection is not
   * a document op).
   *
   * @see src/shared/schema.ts `SceneSelection`
   */
  selection: unknown; // typed as SceneSelection in App.tsx
}

/**
 * Ephemeral shell state — fields that are UI/session-only and are intentionally
 * NOT persisted.  A shell swap (React → Svelte) reinitialises these to their
 * defaults; a page reload resets them.
 *
 * These fields must NOT enter `SceneSnapshot` or the op model.
 *
 * Derived from `App.tsx` state at T1.4 time.  MCP/companion presence and
 * spectator state (P5) are also ephemeral; their slots are noted below.
 */
export interface EphemeralShellState {
  /**
   * Which group the user is "currently inside" for group-scoped operations
   * (add node, export, tag filter).  Derived from `selection` when a group or
   * one of its nodes is selected, otherwise persists the last explicitly
   * navigated group.
   */
  currentGroupId: string | undefined;

  /**
   * Whether a long-running server operation (create group, export, …) is in
   * flight.  Controls busy indicators and disables form controls.
   */
  busy: boolean;

  /** Human-readable status message for the canvas status bar. */
  status: string;

  /** Whether the renderer diagnostics drawer is open. */
  diagnosticsOpen: boolean;

  /** Whether the group side-panel is open. */
  groupPanelOpen: boolean;

  /** Whether the template picker overlay is open. */
  templatePickerOpen: boolean;

  /**
   * The node currently open in inline edit mode (the DOM overlay `<textarea>`
   * is active for this node).  Cleared on Escape, selection change, or overlay
   * commit.
   */
  editingNodeId: string | null;

  /**
   * State for the floating node context menu (position + target node id).
   * Dismissed on Escape, item activation, or canvas click.
   */
  nodeMenu: { nodeId: string; x: number; y: number } | null;

  /**
   * The last node copied by the user (for paste operations).  Local clipboard
   * only; not synced to the server.
   */
  copiedNode: unknown; // typed as SceneNode | null in App.tsx

  /**
   * Export preview content shown after `exportGroup` completes.  Cleared when
   * the user closes the preview or navigates away.
   */
  exportPreview: unknown; // typed as ExportPreview | null in App.tsx

  /**
   * Camera mirror — derived read of `FrameStats.rustCameraX/Y/Zoom` emitted
   * by the engine.  The canvas engine is authoritative; the shell mirrors this
   * value for diagnostics and focus calculations only.
   *
   * MUST NOT be used as a write-source for camera; use
   * {@link ShapeCanvasHost.setCamera} instead.
   */
  camera: CameraState;

  /**
   * Latest renderer frame stats, emitted via `EngineEvent.stats`.
   * Read-only in the shell; never written back to the engine.
   */
  rendererStats: unknown; // typed as RendererStats | null in App.tsx

  /**
   * Renderer health state (`ready | webgpu-unavailable | wasm-unavailable`).
   * Emitted by `RendererCanvasHost` / `ShapeCanvasHost` as an out-of-band
   * health change.  Read-only in the shell.
   */
  rendererHealth: unknown; // typed as RendererHealth | null in App.tsx

  /**
   * Last raw renderer status string (superset of health state).  Shown in the
   * diagnostics drawer.
   */
  rendererStatus: string;

  // ---- P5 slots (net-new, not yet implemented) ----------------------------

  /**
   * Connected MCP clients and per-client status.
   * Document part (write provenance) flows through the op model (T2.5);
   * this slot holds only the ephemeral presence / animation state.
   * @see docs/design/ai-companion-canvas/tasks/T1.4.md §1 MCP connection state
   */
  mcpClientPresence: unknown; // typed by T5.1 when implemented

  /**
   * Companion dock state (which companions are shown, dock layout, active
   * motion).  Ephemeral: the dock only reads MCP state and commands
   * `focusBounds`/`setCamera` on the adapter.
   * @see docs/design/ai-companion-canvas/tasks/T1.4.md §1 Companion dock
   */
  companionDockState: unknown; // typed by T5.1 when implemented

  /**
   * Spectator / follow mode state (follow target, pin, pause/resume).
   * Follow drives camera through {@link ShapeCanvasHost.setCamera} /
   * {@link ShapeCanvasHost.focusBounds}; never owns camera state itself.
   * @see docs/design/ai-companion-canvas/tasks/T1.4.md §1 Spectator / follow controls
   */
  spectatorState: unknown; // typed by T5.3 when implemented
}
