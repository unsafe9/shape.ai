<script lang="ts">
  import { LayoutTemplate, Plus, Trash2, X } from "lucide-svelte";
  import type { TemplateContract } from "../../shared/templates/contract";

  // CC3.3 — template library popover: list (GET /api/templates), apply, save the
  // current selection as a template, and delete (tombstone). The parent owns the
  // network + selection logic; this renders the list and routes actions.
  type Props = {
    templates: TemplateContract[];
    busy: boolean;
    canSaveSelection: boolean;
    onApply: (templateId: string) => void;
    onSaveSelection: (title: string) => void;
    onDelete: (templateId: string) => void;
    onClose: () => void;
  };

  let { templates, busy, canSaveSelection, onApply, onSaveSelection, onDelete, onClose }: Props = $props();

  let newTitle = $state("");

  function handleSave() {
    const title = newTitle.trim();
    if (!title) return;
    onSaveSelection(title);
    newTitle = "";
  }
</script>

<div class="template-library" role="dialog" tabindex="-1" aria-label="Template library" onpointerdown={(event) => event.stopPropagation()}>
  <div class="template-library-header">
    <div class="template-library-title">
      <LayoutTemplate size={14} />
      <span>Templates</span>
    </div>
    <button class="icon-button" type="button" aria-label="Close template library" title="Close" onclick={onClose}>
      <X size={14} />
    </button>
  </div>

  <div class="template-library-list">
    {#if templates.length === 0}
      <p class="template-library-empty">No templates yet. Save a selection to create one.</p>
    {/if}
    {#each templates as template (template.metadata.id)}
      <div class="template-library-item">
        <button
          class="template-library-apply"
          type="button"
          disabled={busy}
          title={template.metadata.description}
          onclick={() => onApply(template.metadata.id)}
        >
          <span class="template-library-item-title">{template.metadata.title}</span>
          <span class="template-library-item-desc">{template.metadata.description}</span>
        </button>
        <button
          class="icon-button template-library-delete"
          type="button"
          disabled={busy}
          aria-label={`Delete ${template.metadata.title}`}
          title="Delete template"
          onclick={() => onDelete(template.metadata.id)}
        >
          <Trash2 size={13} />
        </button>
      </div>
    {/each}
  </div>

  <div class="template-library-save">
    <input
      type="text"
      placeholder={canSaveSelection ? "New template name" : "Select objects first"}
      bind:value={newTitle}
      disabled={!canSaveSelection || busy}
      onkeydown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          handleSave();
        }
      }}
    />
    <button
      class="secondary-button"
      type="button"
      disabled={!canSaveSelection || busy || newTitle.trim().length === 0}
      onclick={handleSave}
    >
      <Plus size={14} />
      Save selection
    </button>
  </div>
</div>
