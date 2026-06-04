import { CheckCircle2, Circle, MessageSquare, Plus } from "lucide-react";
import type { GraphComment } from "../../../shared/schema";

type NodeCommentListProps = {
  comments: GraphComment[];
  interactive?: boolean;
  onToggleComment?: (comment: GraphComment) => void;
};

export function NodeCommentList({ comments, interactive = false, onToggleComment }: NodeCommentListProps) {
  return (
    <div className="node-comment-list">
      {comments.map((comment) =>
        interactive ? (
          <button
            key={comment.id}
            className={`node-comment-row ${comment.resolved ? "is-resolved" : ""}`}
            onClick={() => onToggleComment?.(comment)}
          >
            {comment.resolved ? <CheckCircle2 size={13} /> : <Circle size={13} />}
            <span>{comment.body}</span>
          </button>
        ) : (
          <div key={comment.id} className={`node-comment-note ${comment.resolved ? "is-resolved" : ""}`}>
            {comment.resolved ? <CheckCircle2 size={13} /> : <Circle size={13} />}
            <span>{comment.body}</span>
          </div>
        )
      )}
      {comments.length === 0 ? <span className="muted">No comments yet.</span> : null}
    </div>
  );
}

export function NodeComments({
  comments,
  commentValue,
  busy,
  onCommentChange,
  onAddComment,
  onToggleComment
}: {
  comments: GraphComment[];
  commentValue: string;
  busy: boolean;
  onCommentChange?: (value: string) => void;
  onAddComment?: () => void;
  onToggleComment?: (comment: GraphComment) => void;
}) {
  return (
    <div className="node-comments">
      <div className="node-comments-title">
        <MessageSquare size={13} />
        Comments
      </div>
      <textarea
        value={commentValue}
        onChange={(event) => onCommentChange?.(event.currentTarget.value)}
        placeholder="Leave a question or note."
      />
      <button className="primary-button" onClick={() => onAddComment?.()} disabled={busy || !commentValue.trim()}>
        <Plus size={13} />
        Add comment
      </button>
      <NodeCommentList comments={comments} interactive onToggleComment={onToggleComment} />
    </div>
  );
}
