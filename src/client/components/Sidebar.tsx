import { FileText, Plus, RefreshCw, Tag as TagIcon } from "lucide-react";
import type { CSSProperties } from "react";
import type { SceneGroup, Tag } from "../../shared/schema";

type SidebarProps = {
  groups: SceneGroup[];
  tags: Tag[];
  activeGroupId?: string;
  activeGroup?: SceneGroup | null;
  prompt: string;
  tagName: string;
  busy: boolean;
  activeTagIds: string[];
  onPromptChange: (value: string) => void;
  onTagNameChange: (value: string) => void;
  onCreateGroup: () => void;
  onCreateTag: () => void;
  onSelectGroup: (group: SceneGroup) => void;
  onToggleGroupTag: (tag: Tag) => void;
  onToggleTagFilter: (tagId: string) => void;
  onRefresh: () => void;
};

export function Sidebar(props: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="sidebar-section compose">
        <div className="section-title">
          <FileText size={16} />
          <h2>New group</h2>
        </div>
        <textarea
          value={props.prompt}
          onChange={(event) => props.onPromptChange(event.target.value)}
          placeholder="Describe the group, options, constraints, and desired output."
        />
        <button className="primary-button" onClick={props.onCreateGroup} disabled={props.busy || !props.prompt.trim()}>
          <Plus size={15} />
          Create group
        </button>
      </div>

      <div className="sidebar-section compose">
        <div className="section-title">
          <TagIcon size={16} />
          <h2>Tags</h2>
        </div>
        <div className="tag-create-row">
          <input
            value={props.tagName}
            onChange={(event) => props.onTagNameChange(event.target.value)}
            placeholder="New tag"
            aria-label="New tag name"
          />
          <button className="icon-button" onClick={props.onCreateTag} disabled={props.busy || !props.tagName.trim()} aria-label="Create tag">
            <Plus size={15} />
          </button>
        </div>
        <div className="tag-filter-list">
          {props.tags.map((tag) => (
            <button
              key={tag.id}
              className={`tag-chip ${props.activeTagIds.includes(tag.id) ? "is-active" : ""}`}
              style={{ "--tag-color": tag.color } as CSSProperties}
              onClick={() => props.onToggleTagFilter(tag.id)}
            >
              {tag.name}
            </button>
          ))}
          {props.tags.length === 0 ? <p className="muted">No tags yet.</p> : null}
        </div>
      </div>

      {props.activeGroup ? (
        <div className="sidebar-section selected-group-tags">
          <div className="section-title">
            <TagIcon size={16} />
            <h2>Selected group</h2>
          </div>
          <strong>{props.activeGroup.title}</strong>
          <div className="tag-filter-list">
            {props.tags.map((tag) => (
              <button
                key={tag.id}
                className={`tag-chip ${props.activeGroup?.tagIds.includes(tag.id) ? "is-attached" : ""}`}
                style={{ "--tag-color": tag.color } as CSSProperties}
                onClick={() => props.onToggleGroupTag(tag)}
              >
                {tag.name}
              </button>
            ))}
            {props.tags.length === 0 ? <p className="muted">Create a tag first.</p> : null}
          </div>
        </div>
      ) : null}

      <div className="sidebar-section">
        <div className="section-title section-title--split">
          <span>Groups</span>
          <button className="icon-button" onClick={props.onRefresh} aria-label="Refresh scene">
            <RefreshCw size={15} />
          </button>
        </div>
        <div className="group-list">
          {props.groups.map((group) => (
            <button
              key={group.id}
              className={`group-row ${group.id === props.activeGroupId ? "is-active" : ""}`}
              onClick={() => props.onSelectGroup(group)}
            >
              <strong>{group.title}</strong>
              <span>{group.summary || "Group on scene canvas"}</span>
            </button>
          ))}
          {props.groups.length === 0 ? <p className="muted">No groups yet.</p> : null}
        </div>
      </div>
    </aside>
  );
}
