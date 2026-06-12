<script lang="ts">
  import { onDestroy } from "svelte";
  import { BrainCircuit, Loader2, Copy, Trash2, Group as GroupIcon, Ungroup, MessageSquarePlus, LayoutTemplate, Sun, Moon } from "lucide-svelte";
  import { screenToWorld, applyDocumentTheme, readStoredTheme, type Theme } from "../renderer/scene";
  import type { CameraState } from "../shared/geometry";
  import {
    emptyObjectScene,
    GEOMETRY_QUANTUM_PER_PX,
    type Object as SceneObject,
    type ObjectOp,
    type ObjectScene,
    type ObjectSelection,
    type FeatureResponse
  } from "../shared/object";
  import {
    ShapeCanvasHost,
    type RendererHealth,
    type RendererStats,
    type ShapeCanvasHostCallbacks
  } from "../controller/canvasHost";
  import { SceneClient, type CanvasSummary } from "../runtime/sceneClient";
  import type { PeerPresence } from "../runtime/peers";
  import type { ConnectionStatus } from "../runtime/wsTransport";
  import type { ActiveTool, TransformKind } from "../renderer/engine";
  import { cursorAffordance as deriveCursorAffordance } from "../controller/cursor";
  import type { HoverAffordance } from "../bridge/wasmLoader";
  import {
    loadSceneCore,
    type ObjectCommand,
    type ObjectGesture,
    type SceneCore,
    type UndoStack
  } from "../bridge/sceneCoreWasm";
  import { createShortcutDispatcher } from "../controller/shortcuts";
  import { isFreeRecognizeHold } from "../controller/gestureBindings";
  import { ToastChannel } from "../runtime/statusChannel";
  import {
    MIN_DRAG_EXTENT_PX,
    textOverlayScreenRect,
    THEME_DEFAULT_COLOR,
    type DragSpan,
    type CreateSnap,
    resolveCreateRelease,
    synthesizeReleaseAnchors,
    CREATE_ANCHOR_REUSE_TOLERANCE_PX,
    MERGE_ENDPOINT_TOLERANCE_PX
  } from "../controller/objectPrimitives";
  import {
    routeSelectObject,
    routeMarquee,
    commitBodyDrag,
    draggedRootTransform,
    endpointSnapTarget,
    endpointReleaseOp,
    canonicalizeCreateSnap,
    canonicalizeHoverSnap,
    buildInsertPrimitive,
    buildColorApplyOp,
    buildGroupOps,
    resolveDoubleClick,
    ungroupPickEnabled as ungroupPickEnabledOf,
    popOutPickEnabled as popOutPickEnabledOf,
    resolveContextMenuItems,
    previewPaint,
    buildFeedScene as buildFeedSceneOf,
    type ContextMenuEntry
  } from "../controller/interactions";
  import { isDragCreateShape, type DragCreateShape, type PrimitiveKindId } from "../controller/toolbar";
  import {
    transformOrigin,
    transformsEqual,
    shiftTransform,
    worldToObjectLocalQuantized,
    unionWorldAabb,
    rectPathQuantized
  } from "../controller/transforms";
  import {
    isDiagnosticsOnlyStatus,
    wsBaseUrl as wsBaseUrlOf,
    clientIdentity as clientIdentityOf,
    userIdentity as userIdentityOf
  } from "../controller/session";
  import Toolbar from "./Toolbar.svelte";
  import SettingsModal from "./SettingsModal.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";
  import ContextMenu, { type ContextMenuItem } from "./ContextMenu.svelte";
  import TemplatePopup, { type TemplatePopupItem } from "./TemplatePopup.svelte";
  import PeerCursors from "./PeerCursors.svelte";

  // Canonical object scene.
  let scene = $state<ObjectScene>(emptyObjectScene());

  let selection = $state<ObjectSelection>({ kind: "canvas" });

  // Active drill-in container; null = the canvas root is active.
  let activeContainer = $state<string | null>(null);

  // In the connected path authorOp's scene update is async, so the GPU instance matrix is the ONLY thing
  // holding the object at its previewed position during the commit window. The next canonical scene that
  // reflects this {id, transform} triggers the single rebake, dropping the stale preview with no snap-back.
  let pendingCommit: { id: string; transform: SceneObject["transform"] } | null = null;

  // The in-progress freehand stroke's world points; `feedScene` appends a transient preview. On pointer-up the points lower to a committed `Object` via the wasm core.
  let drawPoints = $state<{ x: number; y: number }[] | null>(null);
  // The stroke's snap bookkeeping: `start` is the snap the stroke BEGAN on, `last` the most recent
  // in-flight snap (sticky, for near-miss release reuse). Both feed the release's endpoint-anchor authoring when the result is OPEN.
  let drawSnap = $state<{ start: CreateSnap | null; last: CreateSnap | null } | null>(null);
  let penWidthPx = $state(2);
  // Free recognition (polygon/curve fallbacks) while Shift is held; mirrored from every key event,
  // consumed only at pen-up so it never collides with Shift's rotate/select roles.
  let freeRecognitionHeld = $state(false);
  // The toolbar's always-visible color: the default fill/stroke for the next NEW shape; recoloring a
  // selected object authors a SetStyle op. The native picker passes a CSS hex through verbatim.
  let selectedColor = $state(THEME_DEFAULT_COLOR);
  // First entry is the theme-default token sentinel, then the fixed hex swatches.
  const PEN_PALETTE = [THEME_DEFAULT_COLOR, "#1f2933", "#ef4444", "#3b82f6", "#22c55e", "#f59e0b", "#ffffff"];
  const PEN_WIDTHS = [1, 2, 4, 8];
  // Partial-erase cut radius in object-local quantized units (~12 logical px).
  const ERASE_RADIUS_QUANTIZED = 12 * GEOMETRY_QUANTUM_PER_PX;

  // Shape drag-create: arming a shape tool sets `createKind`; the pointer down-drag-up rubber-bands a
  // bbox in `createDrag`. `target` is the snapped object id; `lastSnap` is the most recent successful
  // outline snap (sticky across moves off the edge), so a release that misses the 8px snap can reuse it.
  let createKind = $state<DragCreateShape | null>(null);
  let createDrag = $state<{ span: DragSpan; snapped: boolean; target: string | null; startSnap: CreateSnap | null; lastSnap: CreateSnap | null } | null>(null);
  // The pre-drag hover snap: a bare hover over an object's edge sets the snapped world point + target id;
  // `feedScene` renders a PERSISTENT anchor ring from it (only while `createDrag` is null). Null off any edge.
  let createHoverSnap = $state<{ at: { x: number; y: number }; target: string | null } | null>(null);

  // Inline text editing: `textEdit` holds the edited object id + in-progress value; a contenteditable
  // overlay (which handles IME) is positioned over its screen bbox. Blur/Enter commits a set-text op; Esc cancels.
  let textEdit = $state<{ id: string; value: string } | null>(null);

  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.6 });
  let status = $state("Ready");
  // The transient toast channel: persistent hints stay in `status`; transient ACTION notices flow
  // through `showToast` and auto-dismiss. The real setTimeout/clearTimeout are injected so scene-core stays time-free.
  let toast = $state<string | null>(null);
  const toastChannel = new ToastChannel(
    { set: (cb, ms) => setTimeout(cb, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) },
    (message) => (toast = message)
  );
  // A transient action notice (auto-dismisses); persistent text keeps using `status = ...`.
  function showToast(message: string): void {
    toastChannel.show(message);
  }
  let busy = $state(false);
  let diagnosticsOpen = $state(false);
  let settingsOpen = $state(false);
  let theme = $state<Theme>(readStoredTheme(window.localStorage));
  function toggleTheme(): void {
    theme = theme === "dark" ? "light" : "dark";
  }
  // On init and every toggle: flip the root `data-theme` attribute, persist, and drive the renderer theme-bit so canvas + chrome flip together.
  $effect(() => {
    applyDocumentTheme(theme, {
      root: document.documentElement,
      storage: window.localStorage,
      setRendererTheme: (dark) => host?.setObjectTheme?.(dark)
    });
  });
  let templateOpen = $state(false);
  let activeTool = $state<ActiveTool>("select");
  // Space-hold pan + dynamic hover cursor: `spaceHeld` flips the empty cursor to grab and arms the
  // engine pan path; `affordance` is the core's per-move hover classification, mapped to a CSS cursor.
  let spaceHeld = $state(false);
  let affordance = $state<HoverAffordance>("empty");

  type ContextMenuState = { selection: ObjectSelection; x: number; y: number; world: { x: number; y: number } };
  let contextMenu = $state<ContextMenuState | null>(null);
  let pendingContextScreen: { clientX: number; clientY: number } | null = null;

  let peers = $state<PeerPresence[]>([]);
  let lastCursorSentAt = 0;
  const CURSOR_THROTTLE_MS = 40;

  let rendererStats = $state<RendererStats | null>(null);
  let rendererHealth = $state<RendererHealth | null>(null);

  let canvasId = $state("default");
  let sceneClient: SceneClient | null = null;
  let sceneClientReady = false;
  let sceneCore: SceneCore | null = null;
  let commandCatalog = $state<ObjectCommand[]>([]);
  let gestureCatalog = $state<ObjectGesture[]>([]);
  let canvases = $state<CanvasSummary[]>([]);
  let connectionStatus = $state<ConnectionStatus>("offline");
  let canvasBusy = $state(false);

  // Per-actor undo/redo: the core's UndoStack owns the bookkeeping; only this client's ops are undoable. Created once the wasm core loads.
  let undoStack: UndoStack | null = null;

  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;

  // The cursor affordance reflected onto the canvas wrapper; the mapping lives in `cursor.ts` (the single source styles.css mirrors).
  const cursorAffordance = $derived(deriveCursorAffordance(spaceHeld, activeTool, affordance));

  // The inline text-edit overlay rect, recomputed when the edited object, its transform, or the camera
  // changes (so the overlay tracks the object under pan/zoom). Null when not editing or the path is gone.
  const textEditObject = $derived(textEdit ? scene.objects.find((o) => o.id === textEdit.id) ?? null : null);
  const textEditRect = $derived(textEditObject ? textOverlayScreenRect(textEditObject, camera) : null);

  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(scene.objects.length > 0);
  const selectedObject = $derived(selection.kind === "object" ? scene.objects.find((o) => o.id === selection.id) ?? null : null);

  // The scene fed to the renderer: the canonical scene plus any transient NEW-object preview (a live
  // transform of an EXISTING object no longer rebuilds this — it's pushed straight to the GPU instance
  // matrix). A new object bakes once per geometry change since it has no instance to update.
  const feedScene = $derived(
    buildFeedSceneOf(scene, drawPoints, createKind, createDrag, createHoverSnap, nextOrderKey, selectedColor, penWidthPx)
  );

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onStats: (stats) => (rendererStats = stats),
    onStatus: (message) => (status = message),
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyStatus(status)) status = "Ready";
    },
    // A plain click on an object ALREADY in the current Multi keeps the whole Multi (so a group-drag
    // never collapses it to a single); shift/meta toggles the object in/out; anything else replaces it.
    onSelectObject: (id, additive) => selectObject(routeSelectObject(selection, id, additive)),
    onTransformPreview: (id, _matrix, _kind) => {
      // The matrix is already on the GPU instance buffer; this only invalidates a stale pendingCommit so a new drag can't freeze.
      if (pendingCommit && pendingCommit.id !== id) pendingCommit = null;
    },
    onTransformCommit: (id, matrix, kind, detach) => {
      // The canonical scene was never mutated during the drag, so op-apply captures the correct inverse (original transform).
      const src = scene.objects.find((o) => o.id === id);
      // With no src or no scene-core the GPU preview must not linger — revert it to canonical.
      if (!src) return void host?.clearObjectPreview(id);
      if (!sceneCore) return void host?.clearObjectPreview(id);
      // The parent-drag cascade + multi-select union + anchor-follow + Alt-detach collapse into
      // commitBodyDrag, yielding the single commit op plus the full op list (for the snap-back guard).
      const { op, allOps } = commitBodyDrag(sceneCore, scene, selection, id, matrix, kind, detach);
      // Pre-connect authorOp applies synchronously; in the connected path the scene update is async, so
      // the GPU matrix holds the previewed position until commitClientScene sees the transform land.
      const wasConnected = sceneClientReady && sceneClient !== null;
      // pendingCommit is keyed on the dragged `id`, so read ITS composed transform — not allOps[0], which under a multi cascade may be another member.
      const rootTransform = draggedRootTransform(allOps, id, src.transform);
      if (wasConnected && rootTransform) pendingCommit = { id, transform: rootTransform };
      authorOp(op, true, (ok) => {
        if (!ok) {
          pendingCommit = null;
          host?.clearObjectPreview(id);
        }
      });
    },
    // Open-class endpoint drag. The chord deform is already live on the GPU; the preview event only
    // drives the release-snap anchor ring (createHoverSnap -> feedScene), honoring a snap only onto a real, OTHER canonical object.
    onEndpointPreview: (id, _nodeIndex, world, snapped, targetId) => {
      const target = endpointSnapTarget(scene, id, snapped, targetId);
      const next = target !== null ? { at: world, target } : null;
      // Skip the null -> null write so an unsnapped drag never re-feeds the scene.
      if (next !== null || createHoverSnap !== null) createHoverSnap = next;
    },
    // The endpoint-drag release — ONE undoable batch from endpointReleaseOps. The canonical scene was
    // never mutated during the drag, so op-apply captures the correct inverse; a no-op/failed release reverts the GPU deform.
    onEndpointCommit: (id, nodeIndex, world, snapped, targetId) => {
      createHoverSnap = null;
      if (!sceneCore) return void host?.clearObjectEndpointPreview(id);
      const target = endpointSnapTarget(scene, id, snapped, targetId);
      const op = endpointReleaseOp(sceneCore, scene, id, nodeIndex, world, target);
      if (!op) return void host?.clearObjectEndpointPreview(id);
      authorOp(op, true, (ok) => {
        if (!ok) host?.clearObjectEndpointPreview(id);
      });
    },
    onMarquee: (ids) => {
      // Route through the same validation as onSelectObject (drops stale ids and collapses the kind).
      selectObject(routeMarquee(ids));
    },
    onObjectDoubleClick: (payload) => handleObjectDoubleClick(payload),
    onDraw: (phase, world, snap) => handleDraw(phase, world, snap),
    onCreate: (phase, world, snapped, targetId) => handleCreate(phase, world, snapped, targetId),
    onCreateHover: (world, snapped, targetId) => handleCreateHover(world, snapped, targetId),
    onErase: (id, world, partial) => handleErase(id, world, partial),
    onAffordance: (next) => (affordance = next)
  };

  // Boot the scene-core wasm (op-apply + catalog) and open the WS session.
  void bootstrapSceneCore();
  void connectSceneClient();

  async function bootstrapSceneCore(): Promise<void> {
    try {
      sceneCore = await loadSceneCore();
      commandCatalog = sceneCore.objectCommandCatalog();
      gestureCatalog = sceneCore.objectGestureCatalog();
      undoStack = sceneCore.createUndoStack(userIdentity());
    } catch {
      sceneCore = null;
    }
  }

  // Push `feedScene` into the renderer whenever it or the selection changes. A live drag of an EXISTING object rides the GPU instance matrix instead, not this feed.
  $effect(() => {
    const current = feedScene;
    const sel = selection;
    host?.loadObjectScene(current, sel);
  });

  // Push the active tool to the renderer whenever it changes.
  $effect(() => {
    const tool = activeTool;
    host?.setTool(tool);
  });

  // Windowed replica: re-aim the data-layer window at the camera viewport.
  $effect(() => {
    const cam = camera;
    if (!sceneClientReady || !sceneClient) return;
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return;
    const topLeft = screenToWorld({ x: 0, y: 0 }, cam);
    const bottomRight = screenToWorld({ x: rect.width, y: rect.height }, cam);
    sceneClient.setViewport({ x: topLeft.x, y: topLeft.y, width: bottomRight.x - topLeft.x, height: bottomRight.y - topLeft.y });
  });

  // Peer cursor poll: drop fully-silent peers within one interval.
  $effect(() => {
    let cancelled = false;
    const poll = () => {
      if (!cancelled && sceneClientReady && sceneClient) peers = sceneClient.peerCursors;
    };
    poll();
    const id = window.setInterval(poll, 4_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  });

  // Central shortcut dispatch over the object command catalog.
  const dispatchShortcut = $derived(createShortcutDispatcher({ catalog: commandCatalog, handlers: shortcutHandlers() }));

  $effect(() => {
    const dispatch = dispatchShortcut;
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing = target ? ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) || target.isContentEditable : false;
      freeRecognitionHeld = isFreeRecognizeHold(event);
      if (event.key === "Escape") {
        event.preventDefault();
        handleEscape();
        return;
      }
      // Space-hold arms the pointer's pan path; release on keyup. The engine flips the core to hand-pan only for the duration of a Space-held drag.
      if (event.code === "Space" && !typing && !event.repeat) {
        event.preventDefault();
        if (!spaceHeld) {
          spaceHeld = true;
          host?.setSpaceHeld(true);
        }
        return;
      }
      dispatch(event);
    }
    function handleKeyUp(event: KeyboardEvent) {
      freeRecognitionHeld = isFreeRecognizeHold(event);
      if (event.code === "Space" && spaceHeld) {
        event.preventDefault();
        spaceHeld = false;
        host?.setSpaceHeld(false);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("keyup", handleKeyUp);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("keyup", handleKeyUp);
    };
  });

  onDestroy(() => {
    sceneClient?.close();
    // Cancel any in-flight toast timer so it can't fire after teardown.
    toastChannel.dismiss();
  });

  async function connectSceneClient(): Promise<void> {
    const client = new SceneClient({ url: wsBaseUrl(), clientId: clientIdentity(), userId: userIdentity() });
    try {
      const welcome = await client.connect(canvasId);
      sceneClient = client;
      sceneClientReady = true;
      connectionStatus = client.connectionStatus;
      client.onScene((next) => commitClientScene(next));
      client.onStatus((next) => (connectionStatus = next));
      client.onPeers((next) => (peers = next));
      client.onFeature((response) => handleFeatureResponse(response));
      scene = welcome;
      selection = validSelection(welcome, welcome.selection);
      void loadCanvases();
    } catch (error) {
      client.close();
      status = error instanceof Error ? error.message : "Scene load failed";
    }
  }

  async function loadCanvases(): Promise<void> {
    if (!sceneClient) return;
    try {
      canvases = await sceneClient.listCanvases();
    } catch {
      /* best-effort */
    }
  }

  async function switchToCanvas(nextCanvasId: string): Promise<void> {
    if (!sceneClient || nextCanvasId === canvasId) return;
    canvasBusy = true;
    try {
      const welcome = await sceneClient.switchCanvas(nextCanvasId);
      canvasId = nextCanvasId;
      scene = welcome;
      selection = validSelection(welcome, welcome.selection);
      // A switched canvas starts a fresh undo history.
      if (sceneCore) undoStack = sceneCore.createUndoStack(userIdentity());
    } catch (error) {
      status = error instanceof Error ? error.message : "Canvas switch failed";
    } finally {
      canvasBusy = false;
    }
  }

  async function createCanvas(title: string): Promise<void> {
    if (!sceneClient) return;
    canvasBusy = true;
    try {
      const summary = await sceneClient.createCanvas(title);
      await loadCanvases();
      await switchToCanvas(summary.id);
    } catch (error) {
      status = error instanceof Error ? error.message : "Canvas create failed";
    } finally {
      canvasBusy = false;
    }
  }

  // Delete a canvas. Deleting the active one first switches to another remaining canvas (so the session
  // is never left pointing at a deleted id), then deletes the old one and reloads the list.
  async function deleteCanvas(targetCanvasId: string): Promise<void> {
    if (!sceneClient) return;
    if (targetCanvasId === canvasId) {
      const other = canvases.find((c) => c.id !== targetCanvasId);
      if (!other) return;
      await switchToCanvas(other.id);
      // switchToCanvas swallows its own errors (leaving canvasId unchanged); never
      // delete the canvas the session is still pointed at.
      if (canvasId === targetCanvasId) return;
    }
    canvasBusy = true;
    try {
      await sceneClient.deleteCanvas(targetCanvasId);
      await loadCanvases();
    } catch (error) {
      status = error instanceof Error ? error.message : "Canvas delete failed";
    } finally {
      canvasBusy = false;
    }
  }

  // Adopt an engine-driven scene update (acked/remote), keeping selection valid.
  function commitClientScene(next: ObjectScene): void {
    scene = next;
    selection = validSelection(next, selection);
    // The `scene = next` above already rebaked at the committed transform; clear pendingCommit (and
    // defensively revert any residual preview) once the canonical transform lands or the object is gone.
    if (pendingCommit) {
      const committed = next.objects.find((o) => o.id === pendingCommit!.id);
      if (!committed || transformsEqual(committed.transform, pendingCommit.transform)) {
        host?.clearObjectPreview(pendingCommit.id);
        pendingCommit = null;
      }
    }
  }

  function handleFeatureResponse(response: FeatureResponse): void {
    if (response.feature === "featureError") {
      status = response.message;
      return;
    }
    if (response.feature === "templateApplied") {
      showToast(`Inserted ${response.object_ids.length} objects`);
      return;
    }
    if (response.feature === "exportReady") {
      showToast(`Export ready: ${response.artifact_ref}`);
      return;
    }
    if (response.feature === "commentUpserted") {
      showToast("Comment added");
    }
  }

  // Author an ObjectOp: the data layer applies it optimistically through the wasm core and returns the
  // inverse (the undo entry). On success forward+inverse are recorded into the core undo stack, which
  // clears redo. An undo/redo replay is NOT undoable (the core stack drives those via its own handshake).
  function authorOp(op: ObjectOp, undoable = true, onSettled?: (ok: boolean) => void): void {
    if (!sceneClientReady || !sceneClient) {
      // Pre-connect: apply optimistically through the core only, no wire.
      if (sceneCore) {
        const applied = sceneCore.applyObjectOp(scene, op);
        if (applied.errors.length > 0) status = applied.errors.join("; ");
        else {
          scene = applied.scene;
          if (undoable && applied.inverse) undoStack?.record(op, applied.inverse);
        }
        onSettled?.(applied.errors.length === 0);
      } else {
        onSettled?.(false);
      }
      return;
    }
    void sceneClient.applyObjectOp(op).then((result) => {
      if (result.errors.length > 0) {
        status = result.errors.join("; ");
        onSettled?.(false);
        return;
      }
      if (undoable && result.inverse) undoStack?.record(op, result.inverse);
      onSettled?.(true);
      // The engine's onScene callback commits the optimistic scene.
    });
    sceneClient.flush();
  }

  // The core's undo/redo is a two-step async handshake, so back-to-back presses must not re-enter it
  // before the prior settles. Chain every undo/redo onto this tail so they run strictly in order.
  let undoChain: Promise<void> = Promise.resolve();

  function undo(): void {
    // A rejection must never poison the serialization tail (runUndoStep self-handles errors; this catch is belt-and-braces).
    undoChain = undoChain.then(() => runUndoStep("undo")).catch(() => {});
  }

  function redo(): void {
    undoChain = undoChain.then(() => runUndoStep("redo")).catch(() => {});
  }

  // Pull the next op from the core stack and re-author it through the SAME op-apply path; report the
  // resulting inverse to complete the handshake. Awaiting the apply keeps a queued press from re-entering it.
  async function runUndoStep(dir: "undo" | "redo"): Promise<void> {
    if (!undoStack || !sceneClient) return;
    const op = dir === "undo" ? undoStack.undo() : undoStack.redo();
    if (!op) return;
    // The core stack is now mid-handshake; any exit that does NOT complete it MUST abort() it, or the
    // stuck pending corrupts every future undo/redo (the core's debug_assert is compiled out of release wasm).
    try {
      const result = await sceneClient.applyObjectOp(op);
      sceneClient.flush();
      if (result.errors.length > 0 || !result.inverse) {
        if (result.errors.length > 0) status = result.errors.join("; ");
        undoStack.abort();
        return;
      }
      if (dir === "undo") undoStack.noteUndoApplied(result.inverse);
      else undoStack.noteRedoApplied(result.inverse);
    } catch (error) {
      status = error instanceof Error ? error.message : "Undo failed";
      undoStack.abort();
    }
  }

  function nextOrderKey(): string {
    // Order keys sort by plain string Ord; append a char after the current max so a new object lands on
    // top. The wasm core owns true fractional keys; this shell-side monotonic key is only the insertion position.
    const maxOrder = scene.objects.reduce((max, o) => (o.order > max ? o.order : max), "a0");
    return `${maxOrder}~`;
  }

  function freshId(prefix: string): string {
    return `${prefix}-${crypto.randomUUID().slice(0, 8)}`;
  }

  function viewportCenterWorld(): { x: number; y: number } {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect) return { x: 120, y: 120 };
    return screenToWorld({ x: rect.width / 2, y: rect.height / 2 }, camera);
  }

  // Rect/ellipse/line ARM drag-create (sized by a pointer down-drag-up). A context-menu insert passes an
  // explicit `anchor` for an immediate fixed-size drop. Text and frame always insert immediately at an anchor.
  function insertPrimitive(kind: PrimitiveKindId, anchor?: { x: number; y: number }): void {
    if (anchor === undefined && isDragCreateShape(kind)) {
      armCreate(kind);
      return;
    }
    if (!sceneCore) return;
    const center = anchor ?? viewportCenterWorld();
    const object = buildInsertPrimitive(sceneCore, kind, center, freshId(kind), nextOrderKey(), selectedColor);
    authorOp({ kind: "insert-object", object });
    selection = { kind: "object", id: object.id };
    persistSelection(selection);
    showToast(`Inserted ${kind}`);
    // A freshly-created text object enters inline edit immediately.
    if (kind === "text") enterTextEdit(object.id);
  }

  // Arm the shape drag-create submode: the active tool flips to "create" and `createKind` holds which
  // shape the next pointer drag rubber-bands. A second click on the same shape disarms back to select.
  function armCreate(kind: DragCreateShape): void {
    if (activeTool === "create" && createKind === kind) {
      createKind = null;
      setActiveTool("select");
      return;
    }
    createKind = kind;
    createDrag = null;
    createHoverSnap = null;
    setActiveTool("create");
    status = `Drag to create ${kind}`;
  }

  // Drive shape drag-create. `start` anchors the bbox; `move` extends the current corner (snapped to the
  // nearest outline anchor when `snapped`, unless Alt-bypass); `end` commits a sized primitive and selects
  // it; `cancel` discards. A drag below MIN_DRAG_EXTENT_PX is a click: a default fixed-size shape at the start.
  function handleCreate(phase: "start" | "move" | "end" | "cancel", world: { x: number; y: number }, snappedIn: boolean, targetIdIn: string | null): void {
    const kind = createKind;
    if (!kind) return;
    if (phase === "end" && !sceneCore) return;
    // The snap query runs against the renderer's loaded regions, which include the TRANSIENT preview
    // (rides the same feed); its corner under the cursor self-snaps. Honor a snap ONLY when its target
    // is a real canonical object, so the ring + anchor fire on a real edge, never on the preview itself.
    const { snapped, target: targetId } = canonicalizeCreateSnap(scene, snappedIn, targetIdIn);
    if (phase === "start") {
      // A drag takes over the ring, so drop the pre-drag hover snap to avoid a doubled ring — but FIRST
      // capture it: a create STARTED on an edge anchors its START corner, so seed startSnap from the
      // press's own snap OR the hover ring the user aimed at.
      const hover = createHoverSnap;
      createHoverSnap = null;
      const hoverTarget =
        hover && hover.target && scene.objects.some((o) => o.id === hover.target) ? hover.target : null;
      const startSnap: CreateSnap | null =
        snapped && targetId
          ? { at: world, target: targetId }
          : hoverTarget
            ? { at: hover!.at, target: hoverTarget }
            : null;
      // Pull the start corner onto the edge it snapped to (so node 0 sits ON the
      // outline and its anchor binds cleanly); otherwise it's just the press point.
      const start = startSnap ? startSnap.at : world;
      createDrag = { span: { start, end: start }, snapped, target: targetId, startSnap, lastSnap: startSnap };
      return;
    }
    if (phase === "cancel") {
      createDrag = null;
      createHoverSnap = null;
      return;
    }
    if (!createDrag) return;
    if (phase === "move") {
      // Keep the last successful snap sticky: a move off the edge does NOT clear it, so a release just past the 8px tolerance can still reuse it.
      const moveSnap: CreateSnap | null = snapped && targetId ? { at: world, target: targetId } : createDrag.lastSnap;
      createDrag = { span: { start: createDrag.span.start, end: world }, snapped, target: targetId, startSnap: createDrag.startSnap, lastSnap: moveSnap };
      return;
    }
    // Commit a sized primitive (or a default at a click). A release that missed the snap reuses the gesture's last snap when it landed near it.
    const resolved = resolveCreateRelease(
      { end: world, snapped, target: targetId },
      createDrag.lastSnap,
      CREATE_ANCHOR_REUSE_TOLERANCE_PX / camera.zoom
    );
    const startSnap = createDrag.startSnap;
    const span: DragSpan = { start: createDrag.span.start, end: resolved.end };
    const snapTarget = resolved.target;
    createDrag = null;
    const dx = Math.abs(span.end.x - span.start.x);
    const dy = Math.abs(span.end.y - span.start.y);
    const tooSmall = kind === "line" ? dx < MIN_DRAG_EXTENT_PX && dy < MIN_DRAG_EXTENT_PX : dx < MIN_DRAG_EXTENT_PX || dy < MIN_DRAG_EXTENT_PX;
    const object = tooSmall
      ? sceneCore.buildPrimitive(kind, span.start, freshId(kind), nextOrderKey(), selectedColor)
      : sceneCore.buildPrimitiveFromDrag(kind, span, freshId(kind), nextOrderKey(), selectedColor);
    // A snapped drag-create binds the snapped CORNER(s) to the target's outline with a persistent anchor.
    // BOTH ends count (start drawn FROM an edge, end drawn TO one), each binding the nearest node, which
    // then reprojects through the target's transform so the object moves WITH the target. (Alt-create = no
    // anchor.) Same synthesizeReleaseAnchors the freehand pen uses — one release-anchor source for both tools.
    if (!tooSmall && sceneCore) {
      const anchors = synthesizeReleaseAnchors(sceneCore, scene.objects, object, [
        startSnap ? { target: startSnap.target, at: span.start } : null,
        snapTarget ? { target: snapTarget, at: span.end } : null
      ]);
      if (anchors.length) object.anchors = anchors;
    }
    authorOp({ kind: "insert-object", object });
    // Select the new object and return to the select tool so it can be moved/resized immediately.
    selectObject({ kind: "object", id: object.id });
    createKind = null;
    setActiveTool("select");
    showToast(`Inserted ${kind}`);
  }

  // Drive the PERSISTENT pre-drag anchor ring. A bare hover over an object's edge sets `createHoverSnap`;
  // off any edge clears it. Honors a snap ONLY onto a real canonical object so the ring never shows over
  // the preview's own outline. The DRAW tool rides the same probe so the pen shows the ring before pen-down.
  function handleCreateHover(world: { x: number; y: number }, snappedIn: boolean, targetIdIn: string | null): void {
    if (!createKind && activeTool !== "draw") return void (createHoverSnap = null);
    createHoverSnap = canonicalizeHoverSnap(scene, snappedIn, targetIdIn, world);
  }

  // Drive the freehand pen. Accumulate world points across start/move; on end, RECOGNIZE the stroke into
  // one canonical object through the wasm core and author an insert-object op (the tool stays sticky in
  // "draw"); cancel discards. Anchoring mirrors handleCreate: a snap is honored only onto a real canonical object.
  function handleDraw(
    phase: "start" | "move" | "end" | "cancel",
    world: { x: number; y: number },
    snap: { at: { x: number; y: number }; targetId: string } | null
  ): void {
    const canon: CreateSnap | null =
      snap && scene.objects.some((o) => o.id === snap.targetId) ? { at: snap.at, target: snap.targetId } : null;
    if (phase === "start") {
      // A stroke STARTED on an edge anchors its start: seed from the press's own snap OR the hover ring,
      // and pull the first sample onto the edge so node 0 sits ON the outline.
      const hover = createHoverSnap;
      createHoverSnap = null;
      const hoverSnap: CreateSnap | null =
        hover && hover.target && scene.objects.some((o) => o.id === hover.target)
          ? { at: hover.at, target: hover.target }
          : null;
      const startSnap = canon ?? hoverSnap;
      drawSnap = { start: startSnap, last: startSnap };
      drawPoints = [startSnap ? startSnap.at : world];
      return;
    }
    if (phase === "cancel") {
      drawPoints = null;
      drawSnap = null;
      createHoverSnap = null;
      return;
    }
    if (!drawPoints) return;
    if (phase === "move") {
      drawPoints = [...drawPoints, world];
      // Keep the last snap sticky and ride the same hover-ring state as create. Mid-stroke samples stay
      // raw (recognition normalizes the silhouette); only the start/release endpoints pull onto an edge.
      if (canon) drawSnap = { start: drawSnap?.start ?? null, last: canon };
      if (canon !== null || createHoverSnap !== null) createHoverSnap = canon;
      return;
    }
    // Resolve the release against the gesture's last snap (near-miss still anchors), then commit the recognized stroke (>=2 points have extent).
    const startSnap = drawSnap?.start ?? null;
    const recognizeMode = freeRecognitionHeld ? "free" : "basic";
    const resolved = resolveCreateRelease(
      { end: canon ? canon.at : world, snapped: canon !== null, target: canon?.target ?? null },
      drawSnap?.last ?? null,
      CREATE_ANCHOR_REUSE_TOLERANCE_PX / camera.zoom
    );
    drawSnap = null;
    createHoverSnap = null;
    const points = [...drawPoints, resolved.end];
    drawPoints = null;
    if (points.length < 2 || !sceneCore) return;
    // Multi-stroke merge, PRIORITY over insert + anchoring: a stroke end landing on an open-class
    // object's endpoint chains the stroke into it (edit-geometry on the survivor, no insert). Every
    // judgment lives in the core; the shell only branches on the returned ops. Null = no merge.
    const mergeOps = sceneCore.mergeOpenStrokeOps(
      scene,
      points,
      recognizeMode,
      MERGE_ENDPOINT_TOLERANCE_PX / camera.zoom
    );
    if (mergeOps && mergeOps.length > 0) {
      authorOp(mergeOps.length === 1 ? mergeOps[0] : { kind: "batch", ops: mergeOps });
      const survivor = mergeOps.find((op) => op.kind === "edit-geometry");
      if (survivor && survivor.kind === "edit-geometry") selectObject({ kind: "object", id: survivor.id });
      return;
    }
    // freehandToObject is a hex API, so the theme-default sentinel can't pass through — lower with a
    // placeholder hex, then swap the stroke paint to the "text" token so the stroke flips with the theme.
    const strokeHex = selectedColor === THEME_DEFAULT_COLOR ? "#000000" : selectedColor;
    const object = sceneCore.freehandToObject(points, strokeHex, penWidthPx, freshId("draw"), nextOrderKey(), recognizeMode);
    if (selectedColor === THEME_DEFAULT_COLOR && object.stroke) object.stroke.paint = previewPaint(selectedColor);
    // An OPEN recognition authors endpoint anchors through the same release path as drag-create; a CLOSED recognition never anchors.
    if (sceneCore.isOpenClassD(object.geometry.d)) {
      const anchors = synthesizeReleaseAnchors(sceneCore, scene.objects, object, [
        startSnap ? { target: startSnap.target, at: startSnap.at } : null,
        resolved.target ? { target: resolved.target, at: resolved.end } : null
      ]);
      if (anchors.length) object.anchors = anchors;
    }
    authorOp({ kind: "insert-object", object });
    // Select the freshly-drawn stroke; the pen tool stays sticky in "draw".
    selectObject({ kind: "object", id: object.id });
  }

  // Erase the stroke under the cursor. Default = whole-stroke delete. The partial modifier cuts the
  // stroke's subpath at the touch via the scene-core split (world touch → object-local quantized space,
  // split, then `edit-geometry`, or delete when the cut empties it). A drag erases continuously.
  function handleErase(id: string, world: { x: number; y: number }, partial: boolean): void {
    const target = scene.objects.find((o) => o.id === id);
    if (!target) return;
    if (!partial || !sceneCore) {
      authorOp({ kind: "delete", id });
      if (selection.kind === "object" && selection.id === id) selectObject({ kind: "canvas" });
      return;
    }
    const local = worldToObjectLocalQuantized(target, world);
    if (!local) {
      authorOp({ kind: "delete", id });
      return;
    }
    // The core returns the WHOLE op batch: [] on a miss, [delete] when the cut empties the object, else
    // [edit-geometry, ...followers]. The shell authors the result and owns the UI follow-up (clearing a stale selection).
    const ops = sceneCore.partialEraseOps(scene, id, local.x, local.y, ERASE_RADIUS_QUANTIZED);
    if (ops.length === 0) return; // touch missed: stroke left whole
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
    if (ops.some((op) => op.kind === "delete") && selection.kind === "object" && selection.id === id) {
      selectObject({ kind: "canvas" });
    }
  }

  function deleteSelection(): void {
    const ids = currentSelectionIds();
    if (ids.length === 0) return;
    if (ids.length === 1) authorOp({ kind: "delete", id: ids[0] });
    else authorOp({ kind: "batch", ops: ids.map((id) => ({ kind: "delete", id }) as ObjectOp) });
    selection = { kind: "canvas" };
    persistSelection(selection);
  }

  function duplicateSelection(): void {
    const ids = currentSelectionIds();
    const ops: ObjectOp[] = [];
    let order = nextOrderKey();
    for (const id of ids) {
      const src = scene.objects.find((o) => o.id === id);
      if (!src) continue;
      const clone: SceneObject = {
        ...src,
        id: freshId("dup"),
        order,
        transform: shiftTransform(src.transform, 40, 40)
      };
      order = `${order}~`;
      ops.push({ kind: "insert-object", object: clone });
    }
    if (ops.length === 0) return;
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
    showToast("Duplicated selection");
  }

  // Reparent the selected objects under a fresh frame object: a rect sized to their union world-AABB,
  // positioned at the AABB's top-left. Children transforms are world-absolute, so they reparent unchanged.
  function groupSelection(): void {
    const ids = currentSelectionIds();
    // Group works on 1+ objects — a single object under a fresh container is a valid one-child frame.
    if (ids.length < 1) return;
    const objects = ids.map((id) => scene.objects.find((o) => o.id === id)).filter((o): o is SceneObject => Boolean(o));
    const bounds = unionWorldAabb(objects);
    const { ops, frameId } = buildGroupOps(ids, objects, bounds, freshId("frame"), nextOrderKey(), rectPathQuantized);
    authorOp({ kind: "batch", ops });
    selection = { kind: "object", id: frameId };
    persistSelection(selection);
    showToast("Grouped selection");
  }

  // Ungroup reparents children out of the frame, then deletes the now-empty frame in the SAME batch op.
  function ungroupSelection(): void {
    if (selection.kind !== "object") return;
    const id = selection.id;
    const children = scene.objects.filter((o) => o.parent === id);
    if (children.length === 0) return;
    const ops: ObjectOp[] = children.map((child) => ({ kind: "reparent", id: child.id, order: child.order }));
    ops.push({ kind: "delete", id });
    authorOp({ kind: "batch", ops });
    selection = { kind: "canvas" };
    persistSelection(selection);
    showToast("Ungrouped selection");
  }

  // Pop the right-clicked child out one level — reparent to its grandparent (or canvas root). Authored by `popOutOp`; a non-child target yields null.
  function popOutSelection(picked: ObjectSelection): void {
    if (picked.kind !== "object" || !sceneCore) return;
    const op = sceneCore.popOutOp(scene, picked.id);
    if (!op) return;
    authorOp(op);
    showToast("Popped out one level");
  }

  function nudgeSelection(dx: number, dy: number): void {
    const ids = currentSelectionIds();
    const ops: ObjectOp[] = [];
    for (const id of ids) {
      const src = scene.objects.find((o) => o.id === id);
      if (!src) continue;
      ops.push({ kind: "set-transform", id, transform: shiftTransform(src.transform, dx, dy) });
    }
    if (ops.length === 0) return;
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
  }

  function reorderSelection(direction: "front" | "back"): void {
    const ids = currentSelectionIds();
    if (ids.length === 0) return;
    const order = direction === "front" ? nextOrderKey() : backOrderKey();
    const ops: ObjectOp[] = ids.map((id) => ({ kind: "reorder", id, order }));
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
  }

  // True single-step reorder: swap the selected object's order with its next-higher ("forward") or
  // next-lower ("backward") neighbor. Single selection only (a multi-set has no well-defined neighbor).
  function reorderStep(direction: "forward" | "backward"): void {
    const ids = currentSelectionIds();
    if (ids.length !== 1) {
      // Fall back to front/back for the multi case (no single neighbor to swap).
      reorderSelection(direction === "forward" ? "front" : "back");
      return;
    }
    const id = ids[0];
    const sorted = [...scene.objects].sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
    const index = sorted.findIndex((o) => o.id === id);
    if (index < 0) return;
    const neighborIndex = direction === "forward" ? index + 1 : index - 1;
    const neighbor = sorted[neighborIndex];
    if (!neighbor) return;
    const self = sorted[index];
    authorOp({
      kind: "batch",
      ops: [
        { kind: "reorder", id: self.id, order: neighbor.order },
        { kind: "reorder", id: neighbor.id, order: self.order }
      ]
    });
  }

  function backOrderKey(): string {
    const minOrder = scene.objects.reduce((min, o) => (o.order < min ? o.order : min), "z");
    // A short ascii key below "a" so it sorts before the current min (lands at the back).
    return minOrder > "0" ? "0" : `0${minOrder}`;
  }

  function selectAll(): void {
    const ids = scene.objects.map((o) => o.id);
    if (ids.length === 0) return;
    selection = ids.length === 1 ? { kind: "object", id: ids[0] } : { kind: "multi", ids };
    persistSelection(selection);
  }

  // Rename / set text on the selected object via a set-text op.
  function renameSelected(text: string): void {
    if (selection.kind !== "object") return;
    authorOp({ kind: "set-text", id: selection.id, text: { runs: [{ text }] } });
  }

  // Adopt the toolbar's color (the default for the next NEW shape), then recolor a single selection via a `set-style` op.
  function applySelectedColor(color: string): void {
    selectedColor = color;
    if (!sceneCore) return;
    const op = buildColorApplyOp(sceneCore, scene, selection, color);
    if (op) authorOp(op);
  }

  // A double-click on a container drills in (sets the active container); a leaf enters inline text edit.
  // A null signal (missed every object) is a no-op; the container-vs-leaf decision lives in the core.
  function handleObjectDoubleClick(signal: { id: string; hasChildren: boolean } | null): void {
    if (!signal || !sceneCore) return;
    const action = resolveDoubleClick(sceneCore, scene, signal);
    if (action.kind === "drill-in") {
      activeContainer = action.id;
      selectObject({ kind: "object", id: action.id });
      showToast("Entered group");
      return;
    }
    if (action.kind === "edit-leaf") enterTextEdit(action.id);
  }

  // Enter inline edit: seed the overlay value from the object's first text run (empty when the object
  // hasn't landed in `scene` yet — `textEditRect` derives reactively, so the overlay appears on arrival).
  function enterTextEdit(id: string): void {
    const object = scene.objects.find((o) => o.id === id);
    textEdit = { id, value: object?.text?.runs?.[0]?.text ?? "" };
  }

  // Commit the in-progress inline edit as a set-text op (only when the text changed),
  // then dismiss the overlay. Called on blur or Enter.
  function commitTextEdit(): void {
    const edit = textEdit;
    textEdit = null;
    if (!edit) return;
    const object = scene.objects.find((o) => o.id === edit.id);
    if (!object) return;
    const current = object.text?.runs?.[0]?.text ?? "";
    if (edit.value === current) return;
    authorOp({ kind: "set-text", id: edit.id, text: { runs: [{ text: edit.value }] } });
  }

  // Discard the in-progress inline edit without committing (Esc).
  function cancelTextEdit(): void {
    textEdit = null;
  }

  // Svelte action — seed the contenteditable with the object's text and focus it (caret at the end) on mount.
  function mountTextEdit(node: HTMLDivElement, value: string) {
    node.textContent = value;
    node.focus();
    const selection = window.getSelection();
    if (selection) {
      const range = document.createRange();
      range.selectNodeContents(node);
      range.collapse(false);
      selection.removeAllRanges();
      selection.addRange(range);
    }
  }

  // The templates the scroll-popup offers; ids map to buildObjectTemplate.
  const TEMPLATES: TemplatePopupItem[] = [
    { id: "todo_board", title: "Todo board", desc: "Grouped To do / In progress / Done columns" },
    { id: "decision_map", title: "Decision map", desc: "Options and outcomes wired with connectors" },
    { id: "presentation", title: "Presentation", desc: "A deck grouping title and content slides" }
  ];

  function toggleTemplates(): void {
    templateOpen = !templateOpen;
  }

  // Lower the template to a recipe of inline-styled objects via the wasm core, then send a single templateApply feature frame (the server lowers it to insert-object ops).
  function applyTemplate(templateId: string): void {
    if (!sceneCore || !sceneClient) return;
    const anchor = templateAnchor();
    const recipe = sceneCore.buildObjectTemplate(templateId, anchor.x, anchor.y, freshId("tpl"));
    if (recipe.length === 0) {
      status = "Template produced no objects";
      return;
    }
    sceneClient.sendFeature({
      feature: "templateApply",
      canvas_id: canvasId,
      recipe,
      anchor_x: anchor.x,
      anchor_y: anchor.y
    });
    templateOpen = false;
    status = `Inserting ${templateId}`;
  }

  function templateAnchor(): { x: number; y: number } {
    if (scene.objects.length === 0) return viewportCenterWorld();
    let maxX = -Infinity;
    let minY = Infinity;
    for (const object of scene.objects) {
      const [tx, ty] = transformOrigin(object.transform);
      maxX = Math.max(maxX, tx);
      minY = Math.min(minY, ty);
    }
    return { x: Number.isFinite(maxX) ? maxX + 240 : 120, y: Number.isFinite(minY) ? minY : 120 };
  }

  // ----- comments / export (feature frames) -------------------------------

  function addCommentToSelected(): void {
    if (selection.kind !== "object" || !sceneClient) return;
    const body = window.prompt("Comment");
    if (!body) return;
    sceneClient.sendFeature({
      feature: "commentUpsert",
      canvas_id: canvasId,
      object_id: selection.id,
      comment: { id: freshId("c"), author: userIdentity(), body }
    });
  }

  function exportSelection(): void {
    if (!sceneClient) return;
    const scopeIds = currentSelectionIds();
    sceneClient.sendFeature({
      feature: "exportRequest",
      canvas_id: canvasId,
      scope_ids: scopeIds,
      export_type: "markdown",
      request_id: freshId("export")
    });
    status = "Exporting";
  }

  // ----- selection helpers ------------------------------------------------

  function currentSelectionIds(): string[] {
    if (selection.kind === "object") return [selection.id];
    if (selection.kind === "multi") return selection.ids;
    return [];
  }

  function persistSelection(next: ObjectSelection): void {
    if (!sceneClientReady || !sceneClient) return;
    sceneClient.saveSelection(next);
  }

  function selectObject(next: ObjectSelection): void {
    selection = validSelection(scene, next);
    persistSelection(selection);
  }

  function validSelection(currentScene: ObjectScene, currentSelection: ObjectSelection): ObjectSelection {
    if (currentSelection.kind === "canvas") return currentSelection;
    if (currentSelection.kind === "object") {
      return currentScene.objects.some((o) => o.id === currentSelection.id) ? currentSelection : { kind: "canvas" };
    }
    // multi: keep only live ids; collapse to object/canvas as the set shrinks.
    const live = currentSelection.ids.filter((id) => currentScene.objects.some((o) => o.id === id));
    if (live.length >= 2) return { kind: "multi", ids: live };
    if (live.length === 1) return { kind: "object", id: live[0] };
    return { kind: "canvas" };
  }

  function handleEscape(): void {
    // Escape cancels, in priority order: inline text edit, pen stroke, shape drag-create, then disarms the tool.
    if (textEdit) return void cancelTextEdit();
    if (drawPoints) {
      drawPoints = null;
      drawSnap = null;
      return;
    }
    if (createDrag) return void (createDrag = null);
    if (createKind) {
      createKind = null;
      return void setActiveTool("select");
    }
    if (settingsOpen) return void (settingsOpen = false);
    if (templateOpen) return void (templateOpen = false);
    if (diagnosticsOpen) return void (diagnosticsOpen = false);
    if (contextMenu) return void (contextMenu = null);
    selectObject({ kind: "canvas" });
  }

  function setActiveTool(tool: ActiveTool): void {
    // Leaving the create tool disarms the shape and drops the persistent hover ring.
    if (tool !== "create") {
      createKind = null;
      createDrag = null;
      createHoverSnap = null;
    }
    activeTool = tool;
  }

  function shortcutHandlers() {
    return {
      "select-move": () => setActiveTool("select"),
      draw: () => setActiveTool("draw"),
      "insert-rectangle": () => insertPrimitive("rectangle"),
      "insert-ellipse": () => insertPrimitive("ellipse"),
      "insert-line": () => insertPrimitive("line"),
      "insert-text": () => insertPrimitive("text"),
      "insert-frame": () => insertPrimitive("frame"),
      delete: () => deleteSelection(),
      duplicate: () => duplicateSelection(),
      group: () => groupSelection(),
      ungroup: () => ungroupSelection(),
      "select-all": () => selectAll(),
      "nudge-up": () => nudgeSelection(0, -8),
      "nudge-down": () => nudgeSelection(0, 8),
      "nudge-left": () => nudgeSelection(-8, 0),
      "nudge-right": () => nudgeSelection(8, 0),
      "bring-to-front": () => reorderSelection("front"),
      "send-to-back": () => reorderSelection("back"),
      "bring-forward": () => reorderStep("forward"),
      "send-backward": () => reorderStep("backward"),
      undo: () => undo(),
      redo: () => redo(),
      // Enter on a selected object enters inline edit instead of a blocking prompt.
      "edit-text": () => {
        if (selection.kind === "object") enterTextEdit(selection.id);
      },
      "add-comment": () => addCommentToSelected(),
      "clear-selection": () => selectObject({ kind: "canvas" }),
      "zoom-in": () => zoomAtCenter(-160),
      "zoom-out": () => zoomAtCenter(160),
      "zoom-fit": () => host?.fitScene(),
      "open-settings": () => (settingsOpen = !settingsOpen),
      "open-template-library": () => toggleTemplates()
    };
  }

  // Hit-test the object under the cursor and open a menu that acts on it.
  function handleContextMenuRequest(point: { x: number; y: number }): void {
    if (!canvasWrap) return;
    pendingContextScreen = { clientX: point.x, clientY: point.y };
    const rect = canvasWrap.getBoundingClientRect();
    const id = host?.hitTestObjectAt(point.x - rect.left, point.y - rect.top) ?? null;
    const target = contextTargetFromPick(id);
    // Right-clicking an object selects it (unless it is already part of the
    // current multi-select) so the menu acts on the right-clicked object.
    if (target.kind !== "canvas") selectObject(target);
    handleContextPick(target);
  }

  // Build the context-menu target from a picked id; if the pick is part of the current multi-select, keep the whole multi.
  function contextTargetFromPick(id: string | null): ObjectSelection {
    if (!id) return { kind: "canvas" };
    if (selection.kind === "multi" && selection.ids.includes(id)) return selection;
    return { kind: "object", id };
  }

  function handleContextPick(picked: ObjectSelection): void {
    const anchor = pendingContextScreen;
    pendingContextScreen = null;
    if (!canvasWrap || !anchor) return;
    const rect = canvasWrap.getBoundingClientRect();
    const world = screenToWorld({ x: anchor.clientX - rect.left, y: anchor.clientY - rect.top }, camera);
    contextMenu = { selection: picked, x: anchor.clientX, y: anchor.clientY, world };
  }

  $effect(() => {
    if (!contextMenu) return;
    const dismiss = () => (contextMenu = null);
    window.addEventListener("pointerdown", dismiss);
    return () => window.removeEventListener("pointerdown", dismiss);
  });

  // The menu layout + resolver live in controller/interactions; here we only decorate the icon tag with the lucide component and gate `enabled` on core queries.
  const MENU_ICONS: Record<string, typeof Copy> = {
    copy: Copy,
    group: GroupIcon,
    ungroup: Ungroup,
    comment: MessageSquarePlus,
    template: LayoutTemplate,
    trash: Trash2
  };

  // An entry's `enabled` predicate: ungroup needs a container, pop-out needs a parent (both core-only queries, disabled until the core loads); everything else is enabled.
  function contextEntryEnabled(entry: Extract<ContextMenuEntry<string>, { id: string }>, picked: ObjectSelection): boolean {
    if (entry.id === "ungroup") return !!sceneCore && ungroupPickEnabledOf(sceneCore, scene, picked);
    if (entry.id === "pop-out") return !!sceneCore && popOutPickEnabledOf(sceneCore, scene, picked);
    return true;
  }

  function contextMenuItems(menu: ContextMenuState): (ContextMenuItem | null)[] {
    const handlers = contextHandlers(menu);
    const resolved = resolveContextMenuItems<string>(
      menu.selection,
      commandCatalog,
      (id) => handlers[id] !== undefined,
      contextEntryEnabled
    );
    return resolved.map((item) =>
      item === null
        ? null
        : {
            label: item.label,
            icon: item.icon ? MENU_ICONS[item.icon] : undefined,
            danger: item.danger,
            disabled: item.disabled,
            onSelect: () => closeContextThen(handlers[item.id])
          }
    );
  }

  // Map a context-menu command id to the existing shell handler. Canvas inserts
  // anchor at the right-click world point (menu.world).
  function contextHandlers(menu: ContextMenuState): Record<string, () => void> {
    const base = shortcutHandlers();
    return {
      duplicate: base.duplicate,
      group: base.group,
      ungroup: base.ungroup,
      // Operates on the picked object id, not the active selection.
      "pop-out": () => popOutSelection(menu.selection),
      "bring-to-front": base["bring-to-front"],
      "send-to-back": base["send-to-back"],
      "add-comment": base["add-comment"],
      delete: base.delete,
      "insert-rectangle": () => insertPrimitive("rectangle", menu.world),
      "insert-ellipse": () => insertPrimitive("ellipse", menu.world),
      "open-template-library": base["open-template-library"],
      "select-all": base["select-all"]
    };
  }
  function closeContextThen(action: () => void): void {
    contextMenu = null;
    action();
  }

  function contextMenuTitle(picked: ObjectSelection): string {
    if (picked.kind === "object") return `object:${picked.id}`;
    if (picked.kind === "multi") return `${picked.ids.length} objects`;
    return "Canvas";
  }

  // ----- camera helpers ----------------------------------------------------

  function zoomAtCenter(deltaY: number): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    host.wheelAtScreen({ x: rect.width / 2, y: rect.height / 2 }, deltaY);
  }

  async function toggleFullscreen(): Promise<void> {
    if (!canvasWrap) return;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await canvasWrap.requestFullscreen();
    } catch (error) {
      status = error instanceof Error ? error.message : "Fullscreen failed";
    }
  }

  function handleHost(next: ShapeCanvasHost): void {
    host = next;
    host.loadObjectScene(scene, selection);
    host.setTool(activeTool);
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!sceneClientReady || !sceneClient || !canvasWrap) return;
    const now = Date.now();
    if (now - lastCursorSentAt < CURSOR_THROTTLE_MS) return;
    lastCursorSentAt = now;
    const rect = canvasWrap.getBoundingClientRect();
    const cursor = screenToWorld({ x: event.clientX - rect.left, y: event.clientY - rect.top }, camera);
    const topLeft = screenToWorld({ x: 0, y: 0 }, camera);
    const bottomRight = screenToWorld({ x: rect.width, y: rect.height }, camera);
    sceneClient.sendCursor(cursor, { x: topLeft.x, y: topLeft.y, width: bottomRight.x - topLeft.x, height: bottomRight.y - topLeft.y });
  }

  // Bind the ambient page seams (window.location, localStorage, crypto.randomUUID) once and delegate to the controller/session helpers.

  function wsBaseUrl(): string {
    return wsBaseUrlOf(window.location);
  }

  function clientIdentity(): string {
    return clientIdentityOf(() => crypto.randomUUID());
  }

  function userIdentity(): string {
    return userIdentityOf(window.localStorage, () => crypto.randomUUID());
  }
