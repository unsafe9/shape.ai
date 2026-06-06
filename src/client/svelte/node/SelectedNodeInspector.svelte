<script lang="ts">
  import { Pencil } from "lucide-svelte";
  import { nodeTypeLabels } from "../../../shared/graph";
  import type { GraphNode } from "../../../shared/schema";
  import type { StudioNodeData } from "../../lib/flow";
  import NodeEditForm from "./NodeEditForm.svelte";
  import NodeReadNote from "./NodeReadNote.svelte";

  type Props = { data: StudioNodeData };
  let { data }: Props = $props();

  const node = $derived(data.node);
  const selected = $derived(data.selected);
  const editing = $derived(data.editing);

  function stopNodeInteraction(event: PointerEvent) {
    event.stopPropagation();
  }

  function stopWheel(event: WheelEvent) {
    if (event.ctrlKey || event.metaKey) return;
    event.stopPropagation();
  }

  function updateNode(next: GraphNode) {
    data.onUpdateNode?.(next);
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

  function blurOnEnter(event: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (event.key === "Enter") {
      event.preventDefault();
      event.currentTarget.blur();
    }
  }
</script>

<div
  class="node-inspector-card node-inspector-card--{node.status} node-inspector-type--{node.type} {selected ? 'is-selected' : ''} {editing ? 'is-editing' : ''}"
>
  <div class="node-head">
    <span class="node-type">{nodeTypeLabels[node.type]}</span>
    {#if !editing}
      <button
        class="node-edit-icon nodrag nowheel"
        aria-label="Edit node"
        title="Edit node"
        onpointerdown={stopNodeInteraction}
        onclick={(event) => {
          event.stopPropagation();
          data.onStartEdit?.(node.id);
        }}
      >
        <Pencil size={13} />
      </button>
    {/if}
  </div>

  {#if editing}
    <NodeEditForm
      {node}
      comments={data.comments}
      commentValue={data.commentValue}
      busy={data.busy}
      onCommitTitle={commitTitle}
      onCommitSummary={commitSummary}
      onBlurOnEnter={blurOnEnter}
      onUpdateNode={updateNode}
      onCommentChange={data.onCommentChange}
      onAddComment={data.onAddComment}
      onToggleComment={data.onToggleComment}
      onAddLinkedNode={data.onAddLinkedNode}
      onDeleteNode={data.onDeleteNode}
      onStopEdit={data.onStopEdit}
      onPointerDown={stopNodeInteraction}
      onWheel={stopWheel}
    />
  {:else}
    <NodeReadNote {node} {selected} comments={data.comments} onWheel={stopWheel} />
  {/if}
</div>
