<script lang="ts" module>
  // W2-09 — upward scroll-popup template list, opened from the toolbar's More
  // group. Mirrors the ContextMenu pattern (items array, role=menu, pointerdown
  // swallowed) but anchors above the bottom toolbar and reuses the floating
  // template-library popover styles. Selecting an item inserts that template.
  export type TemplatePopupItem = {
    id: string;
    title: string;
    desc: string;
  };
</script>

<script lang="ts">
  type Props = {
    items: TemplatePopupItem[];
    onSelect: (id: string) => void;
  };

  let { items, onSelect }: Props = $props();
</script>

<div
  class="template-library"
  role="menu"
  tabindex="-1"
  aria-label="Templates"
  onpointerdown={(event) => event.stopPropagation()}
>
  <div class="template-library-list">
    {#each items as item (item.id)}
      <div class="template-library-item">
        <button
          class="template-library-apply"
          type="button"
          role="menuitem"
          onclick={() => onSelect(item.id)}
        >
          <span class="template-library-item-title">{item.title}</span>
          <span class="template-library-item-desc">{item.desc}</span>
        </button>
      </div>
    {/each}
  </div>
</div>
