import { FileText, Plus, RefreshCw } from "lucide-react";
import type { Design } from "../../shared/schema";

type SidebarProps = {
  designs: Design[];
  activeDesignId?: string;
  prompt: string;
  busy: boolean;
  onPromptChange: (value: string) => void;
  onCreate: () => void;
  onSelect: (design: Design) => void;
  onRefresh: () => void;
};

export function Sidebar(props: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="sidebar-section compose">
        <div className="section-title">
          <FileText size={16} />
          <h2>New design</h2>
        </div>
        <textarea
          value={props.prompt}
          onChange={(event) => props.onPromptChange(event.target.value)}
          placeholder="Describe the architecture decision, options, constraints, and desired output."
        />
        <button className="primary-button" onClick={props.onCreate} disabled={props.busy || !props.prompt.trim()}>
          <Plus size={15} />
          Create design
        </button>
      </div>

      <div className="sidebar-section">
        <div className="section-title section-title--split">
          <span>Designs</span>
          <button className="icon-button" onClick={props.onRefresh} aria-label="Refresh designs">
            <RefreshCw size={15} />
          </button>
        </div>
        <div className="design-list">
          {props.designs.map((design) => (
            <button
              key={design.id}
              className={`design-row ${design.id === props.activeDesignId ? "is-active" : ""}`}
              onClick={() => props.onSelect(design)}
            >
              <strong>{design.title}</strong>
              <span>{design.graph.nodes.length} nodes / {design.comments.length} comments / {design.artifacts.length} exports</span>
            </button>
          ))}
          {props.designs.length === 0 ? <p className="muted">No designs yet.</p> : null}
        </div>
      </div>
    </aside>
  );
}
