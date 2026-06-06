<script lang="ts">
  import { MessageSquare } from "lucide-svelte";
  import type { GraphComment, GraphNode } from "../../../shared/schema";
  import NodeCommentList from "./NodeCommentList.svelte";

  type Props = {
    node: GraphNode;
    selected: boolean;
    comments: GraphComment[];
    onWheel: (event: WheelEvent) => void;
  };

  let { node, selected, comments, onWheel }: Props = $props();
</script>

<article class="node-note-scroll nodrag nowheel {selected ? 'is-readable' : 'is-preview'}" onwheel={onWheel}>
  <h3 class="node-note-title">{node.title}</h3>
  <p class="node-note-summary">{node.summary}</p>
  {#if node.detail}
    <p class="node-note-detail">{node.detail}</p>
  {/if}

  {#if selected && node.evidenceRefs.length > 0}
    <section class="node-note-section">
      <span>Evidence refs</span>
      <p>{node.evidenceRefs.join(", ")}</p>
    </section>
  {/if}

  {#if selected}
    <section class="node-note-section node-note-comments">
      <span class="node-note-section-title">
        <MessageSquare size={13} />
        Comments
      </span>
      <NodeCommentList {comments} />
    </section>
  {/if}
</article>
