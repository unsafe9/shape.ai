/**
 * T1.1 — Platform-neutral canvas core contract.
 *
 * This file defines `ShapeCanvasCoreContract`, the TypeScript mirror of the
 * `CanvasCore` Rust trait that any canvas adapter must satisfy. It does NOT
 * introduce runtime behavior — types and JSDoc only.
 *
 * Design invariants (see docs/design/ai-companion-canvas/tasks/T1.1.md §3–§4):
 *  - No browser DOM types (`HtmlCanvasElement`, `JsValue`, `web-sys`) may appear
 *    here. Platform-specific concerns live in the adapter layer (T1.2 / T1.3).
 *  - Business semantics (MCP/export/proposal/actor workflow, confidence, evidence
 *    refs, comments) never enter the core. `RenderCard.type`/`status` are opaque
 *    strings; the core only threads them to `styleKey` selection.
 *  - Ephemeral state (`CameraState`, in-flight gestures, render caches) is owned
 *    by the core and never persisted in the app `Scene`; document state is ingested
 *    read-only via `SceneSnapshot` and mutated only through the typed op vocabulary.
 *  - T2.5 operation metadata (`operationId`/`actorId`) is an envelope *around* the
 *    `RenderScenePatch` vocabulary; the core is metadata-agnostic.
 */

import type { SceneSelection } from "../schema";
import type {
  CameraState,
  RenderObjectKind,
  SceneSnapshot,
  WorldPoint,
  WorldRect,
} from "../renderScene";
import type { RenderScenePatch } from "../renderPatch";

// ---------------------------------------------------------------------------
// Input event types
// ---------------------------------------------------------------------------

/**
 * Pointer phase for `PointerEvent` input events fed into `inputBatch`.
 * Mirrors `CanvasInputEvent` pointer variants in `model.rs`.
 */
export type PointerPhase = "down" | "move" | "up" | "cancel";

/**
 * A single normalized input event the adapter forwards into the core.
 *
 * All coordinates are in screen space (pixels). The core converts to world space
 * internally using the live `CameraState`. Adapter-specific event sources
 * (DOM `PointerEvent`/`KeyboardEvent`, `NSEvent`, `UIEvent`) are translated to
 * this envelope before entering the core — keeping input dispatch host-neutral.
 *
 * Mirrors `CanvasInputEvent` in `model.rs` (`#[cfg(feature = "wgpu-probe")]`).
 */
export type CanvasInputEvent =
  | { kind: "pointer"; phase: PointerPhase; x: number; y: number; button: number }
  | { kind: "wheel"; deltaX: number; deltaY: number; x: number; y: number }
  | { kind: "key"; code: string; modifiers: string[] }
  | { kind: "set-camera"; camera: CameraState }
  | { kind: "fit-scene" }
  | { kind: "focus-bounds"; bounds: WorldRect }
  // CC1.4: switch the active pointer tool. Switching to hand cancels any
  // in-flight drag. `tool` is the camelCase ActiveTool ("select" | "hand").
  | { kind: "set-tool"; tool: "select" | "hand" }
  // CC4.1: right-click pick — populates the result `hit` with the picked object
  // (or null) without changing selection or starting a drag.
  | { kind: "context-pick"; screen: WorldPoint };

// ---------------------------------------------------------------------------
// Hit-test contract
// ---------------------------------------------------------------------------

/**
 * Result returned by `hitTest` for a screen-space point.
 *
 * Mirrors `CoreHitResult` in `stats.rs`. `kind` and `field`/`port` are opaque
 * strings so the core never decodes decision-graph card types. `groupId` is
 * present when the hit object belongs to a group frame.
 */
export type CoreHitResult = {
  /** Id of the hit scene object (card, edge, or group). */
  id: string;
  /** Object primitive kind at the hit point. */
  kind: RenderObjectKind;
  /** Group id the hit object belongs to, if any. */
  groupId?: string;
  /** Text field hit (e.g. "title" | "summary" | "detail"), if the hit is inside a card text region. */
  field?: string;
  /** Port id hit, if the hit is on an edge port/connector. */
  port?: string;
  /** World-space coordinates of the hit point. */
  worldPoint: WorldPoint;
  /** Screen-space coordinates of the hit point (passed through from the input). */
  screenPoint: WorldPoint;
};

// ---------------------------------------------------------------------------
// Overlay request contract
// ---------------------------------------------------------------------------

