import { useCallback, useEffect, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { Activity, BrainCircuit, Clipboard, Copy, History, Layers, LayoutTemplate, Loader2, Maximize2, Minus, MoreHorizontal, PanelLeft, Pencil, Plus, Trash2, X } from "lucide-react";
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
} from "./lib/api";
import {
  boundsIntersect,
  expandedBounds,
  nodeBounds
} from "../shared/graph";
import { cloneNodeForPaste, formatNodeMarkdown } from "./lib/nodeClipboard";
import { SelectedNodeInspector } from "./components/SelectedNodeInspector";
import { RendererCanvasHost, type RendererCanvasHostHandle, type RendererHealth, type RendererStats } from "./components/RendererCanvasHost";
import { RendererDiagnosticsDrawer } from "./components/RendererDiagnosticsDrawer";
import { Sidebar } from "./components/Sidebar";
import { ExportDrawer, type ExportPreview } from "./components/ExportDrawer";
import { CompanionDock } from "./components/CompanionDock";
import { CompanionTrace } from "./components/CompanionTrace";
import { TemplatePicker } from "./components/TemplatePicker";
import { buildTemplateInsertion, templateCatalog } from "./lib/templates";
import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../shared/renderPatch";
import type { CameraState } from "../shared/renderScene";
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
  ScenePatch,
  SceneSelection,
  Tag
} from "../shared/schema";
import "./styles.css";

const cardWidth = 270;
const cardHeight = 178;
const selectedCardWidth = 390;
const selectedCardHeight = 390;
const rendererPatchSaveDebounceMs = 80;
const tagColors = ["#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#6f7a86"];
const seedPrompt =
  "Draft an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

type Camera = CameraState;

type NodeMenuState = {
  nodeId: string;
  x: number;
  y: number;
};

type PendingRendererPatchSave = {
  patch: ScenePatch;
  rollbackScene: Scene;
  rollbackSelection: SceneSelection;
  patchKind: RenderScenePatch["kind"];
};

