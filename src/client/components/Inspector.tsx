import { CheckCircle2, Circle, Link2, MessageSquare, Plus, Trash2 } from "lucide-react";
import { edgeTypeLabels, nodeTypeLabels } from "../../shared/graph";
import type {
  DecisionGraph,
  EdgeType,
  GraphComment,
  GraphEdge,
  GraphNode,
  GraphSelection,
  NodeStatus,
  NodeType
} from "../../shared/schema";

type InspectorProps = {
  graph: DecisionGraph;
  selection: GraphSelection;
  comments: GraphComment[];
  commentValue: string;
  busy: boolean;
  onCommentChange: (value: string) => void;
  onAddComment: () => void;
  onToggleComment: (comment: GraphComment) => void;
  onUpdateNode: (node: GraphNode) => void;
  onUpdateEdge: (edge: GraphEdge) => void;
  onDeleteSelection: () => void;
  onAddLinkedNode: (type: NodeType) => void;
};

const editableNodeTypes: NodeType[] = [
  "proposition",
  "decision_point",
  "option",
  "evidence",
  "tradeoff",
  "blocker",
  "subdecision",
  "task",
  "artifact"
];

const editableStatuses: NodeStatus[] = ["draft", "viable", "conditional", "infeasible", "unknown", "selected", "deferred", "complete"];

const editableEdgeTypes: EdgeType[] = [
  "depends_on",
  "supports",
  "blocks",
  "trades_off_with",
  "chooses_between",
  "decomposes_to",
  "produces"
];

export function Inspector(props: InspectorProps) {
  const selected = selectedItem(props.graph, props.selection);
  const scopedComments = props.comments.filter((comment) => sameSelection(comment.target, props.selection));
  return (
    <aside className="inspector">
      <div className="inspector-section">
        <div className="section-title">
          <Link2 size={16} />
          <h2>Inspector</h2>
        </div>
        {selected.kind === "node" ? (
          <NodeDetail
            node={selected.node}
            onUpdate={props.onUpdateNode}
            onDelete={props.onDeleteSelection}
            onAddLinkedNode={props.onAddLinkedNode}
          />
        ) : null}
        {selected.kind === "edge" ? (
          <EdgeDetail edge={selected.edge} onUpdate={props.onUpdateEdge} onDelete={props.onDeleteSelection} />
        ) : null}
        {selected.kind === "graph" ? (
          <div className="detail-block">
            <h3>Whole graph</h3>
            <p>{props.graph.nodes.length} nodes and {props.graph.edges.length} edges selected.</p>
            <div className="quick-actions">
              <button onClick={() => props.onAddLinkedNode("decision_point")}>
                <Plus size={14} />
                Decision
              </button>
              <button onClick={() => props.onAddLinkedNode("option")}>
                <Plus size={14} />
                Option
              </button>
              <button onClick={() => props.onAddLinkedNode("evidence")}>
                <Plus size={14} />
                Evidence
              </button>
            </div>
          </div>
        ) : null}
      </div>

      <div className="inspector-section comment-panel">
        <div className="section-title">
          <MessageSquare size={16} />
          <h2>Comments</h2>
        </div>
        <p className="comment-target">{selectionLabel(props.graph, props.selection)}</p>
        <textarea
          value={props.commentValue}
          onChange={(event) => props.onCommentChange(event.target.value)}
          placeholder="Leave a question, concern, follow-up, or note for an MCP-connected agent."
        />
        <button className="primary-button" onClick={props.onAddComment} disabled={props.busy || !props.commentValue.trim()}>
          <Plus size={15} />
          Add comment
        </button>
        <div className="comment-list">
          {scopedComments.map((comment) => (
            <button
              key={comment.id}
              className={`comment-row ${comment.resolved ? "is-resolved" : ""}`}
              onClick={() => props.onToggleComment(comment)}
            >
              {comment.resolved ? <CheckCircle2 size={15} /> : <Circle size={15} />}
              <span>{comment.body}</span>
            </button>
          ))}
          {scopedComments.length === 0 ? <p className="muted">No comments for this selection.</p> : null}
        </div>
      </div>
    </aside>
  );
}

