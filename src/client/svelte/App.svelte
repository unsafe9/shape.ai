<script lang="ts">
  import { onDestroy } from "svelte";
  import {
    Clipboard,
    Copy,
    Layers as LayersIcon,
    MessageSquarePlus,
    Pencil,
    Tag as TagIcon,
    Trash2,
    Ungroup,
    Activity,
    BrainCircuit,
    Loader2,
    Ellipsis,
    PanelLeft,
    X
  } from "lucide-svelte";
  import { createComment, createGroup, exportGroup, updateComment } from "../lib/sceneServerApi";
  import { boundsIntersect, expandedBounds, nodeBounds } from "../../shared/graph";
  import type { RenderScenePatch } from "../../shared/renderPatch";
  import { applyRenderPatchSync, loadSceneCore, type RenderResult, type SceneCore } from "../scene/sceneCoreWasm";
  import type { CameraState, RenderCard, RenderGroup, WorldRect } from "../../shared/renderScene";
  import { screenToWorld } from "../renderer/scene";
  import { primarySelection } from "../../shared/schema";
  import type {
    EdgeType,
    ExportType,
    GraphComment,
    GraphNode,
    NodeType,
    Scene,
    SceneEdge,
    SceneGroup,
    SceneNode,
    SceneSelection,
    ScenePatch,
    Tag
  } from "../../shared/schema";
  import { cloneNodeForPaste, formatNodeMarkdown } from "../lib/nodeClipboard";
  import { ShapeCanvasHost, type RendererHealth, type RendererStats, type ShapeCanvasHostCallbacks } from "../lib/canvasHost";
  import { SceneClient, type CanvasSummary } from "../lib/sceneClient";
  import type { PeerPresence } from "../lib/peers";
  import type { ConnectionStatus } from "../lib/wsTransport";
  import type { ActiveTool } from "../renderer/engine";
  import { applyTemplate, type TemplateContract } from "../../shared/templates/contract";
  import { createShortcutDispatcher } from "../lib/shortcuts";
  import { primitiveForCommand, type PrimitiveKindId } from "../lib/cockpitCommands";
  import {
    createTemplate as apiCreateTemplate,
    deleteTemplate as apiDeleteTemplate,
    listTemplates as apiListTemplates,
    recipeFromSelection
  } from "../lib/templatesApi";
  import Sidebar from "./Sidebar.svelte";
  import CanvasSwitcher from "./CanvasSwitcher.svelte";
  import CockpitRemote from "./CockpitRemote.svelte";
  import TemplateLibrary from "./TemplateLibrary.svelte";
  import SettingsModal from "./SettingsModal.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";
  import CanvasEditingToolbar from "./CanvasEditingToolbar.svelte";
  import SelectedNodeInspector from "./node/SelectedNodeInspector.svelte";
  import NodeContextMenu from "./NodeContextMenu.svelte";
  import ContextMenu, { type ContextMenuItem } from "./ContextMenu.svelte";
  import PeerCursors from "./PeerCursors.svelte";
  import RendererDiagnosticsDrawer from "./RendererDiagnosticsDrawer.svelte";
  import ExportDrawer, { type ExportPreview } from "./ExportDrawer.svelte";

  const cardWidth = 270;
  const cardHeight = 178;
  const selectedCardWidth = 390;
  const selectedCardHeight = 390;
  const tagColors = ["#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#6f7a86"];
  const seedPrompt =
    "Draft an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

  type NodeMenuState = {
    nodeId: string;
    x: number;
    y: number;
  };

  // CC4.2 right-click context menu over any target kind (node/edge/group/canvas).
  type ContextMenuState = {
    selection: SceneSelection;
    x: number;
    y: number;
    // World point of the right-click, used for "paste here" / "insert here".
    world: { x: number; y: number };
  };

  // ----- document projection (read of server scene) -----
  let scene = $state<Scene | null>(null);

  // ----- ephemeral selection / viewport / chrome -----
  let selection = $state<SceneSelection>({ kind: "canvas" });
  // T2.2 transient multi-select: shell-only ephemeral set of node ids.
  let multiSelectIds = $state<string[]>([]);
  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.28 });
  let activeTagIds = $state<string[]>([]);
  let currentGroupId = $state<string | undefined>(undefined);

  let prompt = $state(seedPrompt);
  let tagName = $state("");
  let status = $state("Ready");
  let busy = $state(false);
  let groupPanelOpen = $state(false);
  let diagnosticsOpen = $state(false);

  // ----- node editing slice -----
  let editingNodeId = $state<string | null>(null);
  let nodeMenu = $state<NodeMenuState | null>(null);
  let contextMenu = $state<ContextMenuState | null>(null);
  let copiedNode = $state<SceneNode | null>(null);
  let commentValue = $state("");

  // ----- cockpit slice (CC1.4/CC2.4/CC3.3/CC5.1) -----
  // active tool is ephemeral shell state (CC0.1) — never persisted.
  let activeTool = $state<ActiveTool>("select");
  // Space-hold temporarily activates Hand; the prior tool is restored on release.
  let spaceToolBeforeHold: ActiveTool | null = null;
  let templateLibraryOpen = $state(false);
  let settingsOpen = $state(false);
  let userTemplates = $state<TemplateContract[]>([]);

  // ----- export drawer slice -----
  let exportPreview = $state<ExportPreview | null>(null);
  let exportPreviewCopied = $state(false);

  // ----- realtime peers (MG6.2): live peer cursors over the canvas -----
  let peers = $state<PeerPresence[]>([]);
  // Throttle gate for outbound cursor presence frames (MG6.2).
  let lastCursorSentAt = 0;

  // ----- ephemeral renderer readout -----
  let rendererStats = $state<RendererStats | null>(null);
  let rendererHealth = $state<RendererHealth | null>(null);
  let rendererStatus = $state("No renderer status yet");

  // ----- non-reactive refs (last-write-wins guards + gesture gate) -----
  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;
  let sceneRequest = 0;
  let gestureActive = false;
  let selectionRef: SceneSelection = { kind: "canvas" };

  // MG-7: transport-backed scene data layer is the SINGLE data path. Scene LOAD,
  // renderer-op SAVE, shell CRUD, tags, and selection all flow through this WS
  // client; the tag filter is applied client-side by the renderer over the full
  // WS-held scene. Only the genuinely server-computed operations that have no
  // scene-core op (group seed, export, comments) still ride HTTP via
  // `sceneServerApi.ts` — see that module for the rationale.
  let canvasId = $state("default");
  let sceneClient: SceneClient | null = null;
  let sceneClientReady = false;
  // MG-7a: scene-core compiled to wasm is the single op-apply implementation. The
  // shell routes its optimistic document apply (and template apply / recipe /
  // insert-primitive where practical) through this handle once loaded; until the
  // async init resolves the shell falls back to the golden-equivalent TS apply so
  // the first interactions stay correct and the build/tests stay green.
  let sceneCore: SceneCore | null = null;
  // MG9.2 multi-canvas + MG8.4 connectivity: the switcher chrome reads these.
  let canvases = $state<CanvasSummary[]>([]);
  let connectionStatus = $state<ConnectionStatus>("offline");
  let canvasBusy = $state(false);

  const activeGroupId = $derived(activeGroupIdForSelection(scene, selection) ?? currentGroupId ?? scene?.groups[0]?.id);
  const activeGroup = $derived(scene?.groups.find((group) => group.id === activeGroupId) ?? null);
  const selectedNode = $derived(selection.kind === "node" ? scene?.nodes.find((node) => node.id === selection.id) ?? null : null);
  const nodeMenuNode = $derived(nodeMenu ? scene?.nodes.find((node) => node.id === nodeMenu.nodeId) ?? null : null);
  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(Boolean(scene?.groups.length));
  const artifacts = $derived(scene?.artifacts.filter((artifact) => artifact.target.kind === "group" && artifact.target.id === activeGroupId) ?? []);
  const selectedTargetLabel = $derived(
    selectedNode
      ? `node:${selectedNode.id} - ${selectedNode.title}`
      : selection.kind === "canvas"
        ? "canvas"
        : selection.kind === "multi"
          ? `multi:${selection.ids.length} objects`
          : `${selection.kind}:${selection.id}`
  );
  // CC3.3: "save selection as template" only makes sense for object selections
  // (single node, multi-select, or a group), not canvas/edge.
  const canSaveSelection = $derived(selection.kind === "node" || selection.kind === "multi" || selection.kind === "group");

  // Renderer ops coalesce on the WS engine's window; a discrete op flushes
  // immediately so it is not held behind the coalesce timer. Continuous gestures
  // (move-group / move-card) ride the coalesce window and flush at gesture end.
  function isContinuousRendererPatch(patch: RenderScenePatch): boolean {
    return patch.kind === "move-group" || patch.kind === "move-card";
  }

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onSelectionChange: handleRendererSelection,
    onPatch: handleRendererPatch,
    onGestureChange: handleRendererGesture,
    onStats: (stats) => (rendererStats = stats),
    onStatus: handleRendererStatus,
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyRendererStatus(status)) status = "Ready";
    },
    onMarquee: handleMarquee,
    onContextPick: handleContextPick
  };

  // MG-7: initial LOAD through the transport client (the single data path). We open
  // the WS session and adopt its welcome snapshot as the initial scene; renderer-op
  // saves and shell CRUD then flow through the client.
  // The renderer wasm boots inside ShapeCanvasHost.mount; here we boot the
  // scene-core wasm op-apply alongside the transport so both halves of the stack
  // are live by the time the user interacts. Both inits are async + best-effort:
  // the shell stays usable (synchronous wasm apply) until they resolve.
  void bootstrapSceneCore();
  void connectSceneClient();

  // Lazy-init the scene-core wasm bridge. Idempotent in the loader; a load failure
  // leaves the async handle null so applyOptimistic falls back to the synchronous
  // wasm apply (applyRenderPatchSync) against the same shared, already-init'd
  // instance. There is no longer a TS op-apply fallback.
  async function bootstrapSceneCore(): Promise<void> {
    try {
      sceneCore = await loadSceneCore();
    } catch {
      sceneCore = null;
    }
  }

  // ONE op-apply implementation: the scene-core wasm bridge (the same Rust the
  // server runs). The async handle is preferred once resolved; otherwise the
  // synchronous wasm apply runs against the shared, already-initialized instance
  // (SceneClient.connect awaits ensureSceneCore before any save is possible).
  // Returns {scene, errors} — the wasm shape; the WS save path sends the raw op
  // over the wire and the server re-applies authoritatively.
  function applyOptimistic(currentScene: Scene, patch: RenderScenePatch, now: string): RenderResult {
    if (sceneCore) return sceneCore.applyRenderPatch(currentScene, patch, now);
    return applyRenderPatchSync(currentScene, patch, now);
  }

  // MG-7: persist a shell CRUD document patch through the WS transport client (the
  // single data path). The patch is decomposed to scene-core ops + durable outbox
  // + coalesced send; the shell already applied the change optimistically and the
  // client's onScene callback reconciles the acked/server scene. A selection field
  // rides presence (no revision bump). A write before the client is connected is
  // dropped on the wire but kept optimistically; the WS welcome reconciles state.
  function persistScenePatch(patch: ScenePatch): void {
    if (!sceneClientReady || !sceneClient) return;
    void sceneClient.applyScenePatch(patch).then((result) => {
      if (result.errors.length > 0) status = result.errors.join("; ");
    });
    sceneClient.flush();
  }

  // MG-7: persist a selection-only change. Selection is presence, not a document
  // op (it never bumps the revision), so the WS client broadcasts it on the
  // presence channel.
  function persistSelection(nextSelection: SceneSelection): void {
    if (!sceneClientReady || !sceneClient) return;
    sceneClient.saveSelection(nextSelection);
  }

  // MG6.2 peer cursor poll: re-read the peer cursor set so a peer that has gone
  // fully silent (no new presence frames to drive ingest-time expiry) drops
  // within one poll interval — peerCursors expires stale peers lazily on read.
  $effect(() => {
    let cancelled = false;
    function poll() {
      if (!cancelled && sceneClientReady && sceneClient) peers = sceneClient.peerCursors;
    }
    poll();
    const id = window.setInterval(poll, 4_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  });

  // Push the document scene into the engine whenever it or the tag filter
  // changes. Selection is read non-reactively (selectionRef) so a selection
  // change alone does NOT trigger a scene reload — mirroring App.tsx.
  $effect(() => {
    const currentScene = scene;
    const tagIds = activeTagIds;
    if (!host || !currentScene) return;
    host.loadScene(currentScene, tagIds, selectionRef);
  });

  // Keep selectionRef in sync with the reactive selection and push selection to
  // the engine. The Rust core understands only single-anchor forms, so a
  // transient `multi` selection is down-projected to its primary node first.
  $effect(() => {
    selectionRef = selection;
    host?.syncSelection(primarySelection(selection));
  });

  // CC1.4: push the active tool to the renderer whenever it changes (and once the
  // host is ready). The CSS cursor is bound on the canvas wrap below.
  $effect(() => {
    const tool = activeTool;
    host?.setTool(tool);
  });

  // Push the transient multi-select set to the renderer so every marquee /
  // shift-click member is highlighted, not just the single anchor. This is
  // additive over the single-anchor `syncSelection` above: the core stores the
  // set separately from the persisted selection, so a `select` op no longer wipes
  // the multi highlight. An empty set clears it.
  $effect(() => {
    const ids = multiSelectIds;
    host?.setMultiSelect(ids);
  });

  // MG9.4 windowed replica: re-aim the data-layer subscription window at the
  // current camera viewport (world space) whenever the camera moves. The client
  // debounces + margin-grows it, so a small pan keeps nearby off-screen objects
  // loaded. This is the DATA window (which objects the client holds), distinct
  // from the renderer's own culling (which held objects it draws each frame).
  $effect(() => {
    const cam = camera;
    if (!sceneClientReady || !sceneClient) return;
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || rect.width === 0 || rect.height === 0) return;
    const topLeft = screenToWorld({ x: 0, y: 0 }, cam);
    const bottomRight = screenToWorld({ x: rect.width, y: rect.height }, cam);
    sceneClient.setViewport({
      x: topLeft.x,
      y: topLeft.y,
      width: bottomRight.x - topLeft.x,
      height: bottomRight.y - topLeft.y
    });
  });

  // CC0.4/CC6.1 central shortcut dispatch. The catalog-driven dispatcher handles
  // tools, shapes, zoom, edit, selection, template, settings; Escape and Space
  // keep bespoke handling (priority Esc handoff + Space-hold pan) that does not
  // map onto a single command.
  const dispatchShortcut = createShortcutDispatcher({ handlers: shortcutHandlers() });

  $effect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      const typing = target ? ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName) || target.isContentEditable : false;

      if (event.key === "Escape") {
        event.preventDefault();
        handleEscape();
        return;
      }

      // CC2.4 Space-hold temporarily activates Hand (pan); restore on keyup.
      if (event.code === "Space" && !typing && !event.repeat) {
        event.preventDefault();
        if (spaceToolBeforeHold === null) {
          spaceToolBeforeHold = activeTool;
          setActiveTool("hand");
        }
        return;
      }

      // Everything else routes through the catalog dispatcher (focus-aware).
      dispatchShortcut(event);
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

  // MG-7: open the transport session and adopt its welcome snapshot as the initial
  // scene. The WS client is the single data path; a connect failure leaves the
  // shell empty with a status (the same Rust process serves /ws and /api/*, so a
  // dead socket means a dead server — there is no separate HTTP load to fall to).
  async function connectSceneClient(): Promise<void> {
    const client = new SceneClient({ url: wsBaseUrl(), clientId: clientIdentity(), userId: userIdentity() });
    try {
      const welcome = await client.connect(canvasId);
      sceneClient = client;
      sceneClientReady = true;
      connectionStatus = client.connectionStatus;
      // Keep the shell's reactive scene in lockstep with the engine's optimistic
      // scene so acked/remote changes (and renderer-op authoring) are reflected.
      client.onScene((next) => commitClientScene(next));
      // MG8.4: surface online/offline so the switcher chrome shows connectivity.
      client.onStatus((next) => (connectionStatus = next));
      // MG6.2: surface live peer cursors for the overlay (latest-wins per userId,
      // stale peers expired by the client's sweep).
      client.onPeers((next) => (peers = next));
      const requestId = ++sceneRequest;
      if (requestId !== sceneRequest) return;
      scene = welcome;
      selection = validSelection(welcome, welcome.selection);
      void loadCanvases();
    } catch (error) {
      client.close();
      status = error instanceof Error ? error.message : "Scene load failed";
    }
  }

  // MG9.2: refresh the canvas list for the switcher chrome (best-effort).
  async function loadCanvases(): Promise<void> {
    if (!sceneClient) return;
    try {
      canvases = await sceneClient.listCanvases();
    } catch {
      /* best-effort; the switcher just shows the active canvas */
    }
  }

  // MG9.2: switch the active canvas. The data layer reconnects + re-subscribes
  // the current window for the new canvasId; we adopt its welcome snapshot.
  async function switchToCanvas(nextCanvasId: string): Promise<void> {
    if (!sceneClient || nextCanvasId === canvasId) return;
    canvasBusy = true;
    try {
      const welcome = await sceneClient.switchCanvas(nextCanvasId);
      canvasId = nextCanvasId;
      sceneRequest += 1;
      scene = welcome;
      selection = validSelection(welcome, welcome.selection);
      currentGroupId = undefined;
      activeTagIds = [];
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

  // Apply an engine-driven scene update without re-triggering a save. Honors the
  // existing gesture gate (no clobber while dragging) and keeps the selection /
  // multi-set / active-group invariants the renderer path relies on.
  function commitClientScene(next: Scene): void {
    if (gestureActive) return;
    const valid = validSelection(next, selection);
    sceneRequest += 1;
    selectionRef = valid;
    scene = next;
    selection = valid;
    multiSelectIds = valid.kind === "multi" ? valid.ids : [];
    const nextGroupId = activeGroupIdForSelection(next, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
  }

  // The WS base for the transport client. In dev the vite proxy forwards /api to
  // the API port; the WS server shares that host, so we derive ws(s):// from the
  // current page origin. The transport appends /ws.
  function wsBaseUrl(): string {
    const loc = window.location;
    const protocol = loc.protocol === "https:" ? "wss:" : "ws:";
    return `${protocol}//${loc.host}`;
  }

  // A stable-enough authoring identity for this tab/session. opIds are namespaced
  // by this clientId so re-sends dedup on the server.
  function clientIdentity(): string {
    return `shell-${crypto.randomUUID().slice(0, 8)}`;
  }

  // MG6.3: a stable user identity for the session, persisted in localStorage so a
  // reload keeps the same peer lane (cursor color / follow target). It is the
  // hello.userId self-skip identity and the tag on every presence frame.
  function userIdentity(): string {
    const key = "shape-ai-user-id";
    try {
      const existing = window.localStorage.getItem(key);
      if (existing) return existing;
      const fresh = `user-${crypto.randomUUID().slice(0, 8)}`;
      window.localStorage.setItem(key, fresh);
      return fresh;
    } catch {
      // Private mode / storage disabled: fall back to an ephemeral id.
      return `user-${crypto.randomUUID().slice(0, 8)}`;
    }
  }

  function handleHost(next: ShapeCanvasHost): void {
    host = next;
    if (scene) host.loadScene(scene, activeTagIds, selection);
    host.syncSelection(primarySelection(selection));
    host.setTool(activeTool);
    host.setMultiSelect(multiSelectIds);
  }

  function handleRendererSelection(next: SceneSelection, additive = false): void {
    if (!scene) return;
    // T2.2 transient multi-select: shift/meta-click on a card toggles it in/out of
    // the ephemeral set instead of replacing the selection. The set never becomes a
    // SceneGroup; it evaporates on a plain (non-additive) click or click-away.
    const resolved = additive && next.kind === "node" ? toggleMultiSelect(selection, next.id) : next;
    const valid = validSelection(scene, resolved);
    multiSelectIds = valid.kind === "multi" ? valid.ids : [];
    const nextGroupId = activeGroupIdForSelection(scene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    if (valid.kind !== "node") editingNodeId = null;
    selection = valid;
    // The canonical persisted selection stays single-anchor so the renderer/Rust
    // core can restore it; the multi-set itself is shell-only ephemeral state.
    persistSelection(primarySelection(valid));
  }

  function handleRendererPatch(patch: RenderScenePatch): void {
    if (!scene) return;
    const now = new Date().toISOString();

    // MG-7: the renderer-op SAVE flows through the WS transport client (optimistic
    // engine apply + durable outbox + coalesced send) and the optimistic document
    // apply runs through the scene-core wasm bridge — the ONE op-apply
    // implementation. The shell commits its local optimistic scene for immediate
    // feedback; the client's onScene callback keeps it in lockstep once acked.
    const applied = applyOptimistic(scene, patch, now);
    if (applied.errors.length > 0) {
      status = applied.errors.join("; ");
      return;
    }
    const optimisticSelection = validSelection(applied.scene, applied.scene.selection);
    sceneRequest += 1;
    // During a continuous gesture the renderer drives the optimistic frame; only
    // commit the scene between gestures so a drag is not clobbered mid-flight.
    if (!isContinuousRendererPatch(patch) || !gestureActive) {
      commitScene(applied.scene, optimisticSelection);
    }
    if (!sceneClientReady || !sceneClient) return;
    void sceneClient.applyRenderPatch(patch).then((result) => {
      if (result.errors.length > 0) status = result.errors.join("; ");
    });
    // Continuous gestures rely on the engine's coalescing window; a discrete op
    // flushes immediately so it isn't held behind the coalesce timer.
    if (!isContinuousRendererPatch(patch)) sceneClient.flush();
  }

  function handleRendererGesture(active: boolean): void {
    gestureActive = active;
    if (active) return;
    if (scene) commitScene(scene, selection);
    // MG-7: flush the engine's coalesced gesture buffer at gesture end.
    sceneClient?.flush();
  }

  function commitScene(nextScene: Scene, nextSelection: SceneSelection): void {
    const valid = validSelection(nextScene, nextSelection);
    scene = nextScene;
    selection = valid;
    // Keep the transient multi-set in lockstep with the committed selection so the
    // editing toolbar doesn't stay in multi-mode after a renderer-driven commit.
    multiSelectIds = valid.kind === "multi" ? valid.ids : [];
    const nextGroupId = activeGroupIdForSelection(nextScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    if (valid.kind !== "node") editingNodeId = null;
  }

  function handleRendererStatus(message: string): void {
    rendererStatus = message;
    if (isDiagnosticsOnlyRendererStatus(message)) return;
    status = message;
  }

  // CC4.2: right-click anywhere — ask the renderer to pick the target (pure hit
  // test, no selection change) then open a context-appropriate menu. The
  // onContextPick callback fires from the host and opens the menu in handleContextPick.
  function handleContextMenuRequest(point: { x: number; y: number }): void {
    if (!host || !canvasWrap) {
      contextMenu = null;
      return;
    }
    const rect = canvasWrap.getBoundingClientRect();
    pendingContextScreen = { clientX: point.x, clientY: point.y };
    host.contextPick({ x: point.x - rect.left, y: point.y - rect.top });
  }

  // The clientX/clientY of the right-click awaiting the renderer pick result.
  let pendingContextScreen: { clientX: number; clientY: number } | null = null;

  // MG6.2: broadcast the local pointer as a presence cursor frame on pointer move,
  // throttled so a fast move rides ~one frame per CURSOR_THROTTLE_MS. Coordinates
  // are WORLD space (camera-projected) plus the current viewport, so a peer with a
  // different camera frames the same canvas point and follow can re-aim to it.
  const CURSOR_THROTTLE_MS = 40;
  function handlePointerMove(event: PointerEvent): void {
    if (!sceneClientReady || !sceneClient || !canvasWrap) return;
    const now = Date.now();
    if (now - lastCursorSentAt < CURSOR_THROTTLE_MS) return;
    lastCursorSentAt = now;
    const rect = canvasWrap.getBoundingClientRect();
    const cursor = screenToWorld({ x: event.clientX - rect.left, y: event.clientY - rect.top }, camera);
    const topLeft = screenToWorld({ x: 0, y: 0 }, camera);
    const bottomRight = screenToWorld({ x: rect.width, y: rect.height }, camera);
    sceneClient.sendCursor(cursor, {
      x: topLeft.x,
      y: topLeft.y,
      width: bottomRight.x - topLeft.x,
      height: bottomRight.y - topLeft.y
    });
  }

  // CC4.1/4.2: the renderer returned a pick for the right-click. Build the menu.
  function handleContextPick(picked: SceneSelection, screen: { x: number; y: number }): void {
    const anchor = pendingContextScreen;
    pendingContextScreen = null;
    if (!anchor || !canvasWrap) return;
    editingNodeId = null;
    nodeMenu = null;
    const rect = canvasWrap.getBoundingClientRect();
    const world = screenToWorld({ x: anchor.clientX - rect.left, y: anchor.clientY - rect.top }, camera);
    contextMenu = { selection: picked, x: anchor.clientX, y: anchor.clientY, world };
  }

  // CC4.2: close the context menu on any pointer-down outside it (the menu stops
  // propagation on its own pointerdown, so this only fires for click-away).
  $effect(() => {
    if (!contextMenu) return;
    function dismiss() {
      contextMenu = null;
    }
    window.addEventListener("pointerdown", dismiss);
    return () => window.removeEventListener("pointerdown", dismiss);
  });

  function contextMenuTitle(picked: SceneSelection): string {
    if (picked.kind === "node") return scene?.nodes.find((node) => node.id === picked.id)?.title ?? "Node";
    if (picked.kind === "group") return scene?.groups.find((group) => group.id === picked.id)?.title ?? "Frame";
    if (picked.kind === "edge") return "Connector";
    return "Canvas";
  }

  // CC4.3: context-appropriate actions per target kind. node: edit/duplicate/
  // delete/comment/tag; edge: delete; group: ungroup/tag; canvas: paste/insert/
  // select-all.
  function contextMenuItems(menu: ContextMenuState): (ContextMenuItem | null)[] {
    const picked = menu.selection;
    if (picked.kind === "node") {
      const nodeId = picked.id;
      return [
        { label: "Edit", icon: Pencil, onSelect: () => closeContextThen(() => startEditingNode(nodeId)) },
        { label: "Duplicate", icon: Copy, onSelect: () => closeContextThen(() => duplicateNode(nodeId)) },
        { label: "Copy as Markdown", icon: Copy, onSelect: () => closeContextThen(() => void copyNode(nodeId)) },
        { label: "Add comment", icon: MessageSquarePlus, onSelect: () => closeContextThen(() => openCommentForNode(nodeId)) },
        { label: "Bring to front", icon: LayersIcon, onSelect: () => closeContextThen(() => moveNodeLayer(nodeId, "front")) },
        { label: "Send to back", icon: LayersIcon, onSelect: () => closeContextThen(() => moveNodeLayer(nodeId, "back")) },
        null,
        { label: "Delete", icon: Trash2, danger: true, onSelect: () => closeContextThen(() => deleteNode(nodeId)) }
      ];
    }
    if (picked.kind === "edge") {
      const edgeId = picked.id;
      return [{ label: "Delete connector", icon: Trash2, danger: true, onSelect: () => closeContextThen(() => handleRendererPatch({ kind: "delete-edge", id: edgeId })) }];
    }
    if (picked.kind === "group") {
      const groupId = picked.id;
      return [
        { label: "Ungroup", icon: Ungroup, onSelect: () => closeContextThen(() => handleRendererPatch({ kind: "ungroup", id: groupId })) },
        { label: "Tags", icon: TagIcon, onSelect: () => closeContextThen(() => selectGroupForTags(groupId)) },
        null,
        { label: "Delete frame", icon: Trash2, danger: true, onSelect: () => closeContextThen(() => handleRendererPatch({ kind: "delete-group", id: groupId })) }
      ];
    }
    // canvas
    const world = menu.world;
    return [
      { label: "Paste node", icon: Clipboard, disabled: copiedNode === null, onSelect: () => closeContextThen(() => pasteCopiedNode()) },
      { label: "Insert rectangle", icon: Pencil, onSelect: () => closeContextThen(() => insertPrimitiveAt("rectangle", world)) },
      { label: "Insert sticky", icon: Pencil, onSelect: () => closeContextThen(() => insertPrimitiveAt("sticky", world)) },
      null,
      { label: "Select all", icon: LayersIcon, onSelect: () => closeContextThen(() => selectAll()) }
    ];
  }

  function closeContextThen(action: () => void): void {
    contextMenu = null;
    action();
  }

  // Open the comment composer for a node by selecting it (the inspector shows the
  // comment field). The right-click menu reuses the existing inspector path.
  function openCommentForNode(nodeId: string): void {
    void selectSceneItem({ kind: "node", id: nodeId });
    commentValue = "";
  }

  function selectGroupForTags(groupId: string): void {
    void selectSceneItem({ kind: "group", id: groupId });
    groupPanelOpen = true;
  }

  // CC2.3: the marquee drag ended; merge the returned ids into the transient
  // multiSelectIds set, preserving the single-anchor persisted-selection invariant.
  function handleMarquee(ids: string[]): void {
    if (!scene) return;
    if (ids.length === 0) {
      // Empty marquee acts as a click-away: clear selection.
      multiSelectIds = [];
      void selectSceneItem({ kind: "canvas" });
      return;
    }
    const merged = Array.from(new Set([...multiSelectIds, ...ids]));
    const nodeIds = merged.filter((id) => scene!.nodes.some((node) => node.id === id));
    if (nodeIds.length >= 2) {
      const next: SceneSelection = { kind: "multi", ids: nodeIds };
      multiSelectIds = nodeIds;
      handleRendererSelection(next);
      return;
    }
    if (nodeIds.length === 1) {
      handleRendererSelection({ kind: "node", id: nodeIds[0] });
      return;
    }
    // Only group ids in the marquee — select the first group (single anchor).
    const groupId = merged.find((id) => scene!.groups.some((group) => group.id === id));
    if (groupId) handleRendererSelection({ kind: "group", id: groupId });
  }

  // CC1.4: switch the active tool (ephemeral). The renderer is updated via the
  // activeTool $effect; the canvas cursor follows toolCursor below.
  function setActiveTool(tool: ActiveTool): void {
    activeTool = tool;
  }

  // Esc priority handoff: settings → template library → diagnostics → context
  // menu → node menu → editing → clear selection.
  function handleEscape(): void {
    if (settingsOpen) {
      settingsOpen = false;
      return;
    }
    if (templateLibraryOpen) {
      templateLibraryOpen = false;
      return;
    }
    if (diagnosticsOpen) {
      diagnosticsOpen = false;
      return;
    }
    if (contextMenu) {
      contextMenu = null;
      return;
    }
    if (nodeMenu) {
      nodeMenu = null;
      return;
    }
    if (editingNodeId) {
      editingNodeId = null;
      return;
    }
    multiSelectIds = [];
    void selectSceneItem({ kind: "canvas" });
  }

  // CC0.4/CC6.1: command-id → handler map for the catalog dispatcher. Handlers
  // reuse the existing shell paths (insertPrimitive, zoom, editing-toolbar ops,
  // copy/paste, selection) so a key and a remote/menu click share one path.
  function shortcutHandlers() {
    return {
      "select-move": () => setActiveTool("select"),
      "hand-pan": () => setActiveTool("hand"),
      "insert-rectangle": () => insertPrimitive("rectangle"),
      "insert-ellipse": () => insertPrimitive("ellipse"),
      "insert-connector": () => insertPrimitive("connector"),
      "insert-sticky": () => insertPrimitive("sticky"),
      "insert-frame": () => insertPrimitive("frame"),
      "zoom-in": () => zoomAtCenter(-160),
      "zoom-out": () => zoomAtCenter(160),
      "zoom-fit": () => fitScene(),
      "toggle-fullscreen": () => void toggleFullscreen(),
      delete: () => deleteSelection(),
      duplicate: () => duplicateSelection(),
      copy: () => {
        if (selection.kind === "node") void copyNode(selection.id);
      },
      paste: () => pasteCopiedNode(selection.kind === "node" ? selection.id : undefined),
      group: () => groupSelection(),
      ungroup: () => ungroupSelection(),
      "bring-to-front": () => moveSelectionLayer("front"),
      "send-to-back": () => moveSelectionLayer("back"),
      "select-all": () => selectAll(),
      "open-template-library": () => (templateLibraryOpen = !templateLibraryOpen),
      "new-canvas": () => {
        if (sceneClient) void createCanvas("New canvas");
      },
      "open-settings": () => (settingsOpen = !settingsOpen)
    };
  }

  // ----- shortcut-driven selection ops (CC6.1) -----------------------------

  function currentSelectionIds(): string[] {
    if (selection.kind === "multi") return selection.ids;
    if (selection.kind === "node" || selection.kind === "group" || selection.kind === "edge") return [selection.id];
    return [];
  }

  function deleteSelection(): void {
    if (!scene) return;
    const ids = currentSelectionIds();
    if (ids.length === 0) return;
    if (ids.length === 1) {
      const id = ids[0];
      if (scene.nodes.some((node) => node.id === id)) handleRendererPatch({ kind: "delete-card", id });
      else if (scene.groups.some((group) => group.id === id)) handleRendererPatch({ kind: "delete-group", id });
      else if (scene.edges.some((edge) => edge.id === id)) handleRendererPatch({ kind: "delete-edge", id });
      return;
    }
    const ops: RenderScenePatch[] = [];
    for (const id of ids) {
      if (scene.nodes.some((node) => node.id === id)) ops.push({ kind: "delete-card", id });
      else if (scene.groups.some((group) => group.id === id)) ops.push({ kind: "delete-group", id });
      else if (scene.edges.some((edge) => edge.id === id)) ops.push({ kind: "delete-edge", id });
    }
    if (ops.length > 0) handleRendererPatch({ kind: "batch", ops });
  }

  function duplicateSelection(): void {
    if (!scene) return;
    const nodeIds = currentSelectionIds().filter((id) => scene!.nodes.some((node) => node.id === id));
    if (nodeIds.length === 0) return;
    handleRendererPatch({ kind: "duplicate-objects", ids: nodeIds, delta: { x: 40, y: 40 } });
  }

  function groupSelection(): void {
    if (!scene) return;
    const ids = currentSelectionIds().filter((id) => scene!.nodes.some((node) => node.id === id) || scene!.groups.some((group) => group.id === id));
    if (ids.length < 2) return;
    const frameId = `frame-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 4)}`;
    handleRendererPatch({ kind: "group-objects", ids, frameId });
  }

  function ungroupSelection(): void {
    if (selection.kind !== "group") return;
    handleRendererPatch({ kind: "ungroup", id: selection.id });
  }

  function moveSelectionLayer(direction: "front" | "back"): void {
    const ids = currentSelectionIds().filter((id) => scene?.nodes.some((node) => node.id === id));
    for (const id of ids) moveNodeLayer(id, direction);
  }

  function selectAll(): void {
    if (!scene) return;
    const nodeIds = scene.nodes.map((node) => node.id);
    if (nodeIds.length === 0) return;
    if (nodeIds.length === 1) {
      handleRendererSelection({ kind: "node", id: nodeIds[0] });
      return;
    }
    multiSelectIds = nodeIds;
    handleRendererSelection({ kind: "multi", ids: nodeIds });
  }

  function openSelectedNodeMenu(event: MouseEvent & { currentTarget: HTMLButtonElement }): void {
    if (!selectedNode) return;
    const rect = event.currentTarget.getBoundingClientRect();
    editingNodeId = null;
    nodeMenu = { nodeId: selectedNode.id, x: rect.right - 220, y: rect.bottom + 8 };
  }

  // ----- node CRUD ---------------------------------------------------------

  function updateNode(node: GraphNode): void {
    if (!scene) return;
    const current = scene.nodes.find((candidate) => candidate.id === node.id);
    if (!current) return;
    const nextNode = { ...current, ...node };
    scene = { ...scene, nodes: scene.nodes.map((candidate) => (candidate.id === node.id ? nextNode : candidate)) };
    persistScenePatch({ nodes: [nextNode] });
  }

  function startEditingNode(nodeId: string): void {
    editingNodeId = nodeId;
    void selectSceneItem({ kind: "node", id: nodeId });
  }

  function deleteNode(nodeId: string): void {
    if (!scene) return;
    scene = {
      ...scene,
      nodes: scene.nodes.filter((node) => node.id !== nodeId),
      edges: scene.edges.filter((edge) => edge.source !== nodeId && edge.target !== nodeId),
      selection: { kind: "canvas" }
    };
    // Reset the reactive shell selection too — otherwise it keeps pointing at the
    // deleted node (stale renderer anchor, extra Esc press, stale auto-save).
    selection = { kind: "canvas" };
    editingNodeId = null;
    multiSelectIds = [];
    persistScenePatch({ removeNodeIds: [nodeId], selection: { kind: "canvas" } });
  }

  function addLinkedNode(type: NodeType): void {
    if (!scene) return;
    const groupId = activeGroupId ?? scene.groups[0]?.id;
    if (!groupId) return;
    const sourceId = selection.kind === "node" ? selection.id : scene.nodes.find((node) => node.groupId === groupId)?.id;
    const id = `${groupId}-${type.replace(/_/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
    const basePosition = sourceId ? nodePosition(sourceId) : activeGroup?.bounds ?? { x: 0, y: 0 };
    const node: SceneNode = {
      id,
      groupId,
      type,
      title: newNodeTitle(type),
      summary: "New item. Edit this short summary.",
      detail: "New item. Add the supporting detail here.",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: [],
      tagIds: [],
      position: openNodePosition(groupId, { x: basePosition.x + 450, y: basePosition.y + 430 }, { width: cardWidth, height: cardHeight }),
      size: { width: cardWidth, height: cardHeight },
      zIndex: nextTopZ(scene.nodes.filter((candidate) => candidate.groupId === groupId)),
      updatedAt: new Date().toISOString()
    };
    const edge: SceneEdge | null = sourceId
      ? {
          id: `${groupId}-edge-${crypto.randomUUID().slice(0, 8)}`,
          groupId,
          type: defaultEdgeType(type),
          source: sourceId,
          target: id,
          label: defaultEdgeLabel(type),
          rationale: "",
          confidence: 0.5,
          tagIds: [],
          updatedAt: new Date().toISOString()
        }
      : null;
    const nextScene = { ...scene, nodes: [...scene.nodes, node], edges: edge ? [...scene.edges, edge] : scene.edges };
    scene = nextScene;
    persistScenePatch({ nodes: [node], edges: edge ? [edge] : [], selection: { kind: "node", id } });
    void selectSceneItem({ kind: "node", id }, nextScene);
  }

  async function copyNode(nodeId: string): Promise<void> {
    const node = scene?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    copiedNode = node;
    if (await writeClipboardText(formatNodeMarkdown(node))) {
      status = "Copied node as Markdown";
      return;
    }
    status = "Copied node locally";
  }

  function duplicateNode(nodeId: string): void {
    const node = scene?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    pasteNode(node, nodeId);
  }

  function pasteCopiedNode(anchorNodeId?: string): void {
    if (!copiedNode) return;
    pasteNode(copiedNode, anchorNodeId ?? copiedNode.id);
  }

  function pasteNode(sourceNode: SceneNode, anchorNodeId: string): void {
    if (!scene) return;
    const id = `${sourceNode.groupId}-${sourceNode.type.replace(/_/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
    const cloned = cloneNodeForPaste(sourceNode, id);
    const anchorPosition = nodePosition(anchorNodeId);
    const node: SceneNode = {
      ...sourceNode,
      ...cloned,
      id,
      groupId: sourceNode.groupId,
      position: openNodePosition(sourceNode.groupId, { x: anchorPosition.x + 450, y: anchorPosition.y + 430 }, sourceNode.size),
      size: sourceNode.size,
      zIndex: nextTopZ(scene.nodes.filter((candidate) => candidate.groupId === sourceNode.groupId)),
      updatedAt: new Date().toISOString()
    };
    scene = { ...scene, nodes: [...scene.nodes, node] };
    persistScenePatch({ nodes: [node], selection: { kind: "node", id } });
    status = "Pasted copied node";
  }

  function moveNodeLayer(nodeId: string, direction: "front" | "back"): void {
    if (!scene) return;
    const node = scene.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    const groupNodes = scene.nodes.filter((candidate) => candidate.groupId === node.groupId);
    const values = groupNodes.map((candidate) => candidate.zIndex);
    const zIndex = direction === "front" ? Math.max(...values, 0) + 1 : Math.min(...values, 0) - 1;
    const updated = { ...node, zIndex, updatedAt: new Date().toISOString() };
    scene = { ...scene, nodes: scene.nodes.map((candidate) => (candidate.id === nodeId ? updated : candidate)) };
    persistScenePatch({ nodes: [updated] });
    status = direction === "front" ? "Brought node to front" : "Sent node to back";
  }

  function nodePosition(nodeId: string): { x: number; y: number } {
    return scene?.nodes.find((node) => node.id === nodeId)?.position ?? { x: 120, y: 120 };
  }

  function openNodePosition(groupId: string, preferred: { x: number; y: number }, size: { width: number; height: number }): { x: number; y: number } {
    if (!scene) return preferred;
    const occupied = scene.nodes.filter((node) => node.groupId === groupId).map(nodeBounds);
    const candidateFree = (position: { x: number; y: number }) => {
      const candidate = expandedBounds({ x: position.x, y: position.y, width: size.width, height: size.height }, 54);
      return occupied.every((bounds) => !boundsIntersect(candidate, bounds));
    };
    if (candidateFree(preferred)) return preferred;

    const stepX = 450;
    const stepY = 430;
    for (let radius = 1; radius <= 8; radius += 1) {
      for (let x = -radius; x <= radius; x += 1) {
        for (let y = -radius; y <= radius; y += 1) {
          if (Math.abs(x) !== radius && Math.abs(y) !== radius) continue;
          const candidate = { x: preferred.x + x * stepX, y: preferred.y + y * stepY };
          if (candidateFree(candidate)) return candidate;
        }
      }
    }
    return preferred;
  }

  // ----- export ------------------------------------------------------------

  async function runExport(type: ExportType): Promise<void> {
    if (!activeGroupId) return;
    const groupId = activeGroupId;
    await withBusy(`Exporting ${type}`, async () => {
      const response = await exportGroup(groupId, type, { kind: "group", id: groupId });
      scene = response.scene;
      exportPreview = response.preview;
      exportPreviewCopied = false;
      status = `Export created: ${response.artifact.title}`;
    });
  }

  async function copyExportPreview(): Promise<void> {
    if (!exportPreview) return;
    if (await writeClipboardText(exportPreview.content)) {
      exportPreviewCopied = true;
      status = "Export preview copied";
      window.setTimeout(() => (exportPreviewCopied = false), 1400);
      return;
    }
    if (selectExportPreviewText()) {
      status = "Export preview selected";
      return;
    }
    status = "Copy failed";
  }

  function selectExportPreviewText(): boolean {
    const preview = document.querySelector(".export-preview-body");
    if (!preview) return false;
    const sel = window.getSelection();
    if (!sel) return false;
    const range = document.createRange();
    range.selectNodeContents(preview);
    sel.removeAllRanges();
    sel.addRange(range);
    return true;
  }

  // ----- comments ----------------------------------------------------------

  async function runAddComment(): Promise<void> {
    if (!commentValue.trim()) return;
    await withBusy("Adding comment", async () => {
      const response = await createComment({ target: selection, body: commentValue.trim() });
      scene = response.scene;
      commentValue = "";
    });
  }

  async function toggleComment(comment: GraphComment): Promise<void> {
    await withBusy("Updating comment", async () => {
      const response = await updateComment(comment.id, { resolved: !comment.resolved });
      scene = response.scene;
    });
  }

  // ----- focus / follow ----------------------------------------------------

  // ----- selection / scene helpers ----------------------------------------

  async function selectSceneItem(nextSelection: SceneSelection, sourceScene: Scene | null = scene): Promise<void> {
    if (!sourceScene) return;
    const valid = validSelection(sourceScene, nextSelection);
    const nextGroupId = activeGroupIdForSelection(sourceScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    if (valid.kind !== "node") editingNodeId = null;
    if (valid.kind === "canvas") multiSelectIds = [];
    selection = valid;
    persistSelection(valid);
  }

  async function runCreateGroup(): Promise<void> {
    await withBusy("Creating group", async () => {
      sceneRequest += 1;
      const response = await createGroup(prompt, undefined, activeTagIds);
      scene = response.scene;
      groupPanelOpen = false;
      exportPreview = null;
      exportPreviewCopied = false;
      status = response.message;
      currentGroupId = response.group.id;
      const firstNode = response.scene.nodes.find((node) => node.groupId === response.group.id);
      if (firstNode) focusNode(firstNode, 0.92);
      else {
        const group = response.scene.groups.find((candidate) => candidate.id === response.group.id);
        if (group) focusGroup(group, { zoom: 0.72 });
      }
      await selectSceneItem({ kind: "group", id: response.group.id }, response.scene);
    });
  }

  // ----- template library (CC3.3) -----------------------------------------

  // Refresh the registered template list (builtins seeded on the server + user
  // templates). Best-effort; the library just shows what it can load.
  async function loadTemplates(): Promise<void> {
    try {
      userTemplates = await apiListTemplates();
    } catch (error) {
      status = error instanceof Error ? error.message : "Template load failed";
    }
  }

  function toggleTemplateLibrary(): void {
    templateLibraryOpen = !templateLibraryOpen;
    if (templateLibraryOpen) void loadTemplates();
  }

  // Lower a registered TemplateContract to canvas objects in an open area and
  // persist via the scene-patch path (mirrors applyTemplate semantics). MG-7a:
  // the lowered groups/nodes/edges are applied optimistically to the local scene
  // and persisted through persistScenePatch (WS ops when connected, HTTP fallback
  // otherwise); the focus/select then read the local optimistic scene rather than
  // a server response. The template lowering itself stays on the TS applyTemplate
  // until the wasm template contract types are mirrored (see notes_for_next).
  async function applyTemplateContract(templateId: string): Promise<void> {
    if (!scene) return;
    const contract = userTemplates.find((candidate) => candidate.metadata.id === templateId);
    if (!contract) return;
    await withBusy("Inserting template", async () => {
      const anchor = templateAnchor();
      const idPrefix = `tpl-${crypto.randomUUID().slice(0, 8)}`;
      const applied = applyTemplate(contract, anchor, idPrefix, new Date().toISOString());
      if (applied.errors.length > 0) {
        status = applied.errors.join("; ");
        return;
      }
      sceneRequest += 1;
      const selectionForGroup: SceneSelection | undefined = applied.group ? { kind: "group", id: applied.group.id } : undefined;
      const nextScene: Scene = {
        ...scene!,
        groups: [...scene!.groups, ...applied.groups],
        nodes: [...scene!.nodes, ...applied.nodes],
        edges: [...scene!.edges, ...applied.edges]
      };
      scene = nextScene;
      persistScenePatch({
        groups: applied.groups,
        nodes: applied.nodes,
        edges: applied.edges,
        selection: selectionForGroup
      });
      exportPreview = null;
      exportPreviewCopied = false;
      if (applied.group) {
        currentGroupId = applied.group.id;
        focusGroup(applied.group, { fit: true });
        await selectSceneItem({ kind: "group", id: applied.group.id }, nextScene);
      }
      status = `Inserted ${contract.metadata.title}`;
    });
  }

  // CC3.1/CC3.3: build a TemplateContract from the current selection (single
  // object / multi-select / single group) and POST it to the server.
  async function saveSelectionAsTemplate(title: string): Promise<void> {
    if (!scene) return;
    const metadata = {
      id: `user-${crypto.randomUUID().slice(0, 8)}`,
      title,
      description: `Saved from selection (${title})`,
      category: "general" as const,
      templateKind: "user"
    };
    const contract = recipeFromSelection(scene, selection, metadata);
    if (!contract) {
      status = "Select objects to save as a template";
      return;
    }
    await withBusy("Saving template", async () => {
      await apiCreateTemplate(contract);
      await loadTemplates();
      status = `Saved template ${title}`;
    });
  }

  async function deleteTemplateById(templateId: string): Promise<void> {
    await withBusy("Deleting template", async () => {
      await apiDeleteTemplate(templateId);
      await loadTemplates();
      status = "Template deleted";
    });
  }

  // Place an applied template to the right of existing content so it lands in view.
  function templateAnchor(): { x: number; y: number } {
    if (!scene || scene.groups.length === 0) return viewportCenterWorld();
    let maxRight = -Infinity;
    let top = Infinity;
    for (const group of scene.groups) {
      maxRight = Math.max(maxRight, group.bounds.x + group.bounds.width);
      top = Math.min(top, group.bounds.y);
    }
    return { x: maxRight + 240, y: Number.isFinite(top) ? top : 120 };
  }

  // ----- primitive palette --------------------------------------------------

  // World-space point at the centre of the current viewport, used to drop new
  // primitives into the open area the user is looking at. Reuses the shared
  // screenToWorld transform so no camera math is duplicated here.
  function viewportCenterWorld(): { x: number; y: number } {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect) return { x: 120, y: 120 };
    return screenToWorld({ x: rect.width / 2, y: rect.height / 2 }, camera);
  }

  // Insert a basic primitive (rectangle/ellipse/connector/sticky/frame) onto the
  // canvas near the viewport. Everything flows through the existing renderPatch
  // create-card/create-group/create-edge ops + handleRendererPatch, which persists
  // discrete ops through the WS transport client (immediate flush).
  // CC4.3: drop a primitive at an explicit world point (used by the canvas
  // right-click menu "insert here"). Delegates to insertPrimitive.
  function insertPrimitiveAt(kind: PrimitiveKindId, world: { x: number; y: number }): void {
    insertPrimitive(kind, world);
  }

  function insertPrimitive(kind: PrimitiveKindId, anchor?: { x: number; y: number }): void {
    if (!scene) return;
    const center = anchor ?? viewportCenterWorld();

    if (kind === "frame") {
      const frame: RenderGroup = {
        id: `palette-frame-${crypto.randomUUID().slice(0, 8)}`,
        title: "Frame",
        summary: "",
        bounds: { x: center.x - 320, y: center.y - 220, width: 640, height: 440 },
        tagIds: [],
        zIndex: 0,
        styleKey: "default"
      };
      handleRendererPatch({ kind: "create-group", group: frame });
      status = "Inserted frame";
      return;
    }

    const groupId = activeGroupId ?? scene.groups[0]?.id;
    if (!groupId) {
      // No frame to host the primitive yet — drop one into a fresh frame in a
      // single batch so the create-card validates against a live group id.
      const frameId = `palette-frame-${crypto.randomUUID().slice(0, 8)}`;
      const frame: RenderGroup = {
        id: frameId,
        title: "Canvas",
        summary: "",
        bounds: { x: center.x - 360, y: center.y - 260, width: 720, height: 520 },
        tagIds: [],
        zIndex: 0,
        styleKey: "default"
      };
      const ops: RenderScenePatch[] = [{ kind: "create-group", group: frame }, ...buildPrimitiveOps(kind, frameId, center)];
      handleRendererPatch({ kind: "batch", ops });
      status = primitiveStatus(kind);
      return;
    }

    const ops = buildPrimitiveOps(kind, groupId, center);
    handleRendererPatch(ops.length === 1 ? ops[0] : { kind: "batch", ops });
    status = primitiveStatus(kind);
  }

  // Build the renderPatch op(s) for a primitive within an existing group, placed
  // in an open slot near the requested world anchor. "frame" is handled by the
  // caller (it is a group, not a card) and never reaches here.
  function buildPrimitiveOps(kind: Exclude<PrimitiveKindId, "frame">, groupId: string, anchor: { x: number; y: number }): RenderScenePatch[] {
    const z = nextTopZ(scene?.nodes.filter((node) => node.groupId === groupId) ?? []);

    if (kind === "connector") {
      // A standalone connector is an edge — represented by two small anchor nodes
      // joined by a create-edge op. Endpoint nodes are created first so the edge
      // validates against live node ids inside the batch.
      const sourceId = `palette-line-${crypto.randomUUID().slice(0, 8)}`;
      const targetId = `palette-line-${crypto.randomUUID().slice(0, 8)}`;
      const handle = { width: 28, height: 28 };
      const start = openNodePosition(groupId, { x: anchor.x - 150, y: anchor.y }, handle);
      const end = openNodePosition(groupId, { x: start.x + 320, y: start.y }, handle);
      return [
        { kind: "create-card", card: primitiveCard(sourceId, groupId, "task", { x: start.x, y: start.y, ...handle }, z, "Line start", "") },
        { kind: "create-card", card: primitiveCard(targetId, groupId, "task", { x: end.x, y: end.y, ...handle }, z + 1, "Line end", "") },
        {
          kind: "create-edge",
          groupId,
          source: sourceId,
          target: targetId,
          edgeId: `palette-edge-${crypto.randomUUID().slice(0, 8)}`,
          label: "connects"
        }
      ];
    }

    const spec = primitiveCardSpec(kind);
    const position = openNodePosition(groupId, { x: anchor.x - spec.size.width / 2, y: anchor.y - spec.size.height / 2 }, spec.size);
    const id = `palette-${kind}-${crypto.randomUUID().slice(0, 8)}`;
    return [
      {
        kind: "create-card",
        card: primitiveCard(id, groupId, spec.type, { x: position.x, y: position.y, ...spec.size }, z, spec.title, spec.summary)
      }
    ];
  }

  // Map a primitive to a card type (drives styleKey in the renderer) + size/copy.
  function primitiveCardSpec(kind: Exclude<PrimitiveKindId, "frame" | "connector">): {
    type: string;
    size: { width: number; height: number };
    title: string;
    summary: string;
  } {
    if (kind === "rectangle") return { type: "task", size: { width: 220, height: 140 }, title: "Rectangle", summary: "" };
    if (kind === "ellipse") return { type: "option", size: { width: 200, height: 200 }, title: "Ellipse", summary: "" };
    // sticky / text box: a card whose body is editable text.
    return { type: "proposition", size: { width: 220, height: 180 }, title: "Note", summary: "Type your note here." };
  }

  function primitiveCard(
    id: string,
    groupId: string,
    type: string,
    bounds: WorldRect,
    zIndex: number,
    title: string,
    summary: string
  ): RenderCard {
    return {
      id,
      groupId,
      title,
      summary,
      detail: "",
      status: "draft",
      type,
      bounds,
      zIndex,
      styleKey: type,
      accessibilityLabel: `${type} ${title}`
    };
  }

  function primitiveStatus(kind: PrimitiveKindId): string {
    const labels: Record<PrimitiveKindId, string> = {
      rectangle: "Inserted rectangle",
      ellipse: "Inserted ellipse",
      connector: "Inserted connector",
      sticky: "Inserted note",
      frame: "Inserted frame"
    };
    return labels[kind];
  }

  // MG-7: tag create is a scene-core `create-tag` op. The shell mints the tag id
  // (scene-core stays randomness-free) and routes it through the renderer-op path,
  // so the optimistic apply + WS save share the single op pipeline.
  function runCreateTag(): void {
    const name = tagName.trim();
    if (!name) return;
    const color = tagColors[(scene?.tags.length ?? 0) % tagColors.length];
    const now = new Date().toISOString();
    const tag: Tag = {
      id: `tag-${crypto.randomUUID().slice(0, 8)}`,
      name,
      color,
      description: "",
      createdAt: now,
      updatedAt: now
    };
    handleRendererPatch({ kind: "create-tag", tag });
    tagName = "";
    status = `Tag created: ${name}`;
  }

  // MG-7: group-tag toggle is a scene-core `set-object-tags` op (frame target),
  // routed through the renderer-op path like every other document mutation.
  function toggleGroupTag(tag: Tag): void {
    if (!activeGroup) return;
    const group = activeGroup;
    const nextTagIds = group.tagIds.includes(tag.id)
      ? group.tagIds.filter((tagId) => tagId !== tag.id)
      : [...group.tagIds, tag.id];
    handleRendererPatch({ kind: "set-object-tags", targetKind: "frame", id: group.id, tagIds: nextTagIds });
    status = "Group tags updated";
  }

  function onSelectGroupFromSidebar(group: SceneGroup): void {
    selection = { kind: "group", id: group.id };
    editingNodeId = null;
    exportPreview = null;
    exportPreviewCopied = false;
    groupPanelOpen = false;
    focusGroup(group, { fit: true });
    void selectSceneItem({ kind: "group", id: group.id });
  }

  function toggleTagFilter(tagId: string): void {
    activeTagIds = activeTagIds.includes(tagId) ? activeTagIds.filter((id) => id !== tagId) : [...activeTagIds, tagId];
  }

  function focusGroup(group: SceneGroup, options: { zoom?: number; fit?: boolean } = {}): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    host.focusBounds(group.bounds, {
      screen: { x: rect.width / 2, y: rect.height / 2 },
      zoom: options.fit ? undefined : options.zoom ?? Math.min(0.65, Math.max(0.18, camera.zoom)),
      padding: options.fit ? { x: 110, y: 140 } : undefined,
      minZoom: options.fit ? 0.36 : undefined,
      maxZoom: options.fit ? 0.58 : undefined
    });
  }

  // Frame the camera to an arbitrary world rect (a peer/companion viewport in
  // follow mode). Centers the rect with a small fit padding, reusing the host's
  // imperative focus surface so no camera math is duplicated here.
  function focusBounds(rect: WorldRect): void {
    const view = canvasWrap?.getBoundingClientRect();
    if (!view || !host) return;
    host.focusBounds(rect, {
      screen: { x: view.width / 2, y: view.height / 2 },
      padding: { x: 80, y: 80 }
    });
  }

  function focusNode(node: SceneNode, targetZoom?: number): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    const zoom = targetZoom ?? Math.max(0.78, camera.zoom);
    const focusY = rect.width < 700 ? rect.height * 0.34 : rect.height / 2;
    host.focusBounds(
      { x: node.position.x, y: node.position.y, width: selectedCardWidth, height: selectedCardHeight },
      { screen: { x: rect.width / 2, y: focusY }, zoom, minZoom: 0.04, maxZoom: 2.8 }
    );
  }

  function zoomAtCenter(deltaY: number): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    host.wheelAtScreen({ x: rect.width / 2, y: rect.height / 2 }, deltaY);
  }

  function fitScene(): void {
    host?.fitScene();
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

  async function withBusy(label: string, action: () => Promise<void>): Promise<void> {
    busy = true;
    status = label;
    try {
      await action();
      if (status === label) status = "Ready";
    } catch (error) {
      status = error instanceof Error ? error.message : "Unknown error";
    } finally {
      busy = false;
    }
  }

  function activeGroupIdForSelection(currentScene: Scene | null, currentSelection: SceneSelection): string | undefined {
    if (!currentScene) return undefined;
    if (currentSelection.kind === "group") return currentSelection.id;
    if (currentSelection.kind === "node") return currentScene.nodes.find((node) => node.id === currentSelection.id)?.groupId;
    if (currentSelection.kind === "edge") return currentScene.edges.find((edge) => edge.id === currentSelection.id)?.groupId;
    return undefined;
  }

  // T2.2: fold a shift/meta-clicked node id into the current selection, producing a
  // transient `multi` set. Re-clicking a member removes it; collapsing to one node
  // returns a plain `node` selection, and to zero returns `canvas`.
  function toggleMultiSelect(current: SceneSelection, nodeId: string): SceneSelection {
    const baseIds = current.kind === "multi" ? current.ids : current.kind === "node" ? [current.id] : [];
    const nextIds = baseIds.includes(nodeId) ? baseIds.filter((id) => id !== nodeId) : [...baseIds, nodeId];
    if (nextIds.length === 0) return { kind: "canvas" };
    if (nextIds.length === 1) return { kind: "node", id: nextIds[0] };
    return { kind: "multi", ids: nextIds };
  }

  function validSelection(currentScene: Scene, currentSelection: SceneSelection): SceneSelection {
    if (currentSelection.kind === "canvas") return currentSelection;
    if (currentSelection.kind === "group" && currentScene.groups.some((group) => group.id === currentSelection.id)) return currentSelection;
    if (currentSelection.kind === "node" && currentScene.nodes.some((node) => node.id === currentSelection.id)) return currentSelection;
    if (currentSelection.kind === "edge" && currentScene.edges.some((edge) => edge.id === currentSelection.id)) return currentSelection;
    if (currentSelection.kind === "multi") {
      // T2.2: keep only live node ids. Collapse to a single node when one remains,
      // to canvas when none do — the multi form is reserved for >=2 objects.
      const liveIds = currentSelection.ids.filter((id) => currentScene.nodes.some((node) => node.id === id));
      if (liveIds.length >= 2) return { kind: "multi", ids: liveIds };
      if (liveIds.length === 1) return { kind: "node", id: liveIds[0] };
      return { kind: "canvas" };
    }
    return { kind: "canvas" };
  }

  function newNodeTitle(type: NodeType): string {
    const labels: Record<NodeType, string> = {
      proposition: "New proposition",
      decision_point: "New decision point",
      option: "New option",
      evidence: "New evidence",
      tradeoff: "New tradeoff",
      blocker: "New blocker",
      subdecision: "New subdecision",
      task: "New task",
      artifact: "New artifact"
    };
    return labels[type];
  }

  function defaultEdgeType(type: NodeType): EdgeType {
    if (type === "blocker") return "blocks";
    if (type === "evidence") return "supports";
    if (type === "tradeoff") return "trades_off_with";
    if (type === "subdecision" || type === "task") return "decomposes_to";
    if (type === "artifact") return "produces";
    if (type === "option") return "chooses_between";
    return "depends_on";
  }

  function defaultEdgeLabel(type: NodeType): string {
    if (type === "blocker") return "blocks";
    if (type === "evidence") return "supports";
    if (type === "tradeoff") return "tradeoff";
    if (type === "artifact") return "produces";
    if (type === "option") return "option";
    return "depends on";
  }

  function nextTopZ(nodes: SceneNode[]): number {
    return Math.max(0, ...nodes.map((node) => node.zIndex)) + 1;
  }

  async function writeClipboardText(text: string): Promise<boolean> {
    if (writeClipboardTextWithTextarea(text)) return true;
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  }

  function writeClipboardTextWithTextarea(text: string): boolean {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.left = "-9999px";
    textarea.style.top = "0";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.select();
    textarea.setSelectionRange(0, textarea.value.length);
    try {
      return document.execCommand("copy");
    } catch {
      return false;
    } finally {
      document.body.removeChild(textarea);
    }
  }

  function isDiagnosticsOnlyRendererStatus(message: string): boolean {
    return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
  }
</script>

<div class="app-shell">
  <main class="studio-stage">
    <section class={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
      <div class="flow-wrap renderer-scene-surface" data-tool={activeTool} bind:this={canvasWrap} onpointermove={handlePointerMove}>
        <div class="canvas-watermark" aria-hidden="true">
          <BrainCircuit size={28} />
          <span>shape.ai</span>
        </div>
        <PeerCursors {peers} {camera} />
        <div class="canvas-switcher-chrome">
          <CanvasSwitcher
            {canvases}
            activeCanvasId={canvasId}
            status={connectionStatus}
            busy={canvasBusy}
            onSelect={(id) => void switchToCanvas(id)}
            onCreate={(title) => void createCanvas(title)}
            onDelete={(id) => void deleteCanvas(id)}
          />
        </div>
        <div class="scene-controls" aria-label="Canvas controls">
          <button
            class="icon-button {diagnosticsOpen ? 'is-active' : ''}"
            type="button"
            onclick={() => (diagnosticsOpen = !diagnosticsOpen)}
            aria-label={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
            aria-expanded={diagnosticsOpen}
            title={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
          >
            <Activity size={15} />
          </button>
        </div>
        <CockpitRemote
          {activeTool}
          {busy}
          templateOpen={templateLibraryOpen}
          onSetTool={setActiveTool}
          onInsertPrimitive={insertPrimitive}
          onToggleTemplates={toggleTemplateLibrary}
          onZoomIn={() => zoomAtCenter(-160)}
          onZoomOut={() => zoomAtCenter(160)}
          onFit={fitScene}
          onFullscreen={() => void toggleFullscreen()}
        />
        {#if templateLibraryOpen}
          <TemplateLibrary
            templates={userTemplates}
            {busy}
            {canSaveSelection}
            onApply={(templateId) => void applyTemplateContract(templateId)}
            onSaveSelection={(title) => void saveSelectionAsTemplate(title)}
            onDelete={(templateId) => void deleteTemplateById(templateId)}
            onClose={() => (templateLibraryOpen = false)}
          />
        {/if}
        <RendererDiagnosticsDrawer
          open={diagnosticsOpen}
          stats={rendererStats}
          health={rendererHealth}
          {camera}
          {selection}
          {selectedTargetLabel}
          {status}
          {rendererStatus}
          onClose={() => (diagnosticsOpen = false)}
        />

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

      {#if scene && selection.kind !== "canvas"}
        <CanvasEditingToolbar {scene} {selection} {multiSelectIds} onPatch={handleRendererPatch} />
      {/if}

      {#if selectedNode && scene}
        <div class="selected-node-panel" aria-label="Selected node">
          <button class="selected-node-menu-button icon-button" aria-label="Node commands" title="Node commands" onclick={openSelectedNodeMenu}>
            <Ellipsis size={15} />
          </button>
          <SelectedNodeInspector
            data={{
              node: selectedNode,
              selected: true,
              editing: editingNodeId === selectedNode.id,
              comments: scene.comments.filter((comment) => comment.target.kind === "node" && comment.target.id === selectedNode.id),
              commentValue,
              busy,
              onUpdateNode: updateNode,
              onCommentChange: (value) => (commentValue = value),
              onAddComment: () => void runAddComment(),
              onToggleComment: (comment) => void toggleComment(comment),
              onAddLinkedNode: addLinkedNode,
              onDeleteNode: () => deleteNode(selectedNode.id),
              onStartEdit: startEditingNode,
              onStopEdit: () => (editingNodeId = null)
            }}
          />
        </div>
      {/if}

      <div class="floating-groups {groupPanelOpen ? 'is-open' : 'is-closed'}">
        <button
          class="group-panel-toggle icon-button {groupPanelOpen ? 'is-active' : ''}"
          onclick={() => (groupPanelOpen = !groupPanelOpen)}
          aria-label={groupPanelOpen ? "Close groups" : "Open groups"}
        >
          {#if groupPanelOpen}
            <X size={16} />
          {:else}
            <PanelLeft size={16} />
          {/if}
        </button>
        <div class="floating-groups-body">
          <Sidebar
            groups={scene?.groups ?? []}
            tags={scene?.tags ?? []}
            {activeGroupId}
            {activeGroup}
            {prompt}
            {tagName}
            {busy}
            {activeTagIds}
            onPromptChange={(value) => (prompt = value)}
            onTagNameChange={(value) => (tagName = value)}
            onCreateGroup={() => void runCreateGroup()}
            onCreateTag={() => void runCreateTag()}
            onSelectGroup={onSelectGroupFromSidebar}
            onToggleGroupTag={(tag) => void toggleGroupTag(tag)}
            onToggleTagFilter={toggleTagFilter}
            onRefresh={() => sceneClient?.resync()}
          />
        </div>
      </div>

      {#if nodeMenu && nodeMenuNode}
        <NodeContextMenu
          node={nodeMenuNode}
          x={nodeMenu.x}
          y={nodeMenu.y}
          canPaste={copiedNode !== null}
          onMoveLayer={(direction) => {
            if (nodeMenu) moveNodeLayer(nodeMenu.nodeId, direction);
            nodeMenu = null;
          }}
          onCopy={() => {
            if (nodeMenu) void copyNode(nodeMenu.nodeId);
            nodeMenu = null;
          }}
          onPaste={() => {
            if (nodeMenu) pasteCopiedNode(nodeMenu.nodeId);
            nodeMenu = null;
          }}
          onDuplicate={() => {
            if (nodeMenu) duplicateNode(nodeMenu.nodeId);
            nodeMenu = null;
          }}
          onEdit={() => {
            if (nodeMenu) editingNodeId = nodeMenu.nodeId;
            nodeMenu = null;
          }}
          onDelete={() => {
            if (nodeMenu) deleteNode(nodeMenu.nodeId);
            nodeMenu = null;
          }}
        />
      {/if}

      {#if contextMenu}
        <ContextMenu x={contextMenu.x} y={contextMenu.y} title={contextMenuTitle(contextMenu.selection)} items={contextMenuItems(contextMenu)} />
      {/if}

      {#if settingsOpen}
        <SettingsModal onClose={() => (settingsOpen = false)} />
      {/if}

      {#if busy || status !== "Ready"}
        <div class="canvas-status" role="status">
          {#if busy}
            <Loader2 class="spin" size={15} />
          {/if}
          {status}
        </div>
      {/if}

      <ExportDrawer
        {artifacts}
        {busy}
        groupId={activeGroupId}
        onExport={(type) => void runExport(type)}
        preview={exportPreview}
        previewCopied={exportPreviewCopied}
        onCopyPreview={() => void copyExportPreview()}
        onClosePreview={() => (exportPreview = null)}
      />
    </section>
  </main>
</div>