export default function App() {
  const [scene, setScene] = useState<Scene | null>(null);
  const [prompt, setPrompt] = useState(seedPrompt);
  const [tagName, setTagName] = useState("");
  const [activeTagIds, setActiveTagIds] = useState<string[]>([]);
  const [selection, setSelection] = useState<SceneSelection>({ kind: "canvas" });
  const [currentGroupId, setCurrentGroupId] = useState<string | undefined>();
  const [commentValue, setCommentValue] = useState("");
  const [status, setStatus] = useState("Ready");
  const [busy, setBusy] = useState(false);
  const [groupPanelOpen, setGroupPanelOpen] = useState(false);
  const [templatePickerOpen, setTemplatePickerOpen] = useState(false);
  const [editingNodeId, setEditingNodeId] = useState<string | null>(null);
  const [nodeMenu, setNodeMenu] = useState<NodeMenuState | null>(null);
  const [copiedNode, setCopiedNode] = useState<SceneNode | null>(null);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);
  const [exportPreviewCopied, setExportPreviewCopied] = useState(false);
  const [camera, setCamera] = useState<Camera>({ x: 140, y: 120, zoom: 0.28 });
  const [rendererStats, setRendererStats] = useState<RendererStats | null>(null);
  const [rendererHealth, setRendererHealth] = useState<RendererHealth | null>(null);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [traceOpen, setTraceOpen] = useState(false);
  const [rendererStatus, setRendererStatus] = useState("No renderer status yet");
  const [mcpClients, setMcpClients] = useState<McpClientInfo[]>([]);
  const canvasRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<RendererCanvasHostHandle | null>(null);
  const sceneRequestRef = useRef(0);
  const sceneRef = useRef<Scene | null>(null);
  const selectionRef = useRef<SceneSelection>({ kind: "canvas" });
  const rendererPatchSaveRef = useRef(0);
  const pendingRendererPatchSaveRef = useRef<PendingRendererPatchSave | null>(null);
  const rendererPatchSaveTimerRef = useRef<number | null>(null);
  const rendererGestureActiveRef = useRef(false);

  const selectedGroupId = activeGroupIdForSelection(scene, selection);
  const activeGroupId = selectedGroupId ?? currentGroupId ?? scene?.groups[0]?.id;
  const activeGroup = scene?.groups.find((group) => group.id === activeGroupId) ?? null;
  const selectedNode = selection.kind === "node" ? scene?.nodes.find((node) => node.id === selection.id) ?? null : null;
  const artifacts = scene?.artifacts.filter((artifact) => artifact.target.kind === "group" && artifact.target.id === activeGroupId) ?? [];
  const selectedTargetLabel = selectedNode
    ? `node:${selectedNode.id} - ${selectedNode.title}`
    : selection.kind === "canvas"
      ? "canvas"
      : `${selection.kind}:${selection.id}`;

  useEffect(() => {
    sceneRef.current = scene;
  }, [scene]);

  useEffect(() => {
    selectionRef.current = selection;
  }, [selection]);

  useEffect(() => {
    return () => {
      if (rendererPatchSaveTimerRef.current !== null) window.clearTimeout(rendererPatchSaveTimerRef.current);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    function poll() {
      fetchMcpClients()
        .then(({ clients }) => { if (!cancelled) setMcpClients(clients); })
        .catch(() => { /* best-effort; dock shows last known state */ });
    }
    poll();
    const id = window.setInterval(poll, 4_000);
    return () => { cancelled = true; window.clearInterval(id); };
  }, []);

  const handleRendererHealth = useCallback((health: RendererHealth) => {
    setRendererHealth(health);
    if (health.state === "ready") {
      setStatus((current) => (isDiagnosticsOnlyRendererStatus(current) ? "Ready" : current));
    }
  }, []);

  const refreshScene = useCallback(async () => {
    const requestId = ++sceneRequestRef.current;
    const nextScene = await fetchScene({ tagIds: activeTagIds });
    if (requestId !== sceneRequestRef.current) return;
    sceneRef.current = nextScene;
    setScene(nextScene);
    setSelection((current) => {
      const serverSelection = validSelection(nextScene, nextScene.selection);
      const localSelection = validSelection(nextScene, current);
      const nextSelection = serverSelection.kind === "canvas" && localSelection.kind !== "canvas" ? localSelection : serverSelection;
      selectionRef.current = nextSelection;
      return nextSelection;
    });
  }, [activeTagIds]);

  useEffect(() => {
    refreshScene().catch((error) => setStatus(error.message));
  }, []);

  useEffect(() => {
    const id = window.setTimeout(() => {
      refreshScene().catch((error) => setStatus(error.message));
    }, 120);
    return () => window.clearTimeout(id);
  }, [refreshScene]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (event.key === "Escape") {
        event.preventDefault();
        if (traceOpen) {
          setTraceOpen(false);
          return;
        }
        if (diagnosticsOpen) {
          setDiagnosticsOpen(false);
          return;
        }
        if (nodeMenu) {
          setNodeMenu(null);
          return;
        }
        if (editingNodeId) {
          setEditingNodeId(null);
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
        setEditingNodeId(selection.id);
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selection, editingNodeId, nodeMenu, copiedNode, scene, diagnosticsOpen, traceOpen]);

  async function runCreateGroup() {
    await withBusy("Creating group", async () => {
      sceneRequestRef.current += 1;
      const response = await createGroup(prompt, undefined, activeTagIds);
      setScene(response.scene);
      setGroupPanelOpen(false);
      setExportPreview(null);
      setExportPreviewCopied(false);
      setStatus(response.message);
      setCurrentGroupId(response.group.id);
      const firstNode = response.scene.nodes.find((node) => node.groupId === response.group.id);
      if (firstNode) focusNode(firstNode, 0.92);
      else focusGroup(response.group, { zoom: 0.72 });
      await selectSceneItem({ kind: "group", id: response.group.id }, response.scene);
    });
  }

  async function applyTemplateById(templateId: string) {
    await withBusy("Inserting template", async () => {
      const built = buildTemplateInsertion(sceneRef.current, templateId);
      if (!built) return;
      sceneRequestRef.current += 1;
      const response = await saveScenePatch(built.patch);
      setScene(response.scene);
      setExportPreview(null);
      setExportPreviewCopied(false);
      if (built.group) {
        const created = response.scene.groups.find((group) => group.id === built.group!.id) ?? built.group;
        setCurrentGroupId(created.id);
        focusGroup(created, { fit: true });
        await selectSceneItem({ kind: "group", id: created.id }, response.scene);
      }
      setStatus(`Inserted ${built.title}`);
    });
  }

  async function runCreateTag() {
    if (!tagName.trim()) return;
    await withBusy("Creating tag", async () => {
      const color = tagColors[(scene?.tags.length ?? 0) % tagColors.length];
      const response = await createTag(tagName.trim(), color);
      setScene(response.scene);
      setTagName("");
      setStatus(`Tag created: ${response.tag.name}`);
    });
  }

  async function toggleGroupTag(tag: Tag) {
    if (!activeGroup || !scene) return;
    const nextTagIds = activeGroup.tagIds.includes(tag.id)
      ? activeGroup.tagIds.filter((tagId) => tagId !== tag.id)
      : [...activeGroup.tagIds, tag.id];
    const optimisticGroup = { ...activeGroup, tagIds: nextTagIds };
    setScene({ ...scene, groups: scene.groups.map((group) => (group.id === activeGroup.id ? optimisticGroup : group)) });
    try {
      const response = await updateGroupTags(activeGroup.id, nextTagIds);
      setScene(response.scene);
      setStatus("Group tags updated");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Tag update failed");
    }
  }

  async function runExport(type: ExportType) {
    if (!activeGroupId) return;
    await withBusy(`Exporting ${type}`, async () => {
      const scope = { kind: "group" as const, id: activeGroupId };
      const response = await exportGroup(activeGroupId, type, scope);
      setScene(response.scene);
      setExportPreview(response.preview);
      setExportPreviewCopied(false);
      setStatus(`Export created: ${response.artifact.title}`);
    });
  }

  async function copyExportPreview() {
    if (!exportPreview) return;
    if (await writeClipboardText(exportPreview.content)) {
      setExportPreviewCopied(true);
      setStatus("Export preview copied");
      window.setTimeout(() => setExportPreviewCopied(false), 1400);
      return;
    }
    if (selectExportPreviewText()) {
      setStatus("Export preview selected");
      return;
    }
    setStatus("Copy failed");
  }

  async function runAddComment() {
    if (!commentValue.trim()) return;
    await withBusy("Adding comment", async () => {
      const response = await createComment({ target: selection, body: commentValue.trim() });
      setScene(response.scene);
      setCommentValue("");
    });
  }

  async function toggleComment(comment: GraphComment) {
    await withBusy("Updating comment", async () => {
      const response = await updateComment(comment.id, { resolved: !comment.resolved });
      setScene(response.scene);
    });
  }

  async function selectSceneItem(nextSelection: SceneSelection, sourceScene = scene) {
    if (!sourceScene) return;
    const valid = validSelection(sourceScene, nextSelection);
    const nextGroupId = activeGroupIdForSelection(sourceScene, valid);
    if (nextGroupId) setCurrentGroupId(nextGroupId);
    if (valid.kind !== "node") setEditingNodeId(null);
    setSelection(valid);
    try {
      await saveScenePatch({ selection: valid });
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Selection save failed");
    }
  }

  function handleRendererSelection(nextSelection: SceneSelection) {
    const currentScene = sceneRef.current;
    if (!currentScene) return;
    const valid = validSelection(currentScene, nextSelection);
    selectionRef.current = valid;
    const nextGroupId = activeGroupIdForSelection(currentScene, valid);
    if (nextGroupId) setCurrentGroupId(nextGroupId);
    if (valid.kind !== "node") setEditingNodeId(null);
    setSelection(valid);
    void saveScenePatch({ selection: valid }).catch((error) => {
      setStatus(error instanceof Error ? error.message : "Selection save failed");
    });
  }

  function handleRendererPatch(patch: RenderScenePatch) {
    const currentScene = sceneRef.current;
    if (!currentScene) return;
    const previousSelection = selectionRef.current;
    const applied = applyRenderPatchToShapeScene(currentScene, patch, new Date().toISOString());
    if (applied.errors.length > 0) {
      setStatus(applied.errors.join("; "));
      return;
    }

    const optimisticSelection = validSelection(applied.scene, applied.scene.selection);
    sceneRequestRef.current += 1;
    sceneRef.current = applied.scene;
    selectionRef.current = optimisticSelection;
    if (isContinuousRendererPatch(patch)) {
      if (!rendererGestureActiveRef.current) commitRendererScene(applied.scene, optimisticSelection);
      queueRendererPatchSave(applied.appPatch, currentScene, previousSelection, patch.kind, rendererGestureActiveRef.current);
      return;
    }
    commitRendererScene(applied.scene, optimisticSelection);
    flushRendererPatchSave();
    saveRendererPatchNow(applied.appPatch, currentScene, previousSelection, patch.kind);
  }

  function handleRendererGesture(active: boolean) {
    rendererGestureActiveRef.current = active;
    if (active) return;
    commitRendererScene();
    flushRendererPatchSave();
  }

  function commitRendererScene(nextScene = sceneRef.current, nextSelection = selectionRef.current) {
    if (!nextScene) return;
    const valid = validSelection(nextScene, nextSelection);
    setScene(nextScene);
    setSelection(valid);
    const nextGroupId = activeGroupIdForSelection(nextScene, valid);
    if (nextGroupId) setCurrentGroupId(nextGroupId);
    if (valid.kind !== "node") setEditingNodeId(null);
  }

  function queueRendererPatchSave(
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"],
    waitForGestureEnd = false
  ) {
    const pending = pendingRendererPatchSaveRef.current;
    pendingRendererPatchSaveRef.current = {
      patch: pending ? mergeScenePatches(pending.patch, patch) : patch,
      rollbackScene: pending?.rollbackScene ?? rollbackScene,
      rollbackSelection: pending?.rollbackSelection ?? rollbackSelection,
      patchKind
    };
    if (rendererPatchSaveTimerRef.current !== null) window.clearTimeout(rendererPatchSaveTimerRef.current);
    if (waitForGestureEnd) {
      rendererPatchSaveTimerRef.current = null;
      return;
    }
    rendererPatchSaveTimerRef.current = window.setTimeout(flushRendererPatchSave, rendererPatchSaveDebounceMs);
  }

  function flushRendererPatchSave() {
    const pending = pendingRendererPatchSaveRef.current;
    if (!pending) return;
    pendingRendererPatchSaveRef.current = null;
    if (rendererPatchSaveTimerRef.current !== null) {
      window.clearTimeout(rendererPatchSaveTimerRef.current);
      rendererPatchSaveTimerRef.current = null;
    }
    saveRendererPatchNow(pending.patch, pending.rollbackScene, pending.rollbackSelection, pending.patchKind, false);
  }

  function saveRendererPatchNow(
    patch: ScenePatch,
    rollbackScene: Scene,
    rollbackSelection: SceneSelection,
    patchKind: RenderScenePatch["kind"],
    applyResponseScene = true
  ) {
    const saveId = ++rendererPatchSaveRef.current;
    void saveScenePatch(patch)
      .then((response) => {
        if (saveId !== rendererPatchSaveRef.current) return;
        if (!applyResponseScene) return;
        if (rendererGestureActiveRef.current) return;
        const savedSelection = validSelection(response.scene, response.scene.selection);
        sceneRef.current = response.scene;
        selectionRef.current = savedSelection;
        setScene(response.scene);
        setSelection(savedSelection);
        const savedGroupId = activeGroupIdForSelection(response.scene, savedSelection);
        if (savedGroupId) setCurrentGroupId(savedGroupId);
      })
      .catch((error) => {
        if (saveId !== rendererPatchSaveRef.current) return;
        const restoredSelection = validSelection(rollbackScene, rollbackSelection);
        sceneRef.current = rollbackScene;
        selectionRef.current = restoredSelection;
        setScene(rollbackScene);
        setSelection(restoredSelection);
        const restoredGroupId = activeGroupIdForSelection(rollbackScene, restoredSelection);
        if (restoredGroupId) setCurrentGroupId(restoredGroupId);
        if (restoredSelection.kind !== "node") setEditingNodeId(null);
        setStatus(error instanceof Error ? error.message : `Renderer patch save failed: ${patchKind}`);
        refreshScene().catch((refreshError) => setStatus(refreshError instanceof Error ? refreshError.message : "Scene refresh failed"));
      });
  }

  function handleRendererContextMenu(point: { x: number; y: number }) {
    const currentSelection = selectionRef.current;
    if (currentSelection.kind !== "node") return;
    setEditingNodeId(null);
    setNodeMenu({ nodeId: currentSelection.id, x: point.x, y: point.y });
  }

  function handleRendererStatus(message: string) {
    setRendererStatus(message);
    if (isDiagnosticsOnlyRendererStatus(message)) return;
    setStatus(message);
  }

  function openSelectedNodeMenu(event: MouseEvent<HTMLButtonElement>) {
    if (!selectedNode) return;
    const rect = event.currentTarget.getBoundingClientRect();
    setEditingNodeId(null);
    setNodeMenu({ nodeId: selectedNode.id, x: rect.right - 220, y: rect.bottom + 8 });
  }

  function updateNode(node: GraphNode) {
    if (!scene) return;
    const current = scene.nodes.find((candidate) => candidate.id === node.id);
    if (!current) return;
    const nextNode = { ...current, ...node };
    setScene({ ...scene, nodes: scene.nodes.map((candidate) => (candidate.id === node.id ? nextNode : candidate)) });
    void saveScenePatch({ nodes: [nextNode] })
      .then((response) => setScene(response.scene))
      .catch((error) => setStatus(error instanceof Error ? error.message : "Save failed"));
  }

  function startEditingNode(nodeId: string) {
    setEditingNodeId(nodeId);
    void selectSceneItem({ kind: "node", id: nodeId });
  }

  function deleteSelection() {
    if (!scene || selection.kind === "canvas") return;
    if (selection.kind === "node") {
      deleteNode(selection.id);
      return;
    }
    if (selection.kind === "edge") {
      setScene({ ...scene, edges: scene.edges.filter((edge) => edge.id !== selection.id), selection: { kind: "canvas" } });
      void saveScenePatch({ removeEdgeIds: [selection.id], selection: { kind: "canvas" } });
    }
  }

  function deleteNode(nodeId: string) {
    if (!scene) return;
    setScene({
      ...scene,
      nodes: scene.nodes.filter((node) => node.id !== nodeId),
      edges: scene.edges.filter((edge) => edge.source !== nodeId && edge.target !== nodeId),
      selection: { kind: "canvas" }
    });
    void saveScenePatch({ removeNodeIds: [nodeId], selection: { kind: "canvas" } });
  }

  function addLinkedNode(type: NodeType) {
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
      zIndex: nextTopZ(scene.nodes.filter((node) => node.groupId === groupId)),
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
    setScene(nextScene);
    void saveScenePatch({ nodes: [node], edges: edge ? [edge] : [], selection: { kind: "node", id } }).then((response) => setScene(response.scene));
    void selectSceneItem({ kind: "node", id }, nextScene);
  }

  async function copyNode(nodeId: string) {
    const node = scene?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    setCopiedNode(node);
    if (await writeClipboardText(formatNodeMarkdown(node))) {
      setStatus("Copied node as Markdown");
      return;
    }
    setStatus("Copied node locally");
  }

  function duplicateNode(nodeId: string) {
    const node = scene?.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    pasteNode(node, nodeId);
  }

  function pasteCopiedNode(anchorNodeId?: string) {
    if (!copiedNode) return;
    pasteNode(copiedNode, anchorNodeId ?? copiedNode.id);
  }

  function pasteNode(sourceNode: SceneNode, anchorNodeId: string) {
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
    setScene({ ...scene, nodes: [...scene.nodes, node] });
    void saveScenePatch({ nodes: [node], selection: { kind: "node", id } }).then((response) => setScene(response.scene));
    setStatus("Pasted copied node");
  }

  function moveNodeLayer(nodeId: string, direction: "front" | "back") {
    if (!scene) return;
    const node = scene.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    const groupNodes = scene.nodes.filter((candidate) => candidate.groupId === node.groupId);
    const values = groupNodes.map((candidate) => candidate.zIndex);
    const zIndex = direction === "front" ? Math.max(...values, 0) + 1 : Math.min(...values, 0) - 1;
    const updated = { ...node, zIndex, updatedAt: new Date().toISOString() };
    setScene({ ...scene, nodes: scene.nodes.map((candidate) => (candidate.id === nodeId ? updated : candidate)) });
    void saveScenePatch({ nodes: [updated] });
    setStatus(direction === "front" ? "Brought node to front" : "Sent node to back");
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

  function handleFocusTarget(target: unknown) {
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

  function focusGroup(group: SceneGroup, options: { zoom?: number; fit?: boolean } = {}) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    rendererRef.current?.focusBounds(group.bounds, {
      screen: { x: rect.width / 2, y: rect.height / 2 },
      zoom: options.fit ? undefined : options.zoom ?? Math.min(0.65, Math.max(0.18, camera.zoom)),
      padding: options.fit ? { x: 110, y: 140 } : undefined,
      minZoom: options.fit ? 0.36 : undefined,
      maxZoom: options.fit ? 0.58 : undefined
    });
  }

  function focusNode(node: SceneNode, targetZoom?: number) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const zoom = targetZoom ?? Math.max(0.78, camera.zoom);
    const focusY = rect.width < 700 ? rect.height * 0.34 : rect.height / 2;
    rendererRef.current?.focusBounds(
      {
        x: node.position.x,
        y: node.position.y,
        width: selectedCardWidth,
        height: selectedCardHeight
      },
      {
        screen: { x: rect.width / 2, y: focusY },
        zoom,
        minZoom: 0.04,
        maxZoom: 2.8
      }
    );
  }

  function zoomAtCanvasCenter(deltaY: number) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    rendererRef.current?.wheelAtScreen({ x: rect.width / 2, y: rect.height / 2 }, deltaY);
  }

  function fitScene() {
    rendererRef.current?.fitScene();
  }

  async function toggleFullscreen() {
    const target = canvasRef.current;
    if (!target) return;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await target.requestFullscreen();
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Fullscreen failed");
    }
  }

  async function withBusy(label: string, action: () => Promise<void>) {
    setBusy(true);
    setStatus(label);
    try {
      await action();
      setStatus((current) => (current === label ? "Ready" : current));
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Unknown error");
    } finally {
      setBusy(false);
    }
  }

  const nodeMenuNode = nodeMenu ? scene?.nodes.find((node) => node.id === nodeMenu.nodeId) : null;

  return (
    <div className="app-shell">
      <main className="studio-stage">
        <section className={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
          <CompanionDock clients={mcpClients} onFocusTarget={handleFocusTarget} />
          <div className="flow-wrap renderer-scene-surface" ref={canvasRef}>
            <div className="canvas-watermark" aria-hidden="true">
              <BrainCircuit size={28} />
              <span>shape.ai</span>
            </div>
            <div className="scene-controls" aria-label="Canvas controls">
              <button
                className={`icon-button ${templatePickerOpen ? "is-active" : ""}`}
                type="button"
                onClick={() => setTemplatePickerOpen((open) => !open)}
                aria-label="Insert template"
                aria-expanded={templatePickerOpen}
                aria-controls="template-picker"
                title="Insert template"
              >
                <LayoutTemplate size={15} />
              </button>
              <button
                className={`icon-button ${diagnosticsOpen ? "is-active" : ""}`}
                type="button"
                onClick={() => setDiagnosticsOpen((open) => !open)}
                aria-label={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
                aria-expanded={diagnosticsOpen}
                aria-controls="renderer-diagnostics"
                title={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
              >
                <Activity size={15} />
              </button>
              <button
                className={`icon-button ${traceOpen ? "is-active" : ""}`}
                type="button"
                onClick={() => setTraceOpen((open) => !open)}
                aria-label={traceOpen ? "Close agent trace" : "Open agent trace"}
                aria-expanded={traceOpen}
                aria-controls="companion-trace"
                title={traceOpen ? "Close agent trace" : "Open agent trace"}
              >
                <History size={15} />
              </button>
              <button className="icon-button" onClick={() => zoomAtCanvasCenter(160)} aria-label="Zoom out" title="Zoom out">
                <Minus size={15} />
              </button>
              <button className="icon-button" onClick={() => zoomAtCanvasCenter(-160)} aria-label="Zoom in" title="Zoom in">
                <Plus size={15} />
              </button>
              <button className="icon-button" onClick={fitScene} aria-label="Fit scene" title="Fit scene">
                <Layers size={15} />
              </button>
              <button className="icon-button" onClick={() => void toggleFullscreen()} aria-label="Fullscreen" title="Fullscreen">
                <Maximize2 size={15} />
              </button>
            </div>
            {templatePickerOpen ? (
              <TemplatePicker
                templates={templateCatalog.map(({ id, title, description }) => ({ id, title, description }))}
                busy={busy}
                onApply={(templateId) => void applyTemplateById(templateId)}
                onClose={() => setTemplatePickerOpen(false)}
              />
            ) : null}
            <RendererDiagnosticsDrawer
              open={diagnosticsOpen}
              stats={rendererStats}
              health={rendererHealth}
              camera={camera}
              selection={selection}
              selectedTargetLabel={selectedTargetLabel}
              status={status}
              rendererStatus={rendererStatus}
              onClose={() => setDiagnosticsOpen(false)}
            />
            <CompanionTrace
              open={traceOpen}
              clients={mcpClients}
              onClose={() => setTraceOpen(false)}
              onFocusTarget={handleFocusTarget}
            />

            <RendererCanvasHost
              ref={rendererRef}
              scene={scene}
              activeTagIds={activeTagIds}
              camera={camera}
              selection={selection}
              onCameraChange={setCamera}
              onSelectionChange={handleRendererSelection}
              onPatch={handleRendererPatch}
              onGestureChange={handleRendererGesture}
              onStats={setRendererStats}
              onStatus={handleRendererStatus}
              onHealthChange={handleRendererHealth}
              onContextMenuRequest={handleRendererContextMenu}
            />
          </div>

          {selectedNode && scene ? (
            <div className="selected-node-panel" aria-label="Selected node">
              <button className="selected-node-menu-button icon-button" aria-label="Node commands" title="Node commands" onClick={openSelectedNodeMenu}>
                <MoreHorizontal size={15} />
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
                  onCommentChange: setCommentValue,
                  onAddComment: runAddComment,
                  onToggleComment: toggleComment,
                  onAddLinkedNode: addLinkedNode,
                  onDeleteNode: () => deleteNode(selectedNode.id),
                  onStartEdit: startEditingNode,
                  onStopEdit: () => setEditingNodeId(null)
                }}
              />
            </div>
          ) : null}

          <div className={`floating-groups ${groupPanelOpen ? "is-open" : "is-closed"}`}>
            <button
              className={`group-panel-toggle icon-button ${groupPanelOpen ? "is-active" : ""}`}
              onClick={() => setGroupPanelOpen((open) => !open)}
              aria-label={groupPanelOpen ? "Close groups" : "Open groups"}
            >
              {groupPanelOpen ? <X size={16} /> : <PanelLeft size={16} />}
            </button>
            <div className="floating-groups-body">
              <Sidebar
                groups={scene?.groups ?? []}
                tags={scene?.tags ?? []}
                activeGroupId={activeGroupId}
                activeGroup={activeGroup}
                prompt={prompt}
                tagName={tagName}
                busy={busy}
                activeTagIds={activeTagIds}
                onPromptChange={setPrompt}
                onTagNameChange={setTagName}
                onCreateGroup={runCreateGroup}
                onCreateTag={runCreateTag}
                onSelectGroup={(group) => {
                  setSelection({ kind: "group", id: group.id });
                  setEditingNodeId(null);
                  setExportPreview(null);
                  setExportPreviewCopied(false);
                  setGroupPanelOpen(false);
                  focusGroup(group, { fit: true });
                  void selectSceneItem({ kind: "group", id: group.id });
                }}
                onToggleGroupTag={toggleGroupTag}
                onToggleTagFilter={(tagId) =>
                  setActiveTagIds((current) => (current.includes(tagId) ? current.filter((id) => id !== tagId) : [...current, tagId]))
                }
                onRefresh={() => refreshScene().catch((error) => setStatus(error.message))}
              />
            </div>
          </div>

          {nodeMenu && nodeMenuNode ? (
            <div
              className="node-context-menu"
              style={{ left: nodeMenu.x, top: nodeMenu.y }}
              onPointerDown={(event) => event.stopPropagation()}
              onContextMenu={(event) => event.preventDefault()}
              role="menu"
            >
              <div className="node-context-menu-title">
                <span>{nodeMenuNode.title}</span>
              </div>
              <button role="menuitem" onClick={() => { moveNodeLayer(nodeMenu.nodeId, "front"); setNodeMenu(null); }}>
                <Layers size={14} />
                Bring to front
              </button>
              <button role="menuitem" onClick={() => { moveNodeLayer(nodeMenu.nodeId, "back"); setNodeMenu(null); }}>
                <Layers size={14} />
                Send to back
              </button>
              <div className="node-context-menu-separator" />
              <button role="menuitem" onClick={() => { void copyNode(nodeMenu.nodeId); setNodeMenu(null); }}>
                <Copy size={14} />
                Copy as Markdown
              </button>
              <button role="menuitem" disabled={!copiedNode} onClick={() => { pasteCopiedNode(nodeMenu.nodeId); setNodeMenu(null); }}>
                <Clipboard size={14} />
                Paste copied node
              </button>
              <button role="menuitem" onClick={() => { duplicateNode(nodeMenu.nodeId); setNodeMenu(null); }}>
                <Copy size={14} />
                Duplicate node
              </button>
              <div className="node-context-menu-separator" />
              <button role="menuitem" onClick={() => { setEditingNodeId(nodeMenu.nodeId); setNodeMenu(null); }}>
                <Pencil size={14} />
                Edit node
              </button>
              <button className="danger-menu-item" role="menuitem" onClick={() => { deleteNode(nodeMenu.nodeId); setNodeMenu(null); }}>
                <Trash2 size={14} />
                Delete node
              </button>
            </div>
          ) : null}

          {busy || status !== "Ready" ? (
            <div className="canvas-status" role="status">
              {busy ? <Loader2 className="spin" size={15} /> : null}
              {status}
            </div>
          ) : null}

          <ExportDrawer
            artifacts={artifacts}
            busy={busy}
            groupId={activeGroupId}
            onExport={runExport}
            preview={exportPreview}
            previewCopied={exportPreviewCopied}
            onCopyPreview={copyExportPreview}
            onClosePreview={() => setExportPreview(null)}
          />
        </section>
      </main>
    </div>
  );
}