/**
 * The target object and field the core resolved for an active-edit overlay.
 *
 * The core computes the world/screen rect and text style; the adapter mounts
 * the actual DOM/native overlay widget. This keeps "Rust computes coords/target,
 * the shell mounts the overlay" (strategy doc `JavaScript API 형태`, principle 4)
 * host-neutral.
 *
 * Mirrors `CoreOverlayTarget` / `CoreOverlayRequest` in `stats.rs`.
 */
export type CoreOverlayTarget = {
  /** Id of the object being edited. */
  id: string;
  /** Primitive kind of the object. */
  kind: RenderObjectKind;
  /** Text field being edited (e.g. "title" | "summary" | "detail"). */
  field: string;
};

export type CoreOverlayStyle = {
  fontSize: number;
  lineHeight: number;
  color: string;
  fontWeight?: string;
  padding?: number;
};

/**
 * Returned by `overlayRequest` when the core has resolved the geometry for an
 * active-edit text region. The adapter uses this to position and size its
 * text-input overlay widget (DOM `textarea`/`contenteditable`, `NSTextView`, etc.).
 */
export type CoreOverlayRequest = {
  target: CoreOverlayTarget;
  /** Bounding rect of the text field in world space. */
  worldRect: WorldRect;
  /** Bounding rect of the text field in screen space (derived from camera + world rect). */
  screenRect: WorldRect;
  style: CoreOverlayStyle;
};

// ---------------------------------------------------------------------------
// Frame / debug stats contract
// ---------------------------------------------------------------------------

/**
 * Per-frame render statistics reported by `renderFrame` and `frameStats`.
 *
 * Derived from `WebGpuFrameStats` in `stats.rs`. Backend-specific GPU counters
 * (wgpu slot compaction, vertex truncation) are in the optional `backendStats`
 * extension so Metal can report its own surface without breaking the shared shape
 * (see T1.1 Open questions: "FrameStats backend field").
 */
export type CoreFrameStats = {
  /** Number of visible groups after viewport cull. */
  visibleGroups: number;
  /** Number of visible cards after viewport cull. */
  visibleCards: number;
  /** Number of visible edges after viewport cull. */
  visibleEdges: number;
  /** Total draw primitives emitted this frame. */
  drawPrimitives: number;
  /** Text glyph cache hits this frame. */
  glyphCacheHits: number;
  /** Text glyph cache misses (atlas uploads) this frame. */
  glyphCacheMisses: number;
  /** Incremental patch counter (ops applied since last full reload). */
  patchCount: number;
  /**
   * Optional backend-specific counters. The wgpu backend populates this;
   * a Metal backend may supply a different shape. Typed as an open record
   * to avoid forking the shared stats shape before T1.3 defines Metal diagnostics.
   */
  backendStats?: Record<string, number | string>;
};

/**
 * Full debug snapshot returned by `debugSnapshot`.
 *
 * Mirrors `WebGpuDebugSnapshot` in `stats.rs`. Includes frame stats plus
 * richer scene-state information useful for devtools overlays and test assertions.
 */
export type CoreDebugSnapshot = {
  frameStats: CoreFrameStats;
  /** Selection bounding rect in world space (null when nothing is selected). */
  selectionWorldRect: WorldRect | null;
  /** Selection bounding rect in screen space (null when nothing is selected). */
  selectionScreenRect: WorldRect | null;
  /** Current camera state at the time of the snapshot. */
  camera: CameraState;
  /** Slot occupancy: total allocated vs used vertex slots. */
  slotOccupancy: { allocated: number; used: number };
  /** Live object counts in the renderer scene. */
  objectCounts: { groups: number; cards: number; edges: number };
};

// ---------------------------------------------------------------------------
// Core contract
// ---------------------------------------------------------------------------

