<script lang="ts">
  import { LayoutTemplate } from "lucide-svelte";

  type TemplatePickerEntry = {
    id: string;
    title: string;
    description: string;
  };

  type Props = {
    templates: TemplatePickerEntry[];
    busy: boolean;
    onApply: (templateId: string) => void;
    onClose: () => void;
  };

  let { templates, busy, onApply, onClose }: Props = $props();
</script>

<div
  class="template-picker"
  role="menu"
  tabindex="-1"
  aria-label="Insert template"
  onpointerdown={(event) => event.stopPropagation()}
>
  <div class="template-picker-header">
    <LayoutTemplate size={14} />
    <span>Insert template</span>
  </div>
  <div class="template-picker-list">
    {#each templates as template (template.id)}
      <button
        class="template-picker-item"
        role="menuitem"
        type="button"
        disabled={busy}
        onclick={() => {
          onApply(template.id);
          onClose();
        }}
      >
        <span class="template-picker-title">{template.title}</span>
        <span class="template-picker-desc">{template.description}</span>
      </button>
    {/each}
  </div>
</div>
