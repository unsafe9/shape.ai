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
    Pencil,
    Plus,
    Scan,
    Square,
    SquareDashed,
    Type,
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

  const shapes: { id: PrimitiveKindId; label: string; icon: typeof Square }[] = [
    { id: "rectangle", label: "Rectangle (R)", icon: Square },
    { id: "ellipse", label: "Ellipse (O)", icon: Circle },
    { id: "line", label: "Line (L)", icon: LineIcon },
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
<div class="toolbar-remote" role="toolbar" tabindex="-1" aria-label="Canvas toolbar" onpointerdown={(event) => event.stopPropagation()}>
  <!-- Move: select picks/drags, hand pans (both are tool toggles). -->
  <div class="toolbar-group" aria-label="Move">
    <span class="toolbar-group-label">Move</span>
    <div class="toolbar-group-buttons">
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
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <!-- Draw: the Pen is a tool toggle (free-draw), not an inserter. -->
  <div class="toolbar-group" aria-label="Draw">
    <span class="toolbar-group-label">Draw</span>
    <div class="toolbar-group-buttons">
      <button
        class="icon-button {activeTool === 'draw' ? 'is-active' : ''}"
        type="button"
        title="Pen (P)"
        aria-label="Pen tool"
        aria-pressed={activeTool === "draw"}
        onclick={() => onSetTool("draw")}
      >
        <Pencil size={16} />
      </button>
    </div>
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <!-- Shapes: object-primitive inserters. -->
  <div class="toolbar-group" aria-label="Shapes">
    <span class="toolbar-group-label">Shapes</span>
    <div class="toolbar-group-buttons">
      {#each shapes as shape (shape.id)}
        <button
          class="icon-button"
          type="button"
          disabled={busy}
          title={shape.label}
          aria-label={`Insert ${shape.label}`}
          onclick={() => onInsertPrimitive(shape.id)}
        >
          <shape.icon size={16} />
        </button>
      {/each}
    </div>
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <!-- Text: object-primitive inserter. -->
  <div class="toolbar-group" aria-label="Text">
    <span class="toolbar-group-label">Text</span>
    <div class="toolbar-group-buttons">
      <button
        class="icon-button"
        type="button"
        disabled={busy}
        title="Text (T)"
        aria-label="Insert Text (T)"
        onclick={() => onInsertPrimitive("text")}
      >
        <Type size={16} />
      </button>
    </div>
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <div class="toolbar-group" aria-label="Templates and export">
    <span class="toolbar-group-label">More</span>
    <div class="toolbar-group-buttons">
      <button
        class="icon-button {templateOpen ? 'is-active' : ''}"
        type="button"
        title="Templates"
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
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <div class="toolbar-group" aria-label="Zoom">
    <span class="toolbar-group-label">Zoom</span>
    <div class="toolbar-group-buttons">
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
</div>