/**
 * Platform-neutral canvas core contract.
 *
 * This interface describes what any canvas adapter must satisfy. The WASM/WebGPU
 * web adapter (T1.2) implements it via `ShapeWebGpuRenderer` + wasm-bindgen glue.
 * A native Metal adapter (T1.3) implements it via a wgpu-on-Metal or native Metal
 * pipeline. Nothing in this interface assumes a browser, DOM, or specific GPU API.
 *
 * Eight capabilities (from the T1.1 breakdown):
 *  1. Scene object identity + load  — `loadScene`
 *  2. Operation application         — `applyOpBatch`
 *  3. Camera                        — `setCamera` / `inputBatch` camera events
 *  4. Geometry + bounds             — `WorldRect` carried on every object; read via snapshot
 *  5. Hit testing                   — `hitTest`
 *  6. Selection geometry            — `selectionGeometry`
 *  7. Render primitive stream       — `renderFrame`
 *  8. Debug stats                   — `frameStats` / `debugSnapshot`
 *
 * Two additional lifecycle methods owned by the adapter side but computed in core:
 *  - `resize`          — display-scale math is core-owned; surface resize is adapter-owned
 *  - `overlayRequest`  — coord/style computation is core-owned; widget mounting is adapter-owned
 *
 * Document vs ephemeral split (§4):
 *  - Document state (snapshot geometry/text/style/z-order/selection) is ingested
 *    read-only via `loadScene` and mutated only through `applyOpBatch`.
 *  - Ephemeral state (`CameraState`, in-flight gestures, vertex/text caches, culling)
 *    is owned by the core and reset without persisting.
 */
export interface ShapeCanvasCoreContract {
  /**
   * Load a full scene snapshot into the core, replacing any previously loaded scene.
   *
   * The snapshot is the renderable-scene projection of the app `Scene` produced by
   * `shapeSceneToRenderSnapshot`. `metadata.source` is app-layer provenance and is
   * ignored by the core. Each object is assigned a stable per-object vertex slot
   * keyed by `id`; identity is the `id` string, never positional.
   *
   * This is a *reload* operation, not an edit unit — consistent with T2.5's
   * "no untracked blob overwrite" invariant.
   *
   * Mirrors `loadScene` / `webgpu.rs:743`.
   */
  loadScene(snapshot: SceneSnapshot): Promise<void> | void;

  /**
   * Apply a batch of typed render ops to the live scene.
   *
   * The op vocabulary (`RenderScenePatch`) covers create/delete/move/edit-text/
   * z-index/edge/select/resize/align/distribute/duplicate/batch/group ops.
   * Rollback-on-error is expected: if any op in the batch fails the core must
   * leave the scene unchanged and surface errors.
   *
   * T2.5 operation metadata (`operationId`/`actorId`/`timestamp`/`baseRevision`) is
   * an envelope the TS layer manages *around* this call; the core is metadata-agnostic.
   *
   * Mirrors `applyPatchBatch` / `webgpu.rs:760`.
   */
  applyOpBatch(ops: RenderScenePatch[]): Promise<void> | void;

  /**
   * Directly set the camera state (viewport position + zoom).
   *
   * Camera math (world↔screen transform, zoom, fit-to-bounds, focus-bounds) is
   * Rust/core-owned (Locked Decision). `CameraState` lives only in the live
   * renderer and `SceneSnapshot`; it is not persisted in the app `Scene`.
   *
   * Mirrors `CameraState` + `SetCamera` variant of `CanvasInputEvent` / `webgpu.rs`.
   */
  setCamera(camera: CameraState): void;

  /**
   * Process a batch of normalized input events.
   *
   * Events cover pointer gestures, wheel, keyboard shortcuts, camera commands, and
   * fit/focus-bounds actions. The adapter translates platform-native events
   * (`PointerEvent`, `NSEvent`, etc.) into `CanvasInputEvent` before calling here —
   * the core never receives raw DOM or AppKit events.
   *
   * Returns the hit result for the final pointer event in the batch (if any),
   * the updated selection, and any overlay requests triggered by the gesture.
   *
   * Mirrors `inputBatch` / `webgpu.rs`.
   */
  inputBatch(events: CanvasInputEvent[]): CoreInputBatchResult;

  /**
   * Hit-test a screen-space point against the live scene.
   *
   * Returns the topmost object at the point (respecting z-order and the current
   * camera), or `null` if nothing is hit. `kind`/`field`/`port` are opaque strings;
   * the core never branches on card type decision-graph meaning.
   *
   * Mirrors `hit_at_screen` → `hit_scene_at_screen` / `webgpu.rs:1582`.
   */
  hitTest(screenPoint: WorldPoint): CoreHitResult | null;

  /**
   * Set the active pointer tool ("select" | "hand"). Switching to hand cancels
   * any in-flight drag. Equivalent to a `set-tool` inputBatch event but callable
   * as a one-off (tool toggles in the shell rarely coincide with a pointer batch).
   * Unknown values are ignored.
   *
   * Mirrors `setTool` / `webgpu.rs`.
   */
  setTool(tool: "select" | "hand"): void;

