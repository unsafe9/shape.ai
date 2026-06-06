<script lang="ts">
  import { onDestroy } from "svelte";
  import { Activity, BrainCircuit, History, LayoutTemplate, Layers, Loader2, Maximize2, Minus, Ellipsis, PanelLeft, Plus, X } from "lucide-svelte";
  import {
    createComment,
    createGroup,
    createTag,
    exportGroup,
    fetchMcpClients,
    fetchScene,
    saveScenePatch,
    updateComment,
    updateGroupTags,
    type McpClientInfo
  } from "../lib/api";
  import { boundsIntersect, expandedBounds, nodeBounds } from "../../shared/graph";
  import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../../shared/renderPatch";
  import type { CameraState } from "../../shared/renderScene";
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
    Tag
  } from "../../shared/schema";
  import { cloneNodeForPaste, formatNodeMarkdown } from "../lib/nodeClipboard";
  import {
    decideFollowCommand,
    initialFollowState,
    jumpToCurrentTarget,
    onUserGrab,
    pauseFollow,
    reconcileFollowee,
    resolveFollowee,
    resumeFollow,
    stopFollow,
    targetKey,
    toggleFollow,
    type FollowState
  } from "../lib/followController";
  import { ShapeCanvasHost, type RendererHealth, type RendererStats, type ShapeCanvasHostCallbacks } from "../lib/canvasHost";
  import { createPatchSaver, isContinuousRendererPatch } from "../lib/patchSaver";
  import { buildTemplateInsertion, templateCatalog } from "../lib/templates";
  import Sidebar from "./Sidebar.svelte";
  import TemplatePicker from "./TemplatePicker.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";
  import CanvasEditingToolbar from "./CanvasEditingToolbar.svelte";
  import SelectedNodeInspector from "./node/SelectedNodeInspector.svelte";
  import NodeContextMenu from "./NodeContextMenu.svelte";
  import CompanionDock from "./CompanionDock.svelte";
  import CompanionTrace from "./CompanionTrace.svelte";
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
  let templatePickerOpen = $state(false);
  let diagnosticsOpen = $state(false);
  let traceOpen = $state(false);

  // ----- node editing slice -----
  let editingNodeId = $state<string | null>(null);
  let nodeMenu = $state<NodeMenuState | null>(null);
  let copiedNode = $state<SceneNode | null>(null);
  let commentValue = $state("");

  // ----- export drawer slice -----
  let exportPreview = $state<ExportPreview | null>(null);
  let exportPreviewCopied = $state(false);

  // ----- MCP companions + follow mode -----
  let mcpClients = $state<McpClientInfo[]>([]);
  let followState = $state<FollowState>(initialFollowState);

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
  // Mirror the follow state for the poll-driven effect so it can read the latest
  // machine without subscribing (matching App.tsx's followStateRef).
  let followStateRef: FollowState = initialFollowState;

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

  // T6.2 §1: debounced, gesture-gated renderer-patch save lives in the
  // framework-neutral patchSaver module; the shell only feeds it.
  const patchSaver = createPatchSaver({
    isGestureActive: () => gestureActive,
    onSaved: (savedScene, savedSelection) => {
      const validated = validSelection(savedScene, savedSelection);
      sceneRequest += 1;
      scene = savedScene;
      selection = validated;
      const savedGroupId = activeGroupIdForSelection(savedScene, validated);
      if (savedGroupId) currentGroupId = savedGroupId;
    },
    onError: (rollbackScene, rollbackSelection, message) => {
      const restored = validSelection(rollbackScene, rollbackSelection);
      scene = rollbackScene;
      selection = restored;
      const restoredGroupId = activeGroupIdForSelection(rollbackScene, restored);
      if (restoredGroupId) currentGroupId = restoredGroupId;
      if (restored.kind !== "node") editingNodeId = null;
      status = message;
      void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene refresh failed"));
    }
  });

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
    }
  };

  // Initial fetch + reactive reload-on-filter, mirroring the App.tsx
  // batched/debounced load. Document writes flow ONLY through patchSaver, so the
  // scene store is never re-derived per keystroke (T6.2 §1 batch-aware wiring).
  void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene load failed"));

  $effect(() => {
    const tagIds = activeTagIds;
    const id = window.setTimeout(() => {
      void refreshScene(tagIds).catch((error) => (status = error instanceof Error ? error.message : "Scene load failed"));
    }, 120);
    return () => window.clearTimeout(id);
  });

  // T5.1 MCP companion poll (best-effort; dock shows last known state).
  $effect(() => {
    let cancelled = false;
    function poll() {
      fetchMcpClients()
        .then(({ clients }) => {
          if (!cancelled) mcpClients = clients;
        })
        .catch(() => {
          /* best-effort */
        });
    }
    poll();
    const id = window.setInterval(poll, 4_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  });

  // T5.3 pinned follow (§3) + followee reconcile (§1/§8). When the MCP poll reports
  // a new lastTarget for the pinned followee — and no user gesture is active (§8) —
  // re-frame the camera through the existing focus path.
  $effect(() => {
    const clients = mcpClients;
    const reconciled = reconcileFollowee(followStateRef, clients);
    if (reconciled !== followStateRef) {
      followState = reconciled;
      followStateRef = reconciled;
      return;
    }
    if (reconciled.mode !== "pinned") return;
    const followee = resolveFollowee(reconciled, clients);
    const command = decideFollowCommand(reconciled, followee, gestureActive);
    if (command.state !== reconciled) {
      followState = command.state;
      followStateRef = command.state;
    }
    if (command.target !== null) followTarget(command.target);
  });

  // Keep followStateRef in sync with the reactive followState.
  $effect(() => {
    followStateRef = followState;
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

  // Global keyboard shortcuts (Esc handoff, copy/paste/edit) — mirrors App.tsx.
  $effect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (event.key === "Escape") {
        event.preventDefault();
        // T5.3 §7 hard handoff: Esc stops following first; camera stays put.
        if (followStateRef.mode !== "off") {
          followState = stopFollow(followStateRef);
          return;
        }
        if (traceOpen) {
          traceOpen = false;
          return;
        }
        if (diagnosticsOpen) {
          diagnosticsOpen = false;
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
        void selectSceneItem({ kind: "canvas" });
        return;
      }
      if (target && ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "v") {
        event.preventDefault();
        pasteCopiedNode(selection.kind === "node" ? selection.id : undefined);
        return;
      }
      if (selection.kind !== "node") return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "c") {
        event.preventDefault();
        void copyNode(selection.id);
        return;
      }
      if (event.key.toLowerCase() === "e" && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        editingNodeId = selection.id;
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  });

  onDestroy(() => patchSaver.dispose());

  async function refreshScene(tagIds = activeTagIds): Promise<void> {
    const requestId = ++sceneRequest;
    const nextScene = await fetchScene({ tagIds });
    if (requestId !== sceneRequest) return;
    scene = nextScene;
    const serverSelection = validSelection(nextScene, nextScene.selection);
    const localSelection = validSelection(nextScene, selection);
    selection = serverSelection.kind === "canvas" && localSelection.kind !== "canvas" ? localSelection : serverSelection;
  }

  function handleHost(next: ShapeCanvasHost): void {
    host = next;
    if (scene) host.loadScene(scene, activeTagIds, selection);
    host.syncSelection(primarySelection(selection));
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
    void saveScenePatch({ selection: primarySelection(valid) }).catch((error) => {
      status = error instanceof Error ? error.message : "Selection save failed";
    });
  }

  function handleRendererPatch(patch: RenderScenePatch): void {
    if (!scene) return;
    const previousScene = scene;
    const previousSelection = selection;
    const applied = applyRenderPatchToShapeScene(scene, patch, new Date().toISOString());
    if (applied.errors.length > 0) {
      status = applied.errors.join("; ");
      return;
    }
    const optimisticSelection = validSelection(applied.scene, applied.scene.selection);
    sceneRequest += 1;
    if (isContinuousRendererPatch(patch)) {
      if (!gestureActive) commitScene(applied.scene, optimisticSelection);
      patchSaver.queue(applied.appPatch, previousScene, previousSelection, patch.kind, gestureActive);
      return;
    }
    commitScene(applied.scene, optimisticSelection);
    patchSaver.flush();
    patchSaver.saveNow(applied.appPatch, previousScene, previousSelection, patch.kind);
  }

  function handleRendererGesture(active: boolean): void {
    gestureActive = active;
    // T5.3 §8 rule 2: a user grab during pinned follow demotes to paused (soft
    // handoff). The followee is remembered; one tap of Resume re-engages.
    if (active) {
      const grabbed = onUserGrab(followStateRef);
      if (grabbed !== followStateRef) followState = grabbed;
      return;
    }
    if (scene) commitScene(scene, selection);
    patchSaver.flush();
  }

  function commitScene(nextScene: Scene, nextSelection: SceneSelection): void {
    const valid = validSelection(nextScene, nextSelection);
    scene = nextScene;
    selection = valid;
    const nextGroupId = activeGroupIdForSelection(nextScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    if (valid.kind !== "node") editingNodeId = null;
  }

  function handleRendererStatus(message: string): void {
    rendererStatus = message;
    if (isDiagnosticsOnlyRendererStatus(message)) return;
    status = message;
  }

  function handleContextMenuRequest(point: { x: number; y: number }): void {
    if (selectionRef.kind !== "node") return;
    editingNodeId = null;
    nodeMenu = { nodeId: selectionRef.id, x: point.x, y: point.y };
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
    void saveScenePatch({ nodes: [nextNode] })
      .then((response) => (scene = response.scene))
      .catch((error) => (status = error instanceof Error ? error.message : "Save failed"));
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
    void saveScenePatch({ removeNodeIds: [nodeId], selection: { kind: "canvas" } });
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
    void saveScenePatch({ nodes: [node], edges: edge ? [edge] : [], selection: { kind: "node", id } }).then((response) => (scene = response.scene));
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
    void saveScenePatch({ nodes: [node], selection: { kind: "node", id } }).then((response) => (scene = response.scene));
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
    void saveScenePatch({ nodes: [updated] });
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

  function handleFocusTarget(target: unknown): void {
    if (!scene || !target || typeof target !== "object") return;
    const t = target as { kind?: string; id?: string; groupId?: string; ids?: { kind: string; id: string }[] };
    if (t.kind === "group" && t.id) {
      const group = scene.groups.find((g) => g.id === t.id);
      if (group) {
        focusGroup(group, { fit: true });
        void selectSceneItem({ kind: "group", id: t.id });
      }
    } else if (t.kind === "node" && t.id) {
      const node = scene.nodes.find((n) => n.id === t.id);
      if (node) {
        focusNode(node);
        void selectSceneItem({ kind: "node", id: t.id });
      }
    } else if (t.kind === "edge" && t.id) {
      void selectSceneItem({ kind: "edge", id: t.id });
    } else if (t.kind === "artifact" && t.groupId) {
      const group = scene.groups.find((g) => g.id === t.groupId);
      if (group) focusGroup(group, { fit: true });
      void selectSceneItem({ kind: "group", id: t.groupId });
    } else if (t.kind === "selection" && Array.isArray(t.ids) && t.ids.length > 0) {
      const first = t.ids[0] as { kind: string; id: string };
      if (first.kind === "group") {
        const group = scene.groups.find((g) => g.id === first.id);
        if (group) focusGroup(group, { fit: true });
        void selectSceneItem({ kind: "group", id: first.id });
      } else if (first.kind === "node") {
        const node = scene.nodes.find((n) => n.id === first.id);
        if (node) focusNode(node);
        void selectSceneItem({ kind: "node", id: first.id });
      } else if (first.kind === "edge") {
        void selectSceneItem({ kind: "edge", id: first.id });
      }
    } else {
      fitScene();
    }
  }

  // T5.3 follow framing: the camera-only subset of handleFocusTarget. Follow is
  // observe-only (§0/§8) — frames the followee's target but never writes state.
  function followTarget(target: unknown): void {
    const currentScene = scene;
    if (!currentScene || !target || typeof target !== "object") return;
    const t = target as { kind?: string; id?: string; groupId?: string; ids?: { kind: string; id: string }[] };
    if (t.kind === "group" && t.id) {
      const group = currentScene.groups.find((g) => g.id === t.id);
      if (group) focusGroup(group, { fit: true });
    } else if (t.kind === "node" && t.id) {
      const node = currentScene.nodes.find((n) => n.id === t.id);
      if (node) focusNode(node);
    } else if (t.kind === "artifact" && t.groupId) {
      const group = currentScene.groups.find((g) => g.id === t.groupId);
      if (group) focusGroup(group, { fit: true });
    } else if (t.kind === "selection" && Array.isArray(t.ids) && t.ids.length > 0) {
      const first = t.ids[0] as { kind: string; id: string };
      if (first.kind === "group") {
        const group = currentScene.groups.find((g) => g.id === first.id);
        if (group) focusGroup(group, { fit: true });
      } else if (first.kind === "node") {
        const node = currentScene.nodes.find((n) => n.id === first.id);
        if (node) focusNode(node);
      }
    }
  }

  function handleToggleFollow(client: McpClientInfo): void {
    const next = toggleFollow(followStateRef, client);
    if (next === followStateRef) return;
    if (next.mode === "pinned") {
      followState = { ...next, lastFramedKey: targetKey(client.lastTarget) };
      followTarget(client.lastTarget);
    } else {
      followState = next;
    }
  }

  function handlePauseResumeFollow(): void {
    const current = followStateRef;
    if (current.mode === "pinned") {
      followState = pauseFollow(current);
      return;
    }
    if (current.mode === "paused") {
      const resumed = resumeFollow(current);
      const followee = resolveFollowee(resumed, mcpClients);
      if (followee && targetKey(followee.lastTarget) !== null) {
        followState = { ...resumed, lastFramedKey: targetKey(followee.lastTarget) };
        followTarget(followee.lastTarget);
      } else {
        followState = resumed;
      }
    }
  }

  function handleJumpToCurrent(): void {
    const followee = resolveFollowee(followStateRef, mcpClients);
    const target = jumpToCurrentTarget(followee);
    if (target !== null) followTarget(target);
  }

  // ----- selection / scene helpers ----------------------------------------

  async function selectSceneItem(nextSelection: SceneSelection, sourceScene: Scene | null = scene): Promise<void> {
    if (!sourceScene) return;
    const valid = validSelection(sourceScene, nextSelection);
    const nextGroupId = activeGroupIdForSelection(sourceScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    if (valid.kind !== "node") editingNodeId = null;
    if (valid.kind === "canvas") multiSelectIds = [];
    selection = valid;
    try {
      await saveScenePatch({ selection: valid });
    } catch (error) {
      status = error instanceof Error ? error.message : "Selection save failed";
    }
  }

  async function runCreateGroup(): Promise<void> {
    await withBusy("Creating group", async () => {
      sceneRequest += 1;
      const response = await createGroup(prompt, undefined, activeTagIds);
      scene = response.scene;
      groupPanelOpen = false;
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

  async function applyTemplateById(templateId: string): Promise<void> {
    await withBusy("Inserting template", async () => {
      const built = buildTemplateInsertion(scene, templateId);
      if (!built) return;
      sceneRequest += 1;
      const response = await saveScenePatch(built.patch);
      scene = response.scene;
      if (built.group) {
        const created = response.scene.groups.find((group) => group.id === built.group!.id) ?? built.group;
        currentGroupId = created.id;
        focusGroup(created, { fit: true });
        await selectSceneItem({ kind: "group", id: created.id }, response.scene);
      }
      status = `Inserted ${built.title}`;
    });
  }

  async function runCreateTag(): Promise<void> {
    if (!tagName.trim()) return;
    await withBusy("Creating tag", async () => {
      const color = tagColors[(scene?.tags.length ?? 0) % tagColors.length];
      const response = await createTag(tagName.trim(), color);
      scene = response.scene;
      tagName = "";
      status = `Tag created: ${response.tag.name}`;
    });
  }

  async function toggleGroupTag(tag: Tag): Promise<void> {
    if (!activeGroup || !scene) return;
    const group = activeGroup;
    const nextTagIds = group.tagIds.includes(tag.id)
      ? group.tagIds.filter((tagId) => tagId !== tag.id)
      : [...group.tagIds, tag.id];
    scene = { ...scene, groups: scene.groups.map((candidate) => (candidate.id === group.id ? { ...group, tagIds: nextTagIds } : candidate)) };
    try {
      const response = await updateGroupTags(group.id, nextTagIds);
      scene = response.scene;
      status = "Group tags updated";
    } catch (error) {
      status = error instanceof Error ? error.message : "Tag update failed";
    }
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
      <CompanionDock
        clients={mcpClients}
        onFocusTarget={handleFocusTarget}
        follow={{
          followeeClientId: followState.followeeClientId,
          mode: followState.mode,
          onToggleFollow: handleToggleFollow,
          onPauseResume: handlePauseResumeFollow,
          onJumpToCurrent: handleJumpToCurrent
        }}
      />
      <div class="flow-wrap renderer-scene-surface" bind:this={canvasWrap}>
        <div class="canvas-watermark" aria-hidden="true">
          <BrainCircuit size={28} />
          <span>shape.ai</span>
        </div>
        <div class="scene-controls" aria-label="Canvas controls">
          <button
            class="icon-button {templatePickerOpen ? 'is-active' : ''}"
            type="button"
            onclick={() => (templatePickerOpen = !templatePickerOpen)}
            aria-label="Insert template"
            aria-expanded={templatePickerOpen}
            title="Insert template"
          >
            <LayoutTemplate size={15} />
          </button>
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
          <button
            class="icon-button {traceOpen ? 'is-active' : ''}"
            type="button"
            onclick={() => (traceOpen = !traceOpen)}
            aria-label={traceOpen ? "Close agent trace" : "Open agent trace"}
            aria-expanded={traceOpen}
            title={traceOpen ? "Close agent trace" : "Open agent trace"}
          >
            <History size={15} />
          </button>
          <button class="icon-button" onclick={() => zoomAtCenter(160)} aria-label="Zoom out" title="Zoom out">
            <Minus size={15} />
          </button>
          <button class="icon-button" onclick={() => zoomAtCenter(-160)} aria-label="Zoom in" title="Zoom in">
            <Plus size={15} />
          </button>
          <button class="icon-button" onclick={fitScene} aria-label="Fit scene" title="Fit scene">
            <Layers size={15} />
          </button>
          <button class="icon-button" onclick={() => void toggleFullscreen()} aria-label="Fullscreen" title="Fullscreen">
            <Maximize2 size={15} />
          </button>
        </div>
        {#if templatePickerOpen}
          <TemplatePicker
            templates={templateCatalog.map(({ id, title, description }) => ({ id, title, description }))}
            {busy}
            onApply={(templateId) => void applyTemplateById(templateId)}
            onClose={() => (templatePickerOpen = false)}
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
        <CompanionTrace open={traceOpen} clients={mcpClients} onClose={() => (traceOpen = false)} onFocusTarget={handleFocusTarget} />

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
            onRefresh={() => void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene refresh failed"))}
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
