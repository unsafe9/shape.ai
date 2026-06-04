import { useCallback, useEffect, useMemo, useState } from "react";
import type { MouseEvent } from "react";
import { Background, Controls, ReactFlow, type Connection, type Edge, type Node, type ReactFlowInstance } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { BrainCircuit, Clipboard, Copy, Layers, Loader2, PanelLeft, Pencil, Trash2, X } from "lucide-react";
import {
  createComment,
  createShape,
  exportShape,
  listShapes,
  saveGraphEdit,
  updateComment,
} from "./lib/api";
import { graphToFlow, type StudioNodeData } from "./lib/flow";
import { cloneNodeForPaste, formatNodeMarkdown } from "./lib/nodeClipboard";
import { DecisionNode } from "./components/DecisionNode";
import { Sidebar } from "./components/Sidebar";
import { ExportDrawer } from "./components/ExportDrawer";
import type {
  DecisionGraph,
  Shape,
  EdgeType,
  ExportType,
  GraphComment,
  GraphEdge,
  GraphLayout,
  GraphNode,
  GraphSelection,
  NodeType
} from "../shared/schema";
import "./styles.css";

const nodeTypes = { studio: DecisionNode };
const cardWidth = 390;
const cardHeight = 390;
const viewportEase = (t: number) => 1 - Math.pow(1 - t, 3);
const pasteOffset = 46;

type NodeMenuState = {
  nodeId: string;
  x: number;
  y: number;
};

const seedPrompt =
  "Shape an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

