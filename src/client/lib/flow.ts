import type { GraphComment, GraphNode, NodeType } from "../../shared/schema";

export type StudioNodeData = {
  node: GraphNode;
  selected: boolean;
  editing: boolean;
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
