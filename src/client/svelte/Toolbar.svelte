<script lang="ts">
  import {
    Activity,
    Circle,
    Download,
    Hand,
    LayoutTemplate,
    Maximize2,
    Minus,
    Minus as LineIcon,
    MousePointer2,
    Plus,
    Scan,
    Square,
    SquareDashed,
    StickyNote,
    Loader2,
    Trash2,
    Wifi,
    WifiOff
  } from "lucide-svelte";
  import type { ActiveTool } from "../renderer/engine";
  import type { PrimitiveKindId } from "../lib/toolbar";
  import type { CanvasSummary } from "../lib/sceneClient";
  import type { ConnectionStatus } from "../lib/wsTransport";
  import type { Object as SceneObject } from "../../shared/object";

  // U1 — the single persistent floating UI: the bottom-center toolbar (tools,
  // object-primitive inserters, template trigger, zoom) plus the absorbed canvas
  // switcher, connection status, diagnostics toggle, export trigger, and a
  // selected-object property panel (the old SelectedNodeInspector). Every action
  // routes through the parent; this component renders + dispatches only.
  type Props = {
    activeTool: ActiveTool;
    busy: boolean;
    templateOpen: boolean;
    diagnosticsOpen: boolean;
    // Selected object (object-native property edit), or null for canvas/multi.
    selectedObject: SceneObject | null;
    // Canvas switcher state.
    canvases: CanvasSummary[];
    activeCanvasId: string;
    connectionStatus: ConnectionStatus;
    canvasBusy: boolean;
    onSetTool: (tool: ActiveTool) => void;
    onInsertPrimitive: (kind: PrimitiveKindId) => void;
    onToggleTemplates: () => void;
    onZoomIn: () => void;
    onZoomOut: () => void;
    onFit: () => void;
    onFullscreen: () => void;
    onToggleDiagnostics: () => void;
    onExport: () => void;
    onSelectCanvas: (id: string) => void;
    onCreateCanvas: (title: string) => void;
    onDeleteCanvas: (id: string) => void;
    onRenameSelected: (text: string) => void;
    onDeleteSelected: () => void;
  };

  let {
    activeTool,
    busy,
    templateOpen,
    diagnosticsOpen,
    selectedObject,
    canvases,
    activeCanvasId,
    connectionStatus,
    canvasBusy,
    onSetTool,
    onInsertPrimitive,
    onToggleTemplates,
    onZoomIn,
    onZoomOut,
    onFit,
    onFullscreen,
    onToggleDiagnostics,
    onExport,
    onSelectCanvas,
    onCreateCanvas,
    onDeleteCanvas,
    onRenameSelected,
    onDeleteSelected
  }: Props = $props();

  const primitives: { id: PrimitiveKindId; label: string; icon: typeof Square }[] = [
    { id: "rectangle", label: "Rectangle (R)", icon: Square },
    { id: "ellipse", label: "Ellipse (O)", icon: Circle },
    { id: "line", label: "Line (L)", icon: LineIcon },
    { id: "text", label: "Text (T)", icon: StickyNote },
    { id: "frame", label: "Frame (F)", icon: SquareDashed }
  ];

  // First text run of the selected object, the editable label in the property panel.
  const selectedText = $derived(selectedObject?.text?.runs?.map((run) => run.text).join("") ?? "");
</script>

<!-- Canvas switcher + connection status (top-left chrome). -->
<div class="toolbar-canvas-switcher" aria-label="Canvas switcher">
  <span class="conn-indicator" title={connectionStatus}>
    {#if connectionStatus === "online"}
      <Wifi size={14} />
    {:else}
      <WifiOff size={14} />
    {/if}
  </span>
  <select
    aria-label="Active canvas"
    value={activeCanvasId}
    disabled={canvasBusy}
    onchange={(event) => onSelectCanvas((event.currentTarget as HTMLSelectElement).value)}
  >
    {#each canvases as canvas (canvas.id)}
      <option value={canvas.id}>{canvas.title}</option>
    {/each}
    {#if !canvases.some((c) => c.id === activeCanvasId)}
      <option value={activeCanvasId}>{activeCanvasId}</option>
    {/if}
  </select>
  <button class="icon-button" type="button" title="New canvas" aria-label="New canvas" disabled={canvasBusy} onclick={() => onCreateCanvas("New canvas")}>
    <Plus size={14} />
  </button>
  <button
    class="icon-button"
    type="button"
    title="Delete canvas"
    aria-label="Delete canvas"
    disabled={canvasBusy || canvases.length <= 1}
    onclick={() => onDeleteCanvas(activeCanvasId)}
  >
    <Trash2 size={14} />
  </button>
  {#if canvasBusy}<Loader2 class="spin" size={14} />{/if}
</div>

<!-- Selected-object property panel (absorbed SelectedNodeInspector). -->
{#if selectedObject}
  <div class="toolbar-properties" aria-label="Selected object properties">
    <span class="prop-id">object:{selectedObject.id}</span>
    <input
      class="prop-text"
      type="text"
      aria-label="Object text"
      placeholder="Text"
      value={selectedText}
      onchange={(event) => onRenameSelected((event.currentTarget as HTMLInputElement).value)}
    />
    <button class="icon-button danger" type="button" title="Delete object" aria-label="Delete object" onclick={onDeleteSelected}>
      <Trash2 size={14} />
    </button>
  </div>
{/if}

<!-- Bottom-center toolbar: the sole persistent floating UI. -->
<div class="cockpit-remote" role="toolbar" tabindex="-1" aria-label="Canvas toolbar" onpointerdown={(event) => event.stopPropagation()}>
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

  <div class="cockpit-group" aria-label="Insert objects">
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

  <div class="cockpit-group" aria-label="Templates and export">
    <button
      class="icon-button {templateOpen ? 'is-active' : ''}"
      type="button"
      title="Templates (T)"
      aria-label="Open templates"
      aria-expanded={templateOpen}
      onclick={onToggleTemplates}
    >
      <LayoutTemplate size={16} />
    </button>
    <button class="icon-button" type="button" title="Export" aria-label="Export selection" onclick={onExport}>
      <Download size={16} />
    </button>
    <button
      class="icon-button {diagnosticsOpen ? 'is-active' : ''}"
      type="button"
      title="Diagnostics"
      aria-label="Toggle diagnostics"
      aria-expanded={diagnosticsOpen}
      onclick={onToggleDiagnostics}
    >
      <Activity size={16} />
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
