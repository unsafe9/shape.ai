import { Handle, Position, type NodeProps } from "@xyflow/react";
import { nodeTypeLabels } from "../../shared/graph";
import type { StudioNodeData } from "../lib/flow";

export function DecisionNode({ data }: NodeProps) {
  const nodeData = data as StudioNodeData;
  const { node, selected, commentCount } = nodeData;
  return (
    <div className={`decision-node decision-node--${node.status} ${selected ? "is-selected" : ""}`}>
      <Handle type="target" position={Position.Left} />
      <div className="node-head">
        <span className="node-type">{nodeTypeLabels[node.type]}</span>
        <span className="node-confidence">
          {commentCount > 0 ? `${commentCount} comments` : `${Math.round(node.confidence * 100)}%`}
        </span>
      </div>
      <strong>{node.title}</strong>
      <p>{node.summary}</p>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}
