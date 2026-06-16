<script lang="ts">
  import { onDestroy } from "svelte";
  import { applyDocumentTheme, readStoredTheme, type Theme } from "../renderer/scene";
  import type { CameraState } from "../shared/geometry";
  import {
    emptyObjectScene,
    GEOMETRY_QUANTUM_PER_PX,
    translateTransform,
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
  import type { HoverAffordance, UiEditRequest, UiIntent } from "../bridge/wasmLoader";
  import {
    loadSceneCore,
    ensureSceneCore,
    createWasmSession,
    type ObjectCommand,
    type ObjectGesture,
    type CreateThresholds,
    type InspectorView,
    type InspectorControlValue,
    type SceneCore,
    type UndoStack,
    type WasmSession
  } from "../bridge/sceneCoreWasm";
  import { createShortcutDispatcher, detectMac, keyChar } from "../controller/shortcuts";
  import { isFreeRecognizeHold } from "../controller/gestureBindings";
  import { TextEditHost, type TextEditHandle } from "../ime/textEditHost";
  import { ToastChannel } from "../runtime/statusChannel";
  import { UiModelFeed } from "../runtime/uiModelFeed";
  import {
    textOverlayScreenRect,
    THEME_DEFAULT_COLOR,
    type DragSpan,
    type CreateSnap
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
    authorInspectorEdit,
    authorInspectorAction,
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
    isDiagnosticsOnlyStatus,
    wsBaseUrl as wsBaseUrlOf,
    clientIdentity as clientIdentityOf,
    userIdentity as userIdentityOf
  } from "../controller/session";
  import CanvasHost from "./ShapeCanvasHost.svelte";

  // Render-only mirror of the core's scene. The Rust core (the offline `session` pre-connect, the
  // SceneClient engine once connected) is the single source of truth; this is written ONCE per frame
  // inside commitClientScene and is never assigned or mutated anywhere else.
  let scene = $state<ObjectScene>(emptyObjectScene());

  // The core session that owns author/apply + scene() on the OFFLINE / pre-connect path, so the core —
  // not the shell — is the source of truth even before a transport connects. Once connected, authoring
  // routes through `sceneClient` (which owns its own engine); this offline session then sits idle.
  let session: WasmSession | null = null;

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

  // Inline text editing: `textEdit` holds the edited object id + in-progress value (a render-only mirror
  // of the OS editing surface). The shared `TextEditHost` library mounts the contenteditable + owns the
  // IME mechanics; this shell only forwards the committed string into a set-text op. Blur/Enter commits; Esc commits too.
  let textEdit = $state<{ id: string; value: string } | null>(null);
  // A freshly-inserted text object held only until the canonical scene lands. The connected insert is
  // async, so textEditObject falls back to this so the overlay mounts + focuses this tick; render-only.
  let pendingTextInsert = $state<SceneObject | null>(null);

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
  let sceneCore = $state<SceneCore | null>(null);
  let commandCatalog = $state<ObjectCommand[]>([]);
  let gestureCatalog = $state<ObjectGesture[]>([]);
  // The create-gesture screen-px thresholds, read from scene-core at bootstrap so the shell keeps no
  // literal copy. Null until the core loads; the create/draw paths gate on `sceneCore` before using them.
  let createThresholds: CreateThresholds | null = null;
  let canvases = $state<CanvasSummary[]>([]);
  let connectionStatus = $state<ConnectionStatus>("offline");
  let canvasBusy = $state(false);

  // Per-actor undo/redo: the core's UndoStack owns the bookkeeping; only this client's ops are undoable. Created once the wasm core loads.
  let undoStack: UndoStack | null = null;

  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;

  // The shared OS text-edit / IME host-port library, realizing both the canvas inline edit and the
  // ui-core TextInput edit through ONE reused contenteditable. Created once the canvas wrapper mounts
  // (it is the positioning context for the absolute-positioned overlay). Holds no canvas/ui decision.
  let textEditHost: TextEditHost | null = null;
  // The open canvas-edit handle, so the reposition effect can track pan/zoom transform-only.
  let canvasEditHandle: TextEditHandle | null = null;

  // The cursor affordance reflected onto the canvas wrapper; the mapping lives in `cursor.ts` (the single source styles.css mirrors).
  const cursorAffordance = $derived(deriveCursorAffordance(spaceHeld, activeTool, affordance));

  // The inline text-edit overlay rect, recomputed when the edited object, its transform, or the camera
  // changes (so the overlay tracks the object under pan/zoom). Null when not editing or the path is gone.
  const textEditObject = $derived(
    textEdit
      ? scene.objects.find((o) => o.id === textEdit.id) ??
          (pendingTextInsert?.id === textEdit.id ? pendingTextInsert : null)
      : null
  );
  const textEditRect = $derived(
    textEditObject && sceneCore
      ? textOverlayScreenRect(sceneCore.objectWorldAabb(textEditObject), (world) => {
          void camera;
          return host?.projectWorldToScreen(world) ?? null;
        })
      : null
  );

  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(scene.objects.length > 0);

  // The dynamic inspector view for the current selection, resolved by the core from the render-only
  // mirror (scene + selection). Null for a canvas selection or before the core loads; the panel renders
  // nothing then. The core decides the role + which controls apply, so the shell computes no derived state.
  const inspectorView = $derived.by<InspectorView | null>(() => {
    if (!sceneCore || selection.kind === "canvas") return null;
    return sceneCore.objectInspectorView(JSON.stringify(scene), JSON.stringify(selection));
  });

  // The scene fed to the renderer: the canonical scene plus any transient NEW-object preview (a live
  // transform of an EXISTING object no longer rebuilds this — it's pushed straight to the GPU instance
  // matrix). A new object bakes once per geometry change since it has no instance to update.
  const feedScene = $derived(
    buildFeedSceneOf(sceneCore, scene, drawPoints, createKind, createDrag, createHoverSnap, nextOrderKey, selectedColor, penWidthPx)
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
      const rootTransform = draggedRootTransform(allOps, id);
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
    onAffordance: (next) => (affordance = next),
    onUiEdit: (edit) => handleUiEdit(edit),
    onUiIntent: (intent) => handleUiIntent(intent)
  };

  // Boot the scene-core wasm (op-apply + catalog) and open the WS session.
  void bootstrapSceneCore();
  void connectSceneClient();

  async function bootstrapSceneCore(): Promise<void> {
    try {
      sceneCore = await loadSceneCore();
      // loadSceneCore initialized the wasm; build the offline session so the core owns author/scene()
      // pre-connect too. The shell mirror is seeded from the session's scene, never a shell-held copy.
      await ensureSceneCore();
      session = createWasmSession({ welcomeScene: emptyObjectScene(), clientId: clientIdentity() });
      commitClientScene(JSON.parse(session.scene()) as ObjectScene, []);
      commandCatalog = sceneCore.objectCommandCatalog();
      gestureCatalog = sceneCore.objectGestureCatalog();
      createThresholds = sceneCore.createThresholds();
      undoStack = sceneCore.createUndoStack(userIdentity());
    } catch {
      sceneCore = null;
      session = null;
      createThresholds = null;
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

  // The active tool as a TOOLBAR COMMAND id (the toolbar marks its matching button active off this
  // mirror). The shell's `create` submode owns no toolbar button — its active insert button is marked via
  // `create_kind` instead, so it maps to no tool command. The core decides nothing here; this only mirrors
  // the shell tool state into the catalog vocabulary the Rust toolbar reads.
  const activeToolCommand = $derived(
    activeTool === "draw" ? "draw" : activeTool === "erase" ? "erase" : activeTool === "create" ? "" : "select-move"
  );
  const createKindCommand = $derived(createKind ? `insert-${createKind}` : null);
  const isMac = detectMac();

  // rAF-coalesced UI-model feed: a presence burst (many cursor frames per animation frame) collapses to
  // one heavy feed per frame. The real requestAnimationFrame is injected so the coalescer stays testable.
  const uiModelFeed = new UiModelFeed(
    { request: (cb) => requestAnimationFrame(cb), cancel: (h) => cancelAnimationFrame(h as number) },
    (json) => host?.setUiModel(json)
  );

  // Feed the built-in UI model to the Rust runtime whenever any UI-affecting state changes. Reading each
  // dependency here makes Svelte re-run the effect (and re-feed) ONLY on a real change — a theme flip /
  // selection change re-derives the small tree off the model and rides the core's partial-patch path, with
  // no per-frame re-feed and no geometry rebake. The runtime then owns build_root / hit-test / dispatch;
  // this shell holds only the render-only mirror it serializes here, authoring no decision. The serialized
  // model is handed to the rAF-coalesced feed so a presence burst collapses to one feed per frame.
  $effect(() => {
    const rect = canvasWrap?.getBoundingClientRect();
    const model = {
      themeDark: theme === "dark",
      viewport: [rect?.width ?? 0, rect?.height ?? 0] as [number, number],
      activeTool: activeToolCommand,
      createKind: createKindCommand,
      selectedColor,
      penPalette: PEN_PALETTE,
      penWidth: penWidthPx,
      penWidths: PEN_WIDTHS,
      templates: TEMPLATES,
      templateOpen,
      canvases: canvases.map((c) => ({ id: c.id, title: c.title })),
      activeCanvasId: canvasId,
      connectionOnline: connectionStatus !== "offline",
      canvasBusy,
      diagnostics: diagnosticsOpen ? diagnosticsModel() : null,
      diagnosticsOpen,
      inspectorView,
      isMac,
      settingsOpen,
      contextMenu: contextMenu
        ? { x: contextMenu.x, y: contextMenu.y, title: contextMenuTitle(contextMenu.selection), items: contextMenuItems(contextMenu) }
        : null,
      peers: projectedPeers(),
      busy,
      status: status === "Ready" ? null : status,
      toast
    };
    uiModelFeed.schedule(JSON.stringify(model));
  });

  // The diagnostics readout rows (pre-formatted display strings; the cores are time-free so the shell
  // computes the frame timing). Mirrors the old Svelte diagnostics panel content.
  function diagnosticsModel(): { state: string; detail: string; objects: string; frameMs: string; camera: string } {
    return {
      state: readyState,
      detail: rendererDetail,
      objects: String(scene.objects.length),
      frameMs: rendererStats?.frameMs != null ? `${rendererStats.frameMs.toFixed(2)} ms` : "—",
      camera: `${camera.x.toFixed(0)}, ${camera.y.toFixed(0)} @ ${camera.zoom.toFixed(2)}x`
    };
  }

  // Project the live peers' WORLD cursors to SCREEN coords through the live core camera, dropping any the
  // camera can't project (mirrors the old PeerCursors projection). Projection stays shell-side — the pure
  // core never sees a camera here; the Rust presence layer renders the already-projected screen points.
  function projectedPeers(): { userId: string; screen: [number, number]; color: string; label: string }[] {
    void camera;
    return peers
      .filter((peer) => peer.cursor !== null)
      .map((peer) => ({ peer, screen: host?.projectWorldToScreen(peer.cursor!) ?? null }))
      .filter((p): p is { peer: PeerPresence; screen: { x: number; y: number } } => p.screen !== null)
      .map(({ peer, screen }) => ({
        userId: peer.userId,
        screen: [screen.x, screen.y] as [number, number],
        color: peer.color,
        label: peer.userId.length > 12 ? `${peer.userId.slice(0, 12)}…` : peer.userId
      }));
  }

  // Mount / track / tear down the canvas inline-edit surface through the shared IME library. Opening
  // seeds the value + focus once (when the edit begins and its screen rect is known); subsequent rect
  // changes only reposition (transform-only, so the caret never resets under pan/zoom). The library owns
  // the contenteditable + IME mechanics; this effect only forwards the mount target and the callbacks.
  $effect(() => {
    const edit = textEdit;
    const rect = textEditRect;
    if (!edit || !rect) {
      canvasEditHandle = null;
      return;
    }
    if (canvasEditHandle) {
      canvasEditHandle.reposition(rect);
      return;
    }
    const imeHost = ensureTextEditHost();
    if (!imeHost) return;
    canvasEditHandle = imeHost.open(
      { rect, value: edit.value },
      {
        onInput: (value) => {
          if (textEdit) textEdit = { id: textEdit.id, value };
        },
        onCommit: (value) => commitTextEdit(value)
      }
    );
  });

  // Windowed replica: re-aim the data-layer window at the camera viewport.
  $effect(() => {
    void camera;
    if (!sceneClientReady || !sceneClient) return;
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return;
    const topLeft = host?.projectScreenToWorld({ x: 0, y: 0 });
    const bottomRight = host?.projectScreenToWorld({ x: rect.width, y: rect.height });
    if (!topLeft || !bottomRight) return;
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
      // The single focus arbiter: a DOM input OR a focused ui-core TextInput owns typing. The latter
      // extends the same predicate (no second focus check) so a focused UI field suppresses the catalog.
      const domTyping = target ? ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) || target.isContentEditable : false;
      const typing = domTyping || (host?.uiHasFocus() ?? false);
      // Forward the key to the UI runtime FIRST (a neutral forward — the core decides whether it owns it);
      // on consume, swallow it before Escape/Space/catalog. No event.key BEHAVIOR branch here. The raw
      // modifier flags ride along so the core tells a shortcut chord (Cmd+Z/Ctrl+C) from a bare key — a
      // focused field must NOT swallow a chord, or undo/copy/paste/select-all would die inside it.
      // SUPPRESSED while the IME surface is mounted: that OS surface is the sole writer for the focused
      // ui-core field (it owns composition and hands back one committed string via onCommit->uiCommitText).
      // Relaying here too would push each composing keydown — incl. the raw mid-composition jamo Safari/
      // Firefox put in event.key — into the core a SECOND time, doubling/corrupting the field.
      const uiKey = textEditHost?.isEditing()
        ? null
        : host?.uiKey({ key: event.key, text: keyChar(event), ctrl: event.ctrlKey, meta: event.metaKey, alt: event.altKey });
      if (uiKey?.consumed) {
        event.preventDefault();
        return;
      }
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
    // Drop a queued UI-model feed so it can't fire against a torn-down host.
    uiModelFeed.dispose();
  });

  async function connectSceneClient(): Promise<void> {
    const client = new SceneClient({ url: wsBaseUrl(), clientId: clientIdentity(), userId: userIdentity() });
    try {
      const welcome = await client.connect(canvasId);
      sceneClient = client;
      sceneClientReady = true;
      connectionStatus = client.connectionStatus;
      client.onScene((next, settledKeys) => commitClientScene(next, settledKeys));
      client.onStatus((next) => (connectionStatus = next));
      client.onPeers((next) => (peers = next));
      client.onFeature((response) => handleFeatureResponse(response));
      selection = validSelection(welcome, welcome.selection);
      // The connected engine is now authoritative; seed the mirror through the single write path.
      commitClientScene(welcome, []);
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
      selection = validSelection(welcome, welcome.selection);
      // The reconnected engine is authoritative for the new canvas; seed the mirror via the one write path.
      commitClientScene(welcome, []);
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

  // Adopt a core-driven scene update (offline author, acked, or remote), keeping selection valid. This
  // is the ONE place `scene` is assigned. `settledKeys` are the `(object,field)` previews the core TOLD
  // us settled (an ack/reject released the last unacked write); we clear the optimistic preview off that
  // signal — never a transform-value compare, which a coincidentally-equal peer write would trip early.
  function commitClientScene(next: ObjectScene, settledKeys: string[]): void {
    scene = next;
    selection = validSelection(next, selection);
    if (pendingCommit) {
      const committed = next.objects.find((o) => o.id === pendingCommit!.id);
      // Clear when the object is gone OR the core reports this object's transform preview settled.
      if (!committed || settledKeys.includes(`${pendingCommit.id}:transform`)) {
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
      // Pre-connect / offline: author through the SAME core session that owns the connected path, so the
      // core (not the shell) applies the op and owns the scene + undo inverse even with no transport.
      if (session) {
        const result = JSON.parse(session.author(JSON.stringify(op), new Date().toISOString())) as {
          errors: string[];
          inverse?: ObjectOp | null;
        };
        if (result.errors.length > 0) status = result.errors.join("; ");
        else {
          // The op applied inside the core; mirror its scene through the one write path (no settle, offline).
          commitClientScene(JSON.parse(session.scene()) as ObjectScene, []);
          if (undoable && result.inverse) undoStack?.record(op, result.inverse);
        }
        onSettled?.(result.errors.length === 0);
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

  // The order key for a NEW object landing on top — minted by the core's fractional indexing. Pre-load
  // (sceneCore null) falls back to a monotonic key above the current max so the early feed still stacks.
  function nextOrderKey(): string {
    if (sceneCore) return sceneCore.nextOrderKey(scene);
    const maxOrder = scene.objects.reduce((max, o) => (o.order > max ? o.order : max), "a0");
    return `${maxOrder}~`;
  }

  function freshId(prefix: string): string {
    return `${prefix}-${crypto.randomUUID().slice(0, 8)}`;
  }

  function viewportCenterWorld(): { x: number; y: number } {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect) return { x: 120, y: 120 };
    return host?.projectScreenToWorld({ x: rect.width / 2, y: rect.height / 2 }) ?? { x: 120, y: 120 };
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
    // Hold the built text object so the overlay can mount this tick on the async connected path, before
    // the canonical scene reflects the insert.
    if (kind === "text") pendingTextInsert = object;
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
    if (phase === "end" && (!sceneCore || !createThresholds)) return;
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
    const resolved = sceneCore.resolveCreateRelease(
      { end: world, snapped, target: targetId },
      createDrag.lastSnap,
      createThresholds.createAnchorReuseTolerancePx / camera.zoom
    );
    const startSnap = createDrag.startSnap;
    const span: DragSpan = { start: createDrag.span.start, end: resolved.end };
    const snapTarget = resolved.target;
    createDrag = null;
    const dx = Math.abs(span.end.x - span.start.x);
    const dy = Math.abs(span.end.y - span.start.y);
    const minExtent = createThresholds.minDragExtentPx;
    const tooSmall = kind === "line" ? dx < minExtent && dy < minExtent : dx < minExtent || dy < minExtent;
    const object = tooSmall
      ? sceneCore.buildPrimitive(kind, span.start, freshId(kind), nextOrderKey(), selectedColor)
      : sceneCore.buildPrimitiveFromDrag(kind, span, freshId(kind), nextOrderKey(), selectedColor);
    // A snapped drag-create binds the snapped CORNER(s) to the target's outline with a persistent anchor.
    // BOTH ends count (start drawn FROM an edge, end drawn TO one), each binding the nearest node, which
    // then reprojects through the target's transform so the object moves WITH the target. (Alt-create = no
    // anchor.) Same core synthesizeCreateAnchorsBoth the freehand pen uses — one release-anchor source for both tools.
    if (!tooSmall && sceneCore) {
      const anchors = sceneCore.synthesizeCreateAnchorsBoth(scene, object, [
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
    if (!sceneCore || !createThresholds) {
      drawSnap = null;
      createHoverSnap = null;
      drawPoints = null;
      return;
    }
    const resolved = sceneCore.resolveCreateRelease(
      { end: canon ? canon.at : world, snapped: canon !== null, target: canon?.target ?? null },
      drawSnap?.last ?? null,
      createThresholds.createAnchorReuseTolerancePx / camera.zoom
    );
    drawSnap = null;
    createHoverSnap = null;
    const points = [...drawPoints, resolved.end];
    drawPoints = null;
    if (points.length < 2) return;
    // Multi-stroke merge, PRIORITY over insert + anchoring: a stroke end landing on an open-class
    // object's endpoint chains the stroke into it (edit-geometry on the survivor, no insert). Every
    // judgment lives in the core; the shell only branches on the returned ops. Null = no merge.
    const mergeOps = sceneCore.mergeOpenStrokeOps(
      scene,
      points,
      recognizeMode,
      createThresholds.mergeEndpointTolerancePx / camera.zoom
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
      const anchors = sceneCore.synthesizeCreateAnchorsBoth(scene, object, [
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
    // The core maps the WORLD touch into object-local quantized space (inverse-affine + quantize) and
    // returns the WHOLE op batch: [] on a miss / singular transform, [delete] when the cut empties the
    // object, else [edit-geometry, ...followers]. The shell authors the result and owns the UI follow-up.
    const ops = sceneCore.partialEraseOps(scene, id, world.x, world.y, ERASE_RADIUS_QUANTIZED);
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
    if (!sceneCore) return;
    const ids = currentSelectionIds();
    // The core mints fresh ids (`{idPrefix}-{n}`), fresh fractional order keys above the scene top, and
    // the canonical +40/+40 offset. A unique idPrefix per gesture keeps the per-call `-n` ids globally unique.
    const ops = sceneCore.duplicateOps(scene, ids, freshId("dup"), 0);
    if (ops.length === 0) return;
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
    showToast("Duplicated selection");
  }

  // Reparent the selected objects under a fresh frame object: the core sizes a clipped rect frame to
  // their union world-AABB and reparents each child under it. Null when fewer than two members resolve.
  function groupSelection(): void {
    if (!sceneCore) return;
    const ids = currentSelectionIds();
    const frameId = freshId("frame");
    const ops = sceneCore.groupOps(scene, ids, frameId);
    if (!ops) return;
    authorOp({ kind: "batch", ops });
    selection = { kind: "object", id: frameId };
    persistSelection(selection);
    showToast("Grouped selection");
  }

  // Ungroup reparents children out of the frame, then deletes the now-empty frame in the SAME batch op.
  // The core authors both halves; only a container (has children) ungroups, so a leaf is never dissolved.
  function ungroupSelection(): void {
    if (selection.kind !== "object" || !sceneCore) return;
    if (!sceneCore.hasChildren(scene, selection.id)) return;
    const ops = sceneCore.ungroupOps(scene, selection.id);
    if (!ops) return;
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

  // Nudge the selection by (dx, dy) through the SAME core path a body drag commits: moveOpsForPick
  // derives the MoveRoots from selection + picked id in-core, so the nudge cascades to a parent's subtree
  // and follows anchors, exactly like an equal-delta drag (unlike the old independent per-id translate).
  function nudgeSelection(dx: number, dy: number): void {
    if (!sceneCore) return;
    const ids = currentSelectionIds();
    if (ids.length === 0) return;
    const ops = sceneCore.moveOpsForPick(scene, selection, ids[0], translateTransform(dx, dy));
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
  // next-lower ("backward") neighbor over the flat scene order. Single selection only (a multi-set has no
  // well-defined neighbor); the core authors the 2-op swap, or null when there is no neighbor that way.
  function reorderStep(direction: "forward" | "backward"): void {
    const ids = currentSelectionIds();
    if (ids.length !== 1) {
      // Fall back to front/back for the multi case (no single neighbor to swap).
      reorderSelection(direction === "forward" ? "front" : "back");
      return;
    }
    if (!sceneCore) return;
    const ops = sceneCore.reorderStepOps(scene, ids[0], direction);
    if (!ops) return;
    authorOp(ops.length === 1 ? ops[0] : { kind: "batch", ops });
  }

  // The order key for a NEW object landing at the back — minted by the core's fractional indexing. Pre-load
  // (sceneCore null) falls back to a short ascii key below the current min.
  function backOrderKey(): string {
    if (sceneCore) return sceneCore.backOrderKey(scene);
    const minOrder = scene.objects.reduce((min, o) => (o.order < min ? o.order : min), "z");
    return minOrder > "0" ? "0" : `0${minOrder}`;
  }

  function selectAll(): void {
    if (!sceneCore) return;
    selection = sceneCore.selectAll(scene);
    persistSelection(selection);
  }

  // Adopt the toolbar's color (the default for the next NEW shape), then recolor a single selection via a `set-style` op.
  function applySelectedColor(color: string): void {
    selectedColor = color;
    if (!sceneCore) return;
    const op = buildColorApplyOp(sceneCore, scene, selection, color);
    if (op) authorOp(op);
  }

  // ----- inspector property panel ------------------------------------------

  // Author a direct property edit from the panel. The control carries its op kind + field (the core's
  // catalog metadata); the core helper lowers it to the matching ObjectOp PER selected id, and these
  // author as one undo unit (a Batch for multi-select). The decompose/recompose for transform edits
  // happens in-core inside inspectorEditOp — never matrix math here.
  function onInspectorEdit(control: InspectorControlValue, value: unknown): void {
    if (!sceneCore) return;
    const op = authorInspectorEdit(sceneCore, scene, control, currentSelectionIds(), value);
    if (op) authorOp(op);
  }

  // A button control (canonicalize) acts on each selected id as one Batch.
  function onInspectorAction(control: InspectorControlValue): void {
    const op = authorInspectorAction(control, currentSelectionIds());
    if (op) authorOp(op);
  }

  // A double-click on a container drills in (sets the active container); a leaf enters inline text edit.
  // A null signal (missed every object) is a no-op; the container-vs-leaf decision lives in the core.
  function handleObjectDoubleClick(signal: { id: string; hasChildren: boolean } | null): void {
    if (!signal || !sceneCore) return;
    const action = resolveDoubleClick(sceneCore, scene, signal);
    if (action.kind === "drill-in") {
      enterContainerScope(action.id);
      selectObject({ kind: "object", id: action.id });
      showToast("Entered group");
      return;
    }
    if (action.kind === "edit-leaf") enterTextEdit(action.id);
  }

  // A focused ui-core TextInput asked the shell to mount an editing surface (the core's EditRequest). Open
  // the SAME shared IME library the canvas inline edit uses, so CJK/IME composition has an OS surface that
  // owns the live text natively. The OS surface is the SOLE writer while mounted: the window key relay is
  // suppressed for it (see the keydown arbiter) so a typed/composed char is not also pushed per-key into
  // the core, and onInput stays a no-op (the core needs no live mirror). On commit the library hands back
  // ONE finished string, which the core lands wholesale via uiCommitText (blur + final TextChanged) —
  // correct even when CJK composition deleted/replaced in place. The library never knows which consumer
  // it serves.
  function handleUiEdit(edit: UiEditRequest): void {
    const imeHost = ensureTextEditHost();
    if (!imeHost) return;
    const [x, y, w, h] = edit.rect;
    imeHost.open(
      { rect: { x, y, width: w, height: h }, value: edit.value },
      {
        onInput: () => {},
        onCommit: (value) => host?.uiCommitText(value)
      }
    );
  }

  // Route a typed intent the Rust built-in UI resolved a widget actuation to (the core's
  // `shape_ui::resolve`) into the shell's EXISTING op-authoring handler. The shell authors no op here and
  // re-derives nothing — each arm forwards to a handler it already has. A `Command` fired from an OPEN
  // context menu routes through contextHandlers (anchored inserts + pop-out on the picked id); otherwise
  // it runs the plain catalog handler. `Dismiss` closes the matching floating overlay.
  function handleUiIntent(intent: UiIntent): void {
    switch (intent.type) {
      case "command": {
        const menu = contextMenu;
        if (menu) {
          // A context-menu command acts at the right-click anchor / on the picked selection, then the
          // menu closes (the scrim's own Dismiss also closes it, but a chosen row dismisses immediately).
          const handler = contextHandlers(menu)[intent.id];
          contextMenu = null;
          if (handler) return void handler();
        }
        shortcutHandlers()[intent.id as keyof ReturnType<typeof shortcutHandlers>]?.();
        return;
      }
      case "selectColor":
        return applySelectedColor(intent.hex);
      case "selectPenWidth":
        return void (penWidthPx = intent.px);
      case "applyTemplate":
        return applyTemplate(intent.id);
      case "selectCanvas":
        return void switchToCanvas(intent.id);
      case "newCanvas":
        return void createCanvas("New canvas");
      case "deleteCanvas":
        return void deleteCanvas(intent.id);
      case "inspectorEdit":
        // The intent carries the catalog metadata (opKind/field/unitScale) the core resolved; rebuild the
        // control value the existing authoring path expects and lower it through onInspectorEdit.
        return onInspectorEdit(
          {
            id: intent.controlId,
            label: intent.controlId,
            widget: { kind: "text" },
            value: null,
            mixed: false,
            opKind: intent.opKind,
            field: intent.field ?? undefined,
            unitScale: intent.unitScale
          },
          intent.value
        );
      case "inspectorAction":
        return onInspectorAction({
          id: intent.controlId,
          label: intent.controlId,
          widget: { kind: "button" },
          value: null,
          mixed: false,
          opKind: "canonicalize",
          unitScale: 1
        });
      case "dismiss":
        settingsOpen = false;
        contextMenu = null;
        return;
    }
  }

  // Enter inline edit: seed the overlay value from the object's first text run (empty when the object
  // hasn't landed in `scene` yet — `textEditRect` derives reactively, so the overlay appears on arrival).
  function enterTextEdit(id: string): void {
    const object = scene.objects.find((o) => o.id === id);
    textEdit = { id, value: object?.text?.runs?.[0]?.text ?? "" };
  }

  // Commit the inline edit's committed string (handed back by the IME library) as a set-text op, then
  // dismiss the edit state. An unchanged commit degrades to a no-op in the core (identical set-text
  // applies nothing and its empty-Batch inverse is skipped by the undo stack), so no shell-side dedup.
  function commitTextEdit(value: string): void {
    const edit = textEdit;
    textEdit = null;
    pendingTextInsert = null;
    if (!edit) return;
    authorOp({ kind: "set-text", id: edit.id, text: { runs: [{ text: value }] } });
  }

  // The templates the More→Templates popup offers; ids map to buildObjectTemplate. Fed to the Rust UI
  // model as TemplateEntry rows ({ id, title, description }).
  const TEMPLATES: { id: string; title: string; description: string }[] = [
    { id: "todo_board", title: "Todo board", description: "Grouped To do / In progress / Done columns" },
    { id: "decision_map", title: "Decision map", description: "Options and outcomes wired with connectors" },
    { id: "presentation", title: "Presentation", description: "A deck grouping title and content slides" }
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
    // The core places the anchor right of the right-most object; pass only the empty-scene viewport fallback.
    const fallback = viewportCenterWorld();
    if (!sceneCore) return fallback;
    return sceneCore.templateAnchor(scene, fallback);
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
    // Keep the forwarded drill-in scope token in lockstep: the core decides whether the new selection
    // still sits in the active container's scope; the shell only forwards selection + container and
    // mirrors the token (retracting it when the verdict says out of scope). Pre-core, do not retract —
    // matches the validSelection pre-core passthrough.
    if (activeContainer !== null && sceneCore && !sceneCore.objectSelectionInScope(scene, selection, activeContainer)) {
      clearContainerScope();
    }
    persistSelection(selection);
  }

  // Forwarded drill-in scope token (like setCoarseRotate): the core scopes the next pointer-down pick to
  // this container's direct children. The shell only mirrors + forwards the id; it never picks off it.
  function enterContainerScope(id: string): void {
    activeContainer = id;
    host?.setActiveContainer(id);
  }

  function clearContainerScope(): void {
    activeContainer = null;
    host?.setActiveContainer(null);
  }

  function validSelection(currentScene: ObjectScene, currentSelection: ObjectSelection): ObjectSelection {
    // The core owns the prune/collapse rule; pre-core (sceneCore null) keeps the selection as-is.
    if (!sceneCore) return currentSelection;
    return sceneCore.validSelection(currentScene, currentSelection);
  }

  function handleEscape(): void {
    // Escape, in priority order: commit the inline text edit (keep typed text), cancel pen stroke, cancel
    // shape drag-create, then disarm the tool. The inline edit's own Escape (mid-edit, contenteditable
    // focused) is handled + stopPropagation'd by the IME library; this window-level fallback blurs the
    // active surface so the library commits the live value (never the stale shell mirror).
    if (textEdit) return void textEditHost?.blurActive();
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
      erase: () => setActiveTool("erase"),
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
      "toggle-fullscreen": () => void toggleFullscreen(),
      export: () => exportSelection(),
      "toggle-diagnostics": () => (diagnosticsOpen = !diagnosticsOpen),
      "toggle-theme": () => toggleTheme(),
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
    const world = host?.projectScreenToWorld({ x: anchor.clientX - rect.left, y: anchor.clientY - rect.top });
    if (!world) return;
    contextMenu = { selection: picked, x: anchor.clientX, y: anchor.clientY, world };
  }

  // An entry's `enabled` predicate: ungroup needs a container, pop-out needs a parent (both core-only queries, disabled until the core loads); everything else is enabled.
  function contextEntryEnabled(entry: Extract<ContextMenuEntry<string>, { id: string }>, picked: ObjectSelection): boolean {
    if (entry.id === "ungroup") return !!sceneCore && ungroupPickEnabledOf(sceneCore, scene, picked);
    if (entry.id === "pop-out") return !!sceneCore && popOutPickEnabledOf(sceneCore, scene, picked);
    return true;
  }

  // The Rust context-menu model rows: each resolved item is a catalog-bound row (`commandId`), a danger
  // flag, and a disabled flag; a null entry becomes a separator (`commandId: null`). The Rust UI renders
  // this and a pressed enabled row resolves to a `Command` intent the shell routes through contextHandlers.
  type ContextItemModel = { commandId: string | null; label: string; danger: boolean; disabled: boolean };
  function contextMenuItems(menu: ContextMenuState): ContextItemModel[] {
    const handlers = contextHandlers(menu);
    const resolved = resolveContextMenuItems<string>(
      menu.selection,
      commandCatalog,
      (id) => handlers[id] !== undefined,
      contextEntryEnabled
    );
    return resolved.map((item) =>
      item === null
        ? { commandId: null, label: "", danger: false, disabled: false }
        : { commandId: item.id, label: item.label, danger: item.danger ?? false, disabled: item.disabled }
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

  // Lazily build the shared IME host against the canvas wrapper (the overlay's positioning context).
  // Returns null until the wrapper is bound. The instance is reused for both the canvas inline edit and
  // the ui-core TextInput edit so there is ever only ONE editing surface.
  function ensureTextEditHost(): TextEditHost | null {
    if (textEditHost) return textEditHost;
    if (!canvasWrap) return null;
    textEditHost = new TextEditHost({
      overlayRoot: canvasWrap,
      className: "text-edit-overlay",
      ariaLabel: "Edit text"
    });
    return textEditHost;
  }

  function handleHost(next: ShapeCanvasHost): void {
    host = next;
    // Apply the persisted theme the moment the host is wired. The theme $effect runs at mount when
    // `host` is still null (a non-reactive `let`, so it never re-runs on assignment), so without this
    // the renderer would never hear the theme and the canvas + Rust UI would stay on the light default
    // — dark mode rendered light. Set it before the scene loads so the canvas is born in-theme.
    host.setObjectTheme(theme === "dark");
    host.loadObjectScene(scene, selection);
    host.setTool(activeTool);
    // Relocate the blur-before-preventDefault hazard into the shared library: the engine commits an
    // active edit through this hook before its mousedown preventDefault swallows the blur.
    const imeHost = ensureTextEditHost();
    if (imeHost) host.setBlurActiveEditable(() => imeHost.blurActive());
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!sceneClientReady || !sceneClient || !canvasWrap) return;
    const now = Date.now();
    if (now - lastCursorSentAt < CURSOR_THROTTLE_MS) return;
    lastCursorSentAt = now;
    const rect = canvasWrap.getBoundingClientRect();
    const cursor = host?.projectScreenToWorld({ x: event.clientX - rect.left, y: event.clientY - rect.top });
    const topLeft = host?.projectScreenToWorld({ x: 0, y: 0 });
    const bottomRight = host?.projectScreenToWorld({ x: rect.width, y: rect.height });
    if (!cursor || !topLeft || !bottomRight) return;
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

<!--
  The shell renders ONLY the surface + the OS-input-forwarding wrapper now. Every product UI — toolbar,
  inspector, settings, context menu, templates, presence, status/toast, watermark, theme toggle, canvas
  switcher, diagnostics — is rendered by the Rust ui extension through the canvas, fed via host.setUiModel
  and actuated through the resolved-intent path (handleUiIntent). No product UI lives in this shell.
-->
<div class="app-shell">
  <main class="studio-stage">
    <section class="canvas-panel">
      <div class="flow-wrap renderer-scene-surface" data-tool={activeTool} data-affordance={cursorAffordance} bind:this={canvasWrap} role="application" aria-label="Canvas" onpointermove={handlePointerMove}>
        <CanvasHost
          initialCamera={camera}
          callbacks={hostCallbacks}
          {readyState}
          {rendererDetail}
          {hasRenderableScene}
          onHost={handleHost}
          onContextMenuRequest={handleContextMenuRequest}
        />
      </div>
    </section>
  </main>
</div>
