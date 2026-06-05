import { useEffect, useRef, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import { Pencil } from "lucide-react";
import { nodeTypeLabels } from "../../shared/graph";
import type { GraphNode } from "../../shared/schema";
import type { StudioNodeData } from "../lib/flow";
import { NodeEditForm } from "./node/NodeEditForm";
import { NodeReadNote } from "./node/NodeReadNote";

export function SelectedNodeInspector({ data }: { data: StudioNodeData }) {
  const nodeData = data;
  const { node, selected, editing } = nodeData;
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
    if (event.ctrlKey || event.metaKey) return;
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
    <div
      className={`node-inspector-card node-inspector-card--${node.status} node-inspector-type--${node.type} ${selected ? "is-selected" : ""} ${editing ? "is-editing" : ""}`}
    >
      <div className="node-head">
        <span className="node-type">{nodeTypeLabels[node.type]}</span>
        {!editing ? (
          <button
            className="node-edit-icon nodrag nowheel"
            aria-label="Edit node"
            title="Edit node"
            onPointerDown={stopNodeInteraction}
            onClick={(event) => {
              event.stopPropagation();
              nodeData.onStartEdit?.(node.id);
            }}
          >
            <Pencil size={13} />
          </button>
        ) : null}
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
          onWheel={stopWheel}
        />
      )}
    </div>
  );
}
