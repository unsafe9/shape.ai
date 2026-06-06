<script lang="ts">
  import { MessageSquare, Plus } from "lucide-svelte";
  import type { GraphComment } from "../../../shared/schema";
  import NodeCommentList from "./NodeCommentList.svelte";

  type Props = {
    comments: GraphComment[];
    commentValue: string;
    busy: boolean;
    onCommentChange?: (value: string) => void;
    onAddComment?: () => void;
    onToggleComment?: (comment: GraphComment) => void;
  };

  let { comments, commentValue, busy, onCommentChange, onAddComment, onToggleComment }: Props = $props();
</script>

<div class="node-comments">
  <div class="node-comments-title">
    <MessageSquare size={13} />
    Comments
  </div>
  <textarea
    value={commentValue}
    oninput={(event) => onCommentChange?.(event.currentTarget.value)}
    placeholder="Leave a question or note."
  ></textarea>
  <button class="primary-button" onclick={() => onAddComment?.()} disabled={busy || !commentValue.trim()}>
    <Plus size={13} />
    Add comment
  </button>
  <NodeCommentList {comments} interactive {onToggleComment} />
</div>
