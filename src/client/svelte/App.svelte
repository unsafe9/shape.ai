<script lang="ts">
  import { onDestroy } from "svelte";
  import { BrainCircuit, Loader2, Copy, Trash2, Group as GroupIcon, Ungroup, MessageSquarePlus, LayoutTemplate } from "lucide-svelte";
  import { screenToWorld } from "../renderer/scene";
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
  import type { ActiveTool } from "../renderer/engine";
  import type { HoverAffordance } from "../renderer/wasmLoader";
  import {
    loadSceneCore,
    type ObjectCommand,
    type SceneCore,
    type UndoStack
  } from "../scene/sceneCoreWasm";
  import { createShortcutDispatcher } from "../lib/shortcuts";
  import { buildPrimitiveObject } from "../lib/objectPrimitives";
  import type { PrimitiveKindId } from "../lib/toolbar";
  import Toolbar from "./Toolbar.svelte";
  import SettingsModal from "./SettingsModal.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";
  import ContextMenu, { type ContextMenuItem } from "./ContextMenu.svelte";
  import PeerCursors from "./PeerCursors.svelte";

  // ----- canonical object scene (D1) -----
  let scene = $state<ObjectScene>(emptyObjectScene());

  // ----- selection: single ObjectSelection + transient multi -----
  let selection = $state<ObjectSelection>({ kind: "canvas" });

  // ----- FC-08: non-destructive live object drag preview. The canonical `scene`
  //       is never mutated mid-drag; the renderer is fed a clone with this one
  //       object shifted, so the commit op on pointer-up captures the correct
  //       inverse (original transform), keeping undo correct (D21). -----
  let dragPreview = $state<{ id: string; dx: number; dy: number } | null>(null);

  // FC-16: in the connected path authorOp's scene update is async (it arrives via
  // commitClientScene), so clearing dragPreview the instant the commit op is
  // authored would let `feedScene` briefly revert to the pre-drag canonical scene
  // — the object snaps back to its start then jumps to final. Instead remember the
  // committed {id, transform} and clear dragPreview inside commitClientScene once
  // the next canonical scene reflects that object's committed transform.
  let pendingCommit: { id: string; transform: SceneObject["transform"] } | null = null;

  // ----- FC-11: freehand pen. While the draw tool is active, the in-progress
  //       stroke's world points accumulate here; `feedScene` appends a transient
  //       preview object so the stroke is visible before it commits. On pointer-up
  //       the points lower to a committed `Object` via the wasm core. -----
  let drawPoints = $state<{ x: number; y: number }[] | null>(null);
  const PEN = { color: "#1f2933", widthPx: 2, epsilon: 2.0 };

  // ----- ephemeral camera / chrome -----
  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.6 });
  let status = $state("Ready");
  let busy = $state(false);
  let diagnosticsOpen = $state(false);
  let settingsOpen = $state(false);
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

  // W2-03: the cursor affordance reflected onto the canvas wrapper. Space-hold
  // shows the pan grab cursor; the draw tool keeps its own crosshair (no override);
  // otherwise the core's per-move hover classification drives it (CSS in styles.css).
  const cursorAffordance = $derived(spaceHeld ? "pan" : activeTool === "draw" ? null : affordance);

  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(scene.objects.length > 0);
  const selectedObject = $derived(selection.kind === "object" ? scene.objects.find((o) => o.id === selection.id) ?? null : null);

  // FC-08: the scene actually fed to the renderer — the canonical scene during
  // normal editing, or a clone with the dragged object shifted during a live drag.
  // FC-11: while a freehand stroke is in progress, append a transient preview
  // object built from the world points (no op-apply — geometry-vocabulary only).
  const feedScene = $derived(buildFeedScene(scene, dragPreview, drawPoints));

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onStats: (stats) => (rendererStats = stats),
    onStatus: (message) => (status = message),
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyStatus(status)) status = "Ready";
    },
    // W2-03: shift/meta-click toggles the object in/out of the multi-select set;
    // a plain click replaces the selection with that object.
    onSelectObject: (id, additive) =>
      selectObject(additive ? toggleObjectSelection(selection, id) : { kind: "object", id }),
    onTransformPreview: (id, dx, dy) => {
      // A new drag supersedes any commit still waiting for its scene update, so a
      // stale pendingCommit can never clear (or freeze) the new preview.
      if (pendingCommit && pendingCommit.id !== id) pendingCommit = null;
      dragPreview = { id, dx, dy };
    },
    onTransformCommit: (id, dx, dy) => {
      // The canonical scene was never mutated during the drag, so op-apply
      // captures the correct inverse (original transform), satisfying D21 undo.
      const src = scene.objects.find((o) => o.id === id);
      if (!src) return void (dragPreview = null);
      const transform = shiftTransform(src.transform, dx, dy);
      // FC-16: pre-connect authorOp applies synchronously (scene is already the
      // committed scene on return), so the preview can clear immediately. In the
      // connected path the scene update is async — hold the preview until
      // commitClientScene sees the committed transform land (no snap-back). If the
      // commit op fails, clear the preview here so it never freezes the object.
      const wasConnected = sceneClientReady && sceneClient !== null;
      if (wasConnected) pendingCommit = { id, transform };
      authorOp({ kind: "set-transform", id, transform }, true, (ok) => {
        if (!ok) {
          pendingCommit = null;
          dragPreview = null;
        }
      });
      if (!wasConnected) dragPreview = null;
    },
    onMarquee: (ids) => {
      // FC-16: route the marquee result through the same validation as
      // onSelectObject (validSelection drops stale ids and collapses the kind).
      const next: ObjectSelection =
        ids.length >= 2 ? { kind: "multi", ids } : ids.length === 1 ? { kind: "object", id: ids[0] } : { kind: "canvas" };
      selectObject(next);
    },
    // FC-11: freehand pen capture. Accumulate world points across start/move; on
    // end, lower the stroke to an object via the wasm core and author an
    // insert-object op (the tool stays sticky in "draw"); cancel discards.
    onDraw: (phase, world) => handleDraw(phase, world),
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
      undoStack = sceneCore.createUndoStack(userIdentity());
    } catch {
      sceneCore = null;
    }
  }

  // Push the object scene into the renderer whenever it or the selection changes.
  // FC-08: feeds `feedScene` (the canonical scene, or a drag-preview clone) so a
  // live drag previews without mutating the canonical state.
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
    // FC-16: once the canonical scene reflects the committed drag transform, drop
    // the preview so `feedScene` falls back to the canonical scene with no flash.
    if (pendingCommit) {
      const committed = next.objects.find((o) => o.id === pendingCommit!.id);
      // Drop the preview once the canonical transform lands (success) or the
      // object is gone (concurrent delete) — either way the canonical scene is
      // now authoritative, so the preview must not linger.
      if (!committed || transformsEqual(committed.transform, pendingCommit.transform)) {
        pendingCommit = null;
        dragPreview = null;
      }
    }
  }

  function handleFeatureResponse(response: FeatureResponse): void {
    if (response.feature === "featureError") {
      status = response.message;
      return;
    }
    if (response.feature === "templateApplied") {
      status = `Inserted ${response.object_ids.length} objects`;
      return;
    }
    if (response.feature === "exportReady") {
      status = `Export ready: ${response.artifact_ref}`;
      return;
    }
    if (response.feature === "commentUpserted") {
      status = "Comment added";
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

  function insertPrimitive(kind: PrimitiveKindId, anchor?: { x: number; y: number }): void {
    const center = anchor ?? viewportCenterWorld();
    const object = buildPrimitiveObject(kind, center, freshId(kind), nextOrderKey());
    authorOp({ kind: "insert-object", object });
    selection = { kind: "object", id: object.id };
    persistSelection(selection);
    status = `Inserted ${kind}`;
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
    const object = sceneCore.freehandToObject(points, PEN.color, PEN.widthPx, PEN.epsilon, freshId("draw"), nextOrderKey());
    authorOp({ kind: "insert-object", object });
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
    status = "Duplicated selection";
  }

  // Group: reparent the selected objects under a fresh frame object (D3). FC-14:
  // the frame geometry encloses the children — a rect sized to the union world-AABB
  // of the selected objects, positioned by a pure-translation transform at the
  // AABB's top-left. Children transforms are world-absolute, so they are reparented
  // unchanged.
  function groupSelection(): void {
    const ids = currentSelectionIds();
    if (ids.length < 2) return;
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
    status = "Grouped selection";
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
    status = "Ungrouped selection";
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

  // ----- templates (buildObjectTemplate -> FeatureRequest.templateApply) ---

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
    // FC-11: a pending Escape first cancels an in-progress pen stroke.
    if (drawPoints) return void (drawPoints = null);
    if (settingsOpen) return void (settingsOpen = false);
    if (templateOpen) return void (templateOpen = false);
    if (diagnosticsOpen) return void (diagnosticsOpen = false);
    if (contextMenu) return void (contextMenu = null);
    selectObject({ kind: "canvas" });
  }

  function setActiveTool(tool: ActiveTool): void {
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
      "edit-text": () => {
        if (selection.kind === "object") renameSelected(window.prompt("Text") ?? selectedObject?.text?.runs?.[0]?.text ?? "");
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
  type ContextMenuEntry = "separator" | { id: string; icon?: typeof Copy; danger?: boolean; disabledFor?: ObjectSelection["kind"] };

  const OBJECT_MENU: ContextMenuEntry[] = [
    { id: "duplicate", icon: Copy },
    { id: "group", icon: GroupIcon, disabledFor: "object" },
    { id: "ungroup", icon: Ungroup },
    { id: "bring-to-front" },
    { id: "send-to-back" },
    { id: "add-comment", icon: MessageSquarePlus },
    "separator",
    { id: "delete", icon: Trash2, danger: true }
  ];

  const CANVAS_MENU: ContextMenuEntry[] = [
    { id: "insert-rectangle" },
    { id: "insert-text", icon: MessageSquarePlus },
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
      return {
        label: command?.label ?? entry.id,
        icon: entry.icon,
        danger: entry.danger,
        disabled: entry.disabledFor === picked.kind,
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
      "bring-to-front": base["bring-to-front"],
      "send-to-back": base["send-to-back"],
      "add-comment": base["add-comment"],
      delete: base.delete,
      "insert-rectangle": () => insertPrimitive("rectangle", menu.world),
      "insert-text": () => insertPrimitive("text", menu.world),
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

  // FC-08: a shallow clone of the scene with one object's transform shifted by
  // (dx,dy) world px. Used only for the live drag preview feed; the canonical
  // scene is never mutated, so undo stays correct.
  function sceneWithObjectShifted(source: ObjectScene, id: string, dx: number, dy: number): ObjectScene {
    return {
      ...source,
      objects: source.objects.map((o) => (o.id === id ? { ...o, transform: shiftTransform(o.transform, dx, dy) } : o))
    };
  }

  // FC-08/FC-11: the renderer feed. Start from the canonical scene, apply a live
  // drag-preview shift, and append a transient pen-stroke preview — neither mutates
  // the canonical `scene` nor runs op-apply (geometry-vocabulary construction only).
  function buildFeedScene(
    source: ObjectScene,
    drag: { id: string; dx: number; dy: number } | null,
    pen: { x: number; y: number }[] | null
  ): ObjectScene {
    let feed = drag ? sceneWithObjectShifted(source, drag.id, drag.dx, drag.dy) : source;
    const preview = pen && pen.length >= 1 ? drawPreviewObject(pen) : null;
    if (preview) feed = { ...feed, objects: [...feed.objects, preview] };
    return feed;
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
      stroke: { paint: { kind: "solid", color: PEN.color }, width: PEN.widthPx * GEOMETRY_QUANTUM_PER_PX, cap: "round", join: "round" }
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
          {busy}
          {templateOpen}
          {diagnosticsOpen}
          {selectedObject}
          {canvases}
          activeCanvasId={canvasId}
          {connectionStatus}
          {canvasBusy}
          onSetTool={setActiveTool}
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

        {#if templateOpen}
          <div class="template-panel" aria-label="Templates">
            <button class="template-item" type="button" onclick={() => applyTemplate("todo_board")}>Todo board</button>
            <button class="template-item" type="button" onclick={() => applyTemplate("decision_map")}>Decision map</button>
            <button class="template-item" type="button" onclick={() => applyTemplate("presentation")}>Presentation</button>
          </div>
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
      </div>

      {#if contextMenu}
        <ContextMenu x={contextMenu.x} y={contextMenu.y} title={contextMenuTitle(contextMenu.selection)} items={contextMenuItems(contextMenu)} />
      {/if}

      {#if settingsOpen}
        <SettingsModal catalog={commandCatalog} onClose={() => (settingsOpen = false)} />
      {/if}

      {#if busy || status !== "Ready"}
        <div class="canvas-status" role="status">
          {#if busy}<Loader2 class="spin" size={15} />{/if}
          {status}
        </div>
      {/if}
    </section>
  </main>
</div>
