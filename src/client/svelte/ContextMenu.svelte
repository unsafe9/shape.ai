<script lang="ts" module>
  import type { Icon } from "lucide-svelte";

  // CC4.2 — context-appropriate right-click menu. Driven by an items array so the
  // shell (CC4.3) can supply node / edge / group / canvas actions without this
  // component branching on target kind. A `null` item renders a separator.
  export type ContextMenuItem = {
    label: string;
    icon?: typeof Icon;
    danger?: boolean;
    disabled?: boolean;
    onSelect: () => void;
  };
</script>

<script lang="ts">

  type Props = {
    x: number;
    y: number;
    title?: string;
    items: (ContextMenuItem | null)[];
  };

  let { x, y, title, items }: Props = $props();
</script>

<div
  class="node-context-menu"
  style="left: {x}px; top: {y}px"
  onpointerdown={(event) => event.stopPropagation()}
  oncontextmenu={(event) => event.preventDefault()}
  role="menu"
  tabindex="-1"
>
  {#if title}
    <div class="node-context-menu-title">
      <span>{title}</span>
    </div>
  {/if}
  {#each items as item, index (index)}
    {#if item === null}
      <div class="node-context-menu-separator"></div>
    {:else}
      <button
        role="menuitem"
        class={item.danger ? "danger-menu-item" : ""}
        disabled={item.disabled}
        onclick={item.onSelect}
      >
        {#if item.icon}
          <item.icon size={14} />
        {/if}
        {item.label}
      </button>
    {/if}
  {/each}
</div>
