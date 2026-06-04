import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, PointerEvent, WheelEvent } from "react";
import { BrainCircuit, Clipboard, Copy, Layers, Loader2, Maximize2, Minus, PanelLeft, Pencil, Plus, Trash2, X } from "lucide-react";
import {
  createComment,
  createGroup,
  createTag,
  exportGroup,
  fetchScene,
  saveScenePatch,
  updateComment,
  updateGroupTags
} from "./lib/api";
import {
  boundsIntersect,
  edgeTypeLabels,
  expandedBounds,
  groupTags,
  limitSceneNodesForLod,
  nodeBounds,
  nodeTypeLabels,
  shouldShowSceneEdges,
  shouldShowSceneNodes
} from "../shared/graph";
import { cloneNodeForPaste, formatNodeMarkdown } from "./lib/nodeClipboard";
import { DecisionNode } from "./components/DecisionNode";
import { Sidebar } from "./components/Sidebar";
import { ExportDrawer, type ExportPreview } from "./components/ExportDrawer";
import type {
  Bounds,
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
} from "../shared/schema";
import "./styles.css";

const cardWidth = 270;
const cardHeight = 178;
const selectedCardWidth = 390;
const selectedCardHeight = 390;
const minZoom = 0.004;
const maxZoom = 3.4;
const detailZoom = 0.48;
const tagColors = ["#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#6f7a86"];
const seedPrompt =
  "Draft an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

type Camera = {
  x: number;
  y: number;
  zoom: number;
};

type NodeMenuState = {
  nodeId: string;
  x: number;
  y: number;
};