export default function App() {
  const [shapes, setShapes] = useState<Shape[]>([]);
  const [activeShape, setActiveShape] = useState<Shape | null>(null);
  const [prompt, setPrompt] = useState(seedPrompt);
  const [selection, setSelection] = useState<GraphSelection>({ kind: "graph" });
  const [commentValue, setCommentValue] = useState("");
  const [status, setStatus] = useState("Ready");
  const [busy, setBusy] = useState(false);
  const [shapePanelOpen, setShapePanelOpen] = useState(false);
  const [editingNodeId, setEditingNodeId] = useState<string | null>(null);
  const [flowInstance, setFlowInstance] = useState<ReactFlowInstance<Node<StudioNodeData>, Edge> | null>(null);
  const [nodeMenu, setNodeMenu] = useState<NodeMenuState | null>(null);
  const [copiedNode, setCopiedNode] = useState<GraphNode | null>(null);

  const flow = useMemo(() => {
    if (!activeShape) return { nodes: [] as Node<StudioNodeData>[], edges: [] as Edge[] };
    return graphToFlow(
      activeShape.graph,
      activeShape.layout,
      selection.kind === "graph" ? undefined : selection.id,
      editingNodeId,
      activeShape.comments,
      updateNode,
      commentValue,
      busy || !activeShape,
      setCommentValue,
      runAddComment,
      toggleComment,
      addLinkedNode,
      deleteSelection,
      startEditingNode,
      () => setEditingNodeId(null)
    );
  }, [activeShape, selection, editingNodeId, commentValue, busy]);

  const refreshShapes = useCallback(async () => {
    const nextShapes = await listShapes();
    const nextActive = activeShape ? nextShapes.find((shape) => shape.id === activeShape.id) ?? activeShape : nextShapes[0] ?? null;
    setShapes(nextShapes);
    setActiveShape(nextActive);
    setSelection(nextActive ? validSelection(nextActive, nextActive.selection) : { kind: "graph" });
  }, [activeShape]);

  useEffect(() => {
    refreshShapes().catch((error) => setStatus(error.message));
  }, []);

  useEffect(() => {
    if (selection.kind !== "node") {
      setEditingNodeId(null);
      return;
    }
    if (editingNodeId && editingNodeId !== selection.id) {
      setEditingNodeId(null);
    }
  }, [selection, editingNodeId]);

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
        resetViewport();
        void selectGraphItem({ kind: "graph" });
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
  }, [selection, editingNodeId, flowInstance, nodeMenu, copiedNode, activeShape]);

  async function runCreate() {
    await withBusy("Creating shape", async () => {
      const response = await createShape(prompt);
      updateShape(response.shape);
      setShapePanelOpen(false);
      setEditingNodeId(null);
      setCommentValue("");
      setStatus(response.message);
    });
  }

  async function runExport(type: ExportType) {
    if (!activeShape) return;
    await withBusy(`Exporting ${type}`, async () => {
      const scope = selection.kind === "graph" ? { kind: "whole_graph" as const } : selection;
      const response = await exportShape(activeShape.id, type, scope);
      updateShape(response.shape);
      setStatus(`Export created: ${response.artifact.title}`);
    });
  }

  async function runAddComment() {
    if (!activeShape || !commentValue.trim()) return;
    await withBusy("Adding comment", async () => {
      const response = await createComment(activeShape.id, {
        target: selection,
        body: commentValue.trim()
      });
      updateShape(response.shape);
      setCommentValue("");
    });
  }

  async function toggleComment(comment: GraphComment) {
    if (!activeShape) return;
    await withBusy("Updating comment", async () => {
      const response = await updateComment(activeShape.id, comment.id, { resolved: !comment.resolved });
      updateShape(response.shape);
    });
  }

  function updateShape(shape: Shape) {
    setActiveShape(shape);
    setSelection(validSelection(shape, shape.selection));
    setShapes((current) => [shape, ...current.filter((candidate) => candidate.id !== shape.id)]);
  }

  async function saveGraph(graph: DecisionGraph, layout = activeShape?.layout, nextSelection = selection) {
    if (!activeShape) return;
    const optimistic = { ...activeShape, graph, layout: layout ?? activeShape.layout, selection: nextSelection };
    updateShape(optimistic);
    try {
      const response = await saveGraphEdit(activeShape.id, { graph, layout, selection: nextSelection });
      updateShape(response.shape);
      setStatus("Saved");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Save failed");
    }
  }

  async function saveLayout(layout: GraphLayout) {
    if (!activeShape) return;
    updateShape({ ...activeShape, layout });
    try {
      const response = await saveGraphEdit(activeShape.id, { layout });
      updateShape(response.shape);
      setStatus("Layout saved");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Layout save failed");
    }
  }

  async function selectGraphItem(nextSelection: GraphSelection) {
    if (!activeShape) return;
    const valid = validSelection(activeShape, nextSelection);
    if (valid.kind !== "node") {
      setEditingNodeId(null);
    }
    setSelection(valid);
    setActiveShape({ ...activeShape, selection: valid });
    try {
      const response = await saveGraphEdit(activeShape.id, { selection: valid });
      updateShape(response.shape);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Selection save failed");
    }
  }

  function updateNode(node: GraphNode) {
    if (!activeShape) return;
    const graph = {
      ...activeShape.graph,
      nodes: activeShape.graph.nodes.map((candidate) => (candidate.id === node.id ? node : candidate))
    };
    void saveGraph(graph);
  }

  function startEditingNode(nodeId: string) {
    setEditingNodeId(nodeId);
    void selectGraphItem({ kind: "node", id: nodeId });
  }

  function deleteSelection() {
    if (!activeShape || selection.kind === "graph") return;
    if (selection.kind === "node") {
      deleteNode(selection.id);
      return;
    }
    const graph = {
      ...activeShape.graph,
      edges: activeShape.graph.edges.filter((edge) => edge.id !== selection.id)
    };
    void saveGraph(graph, activeShape.layout, { kind: "graph" });
  }

  function deleteNode(nodeId: string) {
    if (!activeShape) return;
    const { [nodeId]: _position, ...nodePositions } = activeShape.layout.nodePositions;
    const { [nodeId]: _zOrder, ...nodeZOrder } = activeShape.layout.nodeZOrder ?? {};
    const graph = {
      ...activeShape.graph,
      nodes: activeShape.graph.nodes.filter((node) => node.id !== nodeId),
      edges: activeShape.graph.edges.filter((edge) => edge.source !== nodeId && edge.target !== nodeId)
    };
    const layout = { ...activeShape.layout, nodePositions, nodeZOrder };
    void saveGraph(graph, layout, { kind: "graph" });
  }

  function addLinkedNode(type: NodeType) {
    if (!activeShape) return;
    const sourceId = selection.kind === "node" ? selection.id : activeShape.graph.nodes[0]?.id;
    const id = `${type.replace(/_/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
    const node: GraphNode = {
      id,
      type,
      title: newNodeTitle(type),
      summary: "New item. Edit this short summary.",
      detail: "New item. Add the supporting detail here.",
      status: "draft",
      confidence: 0.5,
      evidenceRefs: [],
      childDecisionIds: []
    };
    const edge: GraphEdge | null = sourceId
      ? {
          id: `edge-${crypto.randomUUID().slice(0, 8)}`,
          type: defaultEdgeType(type),
          source: sourceId,
          target: id,
          label: defaultEdgeLabel(type),
          rationale: "",
          confidence: 0.5
        }
      : null;
    const graph = {
      ...activeShape.graph,
      nodes: [...activeShape.graph.nodes, node],
      edges: edge ? [...activeShape.graph.edges, edge] : activeShape.graph.edges
    };
    const basePosition =
      sourceId && activeShape.layout.nodePositions[sourceId]
        ? activeShape.layout.nodePositions[sourceId]
        : { x: 120, y: 120 };
    const layout = {
      ...activeShape.layout,
      nodePositions: {
        ...activeShape.layout.nodePositions,
        [id]: { x: basePosition.x + 340, y: basePosition.y + 150 }
      },
      nodeZOrder: {
        ...(activeShape.layout.nodeZOrder ?? {}),
        [id]: nextTopZ(activeShape.graph, activeShape.layout)
      }
    };
    void saveGraph(graph, layout, { kind: "node", id });
  }

  function connectNodes(connection: Connection) {
    if (!activeShape || !connection.source || !connection.target || connection.source === connection.target) return;
    const edge: GraphEdge = {
      id: `edge-${crypto.randomUUID().slice(0, 8)}`,
      type: "depends_on",
      source: connection.source,
      target: connection.target,
      label: "depends on",
      rationale: "",
      confidence: 0.5
    };
    const graph = {
      ...activeShape.graph,
      edges: [...activeShape.graph.edges, edge]
    };
    void saveGraph(graph, activeShape.layout, { kind: "edge", id: edge.id });
  }

  function moveNode(node: Node) {
    if (!activeShape) return;
    void saveLayout({
      ...activeShape.layout,
      nodePositions: {
        ...activeShape.layout.nodePositions,
        [node.id]: { x: node.position.x, y: node.position.y }
      }
    });
  }

  function moveNodeLayer(nodeId: string, direction: "front" | "back") {
    if (!activeShape) return;
    const values = activeShape.graph.nodes.map((node, index) => activeShape.layout.nodeZOrder?.[node.id] ?? index);
    const nextZ = direction === "front" ? Math.max(...values, 0) + 1 : Math.min(...values, 0) - 1;
    void saveLayout({
      ...activeShape.layout,
      nodeZOrder: {
        ...(activeShape.layout.nodeZOrder ?? {}),
        [nodeId]: nextZ
      }
    });
    setStatus(direction === "front" ? "Brought node to front" : "Sent node to back");
  }

  async function copyNode(nodeId: string) {
    const node = activeShape?.graph.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    const markdown = formatNodeMarkdown(node);
    setCopiedNode(node);
    if (await writeClipboardText(markdown)) {
      setStatus("Copied node as Markdown");
      return;
    }
    setStatus("Copied node locally");
  }

  function duplicateNode(nodeId: string) {
    const node = activeShape?.graph.nodes.find((candidate) => candidate.id === nodeId);
    if (!node) return;
    pasteNode(node, nodeId);
  }

  function pasteCopiedNode(anchorNodeId?: string) {
    if (!copiedNode) return;
    pasteNode(copiedNode, anchorNodeId ?? copiedNode.id);
  }

  function pasteNode(sourceNode: GraphNode, anchorNodeId: string) {
    if (!activeShape) return;
    const id = `${sourceNode.type.replace(/_/g, "-")}-${crypto.randomUUID().slice(0, 8)}`;
    const node = cloneNodeForPaste(sourceNode, id);
    const anchorPosition = nodePosition(anchorNodeId);
    const graph = {
      ...activeShape.graph,
      nodes: [...activeShape.graph.nodes, node]
    };
    const layout = {
      ...activeShape.layout,
      nodePositions: {
        ...activeShape.layout.nodePositions,
        [id]: { x: anchorPosition.x + pasteOffset, y: anchorPosition.y + pasteOffset }
      },
      nodeZOrder: {
        ...(activeShape.layout.nodeZOrder ?? {}),
        [id]: nextTopZ(activeShape.graph, activeShape.layout)
      }
    };
    void saveGraph(graph, layout, { kind: "node", id });
    setStatus("Pasted copied node");
  }

  function nodePosition(nodeId: string): { x: number; y: number } {
    return activeShape?.layout.nodePositions[nodeId] ?? flow.nodes.find((node) => node.id === nodeId)?.position ?? { x: 120, y: 120 };
  }

  function focusNode(node: Node) {
    void flowInstance?.setCenter(node.position.x + cardWidth / 2, node.position.y + cardHeight / 2, {
      zoom: 1,
      duration: 620,
      ease: viewportEase,
      interpolate: "smooth"
    });
  }

  function focusEdge(edge: Edge) {
    const source = flow.nodes.find((node) => node.id === edge.source);
    const target = flow.nodes.find((node) => node.id === edge.target);
    if (!source || !target) return;
    void flowInstance?.setCenter(
      (source.position.x + target.position.x + cardWidth) / 2,
      (source.position.y + target.position.y + cardHeight) / 2,
      {
        zoom: 0.88,
        duration: 620,
        ease: viewportEase,
        interpolate: "smooth"
      }
    );
  }

  function resetViewport() {
    void flowInstance?.fitView({
      padding: 0.24,
      duration: 620,
      ease: viewportEase,
      interpolate: "smooth"
    });
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

  const nodeMenuNode = nodeMenu ? activeShape?.graph.nodes.find((node) => node.id === nodeMenu.nodeId) : null;

  return (
    <div className="app-shell">
      <main className="studio-stage">
        <section className={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
          <div className="flow-wrap">
            <div className="canvas-watermark" aria-hidden="true">
              <BrainCircuit size={28} />
              <span>shape.ai</span>
            </div>

            {activeShape ? (
              <ReactFlow
                nodes={flow.nodes}
                edges={flow.edges}
                nodeTypes={nodeTypes}
                fitView
                fitViewOptions={{ padding: 0.24, duration: 500, ease: viewportEase, interpolate: "smooth" }}
                minZoom={0.18}
                maxZoom={2.4}
                panOnDrag
                panOnScroll={false}
                elevateNodesOnSelect={false}
                selectionOnDrag={false}
                zoomOnDoubleClick
                zoomOnPinch
                zoomOnScroll
                onInit={setFlowInstance}
                onNodeClick={(event: MouseEvent, node: Node) => {
                  setNodeMenu(null);
                  focusNode(node);
                  if (event.altKey) {
                    setEditingNodeId(node.id);
                  } else if (editingNodeId && editingNodeId !== node.id) {
                    setEditingNodeId(null);
                  }
                  void selectGraphItem({ kind: "node", id: node.id });
                }}
                onNodeContextMenu={(event: MouseEvent, node: Node) => {
                  event.preventDefault();
                  setEditingNodeId(null);
                  setNodeMenu({ nodeId: node.id, x: event.clientX, y: event.clientY });
                  void selectGraphItem({ kind: "node", id: node.id });
                }}
                onEdgeClick={(_event: MouseEvent, edge: Edge) => {
                  setNodeMenu(null);
                  setEditingNodeId(null);
                  focusEdge(edge);
                  void selectGraphItem({ kind: "edge", id: edge.id });
                }}
                onNodeDragStart={() => setNodeMenu(null)}
                onNodeDragStop={(_event, node) => moveNode(node)}
                onConnect={connectNodes}
                onPaneClick={() => {
                  setNodeMenu(null);
                  setEditingNodeId(null);
                  resetViewport();
                  void selectGraphItem({ kind: "graph" });
                }}
              >
                <Background color="#d6dde2" gap={26} />
                <Controls position="bottom-right" />
              </ReactFlow>
            ) : (
              <div className="empty-canvas">
                <h2>Create a shape graph</h2>
                <p>Start with a proposition, architecture concern, or implementation plan that needs sharper decisions.</p>
              </div>
            )}
          </div>

          <div className={`floating-shapes ${shapePanelOpen ? "is-open" : "is-closed"}`}>
            <button
              className={`shape-panel-toggle icon-button ${shapePanelOpen ? "is-active" : ""}`}
              onClick={() => setShapePanelOpen((open) => !open)}
              aria-label={shapePanelOpen ? "Close shapes" : "Open shapes"}
            >
              {shapePanelOpen ? <X size={16} /> : <PanelLeft size={16} />}
            </button>
            <div className="floating-shapes-body">
              <Sidebar
                shapes={shapes}
                activeShapeId={activeShape?.id}
                prompt={prompt}
                busy={busy}
                onPromptChange={setPrompt}
                onCreate={runCreate}
                onSelect={(shape) => {
                  setActiveShape(shape);
                  setSelection(validSelection(shape, shape.selection));
                  setEditingNodeId(null);
                  setShapePanelOpen(false);
                }}
                onRefresh={() => refreshShapes().catch((error) => setStatus(error.message))}
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
              <button
                role="menuitem"
                onClick={() => {
                  moveNodeLayer(nodeMenu.nodeId, "front");
                  setNodeMenu(null);
                }}
              >
                <Layers size={14} />
                Bring to front
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  moveNodeLayer(nodeMenu.nodeId, "back");
                  setNodeMenu(null);
                }}
              >
                <Layers size={14} />
                Send to back
              </button>
              <div className="node-context-menu-separator" />
              <button
                role="menuitem"
                onClick={() => {
                  void copyNode(nodeMenu.nodeId);
                  setNodeMenu(null);
                }}
              >
                <Copy size={14} />
                Copy as Markdown
              </button>
              <button
                role="menuitem"
                disabled={!copiedNode}
                onClick={() => {
                  pasteCopiedNode(nodeMenu.nodeId);
                  setNodeMenu(null);
                }}
              >
                <Clipboard size={14} />
                Paste copied node
              </button>
              <button
                role="menuitem"
                onClick={() => {
                  duplicateNode(nodeMenu.nodeId);
                  setNodeMenu(null);
                }}
              >
                <Copy size={14} />
                Duplicate node
              </button>
              <div className="node-context-menu-separator" />
              <button
                role="menuitem"
                onClick={() => {
                  setEditingNodeId(nodeMenu.nodeId);
                  setNodeMenu(null);
                }}
              >
                <Pencil size={14} />
                Edit node
              </button>
              <button
                className="danger-menu-item"
                role="menuitem"
                onClick={() => {
                  deleteNode(nodeMenu.nodeId);
                  setNodeMenu(null);
                }}
              >
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
            artifacts={activeShape?.artifacts ?? []}
            busy={busy}
            shapeId={activeShape?.id}
            onExport={runExport}
          />
        </section>
      </main>
    </div>
  );
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

function validSelection(shape: Shape, selection: GraphSelection): GraphSelection {
  if (selection.kind === "graph") return selection;
  if (selection.kind === "node" && shape.graph.nodes.some((node) => node.id === selection.id)) return selection;
  if (selection.kind === "edge" && shape.graph.edges.some((edge) => edge.id === selection.id)) return selection;
  return { kind: "graph" };
}

function nextTopZ(graph: DecisionGraph, layout: GraphLayout): number {
  const values = graph.nodes.map((node, index) => layout.nodeZOrder?.[node.id] ?? index);
  return Math.max(...values, 0) + 1;
}

async function writeClipboardText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.left = "-9999px";
    textarea.style.top = "0";
    document.body.appendChild(textarea);
    textarea.select();
    try {
      return document.execCommand("copy");
    } finally {
      document.body.removeChild(textarea);
    }
  }
}
