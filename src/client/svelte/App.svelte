<script lang="ts">
  import { onDestroy } from "svelte";
  import { BrainCircuit, Loader2, Copy, Trash2, Group as GroupIcon, Ungroup, MessageSquarePlus, LayoutTemplate } from "lucide-svelte";
  import { screenToWorld } from "../renderer/scene";
  import type { CameraState } from "../../shared/renderScene";
  import {
    emptyObjectScene,
    translateTransform,
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
  import {
    loadSceneCore,
    type ObjectCommand,
    type SceneCore
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

  // ----- ephemeral camera / chrome -----
  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.6 });
  let status = $state("Ready");
  let busy = $state(false);
  let diagnosticsOpen = $state(false);
  let settingsOpen = $state(false);
  let templateOpen = $state(false);
  let activeTool = $state<ActiveTool>("select");
  let spaceToolBeforeHold: ActiveTool | null = null;

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

  // ----- per-actor undo/redo stacks (D21): inverse ops authored back through
  //       the SAME op-apply path; only this client's ops are undoable. -----
  let undoStack: ObjectOp[] = [];
  let redoStack: ObjectOp[] = [];

  // ----- non-reactive refs -----
  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;

  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(scene.objects.length > 0);
  const selectedObject = $derived(selection.kind === "object" ? scene.objects.find((o) => o.id === selection.id) ?? null : null);

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onStats: (stats) => (rendererStats = stats),
    onStatus: (message) => (status = message),
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyStatus(status)) status = "Ready";
    }
  };

  // Boot the scene-core wasm (op-apply + catalog) and open the WS session.
  void bootstrapSceneCore();
  void connectSceneClient();

  async function bootstrapSceneCore(): Promise<void> {
    try {
      sceneCore = await loadSceneCore();
      commandCatalog = sceneCore.objectCommandCatalog();
    } catch {
      sceneCore = null;
    }
  }

  // Push the object scene into the renderer whenever it or the selection changes.
  $effect(() => {
    const current = scene;
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
      // U5: Space-hold temporarily activates Hand (pan); restore on keyup.
      if (event.code === "Space" && !typing && !event.repeat) {
        event.preventDefault();
        if (spaceToolBeforeHold === null) {
          spaceToolBeforeHold = activeTool;
          setActiveTool("hand");
        }
        return;
      }
      dispatch(event);
    }
    function handleKeyUp(event: KeyboardEvent) {
      if (event.code === "Space" && spaceToolBeforeHold !== null) {
        event.preventDefault();
        setActiveTool(spaceToolBeforeHold);
        spaceToolBeforeHold = null;
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
      undoStack = [];
      redoStack = [];
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

  async function deleteCanvas(targetCanvasId: string): Promise<void> {
    if (!sceneClient || targetCanvasId === canvasId) return;
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
  // wasm core and returns the inverse (the undo entry, D21). A fresh user op
  // clears the redo stack; an undo/redo replay does not (handled by the caller).
  function authorOp(op: ObjectOp, undoable = true): void {
    if (!sceneClientReady || !sceneClient) {
      // Pre-connect: apply optimistically through the core only, no wire.
      if (sceneCore) {
        const applied = sceneCore.applyObjectOp(scene, op);
        if (applied.errors.length > 0) status = applied.errors.join("; ");
        else {
          scene = applied.scene;
          if (undoable && applied.inverse) {
            undoStack.push(applied.inverse);
            redoStack = [];
          }
        }
      }
      return;
    }
    void sceneClient.applyObjectOp(op).then((result) => {
      if (result.errors.length > 0) {
        status = result.errors.join("; ");
        return;
      }
      if (undoable && result.inverse) {
        undoStack.push(result.inverse);
        redoStack = [];
      }
      // The engine's onScene callback commits the optimistic scene.
    });
    sceneClient.flush();
  }

  function undo(): void {
    const inverse = undoStack.pop();
    if (!inverse || !sceneClient) return;
    // Capture this undo's own inverse so redo can re-apply (D21 same pipeline).
    void sceneClient.applyObjectOp(inverse).then((result) => {
      if (result.errors.length === 0 && result.inverse) redoStack.push(result.inverse);
      else if (result.errors.length > 0) status = result.errors.join("; ");
    });
    sceneClient.flush();
  }

  function redo(): void {
    const inverse = redoStack.pop();
    if (!inverse || !sceneClient) return;
    void sceneClient.applyObjectOp(inverse).then((result) => {
      if (result.errors.length === 0 && result.inverse) undoStack.push(result.inverse);
      else if (result.errors.length > 0) status = result.errors.join("; ");
    });
    sceneClient.flush();
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

  // Group: reparent the selected objects under a fresh frame object (D3).
  function groupSelection(): void {
    const ids = currentSelectionIds();
    if (ids.length < 2) return;
    const frame: SceneObject = {
      id: freshId("frame"),
      order: nextOrderKey(),
      geometry: { d: "M 0 0 L 1 0 L 1 1 L 0 1 Z", fillRule: "nonZero" },
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

  function ungroupSelection(): void {
    if (selection.kind !== "object") return;
    const id = selection.id;
    const children = scene.objects.filter((o) => o.parent === id);
    if (children.length === 0) return;
    const ops: ObjectOp[] = children.map((child) => ({ kind: "reparent", id: child.id, order: child.order }));
    authorOp({ kind: "batch", ops });
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
      "hand-pan": () => setActiveTool("hand"),
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
      "bring-forward": () => reorderSelection("front"),
      "send-backward": () => reorderSelection("back"),
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

  function handleContextMenuRequest(point: { x: number; y: number }): void {
    if (!canvasWrap) return;
    pendingContextScreen = { clientX: point.x, clientY: point.y };
    // Object hit-test is part of the deferred renderer object pass; until then,
    // the right-click acts on the current selection (or canvas).
    handleContextPick(selection, point);
  }

  function handleContextPick(picked: ObjectSelection, screen: { x: number; y: number }): void {
    const anchor = pendingContextScreen ?? { clientX: screen.x, clientY: screen.y };
    pendingContextScreen = null;
    if (!canvasWrap) return;
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

  // The unified object context menu, derived from the object command catalog.
  function contextMenuItems(menu: ContextMenuState): (ContextMenuItem | null)[] {
    const picked = menu.selection;
    if (picked.kind === "object" || picked.kind === "multi") {
      return [
        { label: "Duplicate", icon: Copy, onSelect: () => closeContextThen(() => duplicateSelection()) },
        { label: "Group", icon: GroupIcon, disabled: picked.kind !== "multi", onSelect: () => closeContextThen(() => groupSelection()) },
        { label: "Ungroup", icon: Ungroup, onSelect: () => closeContextThen(() => ungroupSelection()) },
        { label: "Bring to front", onSelect: () => closeContextThen(() => reorderSelection("front")) },
        { label: "Send to back", onSelect: () => closeContextThen(() => reorderSelection("back")) },
        { label: "Add comment", icon: MessageSquarePlus, onSelect: () => closeContextThen(() => addCommentToSelected()) },
        null,
        { label: "Delete", icon: Trash2, danger: true, onSelect: () => closeContextThen(() => deleteSelection()) }
      ];
    }
    return [
      { label: "Insert rectangle", onSelect: () => closeContextThen(() => insertPrimitive("rectangle", menu.world)) },
      { label: "Insert text", icon: StickyIcon, onSelect: () => closeContextThen(() => insertPrimitive("text", menu.world)) },
      { label: "Templates", icon: LayoutTemplate, onSelect: () => closeContextThen(() => toggleTemplates()) },
      null,
      { label: "Select all", onSelect: () => closeContextThen(() => selectAll()) }
    ];
  }
  // StickyIcon alias kept local to avoid a second lucide import cluster.
  const StickyIcon = MessageSquarePlus;

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

  function shiftTransform(transform: SceneObject["transform"], dx: number, dy: number): SceneObject["transform"] {
    const [ox, oy] = transformOrigin(transform);
    if (!transform) return translateTransform(dx, dy);
    return [
      [transform[0][0], transform[0][1], ox + dx],
      [transform[1][0], transform[1][1], oy + dy],
      [transform[2][0], transform[2][1], transform[2][2]]
    ];
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
      <div class="flow-wrap renderer-scene-surface" data-tool={activeTool} bind:this={canvasWrap} role="application" aria-label="Canvas" onpointermove={handlePointerMove}>
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