</script>

<div class="app-shell">
  <main class="studio-stage">
    <section class="canvas-panel">
      <div class="flow-wrap renderer-scene-surface" data-tool={activeTool} data-affordance={cursorAffordance} bind:this={canvasWrap} role="application" aria-label="Canvas" onpointermove={handlePointerMove}>
        <div class="canvas-watermark" aria-hidden="true">
          <BrainCircuit size={28} />
          <span>shape.ai</span>
        </div>
        <PeerCursors {peers} {camera} />

        <Toolbar
          {activeTool}
          {createKind}
          {penWidthPx}
          penPalette={PEN_PALETTE}
          penWidths={PEN_WIDTHS}
          {selectedColor}
          dark={theme === "dark"}
          {busy}
          {templateOpen}
          {diagnosticsOpen}
          {selectedObject}
          {canvases}
          activeCanvasId={canvasId}
          {connectionStatus}
          {canvasBusy}
          onSetTool={setActiveTool}
          onSetPenWidth={(width) => (penWidthPx = width)}
          onSelectColor={applySelectedColor}
          onInsertPrimitive={insertPrimitive}
          onToggleTemplates={toggleTemplates}
          onZoomIn={() => zoomAtCenter(-160)}
          onZoomOut={() => zoomAtCenter(160)}
          onFit={() => host?.fitScene()}
          onFullscreen={() => void toggleFullscreen()}
          onToggleDiagnostics={() => (diagnosticsOpen = !diagnosticsOpen)}
          onExport={exportSelection}
          onSelectCanvas={(id) => void switchToCanvas(id)}
          onCreateCanvas={(title) => void createCanvas(title)}
          onDeleteCanvas={(id) => void deleteCanvas(id)}
          onRenameSelected={renameSelected}
          onDeleteSelected={deleteSelection}
        />

        <button
          class="icon-button theme-toggle"
          type="button"
          aria-label="Toggle dark mode"
          aria-pressed={theme === "dark"}
          onclick={toggleTheme}
        >
          {#if theme === "dark"}<Sun size={16} />{:else}<Moon size={16} />{/if}
        </button>

        {#if templateOpen}
          <TemplatePopup items={TEMPLATES} onSelect={applyTemplate} />
        {/if}

        {#if diagnosticsOpen}
          <div class="diagnostics-panel" role="status" aria-label="Renderer diagnostics">
            <div>state: {readyState}</div>
            <div>{rendererDetail}</div>
            <div>objects: {scene.objects.length}</div>
            <div>frame: {rendererStats?.frameMs?.toFixed?.(2) ?? "—"} ms</div>
            <div>camera: {camera.x.toFixed(0)}, {camera.y.toFixed(0)} @ {camera.zoom.toFixed(2)}x</div>
          </div>
        {/if}

        <CanvasHost
          initialCamera={camera}
          callbacks={hostCallbacks}
          {readyState}
          {rendererDetail}
          {hasRenderableScene}
          onHost={handleHost}
          onContextMenuRequest={handleContextMenuRequest}
        />

        {#if textEdit && textEditRect}
          <div
            class="text-edit-overlay"
            contenteditable="plaintext-only"
            role="textbox"
            tabindex="0"
            aria-label="Edit text"
            style:left={`${textEditRect.x}px`}
            style:top={`${textEditRect.y}px`}
            style:width={`${textEditRect.width}px`}
            style:height={`${textEditRect.height}px`}
            use:mountTextEdit={textEdit.value}
            oninput={(event) => {
              if (textEdit) textEdit = { id: textEdit.id, value: (event.currentTarget as HTMLDivElement).textContent ?? "" };
            }}
            onkeydown={(event) => {
              if (event.isComposing) return;
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                event.stopPropagation();
                commitTextEdit();
              } else if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                cancelTextEdit();
              }
            }}
            onblur={() => commitTextEdit()}
          ></div>
        {/if}
      </div>

      {#if contextMenu}
        <ContextMenu x={contextMenu.x} y={contextMenu.y} title={contextMenuTitle(contextMenu.selection)} items={contextMenuItems(contextMenu)} />
      {/if}

      {#if settingsOpen}
        <SettingsModal catalog={commandCatalog} gestures={gestureCatalog} onClose={() => (settingsOpen = false)} />
      {/if}

      {#if busy || status !== "Ready"}
        <div class="canvas-status" role="status">
          {#if busy}<Loader2 class="spin" size={15} />{/if}
          {status}
        </div>
      {/if}

      <!-- Keyed by message so each new notice remounts and replays the fade-in/out. -->
      {#if toast}
        {#key toast}
          <div class="canvas-toast" role="status" aria-live="polite">{toast}</div>
        {/key}
      {/if}
    </section>
  </main>
</div>
