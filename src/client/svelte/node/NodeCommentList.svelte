<script lang="ts">
  import { CircleCheckBig, Circle } from "lucide-svelte";
  import type { GraphComment } from "../../../shared/schema";

  type Props = {
    comments: GraphComment[];
    interactive?: boolean;
    onToggleComment?: (comment: GraphComment) => void;
  };

  let { comments, interactive = false, onToggleComment }: Props = $props();
</script>

<div class="node-comment-list">
  {#each comments as comment (comment.id)}
    {#if interactive}
      <button
        class="node-comment-row {comment.resolved ? 'is-resolved' : ''}"
        onclick={() => onToggleComment?.(comment)}
      >
        {#if comment.resolved}
          <CircleCheckBig size={13} />
        {:else}
          <Circle size={13} />
        {/if}
        <span>{comment.body}</span>
      </button>
    {:else}
      <div class="node-comment-note {comment.resolved ? 'is-resolved' : ''}">
        {#if comment.resolved}
          <CircleCheckBig size={13} />
        {:else}
          <Circle size={13} />
        {/if}
        <span>{comment.body}</span>
      </div>
    {/if}
  {/each}
  {#if comments.length === 0}
    <span class="muted">No comments yet.</span>
  {/if}
</div>
