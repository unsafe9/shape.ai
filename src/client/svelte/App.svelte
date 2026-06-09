<script lang="ts">
  import { onDestroy } from "svelte";
  import { BrainCircuit, Loader2, Copy, Trash2, Group as GroupIcon, Ungroup, MessageSquarePlus, LayoutTemplate, Sun, Moon } from "lucide-svelte";
  import { screenToWorld, applyDocumentTheme, readStoredTheme, type Theme } from "../renderer/scene";
  import type { CameraState } from "../../shared/geometry";
  import {
    emptyObjectScene,
    translateTransform,
    toggleObjectSelection,
    IDENTITY_TRANSFORM,
    GEOMETRY_QUANTUM_PER_PX,
    type Object as SceneObject,
    type ObjectOp,
    type ObjectScene,
    type ObjectSelection,
    type FeatureResponse
  } from "../../shared/object";
  import {
    ShapeCanvasHost,
    type RendererHealth,
    type RendererStats,
    type ShapeCanvasHostCallbacks
  } from "../lib/canvasHost";
  import { SceneClient, type CanvasSummary } from "../lib/sceneClient";
  import type { PeerPresence } from "../lib/peers";
  import type { ConnectionStatus } from "../lib/wsTransport";
  import type { ActiveTool, TransformKind } from "../renderer/engine";
  import { cursorAffordance as deriveCursorAffordance } from "../lib/cursor";
  import type { HoverAffordance } from "../renderer/wasmLoader";
  import {
    loadSceneCore,
    type ObjectCommand,
    type ObjectGesture,
    type SceneCore,
    type UndoStack
  } from "../scene/sceneCoreWasm";
  import { createShortcutDispatcher } from "../lib/shortcuts";
  import { ToastChannel } from "../lib/statusChannel";
  import {
    buildPrimitiveObject,
    buildPrimitiveObjectFromDrag,
    buildSetStyleOp,
    MIN_DRAG_EXTENT_PX,
    paintForColor,
    textOverlayScreenRect,
    THEME_DEFAULT_COLOR,
    type DragSpan
  } from "../lib/objectPrimitives";
  import { isDragCreateShape, type DragCreateShape, type PrimitiveKindId } from "../lib/toolbar";
  import { cascadeTransformOps, cascadeMultiTransformOps } from "../lib/transformCascade";
  import { doubleClickAction, ungroupEnabled, popOutOp } from "../lib/grouping";
  import Toolbar from "./Toolbar.svelte";
  import SettingsModal from "./SettingsModal.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";
  import ContextMenu, { type ContextMenuItem } from "./ContextMenu.svelte";
  import TemplatePopup, { type TemplatePopupItem } from "./TemplatePopup.svelte";
  import PeerCursors from "./PeerCursors.svelte";

  // ----- canonical object scene (D1) -----
  let scene = $state<ObjectScene>(emptyObjectScene());

  // ----- selection: single ObjectSelection + transient multi -----
  let selection = $state<ObjectSelection>({ kind: "canvas" });

  // ----- AP3 (#9): active container (drill-in). A double-click on a container
  //       object (one with children, per RA2b's drill-in signal) drills into it; the
  //       shell holds the active container id so subsequent edits scope to it. Null =
  //       the canvas root is active. -----
  let activeContainer = $state<string | null>(null);

  // ----- W2-11: non-destructive live object transform preview, now GPU-side. The
  //       canonical `scene` is never mutated mid-drag; the dragged object's instance
  //       model matrix is pushed straight to the GPU (engine.setObjectPreviewTransform)
  //       with zero re-tessellation (P4). The commit op on pointer-up still captures
  //       the correct inverse (original transform), keeping undo correct (D21). -----

  // FC-16: in the connected path authorOp's scene update is async (it arrives via
  // commitClientScene), so the GPU instance matrix is the ONLY thing holding the
  // object at its previewed position during the commit window. Remember the
  // committed {id, transform}; the next canonical scene that reflects it triggers the
  // single rebake (via the feedScene->loadObjectScene $effect), which drops the
  // stale preview matrix with no snap-back.
  let pendingCommit: { id: string; transform: SceneObject["transform"] } | null = null;

  // ----- FC-11: freehand pen. While the draw tool is active, the in-progress
  //       stroke's world points accumulate here; `feedScene` appends a transient
  //       preview object so the stroke is visible before it commits. On pointer-up
  //       the points lower to a committed `Object` via the wasm core. -----
  let drawPoints = $state<{ x: number; y: number }[] | null>(null);
  // W2-08: pen brush settings are now reactive state the draw-mode sub-toolbar
  // drives (color + width); epsilon (RDP simplification) stays a constant. The
  // freehand commit + the live preview both read the current brush.
  const PEN_EPSILON = 2.0;
  let penWidthPx = $state(2);
  // D1/#5: the toolbar's always-visible selected color. It is the default fill/
  // stroke for the next NEW shape; recoloring a selected object authors a SetStyle
  // op (applySelectedColor). The native picker passes a CSS hex through verbatim.
  let selectedColor = $state(THEME_DEFAULT_COLOR);
  // W2-08: draw-mode palette + brush sizes the sub-toolbar offers. S2 (#5): the
  // first entry is the theme-default token sentinel, then the fixed hex swatches.
  const PEN_PALETTE = [THEME_DEFAULT_COLOR, "#1f2933", "#ef4444", "#3b82f6", "#22c55e", "#f59e0b", "#ffffff"];
  const PEN_WIDTHS = [1, 2, 4, 8];
  // W2-08: partial-erase cut radius in object-local quantized units (~12 logical
  // px). A node within this radius of the touch is removed when the stroke is cut.
  const ERASE_RADIUS_QUANTIZED = 12 * GEOMETRY_QUANTUM_PER_PX;

  // ----- W2-07: shape drag-create. Arming a shape tool sets `createKind`; the
  //       pointer down-drag-up rubber-bands a bbox accumulated in `createDrag`
  //       (start corner + current corner + whether the current corner is snapped
  //       to an outline anchor). `feedScene` appends a transient preview object so
  //       the rubber-band shows before it commits; on pointer-up the span lowers to
  //       a sized primitive via an insert-object op, then the object is selected. -----
  let createKind = $state<DragCreateShape | null>(null);
  // AP5 (#14): `target` is the id of the object whose outline the dragged corner
  // snapped to (null when not snapped); it is captured per phase so the commit can
  // synthesize a persistent anchor binding the created endpoint to that target.
  let createDrag = $state<{ span: DragSpan; snapped: boolean; target: string | null } | null>(null);
  // W3-G9 (#3): the pre-drag hover snap. While the create tool is armed and no
  // button is down, a bare hover over an existing object's edge sets this to the
  // snapped world point + target id; `feedScene` renders a PERSISTENT anchor ring
  // from it (only while `createDrag` is null, so a drag's own ring takes over). Null
  // when the cursor is off any edge, or once a drag starts / the tool disarms.
  let createHoverSnap = $state<{ at: { x: number; y: number }; target: string | null } | null>(null);

  // ----- W2-10: inline text editing. `textEdit` holds the id of the object being
  //       edited and its in-progress value; a contenteditable overlay is positioned
  //       over the object's screen bbox (worldToScreen) while it is non-null. Blur or
  //       Enter commits a set-text op; Esc cancels. The platform contenteditable
  //       handles IME. Creating a text object enters edit immediately; pressing Enter
  //       on a selected object enters edit. -----
  let textEdit = $state<{ id: string; value: string } | null>(null);

  // ----- ephemeral camera / chrome -----
  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.6 });
  let status = $state("Ready");
  // AP6 (#19): the transient toast channel. Persistent hints stay in `status`
  // (no timer); transient ACTION notices ("Inserted rectangle", "Comment added",
  // "Export ready") flow through `showToast`, render below `.canvas-status`, and
  // auto-dismiss after ~2.5s with a CSS fade. The pure ToastChannel owns the
  // lifecycle; the real setTimeout/clearTimeout are injected here so scene-core
  // stays time-free and the state machine is unit-testable.
  let toast = $state<string | null>(null);
  const toastChannel = new ToastChannel(
    { set: (cb, ms) => setTimeout(cb, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) },
    (message) => (toast = message)
  );
  // A transient action notice (auto-dismisses). Persistent text keeps using
  // `status = ...`; AP1 etc. should reuse THIS for one-shot confirmations.
  function showToast(message: string): void {
    toastChannel.show(message);
  }
  let busy = $state(false);
  let diagnosticsOpen = $state(false);
  let settingsOpen = $state(false);
  // AP4 (#12c): light/dark theme. Initialized from localStorage; the $effect below
  // persists + flips the root `data-theme` attribute (dark-mode CSS) AND drives
  // RB1's renderer theme-bit (`setObjectTheme`) so canvas + chrome flip together.
  let theme = $state<Theme>(readStoredTheme(window.localStorage));
  function toggleTheme(): void {
    theme = theme === "dark" ? "light" : "dark";
  }
  // Single applier: on init and on every toggle, flip the root `data-theme`
  // attribute (dark-mode CSS), persist the choice, and drive RB1's renderer
  // theme-bit so the canvas flips with the chrome.
  $effect(() => {
    applyDocumentTheme(theme, {
      root: document.documentElement,
      storage: window.localStorage,
      setRendererTheme: (dark) => host?.setObjectTheme?.(dark)
    });
  });
  let templateOpen = $state(false);
  let activeTool = $state<ActiveTool>("select");
  // W2-03: Space-hold pan + dynamic hover cursor. `spaceHeld` flips the empty
  // cursor to grab and arms the engine pan path; `affordance` is the core's
  // per-move hover classification, mapped to a CSS cursor on the canvas wrapper.
  let spaceHeld = $state(false);
  let affordance = $state<HoverAffordance>("empty");

  // ----- context menu (U3) -----
  type ContextMenuState = { selection: ObjectSelection; x: number; y: number; world: { x: number; y: number } };
  let contextMenu = $state<ContextMenuState | null>(null);
  let pendingContextScreen: { clientX: number; clientY: number } | null = null;

  // ----- realtime peers -----
  let peers = $state<PeerPresence[]>([]);
  let lastCursorSentAt = 0;
  const CURSOR_THROTTLE_MS = 40;

  // ----- renderer readout -----
  let rendererStats = $state<RendererStats | null>(null);
  let rendererHealth = $state<RendererHealth | null>(null);

  // ----- data layer (the single object data path) -----
  let canvasId = $state("default");
  let sceneClient: SceneClient | null = null;
  let sceneClientReady = false;
  let sceneCore: SceneCore | null = null;
  let commandCatalog = $state<ObjectCommand[]>([]);
  let gestureCatalog = $state<ObjectGesture[]>([]);
  let canvases = $state<CanvasSummary[]>([]);
  let connectionStatus = $state<ConnectionStatus>("offline");
  let canvasBusy = $state(false);

  // ----- per-actor undo/redo (D21/FC-15): the core's UndoStack owns the
  //       bookkeeping; inverse ops are authored back through the SAME op-apply
  //       path. Only this client's ops are undoable. Created once the wasm core
  //       loads (bootstrapSceneCore). -----
  let undoStack: UndoStack | null = null;

  // ----- non-reactive refs -----
  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;

  // W2-03/EN1: the cursor affordance reflected onto the canvas wrapper. Space-hold
  // shows the pan grab cursor; the crosshair tools keep their own cursor (no
  // override); otherwise the core's per-move hover classification drives it. The
  // mapping lives in `lib/cursor` (single source the CSS in styles.css mirrors).
  const cursorAffordance = $derived(deriveCursorAffordance(spaceHeld, activeTool, affordance));

  // W2-10: the on-screen rect for the inline text-edit overlay, recomputed whenever
  // the edited object, its transform, or the camera changes (so the overlay tracks
  // the object under pan/zoom). Null when not editing or the object/path is gone.
  const textEditObject = $derived(textEdit ? scene.objects.find((o) => o.id === textEdit.id) ?? null : null);
  const textEditRect = $derived(textEditObject ? textOverlayScreenRect(textEditObject, camera) : null);

  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(scene.objects.length > 0);
  const selectedObject = $derived(selection.kind === "object" ? scene.objects.find((o) => o.id === selection.id) ?? null : null);

  // FC-08: the scene actually fed to the renderer — the canonical scene during
  // normal editing. W2-11: a live object transform NO LONGER rebuilds this scene;
  // the dragged object's instance model matrix is pushed straight to the GPU
  // (engine.setObjectPreviewTransform) with zero re-tessellation (P4), so `feedScene`
  // depends only on the canonical scene + the new-object previews below.
  // FC-11/W2-07: while a freehand stroke or shape drag-create is in progress, append
  // a transient preview object (a NEW object has no instance to update, so it bakes
  // once per geometry change — correct, not a P4 violation).
  const feedScene = $derived(buildFeedScene(scene, drawPoints, createKind, createDrag, createHoverSnap));

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onStats: (stats) => (rendererStats = stats),
    onStatus: (message) => (status = message),
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyStatus(status)) status = "Ready";
    },
    // W2-03/AP2 (#10): shift/meta-click toggles the object in/out of the multi set.
    // A plain click on an object ALREADY in the current Multi keeps the whole Multi
    // (so the pointer-down that begins a group-drag never collapses it to a single
    // object — the drag then moves every member together, see onTransformCommit).
    // A plain click on anything else replaces the selection with that object.
    onSelectObject: (id, additive) =>
      selectObject(
        additive
          ? toggleObjectSelection(selection, id)
          : selection.kind === "multi" && selection.ids.includes(id)
            ? selection
            : { kind: "object", id }
      ),
    onTransformPreview: (id, _matrix, _kind) => {
      // W2-11: the matrix is already on the GPU instance buffer (pushed by the engine
      // per move). This handler only invalidates a stale pendingCommit so a new drag
      // can never freeze on a commit still waiting for its scene update.
      if (pendingCommit && pendingCommit.id !== id) pendingCommit = null;
    },
    onTransformCommit: (id, matrix, _kind) => {
      // The canonical scene was never mutated during the drag, so op-apply
      // captures the correct inverse (original transform), satisfying D21 undo.
      const src = scene.objects.find((o) => o.id === id);
      // W2-11: with no src the GPU preview must not linger — revert it to canonical.
      if (!src) return void host?.clearObjectPreview(id);
      // AP2 (#15): a parent drag cascades the world-space delta to its descendants
      // (children transforms are world-absolute, D3), so a frame moves with its
      // contents. AP2 (#10): when a Multi selection is dragged, the renderer anchors
      // the gesture on the one picked `id` but the delta applies to EVERY member (and
      // each member's subtree) — so the whole set moves together. A single selection
      // (or a drag of a non-member) cascades only the dragged object's subtree.
      const ops =
        selection.kind === "multi" && selection.ids.includes(id)
          ? cascadeMultiTransformOps(scene.objects, selection.ids, matrix)
          : cascadeTransformOps(scene.objects, id, matrix);
      // AP5 (#14): every object anchored to a moved object reprojects its bound
      // node through that object's NEW transform, so anchored endpoints move WITH
      // the target. No anchors onto anything moved => no extra ops (the no-op case).
      const allOps = [...ops, ...(sceneCore ? sceneCore.anchorFollowOps(scene, ops) : [])];
      const op: ObjectOp = allOps.length === 1 ? allOps[0] : { kind: "batch", ops: allOps };
      // FC-16: pre-connect authorOp applies synchronously (the committed scene is on
      // return, so the rebake $effect drops the preview matrix immediately). In the
      // connected path the scene update is async — the GPU instance matrix holds the
      // previewed position until commitClientScene sees the committed transform land
      // (no snap-back). If the commit op fails, revert the GPU preview to canonical.
      const wasConnected = sceneClientReady && sceneClient !== null;
      // The GPU previewed only the dragged `id`; pendingCommit (snap-back guard) is
      // keyed on it, so read ITS composed transform from the cascade — not ops[0],
      // which under a multi cascade may be another member.
      const draggedOp = ops.find((o) => o.kind === "set-transform" && o.id === id);
      const rootTransform = draggedOp?.kind === "set-transform" ? draggedOp.transform : src.transform;
      if (wasConnected && rootTransform) pendingCommit = { id, transform: rootTransform };
      authorOp(op, true, (ok) => {
        if (!ok) {
          pendingCommit = null;
          host?.clearObjectPreview(id);
        }
      });
    },
    onMarquee: (ids) => {
      // FC-16: route the marquee result through the same validation as
      // onSelectObject (validSelection drops stale ids and collapses the kind).
      const next: ObjectSelection =
        ids.length >= 2 ? { kind: "multi", ids } : ids.length === 1 ? { kind: "object", id: ids[0] } : { kind: "canvas" };
      selectObject(next);
    },
    // RA2b/AP3: a double-click on a container drills in (sets activeContainer); a
    // leaf enters inline text edit through the existing path.
    onObjectDoubleClick: (payload) => handleObjectDoubleClick(payload),
    // FC-11: freehand pen capture. Accumulate world points across start/move; on
    // end, lower the stroke to an object via the wasm core and author an
    // insert-object op (the tool stays sticky in "draw"); cancel discards.
    onDraw: (phase, world) => handleDraw(phase, world),
    // W2-07: shape drag-create rubber-band + commit + select-after-create.
    onCreate: (phase, world, snapped, targetId) => handleCreate(phase, world, snapped, targetId),
    // W3-G9 (#3): pre-drag hover snap probe — drives the persistent anchor ring.
    onCreateHover: (world, snapped, targetId) => handleCreateHover(world, snapped, targetId),
    // W2-08: eraser touch — whole-stroke delete or partial subpath cut.
    onErase: (id, world, partial) => handleErase(id, world, partial),
    // W2-03: the core's per-move hover classification drives the canvas cursor.
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

  // Push the object scene into the renderer whenever it or the selection changes.
  // W2-11: feeds `feedScene` (the canonical scene plus any transient NEW-object
  // preview — pen stroke / shape drag-create). A live drag of an EXISTING object no
  // longer rides this feed; its transform is pushed straight to the GPU instance
  // matrix (engine.setObjectPreviewTransform) without mutating the canonical state.
  $effect(() => {
    const current = feedScene;
    const sel = selection;
    host?.loadObjectScene(current, sel);
  });

  // CC1.4: push the active tool to the renderer whenever it changes.
  $effect(() => {
    const tool = activeTool;
    host?.setTool(tool);
  });

  // MG9.4 windowed replica: re-aim the data-layer window at the camera viewport.
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

  // U4/U5: central shortcut dispatch over the object command catalog, plus
  // Escape priority handoff and Space-hold pan (select-mode pan, U5).
  const dispatchShortcut = $derived(createShortcutDispatcher({ catalog: commandCatalog, handlers: shortcutHandlers() }));

  $effect(() => {
    const dispatch = dispatchShortcut;
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing = target ? ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) || target.isContentEditable : false;
      if (event.key === "Escape") {
        event.preventDefault();
        handleEscape();
        return;
      }
      // W2-03: Space-hold arms the unified pointer's pan path (no separate Hand
      // tool); release on keyup. The engine flips the core to hand-pan only for the
      // duration of a Space-held drag.
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
    // AP6: cancel any in-flight toast timer so it can't fire after teardown.
    toastChannel.dismiss();
  });

  // ----- connection -------------------------------------------------------

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

  // FC-14: delete a canvas, including the active one. Deleting the active canvas
  // first switches to another remaining canvas (so the session is never left
  // pointing at a deleted id), then deletes the old one and reloads the list.
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
    // FC-16/W2-11: once the canonical scene reflects the committed drag transform,
    // the `scene = next` above already triggered the single rebake (feedScene ->
    // loadObjectScene $effect) at the committed transform, dropping the stale GPU
    // preview matrix with no flash. Clear the pendingCommit (and defensively revert
    // any residual preview) once the canonical transform lands (success) or the
    // object is gone (concurrent delete).
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

  // ----- op authoring (the ONE op-apply path, P1) -------------------------

  // Author an ObjectOp: the data layer applies it optimistically through the
  // wasm core and returns the inverse (the undo entry, D21). On success the
  // forward+inverse are recorded into the core undo stack — which clears redo (a
  // fresh user op forks history). An undo/redo replay is NOT undoable (the core
  // stack drives those via its own handshake, below).
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

  // The core's undo/redo is a two-step handshake (hand out an op, apply it, then
  // report the re-inverse). The op-apply resolves asynchronously, so back-to-back
  // presses must not re-enter the handshake before the prior one settles. Chain
  // every undo/redo onto this tail so they run strictly in order.
  let undoChain: Promise<void> = Promise.resolve();

  function undo(): void {
    // A rejection must never poison the serialization tail (else all later
    // undo/redo silently no-op); runUndoStep self-handles errors, the catch is
    // a belt-and-braces guard.
    undoChain = undoChain.then(() => runUndoStep("undo")).catch(() => {});
  }

  function redo(): void {
    undoChain = undoChain.then(() => runUndoStep("redo")).catch(() => {});
  }

  // Pull the next op from the core stack and re-author it through the SAME
  // op-apply path (D21); report the resulting inverse back to complete the
  // handshake. Awaiting the apply keeps the core stack's pending handshake from
  // being re-entered by a queued press.
  async function runUndoStep(dir: "undo" | "redo"): Promise<void> {
    if (!undoStack || !sceneClient) return;
    const op = dir === "undo" ? undoStack.undo() : undoStack.redo();
    if (!op) return;
    // From here the core stack is mid-handshake (pending set). Any exit path that
    // does NOT complete it MUST abort() it, or the stuck pending corrupts every
    // future undo/redo (the core's debug_assert is compiled out of release wasm).
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

  // ----- insertion / editing ----------------------------------------------

  function nextOrderKey(): string {
    // Order keys sort by plain string Ord; append a char after the current max so
    // a new object lands on top. The wasm core owns true fractional keys; this
    // shell-side monotonic key is only the insertion position.
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

  // W2-07: rect/ellipse/line ARM drag-create (the button only selects the tool; the
  // shape is sized by a pointer down-drag-up). A context-menu insert passes an
  // explicit `anchor`, which keeps the legacy immediate fixed-size insert so the
  // right-click "Insert rectangle" still drops a shape at the click point. Text and
  // frame always insert immediately at an anchor (no drag-create, per the card).
  function insertPrimitive(kind: PrimitiveKindId, anchor?: { x: number; y: number }): void {
    if (anchor === undefined && isDragCreateShape(kind)) {
      armCreate(kind);
      return;
    }
    const center = anchor ?? viewportCenterWorld();
    const object = buildPrimitiveObject(kind, center, freshId(kind), nextOrderKey(), selectedColor);
    authorOp({ kind: "insert-object", object });
    selection = { kind: "object", id: object.id };
    persistSelection(selection);
    showToast(`Inserted ${kind}`);
    // W2-10: a freshly-created text object enters inline edit immediately.
    if (kind === "text") enterTextEdit(object.id);
  }

  // W2-07: arm the shape drag-create submode. Mirrors the pen arming the draw tool:
  // the active tool flips to "create" and `createKind` holds which shape the next
  // pointer drag will rubber-band. A second click on the same shape disarms back to
  // select (toggle), matching the pen toggle feel.
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

  // W2-07: drive shape drag-create. `start` anchors the bbox; `move` extends the
  // current corner (already snapped to the nearest outline anchor when `snapped`,
  // unless the Alt snap-bypass modifier was held — handled in the engine); `end`
  // commits a primitive sized to the drag span and selects it; `cancel` discards.
  // A drag that never reaches MIN_DRAG_EXTENT_PX is treated as a click: it drops a
  // default fixed-size shape at the start point (so a single click still creates).
  function handleCreate(phase: "start" | "move" | "end" | "cancel", world: { x: number; y: number }, snappedIn: boolean, targetIdIn: string | null): void {
    const kind = createKind;
    if (!kind) return;
    // W2-07/AP5 (#6): the snap query runs against the renderer's loaded regions,
    // which include the TRANSIENT drag-create preview (it rides the same feed). The
    // preview corner sits under the cursor, so an over-empty-canvas move self-snaps
    // to the preview's own outline — a phantom snap whose target is no real object.
    // Honor a snap ONLY when its target is a real canonical object (the preview /
    // snap-indicator ids never are), so the ring + AP5 anchor fire on a real edge
    // and never on the preview itself.
    const targetId = targetIdIn !== null && scene.objects.some((o) => o.id === targetIdIn) ? targetIdIn : null;
    const snapped = snappedIn && targetId !== null;
    if (phase === "start") {
      // W3-G9 (#3): a drag takes over the ring (its own snap-indicator rides the
      // preview), so drop the pre-drag hover snap to avoid a doubled ring.
      createHoverSnap = null;
      createDrag = { span: { start: world, end: world }, snapped, target: targetId };
      return;
    }
    if (phase === "cancel") {
      createDrag = null;
      createHoverSnap = null;
      return;
    }
    if (!createDrag) return;
    if (phase === "move") {
      createDrag = { span: { start: createDrag.span.start, end: world }, snapped, target: targetId };
      return;
    }
    // phase === "end": commit a sized primitive (or a default at a click).
    const span: DragSpan = { start: createDrag.span.start, end: world };
    const snapTarget = targetId;
    createDrag = null;
    const dx = Math.abs(span.end.x - span.start.x);
    const dy = Math.abs(span.end.y - span.start.y);
    const tooSmall = kind === "line" ? dx < MIN_DRAG_EXTENT_PX && dy < MIN_DRAG_EXTENT_PX : dx < MIN_DRAG_EXTENT_PX || dy < MIN_DRAG_EXTENT_PX;
    const object = tooSmall
      ? buildPrimitiveObject(kind, span.start, freshId(kind), nextOrderKey(), selectedColor)
      : buildPrimitiveObjectFromDrag(kind, span, freshId(kind), nextOrderKey(), selectedColor);
    // AP5 (#14): a snapped drag-create binds the dragged endpoint to the target's
    // outline with a persistent D5 anchor (Alt-create bypasses snap upstream, so
    // `snapTarget` is null and no anchor is authored). The endpoint then reprojects
    // through the target's transform, so the new object moves WITH the target.
    if (!tooSmall && snapTarget && sceneCore) {
      const target = scene.objects.find((o) => o.id === snapTarget);
      const anchors = target ? sceneCore.synthesizeCreateAnchors(object, target, span.end) : null;
      if (anchors) object.anchors = anchors;
    }
    authorOp({ kind: "insert-object", object });
    // Select-after-create (request 6) and return to the select tool so the new
    // object can be moved/resized immediately.
    selectObject({ kind: "object", id: object.id });
    createKind = null;
    setActiveTool("select");
    showToast(`Inserted ${kind}`);
  }

  // W3-G9 (#3): drive the PERSISTENT pre-drag anchor ring. A bare create-tool hover
  // over an object's edge sets `createHoverSnap` (the snapped world point + target);
  // a hover off any edge (or onto the transient preview, never a real object)
  // clears it. The canonicalization mirrors handleCreate (#6): honor a snap ONLY
  // when its target is a REAL canonical object, so the ring never shows over the
  // preview's own outline. The ring renders from feedScene while createDrag is null.
  function handleCreateHover(world: { x: number; y: number }, snappedIn: boolean, targetIdIn: string | null): void {
    if (!createKind) return void (createHoverSnap = null);
    const target = targetIdIn !== null && scene.objects.some((o) => o.id === targetIdIn) ? targetIdIn : null;
    createHoverSnap = snappedIn && target !== null ? { at: world, target } : null;
  }

  // FC-11: drive the freehand pen. Accumulate world points across start/move; on
  // end, lower the stroke to an object through the wasm core and author an
  // insert-object op. The tool stays sticky in "draw". A cancel discards the
  // in-progress stroke.
  function handleDraw(phase: "start" | "move" | "end" | "cancel", world: { x: number; y: number }): void {
    if (phase === "start") {
      drawPoints = [world];
      return;
    }
    if (phase === "cancel") {
      drawPoints = null;
      return;
    }
    if (!drawPoints) return;
    const points = [...drawPoints, world];
    if (phase === "move") {
      drawPoints = points;
      return;
    }
    // phase === "end": commit the stroke to an object (>=2 points have extent).
    drawPoints = null;
    if (points.length < 2 || !sceneCore) return;
    // S2 (#5): freehandToObject is a hex API, so the theme-default sentinel can't be
    // passed through it — lower with a placeholder hex, then swap the stroke paint to
    // the "text" token so the stroke flips with the theme like every other authored color.
    // The pen draws with the single toolbar color (selectedColor); there is no separate pen color.
    const strokeHex = selectedColor === THEME_DEFAULT_COLOR ? "#000000" : selectedColor;
    const object = sceneCore.freehandToObject(points, strokeHex, penWidthPx, PEN_EPSILON, freshId("draw"), nextOrderKey());
    if (selectedColor === THEME_DEFAULT_COLOR && object.stroke) object.stroke.paint = paintForColor(selectedColor);
    authorOp({ kind: "insert-object", object });
    // Request 6: select the freshly-drawn stroke after creating it. The pen tool
    // stays sticky in "draw" so the next stroke draws immediately.
    selectObject({ kind: "object", id: object.id });
  }

  // W2-08: erase the stroke under the cursor. Default = whole-stroke delete (a
  // `delete` op). The partial modifier cuts the stroke's subpath at the touched
  // region via the scene-core split: convert the world touch into the object's
  // local quantized space, split the geometry, then author an `edit-geometry` op
  // (or delete the object when the cut leaves it empty). Both ride the single
  // op-apply path, so undo (D21) captures the inverse. A drag erases continuously.
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
    const cut = sceneCore.splitSubpathAt(target.geometry, local.x, local.y, ERASE_RADIUS_QUANTIZED);
    // No node within the erase radius: the touch missed; leave the stroke whole.
    if (!cut) return;
    // The cut removed every renderable piece — delete the now-empty object. The
    // core omits `d` entirely when the geometry is empty (skip_serializing_if), so
    // guard the undefined case before trimming.
    if ((cut.d ?? "").trim().length === 0) {
      authorOp({ kind: "delete", id });
      if (selection.kind === "object" && selection.id === id) selectObject({ kind: "canvas" });
      return;
    }
    authorOp({ kind: "edit-geometry", id, geometry: cut });
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

  // Group: reparent the selected objects under a fresh frame object (D3). FC-14:
  // the frame geometry encloses the children — a rect sized to the union world-AABB
  // of the selected objects, positioned by a pure-translation transform at the
  // AABB's top-left. Children transforms are world-absolute, so they are reparented
  // unchanged.
  function groupSelection(): void {
    const ids = currentSelectionIds();
    // AP3 (#9): group works on 1+ objects — a single object grouped under a fresh
    // neutral parent container is a valid one-child frame.
    if (ids.length < 1) return;
    const objects = ids.map((id) => scene.objects.find((o) => o.id === id)).filter((o): o is SceneObject => Boolean(o));
    const bounds = unionWorldAabb(objects);
    const frame: SceneObject = {
      id: freshId("frame"),
      order: nextOrderKey(),
      transform: bounds ? translateTransform(bounds.minX, bounds.minY) : translateTransform(0, 0),
      geometry: {
        d: rectPathQuantized(bounds ? bounds.maxX - bounds.minX : 1, bounds ? bounds.maxY - bounds.minY : 1),
        fillRule: "nonZero"
      },
      clip: false
    };
    const ops: ObjectOp[] = [{ kind: "insert-object", object: frame }];
    let order = "a0";
    for (const id of ids) {
      ops.push({ kind: "reparent", id, parent: frame.id, order });
      order = `${order}~`;
    }
    authorOp({ kind: "batch", ops });
    selection = { kind: "object", id: frame.id };
    persistSelection(selection);
    showToast("Grouped selection");
  }

  // FC-14: ungroup reparents children out of the frame, then deletes the now-empty
  // frame in the SAME batch op.
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

  // AP3 (#18): pop the right-clicked child out one level — reparent it to its
  // parent's parent (or canvas root when the parent sits at the root). The op is
  // built by the pure `popOutOp` helper; a non-child picked target yields null.
  function popOutSelection(picked: ObjectSelection): void {
    if (picked.kind !== "object") return;
    const op = popOutOp(scene.objects, picked.id);
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

  // FC-14: true single-step reorder. Sort objects by order string; for "forward"
  // swap the selected object's order with its next-higher neighbor, for "backward"
  // swap with the next-lower neighbor. Applies to the single selected object only
  // (a step reorder over a multi-set has no well-defined neighbor).
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
    // A key that sorts before the current min: trim/prefix is non-trivial; use a
    // short ascii key below "a" so it lands at the back.
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

  // AP1 (#5): adopt the toolbar's selected color, then recolor the current
  // selection. Always hold the latest pick (it becomes the default for the next
  // NEW shape); when a single object is selected, author a `set-style` op so the
  // pick recolors it live through the existing op-apply path (D21 undo).
  function applySelectedColor(color: string): void {
    selectedColor = color;
    if (selection.kind !== "object") return;
    const object = scene.objects.find((o) => o.id === selection.id);
    if (!object) return;
    authorOp(buildSetStyleOp(object, color));
  }

  // ----- AP3 (#9): double-click drill-in -----

  // RA2b surfaces a double-click on an object as { id, hasChildren } on the
  // inputBatch result. Branch it (D6): a container (hasChildren) drills in — the
  // shell sets the active container; a leaf enters inline text edit (the existing
  // path). A null signal (double-click missed every object) is a no-op.
  function handleObjectDoubleClick(signal: { id: string; hasChildren: boolean } | null): void {
    const action = doubleClickAction(signal);
    if (!action) return;
    if (action.kind === "drill-in") {
      activeContainer = action.id;
      selectObject({ kind: "object", id: action.id });
      showToast("Entered group");
      return;
    }
    enterTextEdit(action.id);
  }

  // ----- W2-10: inline text editing -----

  // Enter inline edit for an object: seed the overlay value from the object's first
  // text run, then mount the contenteditable (the mount action focuses it). When the
  // object has not landed in `scene` yet (the connected create path applies the
  // insert-object asynchronously), seed empty — a fresh object carries no text, and
  // `textEditRect` derives reactively, so the overlay appears once the object arrives.
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

  // W2-10: Svelte action — seed the contenteditable with the object's current text
  // and focus it (placing the caret at the end) when the overlay mounts.
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

  // ----- templates (buildObjectTemplate -> FeatureRequest.templateApply) ---

  // W2-09: the templates the scroll-popup offers. Ids map to buildObjectTemplate.
  const TEMPLATES: TemplatePopupItem[] = [
    { id: "todo_board", title: "Todo board", desc: "Grouped To do / In progress / Done columns" },
    { id: "decision_map", title: "Decision map", desc: "Options and outcomes wired with connectors" },
    { id: "presentation", title: "Presentation", desc: "A deck grouping title and content slides" }
  ];

  function toggleTemplates(): void {
    templateOpen = !templateOpen;
  }

  // Apply the default template (object-native): lower it to a recipe of inline-
  // styled objects via the wasm core, then send a single templateApply feature
  // frame (the server lowers the recipe to insert-object ops, OB3.S5/OB4.5).
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
    // W2-10: a pending Escape first cancels an in-progress inline text edit.
    if (textEdit) return void cancelTextEdit();
    // FC-11: a pending Escape first cancels an in-progress pen stroke.
    if (drawPoints) return void (drawPoints = null);
    // W2-07: cancel an in-progress shape drag-create, then disarm the tool.
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
    // W2-07: leaving the create tool (e.g. picking Select/Pen) disarms the shape.
    if (tool !== "create") {
      createKind = null;
      createDrag = null;
      // W3-G9 (#3): leaving create drops the persistent hover ring so it never lingers.
      createHoverSnap = null;
    }
    activeTool = tool;
  }

  // ----- shortcut handlers (object command catalog ids, U4) ----------------

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
      // W2-10: Enter on a selected object enters inline edit (contenteditable
      // overlay) instead of a blocking prompt.
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

  // ----- context menu (U3, from the command catalog) ----------------------

  // FC-13: hit-test the object under the cursor and open a menu that acts on it.
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

  // FC-13: build the context-menu target from a picked id. If the pick is part of
  // the current multi-select, keep the whole multi so the menu acts on the set.
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

  // FC-13: the right-click menu is derived from the object command catalog. The
  // catalog (the wasm core's `objectCommandCatalog()`) supplies the label/order;
  // each id routes to the SAME shell handler the shortcut layer uses. The
  // object/canvas split picks which command ids appear, and a `null` entry renders
  // a separator. Icons + the delete danger flag are decorated here.
  // AP3: `enabled` is an optional predicate gating an entry on the picked target's
  // shape in the forest (children/parent), beyond the coarse `disabledFor` kind
  // check — e.g. ungroup needs a container, pop-out needs a child.
  type ContextMenuEntry =
    | "separator"
    | {
        id: string;
        label?: string;
        icon?: typeof Copy;
        danger?: boolean;
        disabledFor?: ObjectSelection["kind"];
        enabled?: (picked: ObjectSelection) => boolean;
      };

  // AP3 (#13): ungroup is enabled only for a single container object (has children).
  function ungroupPickEnabled(picked: ObjectSelection): boolean {
    return picked.kind === "object" && ungroupEnabled(scene.objects, picked.id);
  }

  // AP3 (#18): pop-out is enabled only when the single picked object has a parent.
  function popOutPickEnabled(picked: ObjectSelection): boolean {
    return picked.kind === "object" && popOutOp(scene.objects, picked.id) !== null;
  }

  const OBJECT_MENU: ContextMenuEntry[] = [
    { id: "duplicate", icon: Copy },
    { id: "group", icon: GroupIcon, disabledFor: "object" },
    // AP3 (#13): ungroup is meaningful only for a single container object (one with
    // children) — disabled for a multi-select and for a childless leaf.
    { id: "ungroup", icon: Ungroup, enabled: ungroupPickEnabled },
    // AP3 (#18): pop a child out one level — only when the picked object has a parent.
    // Shell-only command (no catalog entry), so it carries its own label.
    { id: "pop-out", label: "Pop out one level", enabled: popOutPickEnabled },
    { id: "bring-to-front" },
    { id: "send-to-back" },
    { id: "add-comment", icon: MessageSquarePlus },
    "separator",
    { id: "delete", icon: Trash2, danger: true }
  ];

  // AP3 (#18, D7): the empty-canvas menu — quick inserts, the template library, and
  // select-all. The insert-text entry is gone (D7); text arrives via the toolbar.
  const CANVAS_MENU: ContextMenuEntry[] = [
    { id: "insert-rectangle" },
    { id: "insert-ellipse" },
    { id: "open-template-library", icon: LayoutTemplate },
    "separator",
    { id: "select-all" }
  ];

  function contextMenuItems(menu: ContextMenuState): (ContextMenuItem | null)[] {
    const picked = menu.selection;
    const layout = picked.kind === "object" || picked.kind === "multi" ? OBJECT_MENU : CANVAS_MENU;
    const handlers = contextHandlers(menu);
    return layout.map((entry) => {
      if (entry === "separator") return null;
      const command = commandCatalog.find((c) => c.id === entry.id);
      const run = handlers[entry.id];
      if (!run) return null;
      // AP3: an entry is disabled when its target kind matches `disabledFor`, OR when
      // its `enabled` predicate (children/parent shape) rejects the picked target.
      const disabled = entry.disabledFor === picked.kind || (entry.enabled !== undefined && !entry.enabled(picked));
      return {
        label: entry.label ?? command?.label ?? entry.id,
        icon: entry.icon,
        danger: entry.danger,
        disabled,
        onSelect: () => closeContextThen(run)
      };
    });
  }

  // Map a context-menu command id to the existing shell handler. Canvas inserts
  // anchor at the right-click world point (menu.world).
  function contextHandlers(menu: ContextMenuState): Record<string, () => void> {
    const base = shortcutHandlers();
    return {
      duplicate: base.duplicate,
      group: base.group,
      ungroup: base.ungroup,
      // AP3 (#18): pop the right-clicked child out one level (to its grandparent, or
      // canvas root). Operates on the picked object id, not the active selection.
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

  // ----- pure transform helpers (D7) --------------------------------------

  function transformOrigin(transform: SceneObject["transform"]): [number, number] {
    if (!transform) return [0, 0];
    return [transform[0][2], transform[1][2]];
  }

  // FC-16: structural equality of two (possibly absent) 3x3 transforms. An absent
  // transform is the identity, so it compares equal to an explicit identity.
  function transformsEqual(a: SceneObject["transform"], b: SceneObject["transform"]): boolean {
    const m = a ?? IDENTITY_TRANSFORM;
    const n = b ?? IDENTITY_TRANSFORM;
    return m.every((row, i) => row.every((v, j) => v === n[i][j]));
  }

  function shiftTransform(transform: SceneObject["transform"], dx: number, dy: number): SceneObject["transform"] {
    const [ox, oy] = transformOrigin(transform);
    if (!transform) return translateTransform(dx, dy);
    return [
      [transform[0][0], transform[0][1], ox + dx],
      [transform[1][0], transform[1][1], oy + dy],
      [transform[2][0], transform[2][1], transform[2][2]]
    ];
  }

  // W2-08: map a world point into an object's local quantized geometry space
  // (inverse affine transform, then quantize by GEOMETRY_QUANTUM_PER_PX). Returns
  // null when the transform is non-invertible (degenerate scale). Used to place
  // the partial-erase cut in the same coordinate space as the stored geometry.
  function worldToObjectLocalQuantized(object: SceneObject, world: { x: number; y: number }): { x: number; y: number } | null {
    const t = object.transform ?? IDENTITY_TRANSFORM;
    const a = t[0][0];
    const b = t[0][1];
    const c = t[1][0];
    const d = t[1][1];
    const e = t[0][2];
    const f = t[1][2];
    const det = a * d - b * c;
    if (Math.abs(det) < 1e-9) return null;
    const dx = world.x - e;
    const dy = world.y - f;
    const localX = (d * dx - b * dy) / det;
    const localY = (-c * dx + a * dy) / det;
    return { x: Math.round(localX * GEOMETRY_QUANTUM_PER_PX), y: Math.round(localY * GEOMETRY_QUANTUM_PER_PX) };
  }

  // FC-11/W2-07: the renderer feed. Start from the canonical scene and append a
  // transient pen-stroke / shape-drag-create preview — neither mutates the canonical
  // `scene` nor runs op-apply (geometry-vocabulary construction only). W2-11: an
  // existing object's live transform is NOT previewed here anymore (it rides the GPU
  // instance matrix); only NEW-object previews, which inherently need one bake.
  function buildFeedScene(
    source: ObjectScene,
    pen: { x: number; y: number }[] | null,
    create: DragCreateShape | null,
    createState: { span: DragSpan; snapped: boolean } | null,
    hoverSnap: { at: { x: number; y: number }; target: string | null } | null
  ): ObjectScene {
    let feed = source;
    const preview = pen && pen.length >= 1 ? drawPreviewObject(pen) : null;
    if (preview) feed = { ...feed, objects: [...feed.objects, preview] };
    // W2-07: a transient rubber-band preview of the shape being drag-created, plus
    // a snap indicator marker when the dragged corner is snapped to an outline.
    if (create && createState) {
      const extra = createPreviewObjects(create, createState.span, createState.snapped);
      if (extra.length > 0) feed = { ...feed, objects: [...feed.objects, ...extra] };
    } else if (hoverSnap) {
      // W3-G9 (#3): no drag in progress — render the PERSISTENT pre-drag anchor ring
      // at the hovered edge so the user sees where the next create would anchor.
      feed = { ...feed, objects: [...feed.objects, snapIndicatorObject(hoverSnap.at)] };
    }
    return feed;
  }

  // W2-07: transient preview objects for the in-progress shape drag-create — the
  // rubber-band primitive (built directly in TS like the pen preview, NOT op-apply)
  // and, when the dragged corner is snapped to an outline anchor, a small circle
  // marker at that corner. The committed object replaces them on pointer-up.
  function createPreviewObjects(kind: DragCreateShape, span: DragSpan, snapped: boolean): SceneObject[] {
    const preview = buildPrimitiveObjectFromDrag(kind, span, "create-preview", nextOrderKey());
    const objects: SceneObject[] = [preview];
    if (snapped) objects.push(snapIndicatorObject(span.end));
    return objects;
  }

  // W2-07: a small ring drawn at the snapped corner so the user sees the snap.
  // Geometry is a world-px ellipse (identity transform, so local==world); built in
  // TS like the pen preview (NOT op-apply).
  function snapIndicatorObject(at: { x: number; y: number }): SceneObject {
    const q = (px: number) => Math.round(px * GEOMETRY_QUANTUM_PER_PX);
    const r = 5;
    const k = r * 0.5523;
    const cx = at.x;
    const cy = at.y;
    const d = [
      `M ${q(cx - r)} ${q(cy)}`,
      `C ${q(cx - r)} ${q(cy - k)} ${q(cx - k)} ${q(cy - r)} ${q(cx)} ${q(cy - r)}`,
      `C ${q(cx + k)} ${q(cy - r)} ${q(cx + r)} ${q(cy - k)} ${q(cx + r)} ${q(cy)}`,
      `C ${q(cx + r)} ${q(cy + k)} ${q(cx + k)} ${q(cy + r)} ${q(cx)} ${q(cy + r)}`,
      `C ${q(cx - k)} ${q(cy + r)} ${q(cx - r)} ${q(cy + k)} ${q(cx - r)} ${q(cy)}`,
      "Z"
    ].join(" ");
    return {
      id: "create-snap-indicator",
      order: nextOrderKey(),
      geometry: { d, fillRule: "nonZero" },
      stroke: { paint: { kind: "solid", color: "#ff3b6b" }, width: 2 * GEOMETRY_QUANTUM_PER_PX, cap: "round", join: "round" }
    };
  }

  // FC-11: a transient preview object for the in-progress pen stroke. Geometry is
  // a world-px polyline (identity transform, so local==world) with the pen brush;
  // built directly in TS like objectPrimitives.ts (NOT op-apply). The committed
  // object replaces it on pointer-up.
  function drawPreviewObject(points: { x: number; y: number }[]): SceneObject {
    const q = (px: number) => Math.round(px * GEOMETRY_QUANTUM_PER_PX);
    const d = points.map((p, i) => `${i === 0 ? "M" : "L"} ${q(p.x)} ${q(p.y)}`).join(" ");
    return {
      id: "draw-preview",
      order: nextOrderKey(),
      geometry: { d },
      stroke: { paint: paintForColor(selectedColor), width: penWidthPx * GEOMETRY_QUANTUM_PER_PX, cap: "round", join: "round" }
    };
  }

  // FC-14: the union world-AABB of the given objects, computed from each object's
  // geometry path bbox (object-local quantized px → logical px) transformed by its
  // affine transform. Returns null when no object yields a finite bbox.
  function unionWorldAabb(objects: SceneObject[]): { minX: number; minY: number; maxX: number; maxY: number } | null {
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const object of objects) {
      const local = pathLocalBbox(object.geometry.d);
      if (!local) continue;
      const t = object.transform;
      for (const [lx, ly] of [
        [local.minX, local.minY],
        [local.maxX, local.minY],
        [local.maxX, local.maxY],
        [local.minX, local.maxY]
      ]) {
        const [wx, wy] = t ? [t[0][0] * lx + t[0][1] * ly + t[0][2], t[1][0] * lx + t[1][1] * ly + t[1][2]] : [lx, ly];
        minX = Math.min(minX, wx);
        minY = Math.min(minY, wy);
        maxX = Math.max(maxX, wx);
        maxY = Math.max(maxY, wy);
      }
    }
    return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
  }

  // The object-local bbox (in logical px) of a path-string's coordinate pairs.
  // Coords are quantized integers (GEOMETRY_QUANTUM_PER_PX per px); commands are
  // single letters, so reading every numeric pair covers M/L/C control points —
  // a conservative-enough enclosing box for the group frame.
  function pathLocalBbox(d: string): { minX: number; minY: number; maxX: number; maxY: number } | null {
    const nums = d.match(/-?\d+(?:\.\d+)?/g);
    if (!nums || nums.length < 2) return null;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (let i = 0; i + 1 < nums.length; i += 2) {
      const x = Number(nums[i]) / GEOMETRY_QUANTUM_PER_PX;
      const y = Number(nums[i + 1]) / GEOMETRY_QUANTUM_PER_PX;
      minX = Math.min(minX, x);
      minY = Math.min(minY, y);
      maxX = Math.max(maxX, x);
      maxY = Math.max(maxY, y);
    }
    return Number.isFinite(minX) ? { minX, minY, maxX, maxY } : null;
  }

  // FC-14: a closed object-local rect path of `w`×`h` logical px, quantized.
  function rectPathQuantized(w: number, h: number): string {
    const qw = Math.round(Math.max(1, w) * GEOMETRY_QUANTUM_PER_PX);
    const qh = Math.round(Math.max(1, h) * GEOMETRY_QUANTUM_PER_PX);
    return `M 0 0 L ${qw} 0 L ${qw} ${qh} L 0 ${qh} Z`;
  }

  // ----- status helpers ----------------------------------------------------

  function isDiagnosticsOnlyStatus(message: string): boolean {
    return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
  }

  function wsBaseUrl(): string {
    const loc = window.location;
    const protocol = loc.protocol === "https:" ? "wss:" : "ws:";
    return `${protocol}//${loc.host}`;
  }

  function clientIdentity(): string {
    return `shell-${crypto.randomUUID().slice(0, 8)}`;
  }

  function userIdentity(): string {
    const key = "shape-ai-user-id";
    try {
      const existing = window.localStorage.getItem(key);
      if (existing) return existing;
      const fresh = `user-${crypto.randomUUID().slice(0, 8)}`;
      window.localStorage.setItem(key, fresh);
      return fresh;
    } catch {
      return `user-${crypto.randomUUID().slice(0, 8)}`;
    }
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

      <!-- AP6 (#19): transient action toast. Keyed by message so each new notice
           remounts and replays the fade-in/out; auto-dismissed by toastChannel. -->
      {#if toast}
        {#key toast}
          <div class="canvas-toast" role="status" aria-live="polite">{toast}</div>
        {/key}
      {/if}
    </section>
  </main>
</div>
