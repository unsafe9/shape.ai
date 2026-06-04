import { FileText, Plus, RefreshCw } from "lucide-react";
import type { Shape } from "../../shared/schema";

type SidebarProps = {
  shapes: Shape[];
  activeShapeId?: string;
  prompt: string;
  busy: boolean;
  onPromptChange: (value: string) => void;
  onCreate: () => void;
  onSelect: (shape: Shape) => void;
  onRefresh: () => void;
};

export function Sidebar(props: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="sidebar-section compose">
        <div className="section-title">
          <FileText size={16} />
          <h2>New shape</h2>
        </div>
        <textarea
          value={props.prompt}
          onChange={(event) => props.onPromptChange(event.target.value)}
          placeholder="Describe the shape, options, constraints, and desired output."
        />
        <button className="primary-button" onClick={props.onCreate} disabled={props.busy || !props.prompt.trim()}>
          <Plus size={15} />
          Create shape
        </button>
      </div>

      <div className="sidebar-section">
        <div className="section-title section-title--split">
          <span>Shapes</span>
          <button className="icon-button" onClick={props.onRefresh} aria-label="Refresh shapes">
            <RefreshCw size={15} />
          </button>
        </div>
        <div className="shape-list">
          {props.shapes.map((shape) => (
            <button
              key={shape.id}
              className={`shape-row ${shape.id === props.activeShapeId ? "is-active" : ""}`}
              onClick={() => props.onSelect(shape)}
            >
              <strong>{shape.title}</strong>
              <span>{shape.graph.nodes.length} nodes / {shape.comments.length} comments / {shape.artifacts.length} exports</span>
            </button>
          ))}
          {props.shapes.length === 0 ? <p className="muted">No shapes yet.</p> : null}
        </div>
      </div>
    </aside>
  );
}
