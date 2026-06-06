<script lang="ts">
  import { X } from "lucide-svelte";
  import type { CameraState } from "../../shared/renderScene";
  import type { SceneSelection } from "../../shared/schema";
  import type { RendererHealth, RendererStats } from "../lib/canvasHost";

  type Props = {
    open: boolean;
    stats: RendererStats | null;
    health: RendererHealth | null;
    camera: CameraState;
    selection: SceneSelection;
    selectedTargetLabel: string;
    status: string;
    rendererStatus: string;
    onClose: () => void;
  };

  let { open, stats, health, camera, selection, selectedTargetLabel, status, rendererStatus, onClose }: Props = $props();

  function ms(value: number): string {
    return `${value.toFixed(2)} ms`;
  }

  function bytes(value: number): string {
    return `${Math.round(value / 1024 / 1024)} MB`;
  }

  function yesNo(value: boolean): string {
    return value ? "yes" : "no";
  }

  function numberValue(value: number | null | undefined): string | null {
    return value === null || value === undefined ? null : String(value);
  }

  function cachePair(hits: number | null | undefined, misses: number | null | undefined): string | null {
    if (hits === null || hits === undefined) return null;
    return `${hits} hit / ${misses ?? 0} miss`;
  }

  function slotValue(slots: number | null | undefined, freeSlots: number | null | undefined): string | null {
    if (slots === null || slots === undefined) return null;
    const free = freeSlots ?? 0;
    return `${slots - free} used / ${free} free / ${slots} total`;
  }

  function rustCameraValue(s: RendererStats | null): string | null {
    if (!s || s.rustCameraX === null || s.rustCameraY === null || s.rustCameraZoom === null) return null;
    return `${s.rustCameraZoom.toFixed(3)}x @ ${Math.round(s.rustCameraX)}, ${Math.round(s.rustCameraY)}`;
  }

  function rustLastHitValue(s: RendererStats | null): string | null {
    if (!s?.rustLastHitKind) return null;
    const target = s.rustLastHitId ? `${s.rustLastHitKind}:${s.rustLastHitId}` : s.rustLastHitKind;
    const field = s.rustLastHitField ? `:${s.rustLastHitField}` : "";
    const port = s.rustLastHitPort ? `:${s.rustLastHitPort}` : "";
    const screen =
      s.rustLastHitScreenX === null || s.rustLastHitScreenY === null
        ? ""
        : ` @ ${Math.round(s.rustLastHitScreenX)}, ${Math.round(s.rustLastHitScreenY)}`;
    return `${target}${field}${port}${screen}`;
  }

  function selectionValue(sel: SceneSelection): string {
    if (sel.kind === "canvas") return "canvas";
    if (sel.kind === "multi") return `multi:${sel.ids.length} objects`;
    return `${sel.kind}:${sel.id}`;
  }
</script>