function activeGroupIdForSelection(scene: Scene | null, selection: SceneSelection): string | undefined {
  if (!scene) return undefined;
  if (selection.kind === "group") return selection.id;
  if (selection.kind === "node") return scene.nodes.find((node) => node.id === selection.id)?.groupId;
  if (selection.kind === "edge") return scene.edges.find((edge) => edge.id === selection.id)?.groupId;
  return undefined;
}

function isContinuousRendererPatch(patch: RenderScenePatch): boolean {
  return patch.kind === "move-group" || patch.kind === "move-card";
}

function mergeScenePatches(left: ScenePatch, right: ScenePatch): ScenePatch {
  const groups = mergeById(left.groups, right.groups);
  const nodes = mergeById(left.nodes, right.nodes);
  const edges = mergeById(left.edges, right.edges);
  const translateGroups = mergeTranslateGroups(left.translateGroups, right.translateGroups);
  const removeGroupIds = mergeUnique(left.removeGroupIds, right.removeGroupIds);
  const removeNodeIds = mergeUnique(left.removeNodeIds, right.removeNodeIds);
  const removeEdgeIds = mergeUnique(left.removeEdgeIds, right.removeEdgeIds);
  return {
    ...(groups.length > 0 ? { groups } : {}),
    ...(nodes.length > 0 ? { nodes } : {}),
    ...(edges.length > 0 ? { edges } : {}),
    ...(translateGroups.length > 0 ? { translateGroups } : {}),
    ...(removeGroupIds.length > 0 ? { removeGroupIds } : {}),
    ...(removeNodeIds.length > 0 ? { removeNodeIds } : {}),
    ...(removeEdgeIds.length > 0 ? { removeEdgeIds } : {}),
    ...(right.selection ? { selection: right.selection } : left.selection ? { selection: left.selection } : {})
  };
}

