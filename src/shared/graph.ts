import {
  type DecisionGraph,
  type EdgeType,
  type ExportType,
  type GraphEdge,
  type GraphNode,
  type GraphPatch,
  graphPatchSchema
} from "./schema";

export const nodeTypeLabels: Record<GraphNode["type"], string> = {
  proposition: "Proposition",
  decision_point: "Decision",
  option: "Option",
  evidence: "Evidence",
  tradeoff: "Tradeoff",
  blocker: "Blocker",
  subdecision: "Subdecision",
  task: "Task",
  artifact: "Artifact"
};

export const edgeTypeLabels: Record<EdgeType, string> = {
  depends_on: "depends on",
  supports: "supports",
  blocks: "blocks",
  trades_off_with: "trades off",
  chooses_between: "chooses",
  decomposes_to: "decomposes",
  produces: "produces"
};

export const exportTypeLabels: Record<ExportType, string> = {
  ai_plan_md: "AI task plan",
  design_doc_md: "Design document",
  confluence_html: "Confluence draft",
  mermaid: "Mermaid diagram",
  architecture_image: "Architecture image"
};

export function applyGraphPatch(graph: DecisionGraph, patchInput: GraphPatch): DecisionGraph {
  const patch = graphPatchSchema.parse(patchInput);
  const nodeMap = new Map(graph.nodes.map((node) => [node.id, node]));
  const edgeMap = new Map(graph.edges.map((edge) => [edge.id, edge]));

  for (const nodeId of patch.removeNodeIds) {
    nodeMap.delete(nodeId);
  }
  for (const edgeId of patch.removeEdgeIds) {
    edgeMap.delete(edgeId);
  }
  for (const node of patch.updateNodes) {
    if (nodeMap.has(node.id)) {
      nodeMap.set(node.id, node);
    }
  }
  for (const node of patch.addNodes) {
    nodeMap.set(node.id, node);
  }

  const existingNodeIds = new Set(nodeMap.keys());
  for (const edge of patch.updateEdges) {
    if (edgeMap.has(edge.id) && existingNodeIds.has(edge.source) && existingNodeIds.has(edge.target)) {
      edgeMap.set(edge.id, edge);
    }
  }
  for (const edge of patch.addEdges) {
    if (existingNodeIds.has(edge.source) && existingNodeIds.has(edge.target)) {
      edgeMap.set(edge.id, edge);
    }
  }

  const edges = [...edgeMap.values()].filter(
    (edge) => existingNodeIds.has(edge.source) && existingNodeIds.has(edge.target)
  );

  return {
    version: 1,
    nodes: [...nodeMap.values()],
    edges
  };
}

export function graphTextDigest(graph: DecisionGraph): string {
  const nodes = graph.nodes
    .map((node) => `- ${node.id} [${node.type}/${node.status}]: ${node.title} :: ${node.summary}`)
    .join("\n");
  const edges = graph.edges
    .map((edge) => `- ${edge.id} [${edge.type}]: ${edge.source} -> ${edge.target} :: ${edge.label}`)
    .join("\n");
  return `Nodes\n${nodes || "- none"}\n\nEdges\n${edges || "- none"}`;
}

export function selectedSubgraph(graph: DecisionGraph, scope: { kind: string; id?: string }): DecisionGraph {
  if (scope.kind === "whole_graph" || !scope.id) {
    return graph;
  }

  const nodeIds = new Set<string>();
  const edgeIds = new Set<string>();

  if (scope.kind === "node") {
    nodeIds.add(scope.id);
    for (const edge of graph.edges) {
      if (edge.source === scope.id || edge.target === scope.id) {
        edgeIds.add(edge.id);
        nodeIds.add(edge.source);
        nodeIds.add(edge.target);
      }
    }
    const selected = graph.nodes.find((node) => node.id === scope.id);
    for (const childId of selected?.childDecisionIds ?? []) {
      nodeIds.add(childId);
    }
  }

  if (scope.kind === "edge") {
    const edge = graph.edges.find((candidate) => candidate.id === scope.id);
    if (edge) {
      edgeIds.add(edge.id);
      nodeIds.add(edge.source);
      nodeIds.add(edge.target);
    }
  }

  return {
    version: 1,
    nodes: graph.nodes.filter((node) => nodeIds.has(node.id)),
    edges: graph.edges.filter(
      (edge) => edgeIds.has(edge.id) || (nodeIds.has(edge.source) && nodeIds.has(edge.target))
    )
  };
}

export function makeMermaid(graph: DecisionGraph): string {
  const lines = ["flowchart LR"];
  for (const node of graph.nodes) {
    const safeTitle = node.title.replace(/"/g, "'");
    lines.push(`  ${safeMermaidId(node.id)}["${safeTitle}<br/>${node.type}"]`);
  }
  for (const edge of graph.edges) {
    const label = edge.label || edgeTypeLabels[edge.type];
    lines.push(`  ${safeMermaidId(edge.source)} -->|"${label.replace(/"/g, "'")}" ${safeMermaidId(edge.target)}`);
  }
  return lines.join("\n");
}

function safeMermaidId(id: string): string {
  return id.replace(/[^a-zA-Z0-9_]/g, "_");
}
