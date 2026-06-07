<script lang="ts">
  import {
    Circle,
    Hand,
    LayoutTemplate,
    Maximize2,
    Minus,
    MousePointer2,
    MoveRight,
    Plus,
    Scan,
    Square,
    SquareDashed,
    StickyNote
  } from "lucide-svelte";
  import type { ActiveTool } from "../renderer/engine";
  import type { PrimitiveKindId } from "../lib/cockpitCommands";

  // CC1.1/1.2/1.3 — the bottom-center cockpit remote: tools, shape inserters, a
  // template-library trigger, and zoom controls. Primitive insertion + template
  // apply route through the parent; this component only renders + dispatches.
  type Props = {
    activeTool: ActiveTool;
    busy: boolean;
    templateOpen: boolean;
    onSetTool: (tool: ActiveTool) => void;
    onInsertPrimitive: (kind: PrimitiveKindId) => void;
    onToggleTemplates: () => void;
    onZoomIn: () => void;
    onZoomOut: () => void;
    onFit: () => void;
    onFullscreen: () => void;
  };

  let {
    activeTool,
    busy,
    templateOpen,
    onSetTool,
    onInsertPrimitive,
    onToggleTemplates,
    onZoomIn,
    onZoomOut,
    onFit,
    onFullscreen
  }: Props = $props();

  const primitives: { id: PrimitiveKindId; label: string; icon: typeof Square }[] = [
    { id: "rectangle", label: "Rectangle (R)", icon: Square },
    { id: "ellipse", label: "Ellipse (O)", icon: Circle },
    { id: "connector", label: "Connector (C)", icon: MoveRight },
    { id: "sticky", label: "Sticky / Text (S)", icon: StickyNote },
    { id: "frame", label: "Frame (F)", icon: SquareDashed }
  ];
</script>

<div class="cockpit-remote" role="toolbar" tabindex="-1" aria-label="Canvas cockpit" onpointerdown={(event) => event.stopPropagation()}>
  <div class="cockpit-group" aria-label="Tools">
    <button
      class="icon-button {activeTool === 'select' ? 'is-active' : ''}"
      type="button"
      title="Select / Move (V)"
      aria-label="Select tool"
      aria-pressed={activeTool === "select"}
      onclick={() => onSetTool("select")}
    >
      <MousePointer2 size={16} />
    </button>
    <button
      class="icon-button {activeTool === 'hand' ? 'is-active' : ''}"
      type="button"
      title="Hand / Pan (H)"
      aria-label="Hand tool"
      aria-pressed={activeTool === "hand"}
      onclick={() => onSetTool("hand")}
    >
      <Hand size={16} />
    </button>
  </div>

  <div class="cockpit-sep" aria-hidden="true"></div>

  <div class="cockpit-group" aria-label="Insert shapes">
    {#each primitives as primitive (primitive.id)}
      <button
        class="icon-button"
        type="button"
        disabled={busy}
        title={primitive.label}
        aria-label={`Insert ${primitive.label}`}
        onclick={() => onInsertPrimitive(primitive.id)}
      >
        <primitive.icon size={16} />
      </button>
    {/each}
  </div>

  <div class="cockpit-sep" aria-hidden="true"></div>

  <div class="cockpit-group" aria-label="Templates">
    <button
      class="icon-button {templateOpen ? 'is-active' : ''}"
      type="button"
      title="Template library (T)"
      aria-label="Open template library"
      aria-expanded={templateOpen}
      onclick={onToggleTemplates}
    >
      <LayoutTemplate size={16} />
    </button>
  </div>

  <div class="cockpit-sep" aria-hidden="true"></div>

  <div class="cockpit-group" aria-label="Zoom">
    <button class="icon-button" type="button" title="Zoom out" aria-label="Zoom out" onclick={onZoomOut}>
      <Minus size={16} />
    </button>
    <button class="icon-button" type="button" title="Zoom in" aria-label="Zoom in" onclick={onZoomIn}>
      <Plus size={16} />
    </button>
    <button class="icon-button" type="button" title="Fit scene" aria-label="Fit scene" onclick={onFit}>
      <Scan size={16} />
    </button>
    <button class="icon-button" type="button" title="Fullscreen" aria-label="Fullscreen" onclick={onFullscreen}>
      <Maximize2 size={16} />
    </button>
  </div>
</div>
