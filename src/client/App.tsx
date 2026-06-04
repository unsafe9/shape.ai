import { useCallback, useEffect, useMemo, useState } from "react";
import type { MouseEvent } from "react";
import { Background, Controls, ReactFlow, type Connection, type Edge, type Node, type ReactFlowInstance } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { BrainCircuit, Loader2, PanelLeft, Plug, RefreshCw } from "lucide-react";
import {
  createComment,
  createDesign,
  exportDesign,
  getRuntime,
  listDesigns,
  saveGraphEdit,
  updateComment,
  type RuntimeStatus
} from "./lib/api";
import { graphToFlow, type StudioNodeData } from "./lib/flow";
import { DecisionNode } from "./components/DecisionNode";
import { Sidebar } from "./components/Sidebar";
import { ExportDrawer } from "./components/ExportDrawer";
import type {
  DecisionGraph,
  Design,
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

const seedPrompt =
  "Design an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

export default function App() {
  const [designs, setDesigns] = useState<Design[]>([]);
  const [activeDesign, setActiveDesign] = useState<Design | null>(null);
  const [runtime, setRuntime] = useState<RuntimeStatus | null>(null);
  const [prompt, setPrompt] = useState(seedPrompt);
  const [selection, setSelection] = useState<GraphSelection>({ kind: "graph" });
  const [commentValue, setCommentValue] = useState("");
  const [status, setStatus] = useState("Ready");
  const [busy, setBusy] = useState(false);
  const [designPanelOpen, setDesignPanelOpen] = useState(false);
  const [editingNodeId, setEditingNodeId] = useState<string | null>(null);
  const [flowInstance, setFlowInstance] = useState<ReactFlowInstance<Node<StudioNodeData>, Edge> | null>(null);

  const flow = useMemo(() => {
    if (!activeDesign) return { nodes: [] as Node<StudioNodeData>[], edges: [] as Edge[] };
    return graphToFlow(
      activeDesign.graph,
      activeDesign.layout,
      selection.kind === "graph" ? undefined : selection.id,
      editingNodeId,
      activeDesign.comments,
      updateNode,
      commentValue,
      busy || !activeDesign,
      setCommentValue,
      runAddComment,
      toggleComment,
      addLinkedNode,
      deleteSelection,
      setEditingNodeId,
      () => setEditingNodeId(null)
    );
  }, [activeDesign, selection, editingNodeId, commentValue, busy]);

  const refreshDesigns = useCallback(async () => {
    const [nextDesigns, nextRuntime] = await Promise.all([listDesigns(), getRuntime()]);
    const nextActive = activeDesign ? nextDesigns.find((design) => design.id === activeDesign.id) ?? activeDesign : nextDesigns[0] ?? null;
    setDesigns(nextDesigns);
    setRuntime(nextRuntime);
    setActiveDesign(nextActive);
    setSelection(nextActive ? validSelection(nextActive, nextActive.selection) : { kind: "graph" });
  }, [activeDesign]);

  useEffect(() => {
    refreshDesigns().catch((error) => setStatus(error.message));
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
      if (selection.kind !== "node") return;

      if (event.key === "Escape") {
        event.preventDefault();
        if (editingNodeId) {
          setEditingNodeId(null);
          return;
        }
        resetViewport();
        void selectGraphItem({ kind: "graph" });
        return;
      }

      if (target && ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)) return;

      if (event.key.toLowerCase() === "e" && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        setEditingNodeId(selection.id);
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [selection, editingNodeId, flowInstance]);

  async function runCreate() {
    await withBusy("Creating design", async () => {
      const response = await createDesign(prompt);
      updateDesign(response.design);
      setDesignPanelOpen(false);
      setEditingNodeId(null);
      setCommentValue("");
      setStatus(response.message);
    });
  }

  async function runExport(type: ExportType) {
    if (!activeDesign) return;
    await withBusy(`Exporting ${type}`, async () => {
      const scope = selection.kind === "graph" ? { kind: "whole_graph" as const } : selection;
      const response = await exportDesign(activeDesign.id, type, scope);
      updateDesign(response.design);
      setStatus(`Export created: ${response.artifact.title}`);
    });
  }

  async function runAddComment() {
    if (!activeDesign || !commentValue.trim()) return;
    await withBusy("Adding comment", async () => {
      const response = await createComment(activeDesign.id, {
        target: selection,
        body: commentValue.trim()
      });
      updateDesign(response.design);
      setCommentValue("");
    });
  }

  async function toggleComment(comment: GraphComment) {
    if (!activeDesign) return;
    await withBusy("Updating comment", async () => {
      const response = await updateComment(activeDesign.id, comment.id, { resolved: !comment.resolved });
      updateDesign(response.design);
    });
  }

  function updateDesign(design: Design) {
    setActiveDesign(design);
    setSelection(validSelection(design, design.selection));
    setDesigns((current) => [design, ...current.filter((candidate) => candidate.id !== design.id)]);
  }

  async function saveGraph(graph: DecisionGraph, layout = activeDesign?.layout, nextSelection = selection) {
    if (!activeDesign) return;
    const optimistic = { ...activeDesign, graph, layout: layout ?? activeDesign.layout, selection: nextSelection };
    updateDesign(optimistic);
    try {
      const response = await saveGraphEdit(activeDesign.id, { graph, layout, selection: nextSelection });
      updateDesign(response.design);
      setStatus("Saved");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Save failed");
    }
  }

  async function saveLayout(layout: GraphLayout) {
    if (!activeDesign) return;
    updateDesign({ ...activeDesign, layout });
    try {
      const response = await saveGraphEdit(activeDesign.id, { layout });
      updateDesign(response.design);
      setStatus("Layout saved");
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Layout save failed");
    }
  }

  async function selectGraphItem(nextSelection: GraphSelection) {
    if (!activeDesign) return;
    const valid = validSelection(activeDesign, nextSelection);
    if (valid.kind !== "node") {
      setEditingNodeId(null);
    }
    setSelection(valid);
    setActiveDesign({ ...activeDesign, selection: valid });
    try {
      const response = await saveGraphEdit(activeDesign.id, { selection: valid });
      updateDesign(response.design);
    } catch (error) {
      setStatus(error instanceof Error ? error.message : "Selection save failed");
    }
  }

  function updateNode(node: GraphNode) {
    if (!activeDesign) return;
    const graph = {
      ...activeDesign.graph,
      nodes: activeDesign.graph.nodes.map((candidate) => (candidate.id === node.id ? node : candidate))
    };
    void saveGraph(graph);
  }

  function deleteSelection() {
    if (!activeDesign || selection.kind === "graph") return;
    const graph =
      selection.kind === "node"
        ? {
            ...activeDesign.graph,
            nodes: activeDesign.graph.nodes.filter((node) => node.id !== selection.id),
            edges: activeDesign.graph.edges.filter((edge) => edge.source !== selection.id && edge.target !== selection.id)
          }
        : {
            ...activeDesign.graph,
            edges: activeDesign.graph.edges.filter((edge) => edge.id !== selection.id)
          };
    void saveGraph(graph, activeDesign.layout, { kind: "graph" });
  }

  function addLinkedNode(type: NodeType) {
    if (!activeDesign) return;
    const sourceId = selection.kind === "node" ? selection.id : activeDesign.graph.nodes[0]?.id;
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
      ...activeDesign.graph,
      nodes: [...activeDesign.graph.nodes, node],
      edges: edge ? [...activeDesign.graph.edges, edge] : activeDesign.graph.edges
    };
    const basePosition =
      sourceId && activeDesign.layout.nodePositions[sourceId]
        ? activeDesign.layout.nodePositions[sourceId]
        : { x: 120, y: 120 };
    const layout = {
      ...activeDesign.layout,
      nodePositions: {
        ...activeDesign.layout.nodePositions,
        [id]: { x: basePosition.x + 260, y: basePosition.y + 84 }
      }
    };
    void saveGraph(graph, layout, { kind: "node", id });
  }

  function connectNodes(connection: Connection) {
    if (!activeDesign || !connection.source || !connection.target || connection.source === connection.target) return;
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
      ...activeDesign.graph,
      edges: [...activeDesign.graph.edges, edge]
    };
    void saveGraph(graph, activeDesign.layout, { kind: "edge", id: edge.id });
  }

  function moveNode(node: Node) {
    if (!activeDesign) return;
    void saveLayout({
      ...activeDesign.layout,
      nodePositions: {
        ...activeDesign.layout.nodePositions,
        [node.id]: { x: node.position.x, y: node.position.y }
      }
    });
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

  return (
    <div className="app-shell">
      <main className="studio-stage">
        <header className="floating-commandbar">
          <div className="brand">
            <BrainCircuit size={24} />
            <div>
              <h1>shape.ai</h1>
              <p>Visual decision design for humans and AI agents.</p>
            </div>
          </div>
          <div className="command-separator" />
          <div className="runtime-chip is-configured">
            <Plug size={14} />
            {runtime ? `${runtime.mcp.remoteTransport ?? runtime.mcp.transport ?? "MCP"} MCP` : "MCP"}
          </div>
          <button className="secondary-button" disabled={busy} onClick={() => refreshDesigns().catch((error) => setStatus(error.message))}>
            <RefreshCw size={15} />
            Refresh
          </button>
          <div className="status-line">
            {busy ? <Loader2 className="spin" size={15} /> : null}
            {status}
          </div>
        </header>

        <div className="floating-view-controls" aria-label="Workspace panels">
          <button
            className={`icon-button ${designPanelOpen ? "is-active" : ""}`}
            onClick={() => setDesignPanelOpen((open) => !open)}
            aria-label="Toggle designs"
          >
            <PanelLeft size={16} />
          </button>
        </div>

        <section className={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
          <div className="canvas-header">
            <div>
              <h2>{activeDesign?.title ?? "No design selected"}</h2>
              <p>{activeDesign ? `${activeDesign.graph.nodes.length} nodes, ${activeDesign.graph.edges.length} edges` : "Create a design to start."}</p>
            </div>
          </div>
          <div className="flow-wrap">
            {activeDesign ? (
              <ReactFlow
                nodes={flow.nodes}
                edges={flow.edges}
                nodeTypes={nodeTypes}
                fitView
                fitViewOptions={{ padding: 0.24, duration: 500, ease: viewportEase, interpolate: "smooth" }}
                minZoom={0.18}
                maxZoom={1.35}
                panOnDrag
                selectionOnDrag={false}
                onInit={setFlowInstance}
                onNodeClick={(event: MouseEvent, node: Node) => {
                  focusNode(node);
                  if (event.altKey) {
                    setEditingNodeId(node.id);
                  } else if (editingNodeId && editingNodeId !== node.id) {
                    setEditingNodeId(null);
                  }
                  void selectGraphItem({ kind: "node", id: node.id });
                }}
                onEdgeClick={(_event: MouseEvent, edge: Edge) => {
                  setEditingNodeId(null);
                  focusEdge(edge);
                  void selectGraphItem({ kind: "edge", id: edge.id });
                }}
                onNodeDragStop={(_event, node) => moveNode(node)}
                onConnect={connectNodes}
                onPaneClick={() => {
                  setEditingNodeId(null);
                  resetViewport();
                  void selectGraphItem({ kind: "graph" });
                }}
              >
                <Background color="#d6dde2" gap={26} />
                <Controls position="bottom-left" />
              </ReactFlow>
            ) : (
              <div className="empty-canvas">
                <h2>Create a design graph</h2>
                <p>Start with a proposition, architecture concern, or implementation plan that needs sharper decisions.</p>
              </div>
            )}
          </div>

          <div className={`floating-designs ${designPanelOpen ? "is-open" : "is-closed"}`}>
            <Sidebar
              designs={designs}
              activeDesignId={activeDesign?.id}
              prompt={prompt}
              busy={busy}
              onPromptChange={setPrompt}
              onCreate={runCreate}
              onSelect={(design) => {
                setActiveDesign(design);
                setSelection(validSelection(design, design.selection));
                setEditingNodeId(null);
                setDesignPanelOpen(false);
              }}
              onRefresh={() => refreshDesigns().catch((error) => setStatus(error.message))}
            />
          </div>

          <ExportDrawer
            artifacts={activeDesign?.artifacts ?? []}
            busy={busy}
            designId={activeDesign?.id}
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

function validSelection(design: Design, selection: GraphSelection): GraphSelection {
  if (selection.kind === "graph") return selection;
  if (selection.kind === "node" && design.graph.nodes.some((node) => node.id === selection.id)) return selection;
  if (selection.kind === "edge" && design.graph.edges.some((edge) => edge.id === selection.id)) return selection;
  return { kind: "graph" };
}
