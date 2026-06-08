<script lang="ts">
  import {
    Activity,
    Circle,
    Download,
    LayoutTemplate,
    Maximize2,
    Minus,
    Minus as LineIcon,
    Eraser,
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
  import type { DragCreateShape, PrimitiveKindId } from "../lib/toolbar";
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
    // W2-07: the armed drag-create shape (rect/ellipse/line), or null. Highlights
    // the active shape button while the create tool is armed.
    createKind: DragCreateShape | null;
    // W2-08: draw-mode brush state the contextual sub-toolbar drives (only shown
    // while the pen/eraser tool is active). Pure UI chrome — the parent owns state.
    penColor: string;
    penWidthPx: number;
    penPalette: string[];
    penWidths: number[];
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
    // W2-08: draw-mode sub-toolbar setters.
    onSetPenColor: (color: string) => void;
    onSetPenWidth: (widthPx: number) => void;
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
    createKind,
    penColor,
    penWidthPx,
    penPalette,
    penWidths,
    busy,
    templateOpen,
    diagnosticsOpen,
    selectedObject,
    canvases,
    activeCanvasId,
    connectionStatus,
    canvasBusy,
    onSetTool,
    onSetPenColor,
    onSetPenWidth,
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

<!-- W2-08: contextual draw-mode sub-toolbar (brush size + color + eraser hint).
     Only shown while the pen/eraser tool is active; thin UI chrome that drives the
     reactive brush state in the parent. -->
{#if activeTool === "draw" || activeTool === "erase"}
  <div class="toolbar-draw" role="toolbar" tabindex="-1" aria-label="Draw settings" onpointerdown={(event) => event.stopPropagation()}>
    <div class="toolbar-group" aria-label="Brush size">
      <span class="toolbar-group-label">Size</span>
      <div class="toolbar-group-buttons">
        {#each penWidths as width (width)}
          <button
            class="icon-button brush-size {penWidthPx === width ? 'is-active' : ''}"
            type="button"
            title={`${width}px`}
            aria-label={`Brush ${width}px`}
            aria-pressed={penWidthPx === width}
            onclick={() => onSetPenWidth(width)}
          >
            <span class="brush-dot" style={`width:${Math.min(16, width * 2)}px;height:${Math.min(16, width * 2)}px;`}></span>
          </button>
        {/each}
      </div>
    </div>

    <div class="toolbar-sep" aria-hidden="true"></div>

    <div class="toolbar-group" aria-label="Brush color">
      <span class="toolbar-group-label">Color</span>
      <div class="toolbar-group-buttons">
        {#each penPalette as color (color)}
          <button
            class="icon-button swatch {penColor === color ? 'is-active' : ''}"
            type="button"
            title={color}
            aria-label={`Color ${color}`}
            aria-pressed={penColor === color}
            onclick={() => onSetPenColor(color)}
          >
            <span class="swatch-fill" style={`background:${color};`}></span>
          </button>
        {/each}
      </div>
    </div>
  </div>
{/if}

<!-- Bottom-center toolbar: the sole persistent floating UI. -->
<div class="toolbar-remote" role="toolbar" tabindex="-1" aria-label="Canvas toolbar" onpointerdown={(event) => event.stopPropagation()}>
  <!-- W2-03: one unified Move/Select pointer (picks/drags/marquees). Pan rides
       Space-hold / middle-button / wheel, so there is no separate Hand tool. -->
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
    </div>
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <!-- Draw: the Pen is a tool toggle (free-draw), not an inserter. The Eraser is
       a sibling draw-mode tool (whole-stroke delete, or partial cut with Alt). -->
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
      <button
        class="icon-button {activeTool === 'erase' ? 'is-active' : ''}"
        type="button"
        title="Eraser (hold Alt to partial-erase)"
        aria-label="Eraser tool"
        aria-pressed={activeTool === "erase"}
        onclick={() => onSetTool("erase")}
      >
        <Eraser size={16} />
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
          class="icon-button {createKind === shape.id ? 'is-active' : ''}"
          type="button"
          disabled={busy}
          title={shape.label}
          aria-label={`Insert ${shape.label}`}
          aria-pressed={createKind === shape.id}
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