type DragState =
  | { kind: "pan"; pointerId: number; startX: number; startY: number; camera: Camera }
  | { kind: "node"; pointerId: number; nodeId: string; startX: number; startY: number; startPosition: { x: number; y: number } }
  | {
      kind: "group";
      pointerId: number;
      groupId: string;
      startX: number;
      startY: number;
      startBounds: Bounds;
      startNodes: Array<{ id: string; position: { x: number; y: number } }>;
      moved: boolean;
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
  const [editingNodeId, setEditingNodeId] = useState<string | null>(null);
  const [nodeMenu, setNodeMenu] = useState<NodeMenuState | null>(null);
  const [copiedNode, setCopiedNode] = useState<SceneNode | null>(null);
  const [exportPreview, setExportPreview] = useState<ExportPreview | null>(null);
  const [exportPreviewCopied, setExportPreviewCopied] = useState(false);
  const [camera, setCamera] = useState<Camera>({ x: 140, y: 120, zoom: 0.28 });
  const [interacting, setInteracting] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<DragState | null>(null);
  const suppressGroupClickRef = useRef(false);
  const interactionTimerRef = useRef<number | null>(null);
  const sceneRequestRef = useRef(0);

  const viewport = useMemo(() => viewportBounds(camera, canvasRef.current), [camera]);
  const selectedGroupId = activeGroupIdForSelection(scene, selection);
  const activeGroupId = selectedGroupId ?? currentGroupId ?? scene?.groups[0]?.id;
  const activeGroup = scene?.groups.find((group) => group.id === activeGroupId) ?? null;
  const selectedNode = selection.kind === "node" ? scene?.nodes.find((node) => node.id === selection.id) ?? null : null;
  const artifacts = scene?.artifacts.filter((artifact) => artifact.target.kind === "group" && artifact.target.id === activeGroupId) ?? [];

  const visible = useMemo(() => {
    if (!scene) return { groups: [] as SceneGroup[], nodes: [] as SceneNode[], edges: [] as SceneEdge[] };
    const padded = expandedBounds(viewport, Math.max(800, 1600 / Math.max(camera.zoom, 0.02)));
    const groups = scene.groups.filter((group) => activeTagIds.length === 0 || activeTagIds.every((tagId) => group.tagIds.includes(tagId)));
    const visibleGroups = groups.filter((group) => boundsIntersect(group.bounds, padded));
    const groupIds = new Set(visibleGroups.map((group) => group.id));
    const focusGroupId = activeGroupId && groupIds.has(activeGroupId) && camera.zoom >= 0.36 ? activeGroupId : undefined;
    const nodeGroupIds = focusGroupId ? new Set([focusGroupId]) : groupIds;
    const nodeGroupCount = focusGroupId ? 1 : visibleGroups.length;
    const showNodes = shouldShowSceneNodes(camera.zoom, nodeGroupCount);
    const candidateNodes = showNodes ? scene.nodes.filter((node) => nodeGroupIds.has(node.groupId) && boundsIntersect(nodeBounds(node), padded)) : [];
    const nodes = limitSceneNodesForLod(candidateNodes, camera.zoom, nodeGroupCount);
    const nodeIds = new Set(nodes.map((node) => node.id));
    const showEdges = shouldShowSceneEdges(camera.zoom, nodeGroupCount);
    const edges = showEdges ? scene.edges.filter((edge) => nodeGroupIds.has(edge.groupId) && nodeIds.has(edge.source) && nodeIds.has(edge.target)) : [];
    return { groups: visibleGroups, nodes, edges };
  }, [scene, viewport, camera.zoom, activeTagIds, activeGroupId]);

  const fullNodeIds = useMemo(() => {
    if (camera.zoom < detailZoom) return new Set<string>();
    if (!interacting) return new Set(visible.nodes.map((node) => node.id));
    return new Set([selection.kind === "node" ? selection.id : ""].filter(Boolean));
  }, [camera.zoom, interacting, visible.nodes, selection]);

  const refreshScene = useCallback(async () => {
    const requestId = ++sceneRequestRef.current;
    const nextScene = await fetchScene({ viewport, zoom: camera.zoom, tagIds: activeTagIds, focusGroupId: activeGroupId });
    if (requestId !== sceneRequestRef.current) return;
    setScene(nextScene);
    setSelection((current) => {
      const serverSelection = validSelection(nextScene, nextScene.selection);
      const localSelection = validSelection(nextScene, current);
      return serverSelection.kind === "canvas" && localSelection.kind !== "canvas" ? localSelection : serverSelection;
    });
  }, [viewport, camera.zoom, activeTagIds, activeGroupId]);

  useEffect(() => {
    refreshScene().catch((error) => setStatus(error.message));
  }, []);

  useEffect(() => {
    const id = window.setTimeout(() => {
      const drag = dragRef.current;
      if (drag && (drag.kind === "node" || drag.kind === "group")) return;
      refreshScene().catch((error) => setStatus(error.message));
    }, 120);
    return () => window.clearTimeout(id);
  }, [refreshScene]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      const target = event.target as HTMLElement | null;
      if (event.key === "Escape") {
        event.preventDefault();
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
  }, [selection, editingNodeId, nodeMenu, copiedNode, scene]);

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
      else focusGroup(response.group, 0.72);
      await selectSceneItem({ kind: "group", id: response.group.id }, response.scene);
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
          updatedAt: new Date().toISOString()
        }
      : null;
    setScene({ ...scene, nodes: [...scene.nodes, node], edges: edge ? [...scene.edges, edge] : scene.edges });
    void saveScenePatch({ nodes: [node], edges: edge ? [edge] : [], selection: { kind: "node", id } }).then((response) => setScene(response.scene));
    void selectSceneItem({ kind: "node", id });
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

  function focusGroup(group: SceneGroup, zoom = Math.min(0.65, Math.max(0.18, camera.zoom))) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    setCamera({
      zoom,
      x: rect.width / 2 - (group.bounds.x + group.bounds.width / 2) * zoom,
      y: rect.height / 2 - (group.bounds.y + group.bounds.height / 2) * zoom
    });
    markInteracting();
  }

  function groupFocusZoom(group: SceneGroup): number {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return Math.min(0.58, Math.max(0.38, camera.zoom));
    const usableWidth = Math.max(360, rect.width - 220);
    const usableHeight = Math.max(320, rect.height - 280);
    const fitZoom = Math.min(usableWidth / group.bounds.width, usableHeight / group.bounds.height);
    return clamp(Math.max(0.38, fitZoom), 0.36, 0.58);
  }

  function focusNode(node: SceneNode, targetZoom?: number) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const zoom = targetZoom ?? Math.max(0.78, camera.zoom);
    const focusY = rect.width < 700 ? rect.height * 0.34 : rect.height / 2;
    setCamera({
      zoom,
      x: rect.width / 2 - (node.position.x + selectedCardWidth / 2) * zoom,
      y: focusY - (node.position.y + selectedCardHeight / 2) * zoom
    });
    markInteracting();
  }

  function zoomAtCanvasCenter(multiplier: number) {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const screenX = rect.width / 2;
    const screenY = rect.height / 2;
    const world = screenToWorld(screenX, screenY, camera);
    const zoom = clamp(camera.zoom * multiplier, minZoom, maxZoom);
    setCamera({
      zoom,
      x: screenX - world.x * zoom,
      y: screenY - world.y * zoom
    });
    markInteracting();
  }

  async function fitScene() {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    let groups = scene?.groups ?? [];
    try {
      const overviewScene = await fetchScene({ zoom: 0.05, tagIds: activeTagIds });
      sceneRequestRef.current += 1;
      setScene(overviewScene);
      groups = overviewScene.groups;
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Fit scene failed");
    }
    if (groups.length === 0) return;
    const bounds = unionBounds(groups.map((group) => group.bounds));
    const usableWidth = Math.max(320, rect.width - 160);
    const usableHeight = Math.max(260, rect.height - 160);
    const zoom = clamp(Math.min(usableWidth / bounds.width, usableHeight / bounds.height), minZoom, 1.1);
    setCamera({
      zoom,
      x: rect.width / 2 - (bounds.x + bounds.width / 2) * zoom,
      y: rect.height / 2 - (bounds.y + bounds.height / 2) * zoom
    });
    markInteracting();
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

  function onCanvasPointerDown(event: PointerEvent<HTMLDivElement>) {
    if (event.button !== 0 || event.target !== event.currentTarget) return;
    dragRef.current = { kind: "pan", pointerId: event.pointerId, startX: event.clientX, startY: event.clientY, camera };
    event.currentTarget.setPointerCapture(event.pointerId);
    markInteracting();
  }

  function onNodePointerDown(event: PointerEvent<HTMLDivElement>, node: SceneNode) {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest("button,input,textarea,select,.node-note-scroll")) return;
    event.stopPropagation();
    dragRef.current = { kind: "node", pointerId: event.pointerId, nodeId: node.id, startX: event.clientX, startY: event.clientY, startPosition: node.position };
    event.currentTarget.setPointerCapture(event.pointerId);
    markInteracting();
  }

  function onGroupPointerDown(event: PointerEvent<HTMLButtonElement>, group: SceneGroup) {
    if (event.button !== 0 || !scene) return;
    event.stopPropagation();
    setNodeMenu(null);
    setCurrentGroupId(group.id);
    setSelection({ kind: "group", id: group.id });
    dragRef.current = {
      kind: "group",
      pointerId: event.pointerId,
      groupId: group.id,
      startX: event.clientX,
      startY: event.clientY,
      startBounds: group.bounds,
      startNodes: scene.nodes
        .filter((node) => node.groupId === group.id)
        .map((node) => ({ id: node.id, position: node.position })),
      moved: false
    };
    event.currentTarget.setPointerCapture(event.pointerId);
    markInteracting();
  }

  function onPointerMove(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag) return;
    markInteracting();
    if (drag.kind === "pan") {
      setCamera({ ...drag.camera, x: drag.camera.x + event.clientX - drag.startX, y: drag.camera.y + event.clientY - drag.startY });
      return;
    }
    if (!scene) return;
    if (drag.kind === "group") {
      const dx = (event.clientX - drag.startX) / camera.zoom;
      const dy = (event.clientY - drag.startY) / camera.zoom;
      if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) > 3) {
        drag.moved = true;
        suppressGroupClickRef.current = true;
      }
      const startNodePositions = new Map(drag.startNodes.map((node) => [node.id, node.position]));
      const now = new Date().toISOString();
      setScene({
        ...scene,
        groups: scene.groups.map((group) =>
          group.id === drag.groupId
            ? {
                ...group,
                bounds: { ...drag.startBounds, x: drag.startBounds.x + dx, y: drag.startBounds.y + dy },
                updatedAt: now
              }
            : group
        ),
        nodes: scene.nodes.map((node) => {
          const startPosition = startNodePositions.get(node.id);
          return startPosition
            ? {
                ...node,
                position: { x: startPosition.x + dx, y: startPosition.y + dy },
                updatedAt: now
              }
            : node;
        })
      });
      return;
    }
    const node = scene.nodes.find((candidate) => candidate.id === drag.nodeId);
    if (!node) return;
    const nextNode = {
      ...node,
      position: {
        x: drag.startPosition.x + (event.clientX - drag.startX) / camera.zoom,
        y: drag.startPosition.y + (event.clientY - drag.startY) / camera.zoom
      },
      updatedAt: new Date().toISOString()
    };
    setScene({ ...scene, nodes: scene.nodes.map((candidate) => (candidate.id === node.id ? nextNode : candidate)) });
  }

  function onPointerUp(event: PointerEvent<HTMLDivElement>) {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (drag.kind === "node" && scene) {
      const node = scene.nodes.find((candidate) => candidate.id === drag.nodeId);
      if (node) void saveScenePatch({ nodes: [node] }).then((response) => setScene(response.scene));
    }
    if (drag.kind === "group" && drag.moved) {
      void saveScenePatch({
        translateGroups: [
          {
            groupId: drag.groupId,
            dx: (event.clientX - drag.startX) / camera.zoom,
            dy: (event.clientY - drag.startY) / camera.zoom
          }
        ]
      }).then((response) => setScene(response.scene));
      window.setTimeout(() => {
        suppressGroupClickRef.current = false;
      }, 0);
    }
    try {
      event.currentTarget.releasePointerCapture(drag.pointerId);
    } catch {
      // Pointer capture may already be released by the browser.
    }
  }

  function onWheel(event: WheelEvent<HTMLDivElement>) {
    event.preventDefault();
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const screenX = event.clientX - rect.left;
    const screenY = event.clientY - rect.top;
    const world = screenToWorld(screenX, screenY, camera);
    const nextZoom = clamp(camera.zoom * Math.exp(-event.deltaY * 0.0012), minZoom, maxZoom);
    setCamera({
      zoom: nextZoom,
      x: screenX - world.x * nextZoom,
      y: screenY - world.y * nextZoom
    });
    markInteracting();
  }

  function markInteracting() {
    setInteracting(true);
    if (interactionTimerRef.current) window.clearTimeout(interactionTimerRef.current);
    interactionTimerRef.current = window.setTimeout(() => setInteracting(false), 180);
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
  const lodLabel = visible.nodes.length === 0 ? "overview" : camera.zoom < detailZoom ? "compact" : interacting ? "active-cull" : "detail";

  return (
    <div className="app-shell">
      <main className="studio-stage">
        <section className={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
          <div
            className="flow-wrap scene-canvas"
            ref={canvasRef}
            onPointerDown={onCanvasPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onWheel={onWheel}
          >
            <div className="canvas-watermark" aria-hidden="true">
              <BrainCircuit size={28} />
              <span>shape.ai</span>
            </div>
            <div className="scene-perf-hud" aria-label="Scene performance mode">
              {lodLabel} · {visible.groups.length}g/{visible.nodes.length}n · {Math.round(camera.zoom * 100)}%
            </div>
            <div className="scene-controls" aria-label="Canvas controls">
              <button className="icon-button" onClick={() => zoomAtCanvasCenter(0.82)} aria-label="Zoom out" title="Zoom out">
                <Minus size={15} />
              </button>
              <button className="icon-button" onClick={() => zoomAtCanvasCenter(1.22)} aria-label="Zoom in" title="Zoom in">
                <Plus size={15} />
              </button>
              <button className="icon-button" onClick={() => void fitScene()} aria-label="Fit scene" title="Fit scene">
                <Layers size={15} />
              </button>
              <button className="icon-button" onClick={() => void toggleFullscreen()} aria-label="Fullscreen" title="Fullscreen">
                <Maximize2 size={15} />
              </button>
            </div>

            {scene && scene.groups.length > 0 ? (
              <div
                className="scene-viewport"
                style={{
                  transform: `matrix(${camera.zoom}, 0, 0, ${camera.zoom}, ${camera.x}, ${camera.y})`
                }}
              >
                <svg className="scene-edge-layer" width="120000" height="120000" viewBox="-20000 -20000 120000 120000">
                  {visible.edges.map((edge) => (
                    <SceneEdgeLine
                      key={edge.id}
                      edge={edge}
                      nodes={scene.nodes}
                      selected={selection.kind === "edge" && selection.id === edge.id}
                      selectedNodeId={selection.kind === "node" ? selection.id : undefined}
                      zoom={camera.zoom}
                      onClick={() => {
                        setNodeMenu(null);
                        void selectSceneItem({ kind: "edge", id: edge.id });
                      }}
                    />
                  ))}
                </svg>

                {visible.groups.map((group) => (
                  <GroupFrame
                    key={group.id}
                    group={group}
                    tags={groupTags(group, scene.tags)}
                    selected={selection.kind === "group" && selection.id === group.id}
                    zoom={camera.zoom}
                    onPointerDown={(event) => onGroupPointerDown(event, group)}
                    onClick={() => {
                      if (suppressGroupClickRef.current) {
                        suppressGroupClickRef.current = false;
                        return;
                      }
                      setNodeMenu(null);
                      setCurrentGroupId(group.id);
                      focusGroup(group, groupFocusZoom(group));
                      void selectSceneItem({ kind: "group", id: group.id });
                    }}
                    onDoubleClick={() => focusGroup(group, 0.72)}
                  />
                ))}

                {visible.nodes.map((node) => {
                  const selected = selection.kind === "node" && selection.id === node.id;
                  const renderInteractive = fullNodeIds.has(node.id) || selected || editingNodeId === node.id;
                  return (
                    <div
                      key={node.id}
                      className={`scene-node ${renderInteractive ? "is-interactive" : "is-preview"} ${selected ? "is-selected" : ""}`}
                      style={{
                        transform: `translate3d(${node.position.x}px, ${node.position.y}px, 0)`,
                        zIndex: node.zIndex
                      }}
                      onPointerDown={(event) => onNodePointerDown(event, node)}
                      onClick={(event) => {
                        event.stopPropagation();
                        setNodeMenu(null);
                        focusNode(node);
                        if (event.altKey) setEditingNodeId(node.id);
                        else if (editingNodeId && editingNodeId !== node.id) setEditingNodeId(null);
                        void selectSceneItem({ kind: "node", id: node.id });
                      }}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        setEditingNodeId(null);
                        setNodeMenu({ nodeId: node.id, x: event.clientX, y: event.clientY });
                        void selectSceneItem({ kind: "node", id: node.id });
                      }}
                    >
                      {renderInteractive ? (
                        <DecisionNode
                          data={{
                            node,
                            selected,
                            editing: editingNodeId === node.id,
                            comments: scene.comments.filter((comment) => comment.target.kind === "node" && comment.target.id === node.id),
                            commentValue,
                            busy,
                            onUpdateNode: updateNode,
                            onCommentChange: setCommentValue,
                            onAddComment: runAddComment,
                            onToggleComment: toggleComment,
                            onAddLinkedNode: addLinkedNode,
                            onDeleteNode: () => deleteNode(node.id),
                            onStartEdit: startEditingNode,
                            onStopEdit: () => setEditingNodeId(null)
                          }}
                        />
                      ) : (
                        <NodePreviewCard node={node} />
                      )}
                    </div>
                  );
                })}
              </div>
            ) : (
              <div className="empty-canvas">
                <h2>Create a group</h2>
                <p>Start with a proposition, architecture concern, or implementation plan. It will become a group on the infinite canvas.</p>
              </div>
            )}
          </div>

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
                  focusGroup(group, groupFocusZoom(group));
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

function SceneEdgeLine({
  edge,
  nodes,
  selected,
  selectedNodeId,
  zoom,
  onClick
}: {
  edge: SceneEdge;
  nodes: SceneNode[];
  selected: boolean;
  selectedNodeId?: string;
  zoom: number;
  onClick: () => void;
}) {
  const source = nodes.find((node) => node.id === edge.source);
  const target = nodes.find((node) => node.id === edge.target);
  if (!source || !target) return null;
  const sourceSize = visualNodeSize(source, selectedNodeId);
  const targetSize = visualNodeSize(target, selectedNodeId);
  const sourceCenter = { x: source.position.x + sourceSize.width / 2, y: source.position.y + sourceSize.height / 2 };
  const targetCenter = { x: target.position.x + targetSize.width / 2, y: target.position.y + targetSize.height / 2 };
  const leftToRight = sourceCenter.x <= targetCenter.x;
  const x1 = leftToRight ? source.position.x + sourceSize.width : source.position.x;
  const y1 = sourceCenter.y;
  const x2 = leftToRight ? target.position.x : target.position.x + targetSize.width;
  const y2 = targetCenter.y;
  const bend = Math.min(280, Math.max(120, Math.abs(x2 - x1) * 0.45));
  const c1x = leftToRight ? x1 + bend : x1 - bend;
  const c2x = leftToRight ? x2 - bend : x2 + bend;
  const midX = (x1 + x2) / 2;
  const path = `M ${x1} ${y1} C ${c1x} ${y1}, ${c2x} ${y2}, ${x2} ${y2}`;
  return (
    <g
      className={`scene-edge ${selected ? "is-selected" : ""} ${zoom < detailZoom ? "is-compact" : ""}`}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
    >
      <path d={path} />
      {zoom >= detailZoom || selected ? <text x={midX} y={(y1 + y2) / 2 - 8}>{edge.label || edgeTypeLabels[edge.type]}</text> : null}
    </g>
  );
}

function visualNodeSize(node: SceneNode, selectedNodeId?: string): { width: number; height: number } {
  if (node.id === selectedNodeId) return { width: selectedCardWidth, height: selectedCardHeight };
  return { width: cardWidth, height: cardHeight };
}

function GroupFrame({
  group,
  tags,
  selected,
  zoom,
  onPointerDown,
  onClick,
  onDoubleClick
}: {
  group: SceneGroup;
  tags: Tag[];
  selected: boolean;
  zoom: number;
  onPointerDown: (event: PointerEvent<HTMLButtonElement>) => void;
  onClick: () => void;
  onDoubleClick: () => void;
}) {
  const color = tags[0]?.color ?? "#7b8794";
  return (
    <button
      className={`scene-group-frame ${selected ? "is-selected" : ""} ${zoom < 0.12 ? "is-overview" : ""}`}
      style={{
        transform: `translate3d(${group.bounds.x}px, ${group.bounds.y}px, 0)`,
        width: group.bounds.width,
        height: group.bounds.height,
        "--group-color": color
      } as CSSProperties}
      onPointerDown={onPointerDown}
      onClick={(event) => {
        event.stopPropagation();
        onClick();
      }}
      onDoubleClick={(event) => {
        event.stopPropagation();
        onDoubleClick();
      }}
    >
      <strong>{group.title}</strong>
      <span>{tags.map((tag) => tag.name).join(" / ") || "untagged"}</span>
    </button>
  );
}

function NodePreviewCard({ node }: { node: SceneNode }) {
  return (
    <div className={`decision-node decision-node--${node.status} decision-node-type--${node.type} is-lod-preview`}>
      <div className="node-head">
        <span className="node-type">{nodeTypeLabels[node.type]}</span>
      </div>
      <article className="node-note-scroll is-preview">
        <h3 className="node-note-title">{node.title}</h3>
        <p className="node-note-summary">{node.summary}</p>
        {node.detail ? <p className="node-note-detail">{node.detail}</p> : null}
      </article>
    </div>
  );
}

function viewportBounds(camera: Camera, element: HTMLDivElement | null): Bounds {
  const width = element?.clientWidth ?? window.innerWidth;
  const height = element?.clientHeight ?? window.innerHeight;
  return {
    x: -camera.x / camera.zoom,
    y: -camera.y / camera.zoom,
    width: width / camera.zoom,
    height: height / camera.zoom
  };
}

function screenToWorld(x: number, y: number, camera: Camera): { x: number; y: number } {
  return {
    x: (x - camera.x) / camera.zoom,
    y: (y - camera.y) / camera.zoom
  };
}

function activeGroupIdForSelection(scene: Scene | null, selection: SceneSelection): string | undefined {
  if (!scene) return undefined;
  if (selection.kind === "group") return selection.id;
  if (selection.kind === "node") return scene.nodes.find((node) => node.id === selection.id)?.groupId;
  if (selection.kind === "edge") return scene.edges.find((edge) => edge.id === selection.id)?.groupId;
  return undefined;
}

function validSelection(scene: Scene, selection: SceneSelection): SceneSelection {
  if (selection.kind === "canvas") return selection;
  if (selection.kind === "group" && scene.groups.some((group) => group.id === selection.id)) return selection;
  if (selection.kind === "node" && scene.nodes.some((node) => node.id === selection.id)) return selection;
  if (selection.kind === "edge" && scene.edges.some((edge) => edge.id === selection.id)) return selection;
  return { kind: "canvas" };
}

function unionBounds(boundsList: Bounds[]): Bounds {
  const minX = Math.min(...boundsList.map((bounds) => bounds.x));
  const minY = Math.min(...boundsList.map((bounds) => bounds.y));
  const maxX = Math.max(...boundsList.map((bounds) => bounds.x + bounds.width));
  const maxY = Math.max(...boundsList.map((bounds) => bounds.y + bounds.height));
  return { x: minX, y: minY, width: Math.max(1, maxX - minX), height: Math.max(1, maxY - minY) };
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
