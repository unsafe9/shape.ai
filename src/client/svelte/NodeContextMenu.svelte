<script lang="ts">
  import { Clipboard, Copy, Layers, Pencil, Trash2 } from "lucide-svelte";
  import type { SceneNode } from "../../shared/schema";

  type Props = {
    node: SceneNode;
    x: number;
    y: number;
    canPaste: boolean;
    onMoveLayer: (direction: "front" | "back") => void;
    onCopy: () => void;
    onPaste: () => void;
    onDuplicate: () => void;
    onEdit: () => void;
    onDelete: () => void;
  };

  let { node, x, y, canPaste, onMoveLayer, onCopy, onPaste, onDuplicate, onEdit, onDelete }: Props = $props();
</script>

<div
  class="node-context-menu"
  style="left: {x}px; top: {y}px"
  onpointerdown={(event) => event.stopPropagation()}
  oncontextmenu={(event) => event.preventDefault()}
  role="menu"
  tabindex="-1"
>
  <div class="node-context-menu-title">
    <span>{node.title}</span>
  </div>
  <button role="menuitem" onclick={() => onMoveLayer("front")}>
    <Layers size={14} />
    Bring to front
  </button>
  <button role="menuitem" onclick={() => onMoveLayer("back")}>
    <Layers size={14} />
    Send to back
  </button>
  <div class="node-context-menu-separator"></div>
  <button role="menuitem" onclick={onCopy}>
    <Copy size={14} />
    Copy as Markdown
  </button>
  <button role="menuitem" disabled={!canPaste} onclick={onPaste}>
    <Clipboard size={14} />
    Paste copied node
  </button>
  <button role="menuitem" onclick={onDuplicate}>
    <Copy size={14} />
    Duplicate node
  </button>
  <div class="node-context-menu-separator"></div>
  <button role="menuitem" onclick={onEdit}>
    <Pencil size={14} />
    Edit node
  </button>
  <button class="danger-menu-item" role="menuitem" onclick={onDelete}>
    <Trash2 size={14} />
    Delete node
  </button>
</div>
