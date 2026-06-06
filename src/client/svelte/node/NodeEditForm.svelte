<script lang="ts">
  import { Check, Plus, Trash2 } from "lucide-svelte";
  import { nodeTypeLabels } from "../../../shared/graph";
  import type { GraphComment, GraphNode, NodeStatus, NodeType } from "../../../shared/schema";
  import { editableNodeTypes, editableStatuses } from "../../components/node/options";
  import NodeComments from "./NodeComments.svelte";

  type Props = {
    node: GraphNode;
    comments: GraphComment[];
    commentValue: string;
    busy: boolean;
    onCommitTitle: (target: HTMLInputElement) => void;
    onCommitSummary: (target: HTMLTextAreaElement) => void;
    onBlurOnEnter: (event: KeyboardEvent & { currentTarget: HTMLInputElement }) => void;
    onUpdateNode: (node: GraphNode) => void;
    onCommentChange?: (value: string) => void;
    onAddComment?: () => void;
    onToggleComment?: (comment: GraphComment) => void;
    onAddLinkedNode?: (type: NodeType) => void;
    onDeleteNode?: () => void;
    onStopEdit?: () => void;
    onPointerDown: (event: PointerEvent) => void;
    onWheel: (event: WheelEvent) => void;
  };

  let {
    node,
    comments,
    commentValue,
    busy,
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
  }: Props = $props();

  // Mirror App.tsx's titleInputRef focus/select effect: when the edit form mounts
  // (or the edited node changes), focus and select the title input.
  function autoFocus(element: HTMLInputElement) {
    element.focus();
    element.select();
  }
</script>

<div
  class="node-expanded-editor nodrag nowheel"
  onpointerdown={onPointerDown}
  onwheel={onWheel}
  onclick={(event) => event.stopPropagation()}
  onkeydown={() => {}}
  role="presentation"
>
  <div class="node-edit-toolbar">
    <span>Editing note</span>
    <button onclick={onStopEdit}>
      <Check size={13} />
      Done
    </button>
  </div>

  {#key node.id}
    <input
      use:autoFocus
      class="node-title-input"
      aria-label="Node title"
      value={node.title}
      onblur={(event) => onCommitTitle(event.currentTarget)}
      onkeydown={onBlurOnEnter}
    />
  {/key}
  {#key `${node.id}-summary-${node.summary}`}
    <textarea
      class="node-summary-input"
      aria-label="Node summary"
      value={node.summary}
      onblur={(event) => onCommitSummary(event.currentTarget)}
    ></textarea>
  {/key}

  <div class="node-inline-grid">
    <label>
      <span>Type</span>
      <select value={node.type} onchange={(event) => onUpdateNode({ ...node, type: event.currentTarget.value as NodeType })}>
        {#each editableNodeTypes as type (type)}
          <option value={type}>{nodeTypeLabels[type]}</option>
        {/each}
      </select>
    </label>
    <label>
      <span>Status</span>
      <select value={node.status} onchange={(event) => onUpdateNode({ ...node, status: event.currentTarget.value as NodeStatus })}>
        {#each editableStatuses as status (status)}
          <option value={status}>{status}</option>
        {/each}
      </select>
    </label>
  </div>

  <label class="node-detail-field">
    <span>Detail</span>
    {#key `${node.id}-detail-${node.detail}`}
      <textarea
        value={node.detail}
        onblur={(event) => onUpdateNode({ ...node, detail: event.currentTarget.value.trim() })}
      ></textarea>
    {/key}
  </label>

  <label class="node-detail-field">
    <span>Evidence refs</span>
    {#key `${node.id}-refs-${node.evidenceRefs.join("|")}`}
      <input
        value={node.evidenceRefs.join(", ")}
        onblur={(event) =>
          onUpdateNode({
            ...node,
            evidenceRefs: event.currentTarget.value
              .split(",")
              .map((value) => value.trim())
              .filter(Boolean)
          })}
      />
    {/key}
  </label>

  <div class="node-inline-actions">
    <button onclick={() => onAddLinkedNode?.("option")}>
      <Plus size={12} />
      Option
    </button>
    <button onclick={() => onAddLinkedNode?.("evidence")}>
      <Plus size={12} />
      Evidence
    </button>
    <button onclick={() => onAddLinkedNode?.("blocker")}>
      <Plus size={12} />
      Blocker
    </button>
    <button class="danger-button" onclick={() => onDeleteNode?.()}>
      <Trash2 size={12} />
      Delete
    </button>
  </div>

  <NodeComments
    {comments}
    {commentValue}
    {busy}
    {onCommentChange}
    {onAddComment}
    {onToggleComment}
  />
</div>
