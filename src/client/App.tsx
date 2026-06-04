import { useCallback, useEffect, useMemo, useState } from "react";
import type { MouseEvent } from "react";
import { Background, Controls, ReactFlow, type Connection, type Edge, type Node } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { BrainCircuit, Loader2, Plug, RefreshCw } from "lucide-react";
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
import { Inspector } from "./components/Inspector";
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

  const flow = useMemo(() => {
    if (!activeDesign) return { nodes: [] as Node<StudioNodeData>[], edges: [] as Edge[] };
    return graphToFlow(
      activeDesign.graph,
      activeDesign.layout,
      selection.kind === "graph" ? undefined : selection.id,
      activeDesign.comments
    );
  }, [activeDesign, selection]);

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

  async function runCreate() {
    await withBusy("Creating design", async () => {
      const response = await createDesign(prompt);
      updateDesign(response.design);
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

  function updateEdge(edge: GraphEdge) {
    if (!activeDesign) return;
    const graph = {
      ...activeDesign.graph,
      edges: activeDesign.graph.edges.map((candidate) => (candidate.id === edge.id ? edge : candidate))
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
      <header className="topbar">
        <div className="brand">
          <BrainCircuit size={24} />
          <div>
            <h1>Charrette</h1>
            <p>Visual decision design for humans and AI agents.</p>
          </div>
        </div>
        <div className="topbar-actions">
          <div className="runtime-chip is-configured">
            <Plug size={14} />
            {runtime ? `${runtime.mcp.transport} MCP` : "MCP"}
          </div>
          <button className="secondary-button" disabled={busy} onClick={() => refreshDesigns().catch((error) => setStatus(error.message))}>
            <RefreshCw size={15} />
            Refresh
          </button>
          <div className="status-line">
            {busy ? <Loader2 className="spin" size={15} /> : null}
            {status}
          </div>
        </div>
      </header>

      <main className="studio-grid">
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
          }}
          onRefresh={() => refreshDesigns().catch((error) => setStatus(error.message))}
        />

        <section className="canvas-panel">
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
                minZoom={0.18}
                maxZoom={1.35}
                onNodeClick={(_event: MouseEvent, node: Node) => void selectGraphItem({ kind: "node", id: node.id })}
                onEdgeClick={(_event: MouseEvent, edge: Edge) => void selectGraphItem({ kind: "edge", id: edge.id })}
                onNodeDragStop={(_event, node) => moveNode(node)}
                onConnect={connectNodes}
                onPaneClick={() => void selectGraphItem({ kind: "graph" })}
              >
                <Background color="#ccd8d2" gap={24} />
                <Controls position="bottom-left" />
              </ReactFlow>
            ) : (
              <div className="empty-canvas">
                <h2>Create a design graph</h2>
                <p>Start with a proposition, architecture concern, or implementation plan that needs sharper decisions.</p>
              </div>
            )}
          </div>
          <ExportDrawer
            artifacts={activeDesign?.artifacts ?? []}
            busy={busy}
            designId={activeDesign?.id}
            onExport={runExport}
          />
        </section>

        <Inspector
          graph={activeDesign?.graph ?? { version: 1, nodes: [], edges: [] }}
          selection={selection}
          comments={activeDesign?.comments ?? []}
          commentValue={commentValue}
          busy={busy || !activeDesign}
          onCommentChange={setCommentValue}
          onAddComment={runAddComment}
          onToggleComment={toggleComment}
          onUpdateNode={updateNode}
          onUpdateEdge={updateEdge}
          onDeleteSelection={deleteSelection}
          onAddLinkedNode={addLinkedNode}
        />
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