  /**
   * Compute the bounding rect (world space) of the current selection.
   *
   * Returns `null` when nothing is selected. The result is the seed for multi-select
   * hull and group-hull geometry (T2.4 will widen the return type to a richer shape;
   * the method name is reserved).
   *
   * Mirrors `selection_world_rect` + `WebGpuDebugSnapshot.selection_world_rect` /
   * `stats.rs:127`.
   */
  selectionGeometry(): WorldRect | null;

  /**
   * Build the culled draw list and emit GPU primitives for the current frame.
   *
   * Performs viewport-rect intersection culling over groups/edges/cards, builds
   * the vertex/draw-range arrays, and submits to the GPU backend. Returns
   * per-frame render statistics.
   *
   * The *primitive stream* (vertex/draw-range build) is a core responsibility;
   * the GPU surface it writes into is adapter-owned.
   *
   * Mirrors `renderFrame` / `webgpu.rs:1172`.
   */
  renderFrame(): CoreFrameStats;

  /**
   * Return the most recent per-frame render statistics without triggering a render.
   *
   * Useful for devtools overlays and test assertions that need stats without
   * side-effecting the render pipeline.
   *
   * Mirrors `WebGpuFrameStats` / `stats.rs:6`.
   */
  frameStats(): CoreFrameStats;

  /**
   * Return a full debug snapshot of the current renderer state.
   *
   * Includes frame stats, selection rects (world + screen), camera state, slot
   * occupancy, and object counts. Intended for devtools and test assertions.
   *
   * Mirrors `WebGpuDebugSnapshot` / `stats.rs:127` + `debugSnapshot` / `webgpu.rs:1323`.
   */
  debugSnapshot(): CoreDebugSnapshot;

  /**
   * Compute the world/screen rect and text style for an active-edit text overlay.
   *
   * The core resolves the exact bounding rect and style for `targetId`+`field`;
   * the adapter mounts the actual overlay widget (DOM textarea/contenteditable,
   * `NSTextView`, etc.) at those coordinates.
   *
   * Returns `null` when the target id/field is not in the live scene.
   *
   * Mirrors `overlay_request` / `webgpu.rs:1318`.
   */
  overlayRequest(targetId: string, field: string): CoreOverlayRequest | null;

  /**
   * Notify the core of a display size change.
   *
   * The core recomputes camera/viewport math (display-scale transform, aspect ratio,
   * LOD thresholds). The GPU *surface* resize (swapchain/drawable) is adapter-owned
   * and must happen before or after this call as the adapter requires.
   *
   * Mirrors `resize` / `webgpu.rs:731`.
   */
  resize(width: number, height: number, devicePixelRatio: number): void;
}

// ---------------------------------------------------------------------------
// inputBatch result
// ---------------------------------------------------------------------------

/**
 * Result returned by `inputBatch`.
 *
 * Contains the hit for the last pointer event in the batch (if any), the
 * selection after all events were processed, and any overlay requests triggered
 * (e.g. double-tap on a card field opens an edit overlay).
 *
 * Mirrors `CoreInputBatchResult` in `stats.rs` (`#[cfg(feature = "wgpu-probe")]`).
 */
export type CoreInputBatchResult = {
  /** Camera state after the batch was applied. */
  camera: CameraState;
  /** Hit result for the final pointer event in the batch, if any. */
  hit: CoreHitResult | null;
  /**
   * Selection state after the batch was applied. Can be the `multi` variant
   * (`{ kind: "multi", ids }`) — the core never persists it; the shell merges the
   * ids into its transient multiSelectIds set.
   */
  selection: SceneSelection;
  /** Render-op patches the batch produced (applied in order). */
  patches: RenderScenePatch[];
  /** Overlay requests triggered by gestures in the batch (e.g. text field activation). */
  overlayRequests: CoreOverlayRequest[];
  /**
   * CC2.3: non-null ONLY on the pointer-up that ends a marquee drag. `rect` is
   * the final world-space marquee rect; `ids` are node ids first then group ids
   * whose world bounds AABB-intersect the rect. The shell merges `ids` into its
   * transient multiSelectIds set.
   */
  marquee: { rect: WorldRect; ids: string[] } | null;
};
