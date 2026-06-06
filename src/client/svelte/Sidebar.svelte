<script lang="ts">
  import { FileText, Plus, RefreshCw, Tag as TagIcon } from "lucide-svelte";
  import type { SceneGroup, Tag } from "../../shared/schema";

  type Props = {
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

  let {
    groups,
    tags,
    activeGroupId,
    activeGroup,
    prompt,
    tagName,
    busy,
    activeTagIds,
    onPromptChange,
    onTagNameChange,
    onCreateGroup,
    onCreateTag,
    onSelectGroup,
    onToggleGroupTag,
    onToggleTagFilter,
    onRefresh
  }: Props = $props();
</script>

<aside class="sidebar">
  <div class="sidebar-section compose">
    <div class="section-title">
      <FileText size={16} />
      <h2>New group</h2>
    </div>
    <textarea
      value={prompt}
      oninput={(event) => onPromptChange(event.currentTarget.value)}
      placeholder="Describe the group, options, constraints, and desired output."
    ></textarea>
    <button class="primary-button" onclick={onCreateGroup} disabled={busy || !prompt.trim()}>
      <Plus size={15} />
      Create group
    </button>
  </div>

  <div class="sidebar-section compose">
    <div class="section-title">
      <TagIcon size={16} />
      <h2>Tags</h2>
    </div>
    <div class="tag-create-row">
      <input
        value={tagName}
        oninput={(event) => onTagNameChange(event.currentTarget.value)}
        placeholder="New tag"
        aria-label="New tag name"
      />
      <button class="icon-button" onclick={onCreateTag} disabled={busy || !tagName.trim()} aria-label="Create tag">
        <Plus size={15} />
      </button>
    </div>
    <div class="tag-filter-list">
      {#each tags as tag (tag.id)}
        <button
          class="tag-chip {activeTagIds.includes(tag.id) ? 'is-active' : ''}"
          style="--tag-color: {tag.color}"
          onclick={() => onToggleTagFilter(tag.id)}
        >
          {tag.name}
        </button>
      {/each}
      {#if tags.length === 0}
        <p class="muted">No tags yet.</p>
      {/if}
    </div>
  </div>

  {#if activeGroup}
    <div class="sidebar-section selected-group-tags">
      <div class="section-title">
        <TagIcon size={16} />
        <h2>Selected group</h2>
      </div>
      <strong>{activeGroup.title}</strong>
      <div class="tag-filter-list">
        {#each tags as tag (tag.id)}
          <button
            class="tag-chip {activeGroup?.tagIds.includes(tag.id) ? 'is-attached' : ''}"
            style="--tag-color: {tag.color}"
            onclick={() => onToggleGroupTag(tag)}
          >
            {tag.name}
          </button>
        {/each}
        {#if tags.length === 0}
          <p class="muted">Create a tag first.</p>
        {/if}
      </div>
    </div>
  {/if}

  <div class="sidebar-section">
    <div class="section-title section-title--split">
      <span>Groups</span>
      <button class="icon-button" onclick={onRefresh} aria-label="Refresh scene">
        <RefreshCw size={15} />
      </button>
    </div>
    <div class="group-list">
      {#each groups as group (group.id)}
        <button
          class="group-row {group.id === activeGroupId ? 'is-active' : ''}"
          onclick={() => onSelectGroup(group)}
        >
          <strong>{group.title}</strong>
          <span>{group.summary || "Group on scene canvas"}</span>
        </button>
      {/each}
      {#if groups.length === 0}
        <p class="muted">No groups yet.</p>
      {/if}
    </div>
  </div>
</aside>