function NodeDetail({
  node,
  onUpdate,
  onDelete,
  onAddLinkedNode
}: {
  node: GraphNode;
  onUpdate: (node: GraphNode) => void;
  onDelete: () => void;
  onAddLinkedNode: (type: NodeType) => void;
}) {
  return (
    <div className="detail-block" key={node.id}>
      <div className="detail-kicker">{nodeTypeLabels[node.type]} / {node.status}</div>
      <label className="field">
        <span>Title</span>
        <input defaultValue={node.title} onBlur={(event) => onUpdate({ ...node, title: event.currentTarget.value || node.title })} />
      </label>
      <div className="field-grid">
        <label className="field">
          <span>Type</span>
          <select value={node.type} onChange={(event) => onUpdate({ ...node, type: event.currentTarget.value as NodeType })}>
            {editableNodeTypes.map((type) => (
              <option key={type} value={type}>
                {nodeTypeLabels[type]}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Status</span>
          <select value={node.status} onChange={(event) => onUpdate({ ...node, status: event.currentTarget.value as NodeStatus })}>
            {editableStatuses.map((status) => (
              <option key={status} value={status}>
                {status}
              </option>
            ))}
          </select>
        </label>
      </div>
      <label className="field">
        <span>Summary</span>
        <textarea defaultValue={node.summary} onBlur={(event) => onUpdate({ ...node, summary: event.currentTarget.value })} />
      </label>
      <label className="field">
        <span>Detail</span>
        <textarea defaultValue={node.detail} onBlur={(event) => onUpdate({ ...node, detail: event.currentTarget.value })} />
      </label>
      <label className="field">
        <span>Evidence refs</span>
        <input
          defaultValue={node.evidenceRefs.join(", ")}
          onBlur={(event) =>
            onUpdate({
              ...node,
              evidenceRefs: event.currentTarget.value
                .split(",")
                .map((value) => value.trim())
                .filter(Boolean)
            })
          }
        />
      </label>
      <div className="detail-meta">Confidence {Math.round(node.confidence * 100)}%</div>
      <div className="quick-actions">
        <button onClick={() => onAddLinkedNode("option")}>
          <Plus size={14} />
          Option
        </button>
        <button onClick={() => onAddLinkedNode("evidence")}>
          <Plus size={14} />
          Evidence
        </button>
        <button onClick={() => onAddLinkedNode("blocker")}>
          <Plus size={14} />
          Blocker
        </button>
        <button className="danger-button" onClick={onDelete}>
          <Trash2 size={14} />
          Delete
        </button>
      </div>
    </div>
  );
}

function EdgeDetail({
  edge,
  onUpdate,
  onDelete
}: {
  edge: GraphEdge;
  onUpdate: (edge: GraphEdge) => void;
  onDelete: () => void;
}) {
  return (
    <div className="detail-block" key={edge.id}>
      <div className="detail-kicker">{edgeTypeLabels[edge.type]}</div>
      <label className="field">
        <span>Relationship</span>
        <select value={edge.type} onChange={(event) => onUpdate({ ...edge, type: event.currentTarget.value as EdgeType })}>
          {editableEdgeTypes.map((type) => (
            <option key={type} value={type}>
              {edgeTypeLabels[type]}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Label</span>
        <input defaultValue={edge.label} onBlur={(event) => onUpdate({ ...edge, label: event.currentTarget.value })} />
      </label>
      <label className="field">
        <span>Rationale</span>
        <textarea defaultValue={edge.rationale} onBlur={(event) => onUpdate({ ...edge, rationale: event.currentTarget.value })} />
      </label>
      <dl>
        <div>
          <dt>Source</dt>
          <dd>{edge.source}</dd>
        </div>
        <div>
          <dt>Target</dt>
          <dd>{edge.target}</dd>
        </div>
      </dl>
      <div className="quick-actions">
        <button className="danger-button" onClick={onDelete}>
          <Trash2 size={14} />
          Delete edge
        </button>
      </div>
    </div>
  );
}

function selectedItem(graph: DecisionGraph, selection: GraphSelection) {
  if (selection.kind === "node") {
    const node = graph.nodes.find((candidate) => candidate.id === selection.id);
    if (node) return { kind: "node" as const, node };
  }
  if (selection.kind === "edge") {
    const edge = graph.edges.find((candidate) => candidate.id === selection.id);
    if (edge) return { kind: "edge" as const, edge };
  }
  return { kind: "graph" as const };
}

function sameSelection(a: GraphSelection, b: GraphSelection): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "graph" && b.kind === "graph") return true;
  return "id" in a && "id" in b && a.id === b.id;
}

function selectionLabel(graph: DecisionGraph, selection: GraphSelection): string {
  if (selection.kind === "graph") return "Whole design graph";
  if (selection.kind === "node") {
    const node = graph.nodes.find((candidate) => candidate.id === selection.id);
    return node ? `Node: ${node.title}` : `Node: ${selection.id}`;
  }
  const edge = graph.edges.find((candidate) => candidate.id === selection.id);
  return edge ? `Edge: ${edge.source} -> ${edge.target}` : `Edge: ${selection.id}`;
}
