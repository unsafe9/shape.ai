import type { KeyboardEvent, PointerEvent, RefObject, WheelEvent } from "react";
import { Check, Plus, Trash2 } from "lucide-react";
import { nodeTypeLabels } from "../../../shared/graph";
import type { GraphComment, GraphNode, NodeStatus, NodeType } from "../../../shared/schema";
import { NodeComments } from "./NodeComments";
import { editableNodeTypes, editableStatuses } from "./options";

export function NodeEditForm({
  node,
  comments,
  commentValue,
  busy,
  titleInputRef,
  onCommitTitle,
  onCommitSummary,
  onBlurOnEnter,
  onUpdateNode,
  onCommentChange,
  onAddComment,
  onToggleComment,
  onAddLinkedNode,
  onDeleteNode,
  onStopEdit,
  onPointerDown,
  onWheel
}: {
  node: GraphNode;
  comments: GraphComment[];
  commentValue: string;
  busy: boolean;
  titleInputRef: RefObject<HTMLInputElement | null>;
  onCommitTitle: (target: HTMLInputElement) => void;
  onCommitSummary: (target: HTMLTextAreaElement) => void;
  onBlurOnEnter: (event: KeyboardEvent<HTMLInputElement>) => void;
  onUpdateNode: (node: GraphNode) => void;
  onCommentChange?: (value: string) => void;
  onAddComment?: () => void;
  onToggleComment?: (comment: GraphComment) => void;
  onAddLinkedNode?: (type: NodeType) => void;
  onDeleteNode?: () => void;
  onStopEdit?: () => void;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
  onWheel: (event: WheelEvent<HTMLElement>) => void;
}) {
  return (
    <div className="node-expanded-editor nodrag nowheel" onPointerDown={onPointerDown} onWheel={onWheel} onClick={(event) => event.stopPropagation()}>
      <div className="node-edit-toolbar">
        <span>Editing note</span>
        <button onClick={onStopEdit}>
          <Check size={13} />
          Done
        </button>
      </div>

      <input
        key={`${node.id}-title-${node.title}`}
        ref={titleInputRef}
        className="node-title-input"
        aria-label="Node title"
        defaultValue={node.title}
        onBlur={(event) => onCommitTitle(event.currentTarget)}
        onKeyDown={onBlurOnEnter}
      />
      <textarea
        key={`${node.id}-summary-${node.summary}`}
        className="node-summary-input"
        aria-label="Node summary"
        defaultValue={node.summary}
        onBlur={(event) => onCommitSummary(event.currentTarget)}
      />

      <div className="node-inline-grid">
        <label>
          <span>Type</span>
          <select value={node.type} onChange={(event) => onUpdateNode({ ...node, type: event.currentTarget.value as NodeType })}>
            {editableNodeTypes.map((type) => (
              <option key={type} value={type}>
                {nodeTypeLabels[type]}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Status</span>
          <select value={node.status} onChange={(event) => onUpdateNode({ ...node, status: event.currentTarget.value as NodeStatus })}>
            {editableStatuses.map((status) => (
              <option key={status} value={status}>
                {status}
              </option>
            ))}
          </select>
        </label>
      </div>

      <label className="node-detail-field">
        <span>Detail</span>
        <textarea
          key={`${node.id}-detail-${node.detail}`}
          defaultValue={node.detail}
          onBlur={(event) => onUpdateNode({ ...node, detail: event.currentTarget.value.trim() })}
        />
      </label>

      <label className="node-detail-field">
        <span>Evidence refs</span>
        <input
          key={`${node.id}-refs-${node.evidenceRefs.join("|")}`}
          defaultValue={node.evidenceRefs.join(", ")}
          onBlur={(event) =>
            onUpdateNode({
              ...node,
              evidenceRefs: event.currentTarget.value
                .split(",")
                .map((value) => value.trim())
                .filter(Boolean)
            })
          }
        />
      </label>

      <div className="node-inline-actions">
        <button onClick={() => onAddLinkedNode?.("option")}>
          <Plus size={12} />
          Option
        </button>
        <button onClick={() => onAddLinkedNode?.("evidence")}>
          <Plus size={12} />
          Evidence
        </button>
        <button onClick={() => onAddLinkedNode?.("blocker")}>
          <Plus size={12} />
          Blocker
        </button>
        <button className="danger-button" onClick={() => onDeleteNode?.()}>
          <Trash2 size={12} />
          Delete
        </button>
      </div>

      <NodeComments
        comments={comments}
        commentValue={commentValue}
        busy={busy}
        onCommentChange={onCommentChange}
        onAddComment={onAddComment}
        onToggleComment={onToggleComment}
      />
    </div>
  );
}