{#snippet row(label: string, value: string | null | undefined)}
  <div class="renderer-diagnostics-row">
    <dt>{label}</dt>
    <dd>{value ?? "-"}</dd>
  </div>
{/snippet}

{#if open}
  <aside
    id="renderer-diagnostics"
    class="renderer-diagnostics-drawer"
    aria-label="Renderer diagnostics"
    onpointerdown={(event) => event.stopPropagation()}
    oncontextmenu={(event) => event.preventDefault()}
  >
    <div class="renderer-diagnostics-head">
      <div>
        <span>Diagnostics</span>
        <strong>{stats?.drawBackend ?? "renderer pending"}</strong>
      </div>
      <button class="icon-button" type="button" onclick={onClose} aria-label="Close diagnostics" title="Close diagnostics">
        <X size={15} />
      </button>
    </div>

    <section class="renderer-diagnostics-section">
      <h2>Health</h2>
      <dl>
        {@render row("Backend", stats?.backend)}
        {@render row("Renderer state", health?.state)}
        {@render row("Renderer detail", health?.detail)}
        {@render row("Rust core", health ? yesNo(health.rustAvailable) : null)}
        {@render row("Rust backend", health?.rustBackend)}
        {@render row("Draw backend", stats?.drawBackend)}
        {@render row("WebGPU available", stats ? yesNo(stats.webGpuRendererAvailable) : health ? yesNo(health.webGpuRendererAvailable) : null)}
        {@render row("App status/error", status || "Ready")}
        {@render row("Renderer status/error", rendererStatus || "No renderer status yet")}
      </dl>
    </section>

    <section class="renderer-diagnostics-section">
      <h2>Frame</h2>
      <dl>
        {@render row("Frame", stats ? ms(stats.frameMs) : null)}
        {@render row("Render", stats ? ms(stats.renderMs) : null)}
        {@render row("Visible", stats ? `${stats.visibleGroups} groups / ${stats.visibleCards} cards / ${stats.visibleEdges} edges` : null)}
        {@render row("Total", stats ? `${stats.totalGroups} groups / ${stats.totalCards} cards / ${stats.totalEdges} edges` : null)}
        {@render row("Memory", stats?.memoryBytes === null || stats?.memoryBytes === undefined ? null : bytes(stats.memoryBytes))}
        {@render row("Boundary calls", stats ? String(stats.boundaryCalls) : null)}
        {@render row("Rust boundary calls", stats ? String(stats.rustBoundaryCalls) : null)}
        {@render row("Input batch", stats ? String(stats.inputBatchSize) : null)}
      </dl>
    </section>

    <section class="renderer-diagnostics-section">
      <h2>Glyphs And Cache</h2>
      <dl>
        {@render row("Scene cache", stats ? `${stats.cacheHits} hit / ${stats.cacheMisses} miss` : null)}
        {@render row("Shaped glyphs", numberValue(stats?.rustTextGlyphs))}
        {@render row("Fallback glyphs", numberValue(stats?.rustFallbackGlyphs))}
        {@render row("Fallback runs", numberValue(stats?.rustFontFallbackRuns))}
        {@render row("CJK glyphs", numberValue(stats?.rustCjkGlyphs))}
        {@render row("Missing glyphs", numberValue(stats?.rustMissingGlyphs))}
        {@render row("Atlas glyphs", numberValue(stats?.rustTextAtlasGlyphs))}
        {@render row("Atlas overflow", numberValue(stats?.rustTextAtlasOverflowGlyphs))}
        {@render row("Missing rasters", numberValue(stats?.rustTextMissingRasterGlyphs))}
        {@render row("Raster cache", cachePair(stats?.rustTextRasterCacheHits, stats?.rustTextRasterCacheMisses))}
        {@render row("Layout cache", cachePair(stats?.rustTextLayoutCacheHits, stats?.rustTextLayoutCacheMisses))}
        {@render row("Style tokens", numberValue(stats?.rustStyleTokens))}
      </dl>
    </section>

    <section class="renderer-diagnostics-section">
      <h2>GPU Writes</h2>
      <dl>
        {@render row("Patch updates", numberValue(stats?.rustPatchUpdates))}
        {@render row("Dirty writes", numberValue(stats?.rustDirtyWrites))}
        {@render row("Full rebuilds", numberValue(stats?.rustFullRebuilds))}
        {@render row("GPU vertices", numberValue(stats?.rustGpuVertices))}
        {@render row("Drawn vertices", numberValue(stats?.rustDrawnVertices))}
        {@render row("Draw ranges", numberValue(stats?.rustDrawRanges))}
        {@render row("Vertex truncations", numberValue(stats?.rustVertexTruncations))}
        {@render row("Truncated vertices", numberValue(stats?.rustTruncatedVertices))}
      </dl>
    </section>

    <section class="renderer-diagnostics-section">
      <h2>Buffers</h2>
      <dl>
        {@render row("Group slots", slotValue(stats?.rustGroupSlots, stats?.rustFreeGroupSlots))}
        {@render row("Group grows", numberValue(stats?.rustGroupCapacityGrows))}
        {@render row("Group compactions", numberValue(stats?.rustGroupCompactions))}
        {@render row("Card slots", slotValue(stats?.rustCardSlots, stats?.rustFreeCardSlots))}
        {@render row("Card grows", numberValue(stats?.rustCardCapacityGrows))}
        {@render row("Card compactions", numberValue(stats?.rustCardCompactions))}
        {@render row("Edge slots", slotValue(stats?.rustEdgeSlots, stats?.rustFreeEdgeSlots))}
        {@render row("Edge grows", numberValue(stats?.rustEdgeCapacityGrows))}
        {@render row("Edge compactions", numberValue(stats?.rustEdgeCompactions))}
      </dl>
    </section>

    <section class="renderer-diagnostics-section">
      <h2>Camera And Target</h2>
      <dl>
        {@render row("App camera", `${camera.zoom.toFixed(3)}x @ ${Math.round(camera.x)}, ${Math.round(camera.y)}`)}
        {@render row("Rust camera", rustCameraValue(stats))}
        {@render row("App selection", selectionValue(selection))}
        {@render row("Rust selection", stats?.rustSelectionKind ? selectionValue({ kind: stats.rustSelectionKind, id: stats.rustSelectionId ?? undefined } as SceneSelection) : null)}
        {@render row("Rust last hit", rustLastHitValue(stats))}
        {@render row("Selected target", selectedTargetLabel)}
      </dl>
    </section>
  </aside>
{/if}
