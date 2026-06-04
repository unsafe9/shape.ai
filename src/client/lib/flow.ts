import { MarkerType, type Edge, type Node } from "@xyflow/react";
import { edgeTypeLabels } from "../../shared/graph";
import type { DecisionGraph, GraphComment, GraphEdge, GraphLayout, GraphNode } from "../../shared/schema";
import type { NodeType } from "../../shared/schema";

export type StudioNodeData = {
  node: GraphNode;
  selected: boolean;
  editing: boolean;
  commentCount: number;
  comments: GraphComment[];
  commentValue: string;
  busy: boolean;
  onUpdateNode?: (node: GraphNode) => void;
  onCommentChange?: (value: string) => void;
  onAddComment?: () => void;
  onToggleComment?: (comment: GraphComment) => void;
  onAddLinkedNode?: (type: NodeType) => void;
  onDeleteNode?: () => void;
  onStartEdit?: (nodeId: string) => void;
  onStopEdit?: () => void;
};

const columns: Record<GraphNode["type"], number> = {
  proposition: 0,
  decision_point: 1,
  option: 2,
  evidence: 3,
  tradeoff: 3,
  blocker: 3,
  subdecision: 4,
  task: 5,
  artifact: 5
};

const edgeColors: Record<GraphEdge["type"], string> = {
  depends_on: "#5f6f7a",
  supports: "#177a68",
  blocks: "#a83d4a",
  trades_off_with: "#a96a13",
  chooses_between: "#2f68a6",
  decomposes_to: "#6a5c9a",
  produces: "#27705f"
};

export function graphToFlow(
  graph: DecisionGraph,
  layout: GraphLayout | undefined,
  selectedId?: string,
  editingId?: string | null,
  comments: GraphComment[] = [],
  onUpdateNode?: (node: GraphNode) => void,
  commentValue = "",
  busy = false,
  onCommentChange?: (value: string) => void,
  onAddComment?: () => void,
  onToggleComment?: (comment: GraphComment) => void,
  onAddLinkedNode?: (type: NodeType) => void,
  onDeleteNode?: () => void,
  onStartEdit?: (nodeId: string) => void,
  onStopEdit?: () => void
): { nodes: Node<StudioNodeData>[]; edges: Edge[] } {
  const groups = new Map<number, GraphNode[]>();
  for (const node of graph.nodes) {
    const column = columns[node.type];
    groups.set(column, [...(groups.get(column) ?? []), node]);
  }

  const nodes: Node<StudioNodeData>[] = graph.nodes.map((node) => {
    const column = columns[node.type];
    const index = groups.get(column)?.findIndex((candidate) => candidate.id === node.id) ?? 0;
    const nodeComments = comments.filter((comment) => comment.target.kind === "node" && comment.target.id === node.id);
    return {
      id: node.id,
      type: "studio",
      selected: selectedId === node.id,
      position: layout?.nodePositions[node.id] ?? {
        x: 34 + column * 250,
        y: 48 + index * 126 + (column % 2) * 26
      },
      data: {
        node,
        selected: selectedId === node.id,
        editing: editingId === node.id,
        commentCount: nodeComments.filter((comment) => !comment.resolved).length,
        comments: nodeComments,
        commentValue,
        busy,
        onUpdateNode,
        onCommentChange,
        onAddComment,
        onToggleComment,
        onAddLinkedNode,
        onDeleteNode,
        onStartEdit,
        onStopEdit
      }
    };
  });

  const edges: Edge[] = graph.edges.map((edge) => ({
    id: edge.id,
    source: edge.source,
    target: edge.target,
    type: "smoothstep",
    label: edge.label || edgeTypeLabels[edge.type],
    animated: selectedId === edge.id,
    markerEnd: {
      type: MarkerType.ArrowClosed,
      color: edgeColors[edge.type]
    },
    style: {
      stroke: edgeColors[edge.type],
      strokeWidth: selectedId === edge.id ? 2.5 : 1.5
    },
    labelStyle: {
      fontSize: 11,
      fontWeight: 700,
      fill: "#35413d"
    },
    labelBgStyle: {
      fill: "#f8faf8",
      fillOpacity: 0.92
    },
    labelBgPadding: [6, 3],
    data: { edge }
  }));

  return { nodes, edges };
}
