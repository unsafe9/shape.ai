import type { NodeStatus, NodeType } from "../../../shared/schema";

export const editableNodeTypes: NodeType[] = [
  "proposition",
  "decision_point",
  "option",
  "evidence",
  "tradeoff",
  "blocker",
  "subdecision",
  "task",
  "artifact"
];

export const editableStatuses: NodeStatus[] = ["draft", "viable", "conditional", "infeasible", "unknown", "selected", "deferred", "complete"];