function mergeById<T extends { id: string }>(left: T[] | undefined, right: T[] | undefined): T[] {
  const items = new Map<string, T>();
  for (const item of left ?? []) items.set(item.id, item);
  for (const item of right ?? []) items.set(item.id, item);
  return Array.from(items.values());
}

function mergeTranslateGroups(
  left: ScenePatch["translateGroups"] | undefined,
  right: ScenePatch["translateGroups"] | undefined
): NonNullable<ScenePatch["translateGroups"]> {
  const movements = new Map<string, { groupId: string; dx: number; dy: number }>();
  for (const movement of [...(left ?? []), ...(right ?? [])]) {
    const current = movements.get(movement.groupId);
    movements.set(movement.groupId, {
      groupId: movement.groupId,
      dx: (current?.dx ?? 0) + movement.dx,
      dy: (current?.dy ?? 0) + movement.dy
    });
  }
  return Array.from(movements.values());
}

function mergeUnique(left: string[] | undefined, right: string[] | undefined): string[] {
  return Array.from(new Set([...(left ?? []), ...(right ?? [])]));
}

function validSelection(scene: Scene, selection: SceneSelection): SceneSelection {
  if (selection.kind === "canvas") return selection;
  if (selection.kind === "group" && scene.groups.some((group) => group.id === selection.id)) return selection;
  if (selection.kind === "node" && scene.nodes.some((node) => node.id === selection.id)) return selection;
  if (selection.kind === "edge" && scene.edges.some((edge) => edge.id === selection.id)) return selection;
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

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
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

function selectExportPreviewText(): boolean {
  const preview = document.querySelector(".export-preview-body");
  if (!preview) return false;
  const selection = window.getSelection();
  if (!selection) return false;
  const range = document.createRange();
  range.selectNodeContents(preview);
  selection.removeAllRanges();
  selection.addRange(range);
  return true;
}

function isDiagnosticsOnlyRendererStatus(message: string): boolean {
  return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
}
