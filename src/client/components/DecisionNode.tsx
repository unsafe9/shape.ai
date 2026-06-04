import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import { nodeTypeLabels } from "../../shared/graph";
import type { GraphNode } from "../../shared/schema";
import type { StudioNodeData } from "../lib/flow";
import { NodeEditForm } from "./node/NodeEditForm";
import { NodeReadNote } from "./node/NodeReadNote";

export function DecisionNode({ data }: NodeProps) {
  const nodeData = data as StudioNodeData;
  const { node, selected, editing, commentCount } = nodeData;
  const titleInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!editing) return;
    titleInputRef.current?.focus();
    titleInputRef.current?.select();
  }, [editing, node.id]);

  function stopNodeInteraction(event: PointerEvent<HTMLElement>) {
    event.stopPropagation();
  }

  function stopWheel(event: WheelEvent<HTMLElement>) {
    event.stopPropagation();
  }

  function updateNode(next: GraphNode) {
    nodeData.onUpdateNode?.(next);
  }

  function commitTitle(target: HTMLInputElement) {
    const title = target.value.trim() || node.title;
    target.value = title;
    if (title !== node.title) {
      updateNode({ ...node, title });
    }
  }

  function commitSummary(target: HTMLTextAreaElement) {
    const summary = target.value.trim();
    target.value = summary;
    if (summary !== node.summary) {
      updateNode({ ...node, summary });
    }
  }

  function blurOnEnter(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === "Enter") {
      event.preventDefault();
      event.currentTarget.blur();
    }
  }

  return (
    <div className={`decision-node decision-node--${node.status} ${selected ? "is-selected" : ""} ${editing ? "is-editing" : ""}`}>
      <Handle type="target" position={Position.Left} />
      <div className="node-head">
        <span className="node-type">{nodeTypeLabels[node.type]}</span>
        <span className="node-confidence">
          {commentCount > 0 ? `${commentCount} comments` : `${Math.round(node.confidence * 100)}%`}
        </span>
      </div>

      {editing ? (
        <NodeEditForm
          node={node}
          comments={nodeData.comments}
          commentValue={nodeData.commentValue}
          busy={nodeData.busy}
          titleInputRef={titleInputRef}
          onCommitTitle={commitTitle}
          onCommitSummary={commitSummary}
          onBlurOnEnter={blurOnEnter}
          onUpdateNode={updateNode}
          onCommentChange={nodeData.onCommentChange}
          onAddComment={nodeData.onAddComment}
          onToggleComment={nodeData.onToggleComment}
          onAddLinkedNode={nodeData.onAddLinkedNode}
          onDeleteNode={nodeData.onDeleteNode}
          onStopEdit={nodeData.onStopEdit}
          onPointerDown={stopNodeInteraction}
          onWheel={stopWheel}
        />
      ) : (
        <NodeReadNote
          node={node}
          selected={selected}
          comments={nodeData.comments}
          onStartEdit={() => nodeData.onStartEdit?.(node.id)}
          onWheel={stopWheel}
          onPointerDown={stopNodeInteraction}
        />
      )}

      <Handle type="source" position={Position.Right} />
    </div>
  );
}
