import type { PointerEvent, WheelEvent } from "react";
import { MessageSquare, Pencil } from "lucide-react";
import type { GraphComment, GraphNode } from "../../../shared/schema";
import { NodeCommentList } from "./NodeComments";

export function NodeReadNote({
  node,
  selected,
  comments,
  onStartEdit,
  onWheel,
  onPointerDown
}: {
  node: GraphNode;
  selected: boolean;
  comments: GraphComment[];
  onStartEdit: () => void;
  onWheel: (event: WheelEvent<HTMLElement>) => void;
  onPointerDown: (event: PointerEvent<HTMLElement>) => void;
}) {
  return (
    <>
      <article className={`node-note-scroll nodrag nowheel ${selected ? "is-readable" : "is-preview"}`} onWheel={onWheel}>
        <h3 className="node-note-title">{node.title}</h3>
        <p className="node-note-summary">{node.summary}</p>
        {node.detail ? <p className="node-note-detail">{node.detail}</p> : null}

        {selected && node.evidenceRefs.length > 0 ? (
          <section className="node-note-section">
            <span>Evidence refs</span>
            <p>{node.evidenceRefs.join(", ")}</p>
          </section>
        ) : null}

        {selected ? (
          <section className="node-note-section node-note-comments">
            <span className="node-note-section-title">
              <MessageSquare size={13} />
              Comments
            </span>
            <NodeCommentList comments={comments} />
          </section>
        ) : null}
      </article>

      {selected ? (
        <div className="node-read-actions nodrag nowheel" onPointerDown={onPointerDown} onClick={(event) => event.stopPropagation()}>
          <button className="primary-button" onClick={onStartEdit}>
            <Pencil size={13} />
            Edit
          </button>
          <span>Press E or Option-click to edit.</span>
        </div>
      ) : null}
    </>
  );
}
