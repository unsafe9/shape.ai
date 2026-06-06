import {
  decisionGraphSchema,
  graphPatchSchema,
  type Bounds,
  type DecisionGraph,
  type EdgeType,
  type ExportType,
  type GraphEdge,
  type GraphNode,
  type NodeType,
  type Scene,
  type SceneEdge,
  type SceneGroup,
  type SceneNode,
  type SceneSelection,
  type Tag
} from "./schema";

export const nodeTypeLabels: Record<NodeType, string> = {
  proposition: "Proposition",
  decision_point: "Decision point",
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
  madr: "MADR Markdown",
  yadr: "YADR YAML",
  image_prompt: "Image prompt",
  ai_plan_md: "AI task plan",
  design_doc_md: "MADR Markdown",
  confluence_html: "Confluence draft",
  mermaid: "Mermaid diagram",
  architecture_image: "Image prompt"
};

export function applyGraphPatch(graph: DecisionGraph, patchInput: unknown): DecisionGraph {
  const patch = graphPatchSchema.parse(patchInput);
  const nodeMap = new Map(graph.nodes.map((node) => [node.id, node]));
  const edgeMap = new Map(graph.edges.map((edge) => [edge.id, edge]));

  for (const nodeId of patch.removeNodeIds) {
    nodeMap.delete(nodeId);
    for (const [edgeId, edge] of edgeMap) {
      if (edge.source === nodeId || edge.target === nodeId) edgeMap.delete(edgeId);
    }
  }
  for (const edgeId of patch.removeEdgeIds) edgeMap.delete(edgeId);
  for (const node of patch.addNodes) nodeMap.set(node.id, node);
  for (const node of patch.updateNodes) nodeMap.set(node.id, node);
  for (const edge of patch.addEdges) edgeMap.set(edge.id, edge);
  for (const edge of patch.updateEdges) edgeMap.set(edge.id, edge);

  return decisionGraphSchema.parse({
    version: 1,
    nodes: Array.from(nodeMap.values()),
    edges: Array.from(edgeMap.values()).filter((edge) => nodeMap.has(edge.source) && nodeMap.has(edge.target))
  });
}

export function graphTextDigest(graph: DecisionGraph): string {
  const nodes = graph.nodes
    .map((node) => `- [${node.type}/${node.status}/${Math.round(node.confidence * 100)}%] ${node.id}: ${node.title} — ${node.summary}`)
    .join("\n");
  const edges = graph.edges
    .map((edge) => `- [${edge.type}/${Math.round(edge.confidence * 100)}%] ${edge.source} -> ${edge.target}: ${edge.label || edge.rationale}`)
    .join("\n");
  return [`Nodes:`, nodes || "- none", "", "Edges:", edges || "- none"].join("\n");
}

export function selectedSubgraph(graph: DecisionGraph, scope: { kind: string; id?: string }): DecisionGraph {
  if (scope.kind === "group" || scope.kind === "selection" || !scope.id) {
    return graph;
  }

  if (scope.kind === "node") {
    const nodeIds = new Set<string>([scope.id]);
    for (const edge of graph.edges) {
      if (edge.source === scope.id) nodeIds.add(edge.target);
      if (edge.target === scope.id) nodeIds.add(edge.source);
    }
    return pickSubgraph(graph, nodeIds);
  }

  if (scope.kind === "edge") {
    const edge = graph.edges.find((candidate) => candidate.id === scope.id);
    if (!edge) return { version: 1, nodes: [], edges: [] };
    return pickSubgraph(graph, new Set([edge.source, edge.target]), new Set([edge.id]));
  }

  return graph;
}

export function makeMermaid(graph: DecisionGraph): string {
  const lines = ["flowchart LR"];
  for (const node of graph.nodes) {
    lines.push(`  ${safeMermaidId(node.id)}["${escapeMermaid(`${nodeTypeLabels[node.type]}: ${node.title}`)}"]`);
  }
  for (const edge of graph.edges) {
    const label = edge.label || edgeTypeLabels[edge.type];
    lines.push(`  ${safeMermaidId(edge.source)} -->|"${escapeMermaid(label)}"| ${safeMermaidId(edge.target)}`);
  }
  return lines.join("\n");
}

export function sceneGraphForGroup(scene: Scene, groupId: string): DecisionGraph {
  const groupIds = descendantGroupIds(scene.groups, groupId);
  const nodes = scene.nodes.filter((node) => groupIds.has(node.groupId)).map(stripSceneNode);
  const nodeIds = new Set(nodes.map((node) => node.id));
  const edges = scene.edges
    .filter((edge) => groupIds.has(edge.groupId) && nodeIds.has(edge.source) && nodeIds.has(edge.target))
    .map(stripSceneEdge);
  return { version: 1, nodes, edges };
}

export function boundsIntersect(a: Bounds, b: Bounds): boolean {
  return a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y;
}

export function pointInBounds(x: number, y: number, bounds: Bounds): boolean {
  return x >= bounds.x && x <= bounds.x + bounds.width && y >= bounds.y && y <= bounds.y + bounds.height;
}

export function nodeBounds(node: SceneNode): Bounds {
  return {
    x: node.position.x,
    y: node.position.y,
    width: node.size.width,
    height: node.size.height
  };
}

export function expandedBounds(bounds: Bounds, padding: number): Bounds {
  return {
    x: bounds.x - padding,
    y: bounds.y - padding,
    width: bounds.width + padding * 2,
    height: bounds.height + padding * 2
  };
}

export function groupTags(group: SceneGroup, tags: Tag[]): Tag[] {
  const byId = new Map(tags.map((tag) => [tag.id, tag]));
  return group.tagIds.map((tagId) => byId.get(tagId)).filter((tag): tag is Tag => Boolean(tag));
}

export function selectionTarget(selection: SceneSelection): string {
  if (selection.kind === "canvas") return "canvas";
  if (selection.kind === "multi") return `multi:${selection.ids.join(",")}`;
  return `${selection.kind}:${selection.id}`;
}

function pickSubgraph(graph: DecisionGraph, nodeIds: Set<string>, edgeIds?: Set<string>): DecisionGraph {
  return {
    version: 1,
    nodes: graph.nodes.filter((node) => nodeIds.has(node.id)),
    edges: graph.edges.filter(
      (edge) => nodeIds.has(edge.source) && nodeIds.has(edge.target) && (!edgeIds || edgeIds.has(edge.id))
    )
  };
}

function stripSceneNode(node: SceneNode): GraphNode {
  const { groupId: _groupId, position: _position, size: _size, zIndex: _zIndex, updatedAt: _updatedAt, ...graphNode } = node;
  return graphNode;
}

function stripSceneEdge(edge: SceneEdge): GraphEdge {
  const { groupId: _groupId, updatedAt: _updatedAt, ...graphEdge } = edge;
  return graphEdge;
}

function descendantGroupIds(groups: SceneGroup[], rootGroupId: string): Set<string> {
  const result = new Set<string>([rootGroupId]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const group of groups) {
      if (group.parentGroupId && result.has(group.parentGroupId) && !result.has(group.id)) {
        result.add(group.id);
        changed = true;
      }
    }
  }
  return result;
}

function safeMermaidId(id: string): string {
  return id.replace(/[^a-zA-Z0-9_]/g, "_");
}

function escapeMermaid(value: string): string {
  return value.replace(/"/g, "'");
}
