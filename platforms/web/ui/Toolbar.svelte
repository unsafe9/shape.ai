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
    PenTool,
    Plus,
    Scan,
    Square,
    Loader2,
    Trash2,
    Wifi,
    WifiOff
  } from "lucide-svelte";
  import type { ActiveTool } from "../renderer/engine";
  import { toolbarShapeKinds, toggleColorPopup, toggleStrokePopup, type DragCreateShape, type PrimitiveKindId } from "../controller/toolbar";
  import { THEME_DEFAULT_COLOR } from "../controller/objectPrimitives";
  import type { CanvasSummary } from "../runtime/sceneClient";
  import type { ConnectionStatus } from "../runtime/wsTransport";
  import type { Object as SceneObject } from "../shared/object";

  // The single persistent floating UI: the bottom-center toolbar plus the canvas switcher, connection
  // status, diagnostics/export triggers, and a selected-object property panel. This component renders +
  // dispatches only; every action routes through the parent.
  type Props = {
    activeTool: ActiveTool;
    // The armed drag-create shape, or null.
    createKind: DragCreateShape | null;
    // Color is owned by the separate Color control, so the Stroke popup carries width only.
    penWidthPx: number;
    penPalette: string[];
    penWidths: number[];
    // Always-visible toolbar color, applied to the selection via SetStyle.
    selectedColor: string;
    // Dark-mode flag so the theme-default swatch renders contrasting instead of the raw sentinel string.
    dark: boolean;
    busy: boolean;
    templateOpen: boolean;
    diagnosticsOpen: boolean;
    // Selected object, or null for canvas/multi.
    selectedObject: SceneObject | null;
    canvases: CanvasSummary[];
    activeCanvasId: string;
    connectionStatus: ConnectionStatus;
    canvasBusy: boolean;
    onSetTool: (tool: ActiveTool) => void;
    onSetPenWidth: (widthPx: number) => void;
    onSelectColor: (color: string) => void;
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
    penWidthPx,
    penPalette,
    penWidths,
    selectedColor,
    dark,
    busy,
    templateOpen,
    diagnosticsOpen,
    selectedObject,
    canvases,
    activeCanvasId,
    connectionStatus,
    canvasBusy,
    onSetTool,
    onSetPenWidth,
    onSelectColor,
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

  // The basic-shape inserters; `toolbarShapeKinds` is the single source of which buttons render.
  const SHAPE_META: Record<DragCreateShape, { label: string; icon: typeof Square }> = {
    rectangle: { label: "Rectangle (R)", icon: Square },
    ellipse: { label: "Ellipse (O)", icon: Circle },
    line: { label: "Line (L)", icon: LineIcon }
  };
  const shapes = toolbarShapeKinds.map((id) => ({ id, ...SHAPE_META[id] }));

  // First text run of the selected object, the editable label in the property panel.
  const selectedText = $derived(selectedObject?.text?.runs?.map((run) => run.text).join("") ?? "");

  // The color control toggles a popup holding the native picker + fixed swatches. Local UI state only.
  let colorPopupOpen = $state(false);
  let colorControl = $state<HTMLDivElement | null>(null);
  function toggleColorPopupOpen(): void {
    colorPopupOpen = toggleColorPopup(colorPopupOpen);
  }
  // Close on outside-click / Escape (re-click on the button is handled by the toggle above).
  $effect(() => {
    if (!colorPopupOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (colorControl && !colorControl.contains(event.target as Node)) colorPopupOpen = false;
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") colorPopupOpen = false;
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  });

  // The Stroke control mirrors the color control; independent open state so it never fights the color popup.
  let strokePopupOpen = $state(false);
  let strokeControl = $state<HTMLDivElement | null>(null);
  function toggleStrokePopupOpen(): void {
    strokePopupOpen = toggleStrokePopup(strokePopupOpen);
  }

  // The theme-default swatch authors a Paint::Token; render it contrasting with a "Theme default" label so the sentinel never reaches the UI.
  function swatchFill(color: string): string {
    return color === THEME_DEFAULT_COLOR ? (dark ? "#ffffff" : "#000000") : color;
  }
  function swatchLabel(color: string): string {
    return color === THEME_DEFAULT_COLOR ? "Theme default" : color;
  }

  $effect(() => {
    if (!strokePopupOpen) return;
    const onPointerDown = (event: PointerEvent) => {
      if (strokeControl && !strokeControl.contains(event.target as Node)) strokePopupOpen = false;
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") strokePopupOpen = false;
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown);
    };
  });
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
  <!-- One unified Move/Select pointer; pan rides Space-hold / middle-button / wheel, no separate Hand tool. -->
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

  <!-- The Eraser does whole-stroke delete, or a partial cut with Alt. -->
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

  <!-- The popup stops pointerdown so the canvas never sees the click. -->
  <div class="toolbar-group" aria-label="Stroke" bind:this={strokeControl}>
    <span class="toolbar-group-label">Stroke</span>
    <div class="toolbar-group-buttons">
      <button
        class="icon-button {strokePopupOpen ? 'is-active' : ''}"
        type="button"
        title="Stroke"
        aria-label="Stroke"
        aria-haspopup="dialog"
        aria-expanded={strokePopupOpen}
        onclick={toggleStrokePopupOpen}
      >
        <PenTool size={16} />
      </button>
    </div>
    {#if strokePopupOpen}
      <div class="color-popup" role="dialog" aria-label="Stroke settings" onpointerdown={(event) => event.stopPropagation()}>
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
      </div>
    {/if}
  </div>

  <div class="toolbar-sep" aria-hidden="true"></div>

  <div class="toolbar-group" aria-label="Color" bind:this={colorControl}>
    <span class="toolbar-group-label">Color</span>
    <div class="toolbar-group-buttons">
      <button
        class="icon-button color-trigger {colorPopupOpen ? 'is-active' : ''}"
        type="button"
        title="Color"
        aria-label="Color"
        aria-haspopup="dialog"
        aria-expanded={colorPopupOpen}
        onclick={toggleColorPopupOpen}
      >
        <span class="color-trigger-rainbow"></span>
        <span class="color-trigger-current" style={`background:${swatchFill(selectedColor)};`}></span>
      </button>
    </div>
    {#if colorPopupOpen}
      <div class="color-popup" role="dialog" aria-label="Choose color" onpointerdown={(event) => event.stopPropagation()}>
        <div class="color-popup-swatches">
          {#each penPalette as color (color)}
            <button
              class="icon-button swatch {selectedColor === color ? 'is-active' : ''}"
              type="button"
              title={swatchLabel(color)}
              aria-label={`Color ${swatchLabel(color)}`}
              aria-pressed={selectedColor === color}
              onclick={() => onSelectColor(color)}
            >
              <span class="swatch-fill" style={`background:${swatchFill(color)};`}></span>
            </button>
          {/each}
        </div>
        <label class="color-popup-picker" title="Custom color" aria-label="Custom color">
          <span class="swatch-fill" style={`background:${swatchFill(selectedColor)};`}></span>
          <span class="color-popup-picker-label">Custom</span>
          <input
            type="color"
            value={swatchFill(selectedColor)}
            oninput={(event) => onSelectColor((event.currentTarget as HTMLInputElement).value)}
          />
        </label>
      </div>
    {/if}
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
