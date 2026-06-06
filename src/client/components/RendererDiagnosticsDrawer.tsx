import { X } from "lucide-react";
import type { ReactNode } from "react";
import type { CameraState } from "../../shared/renderScene";
import type { SceneSelection } from "../../shared/schema";
import type { RendererHealth, RendererStats } from "./RendererCanvasHost";

type RendererDiagnosticsDrawerProps = {
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

export function RendererDiagnosticsDrawer({
  open,
  stats,
  health,
  camera,
  selection,
  selectedTargetLabel,
  status,
  rendererStatus,
  onClose
}: RendererDiagnosticsDrawerProps) {
  if (!open) return null;

  return (
    <aside
      id="renderer-diagnostics"
      className="renderer-diagnostics-drawer"
      aria-label="Renderer diagnostics"
      onPointerDown={(event) => event.stopPropagation()}
      onContextMenu={(event) => event.preventDefault()}
    >
      <div className="renderer-diagnostics-head">
        <div>
          <span>Diagnostics</span>
          <strong>{stats?.drawBackend ?? "renderer pending"}</strong>
        </div>
        <button className="icon-button" type="button" onClick={onClose} aria-label="Close diagnostics" title="Close diagnostics">
          <X size={15} />
        </button>
      </div>

      <DiagnosticsSection title="Health">
        <DiagnosticRow label="Backend" value={stats?.backend} />
        <DiagnosticRow label="Renderer state" value={health?.state} />
        <DiagnosticRow label="Renderer detail" value={health?.detail} />
        <DiagnosticRow label="Rust core" value={health ? yesNo(health.rustAvailable) : null} />
        <DiagnosticRow label="Rust backend" value={health?.rustBackend} />
        <DiagnosticRow label="Draw backend" value={stats?.drawBackend} />
        <DiagnosticRow label="WebGPU available" value={stats ? yesNo(stats.webGpuRendererAvailable) : health ? yesNo(health.webGpuRendererAvailable) : null} />
        <DiagnosticRow label="App status/error" value={status || "Ready"} />
        <DiagnosticRow label="Renderer status/error" value={rendererStatus || "No renderer status yet"} />
      </DiagnosticsSection>

      <DiagnosticsSection title="Frame">
        <DiagnosticRow label="Frame" value={stats ? ms(stats.frameMs) : null} />
        <DiagnosticRow label="Render" value={stats ? ms(stats.renderMs) : null} />
        <DiagnosticRow label="Visible" value={stats ? `${stats.visibleGroups} groups / ${stats.visibleCards} cards / ${stats.visibleEdges} edges` : null} />
        <DiagnosticRow label="Total" value={stats ? `${stats.totalGroups} groups / ${stats.totalCards} cards / ${stats.totalEdges} edges` : null} />
        <DiagnosticRow label="Memory" value={stats?.memoryBytes === null || stats?.memoryBytes === undefined ? null : bytes(stats.memoryBytes)} />
        <DiagnosticRow label="Boundary calls" value={stats ? String(stats.boundaryCalls) : null} />
        <DiagnosticRow label="Rust boundary calls" value={stats ? String(stats.rustBoundaryCalls) : null} />
        <DiagnosticRow label="Input batch" value={stats ? String(stats.inputBatchSize) : null} />
      </DiagnosticsSection>

      <DiagnosticsSection title="Glyphs And Cache">
        <DiagnosticRow label="Scene cache" value={stats ? `${stats.cacheHits} hit / ${stats.cacheMisses} miss` : null} />
        <DiagnosticRow label="Shaped glyphs" value={numberValue(stats?.rustTextGlyphs)} />
        <DiagnosticRow label="Fallback glyphs" value={numberValue(stats?.rustFallbackGlyphs)} />
        <DiagnosticRow label="Fallback runs" value={numberValue(stats?.rustFontFallbackRuns)} />
        <DiagnosticRow label="CJK glyphs" value={numberValue(stats?.rustCjkGlyphs)} />
        <DiagnosticRow label="Missing glyphs" value={numberValue(stats?.rustMissingGlyphs)} />
        <DiagnosticRow label="Atlas glyphs" value={numberValue(stats?.rustTextAtlasGlyphs)} />
        <DiagnosticRow label="Atlas overflow" value={numberValue(stats?.rustTextAtlasOverflowGlyphs)} />
        <DiagnosticRow label="Missing rasters" value={numberValue(stats?.rustTextMissingRasterGlyphs)} />
        <DiagnosticRow label="Raster cache" value={cachePair(stats?.rustTextRasterCacheHits, stats?.rustTextRasterCacheMisses)} />
        <DiagnosticRow label="Layout cache" value={cachePair(stats?.rustTextLayoutCacheHits, stats?.rustTextLayoutCacheMisses)} />
        <DiagnosticRow label="Style tokens" value={numberValue(stats?.rustStyleTokens)} />
      </DiagnosticsSection>

      <DiagnosticsSection title="GPU Writes">
        <DiagnosticRow label="Patch updates" value={numberValue(stats?.rustPatchUpdates)} />
        <DiagnosticRow label="Dirty writes" value={numberValue(stats?.rustDirtyWrites)} />
        <DiagnosticRow label="Full rebuilds" value={numberValue(stats?.rustFullRebuilds)} />
        <DiagnosticRow label="GPU vertices" value={numberValue(stats?.rustGpuVertices)} />
        <DiagnosticRow label="Drawn vertices" value={numberValue(stats?.rustDrawnVertices)} />
        <DiagnosticRow label="Draw ranges" value={numberValue(stats?.rustDrawRanges)} />
        <DiagnosticRow label="Vertex truncations" value={numberValue(stats?.rustVertexTruncations)} />
        <DiagnosticRow label="Truncated vertices" value={numberValue(stats?.rustTruncatedVertices)} />
      </DiagnosticsSection>

      <DiagnosticsSection title="Buffers">
        <DiagnosticRow label="Group slots" value={slotValue(stats?.rustGroupSlots, stats?.rustFreeGroupSlots)} />
        <DiagnosticRow label="Group grows" value={numberValue(stats?.rustGroupCapacityGrows)} />
        <DiagnosticRow label="Group compactions" value={numberValue(stats?.rustGroupCompactions)} />
        <DiagnosticRow label="Card slots" value={slotValue(stats?.rustCardSlots, stats?.rustFreeCardSlots)} />
        <DiagnosticRow label="Card grows" value={numberValue(stats?.rustCardCapacityGrows)} />
        <DiagnosticRow label="Card compactions" value={numberValue(stats?.rustCardCompactions)} />
        <DiagnosticRow label="Edge slots" value={slotValue(stats?.rustEdgeSlots, stats?.rustFreeEdgeSlots)} />
        <DiagnosticRow label="Edge grows" value={numberValue(stats?.rustEdgeCapacityGrows)} />
        <DiagnosticRow label="Edge compactions" value={numberValue(stats?.rustEdgeCompactions)} />
      </DiagnosticsSection>

      <DiagnosticsSection title="Camera And Target">
        <DiagnosticRow label="App camera" value={`${camera.zoom.toFixed(3)}x @ ${Math.round(camera.x)}, ${Math.round(camera.y)}`} />
        <DiagnosticRow label="Rust camera" value={rustCameraValue(stats)} />
        <DiagnosticRow label="App selection" value={selectionValue(selection)} />
        <DiagnosticRow label="Rust selection" value={stats?.rustSelectionKind ? selectionValue({ kind: stats.rustSelectionKind, id: stats.rustSelectionId ?? undefined } as SceneSelection) : null} />
        <DiagnosticRow label="Rust last hit" value={rustLastHitValue(stats)} />
        <DiagnosticRow label="Selected target" value={selectedTargetLabel} />
      </DiagnosticsSection>
    </aside>
  );
}

function DiagnosticsSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="renderer-diagnostics-section">
      <h2>{title}</h2>
      <dl>{children}</dl>
    </section>
  );
}

function DiagnosticRow({ label, value }: { label: string; value: string | null | undefined }) {
  return (
    <div className="renderer-diagnostics-row">
      <dt>{label}</dt>
      <dd>{value ?? "-"}</dd>
    </div>
  );
}

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

function rustCameraValue(stats: RendererStats | null): string | null {
  if (!stats || stats.rustCameraX === null || stats.rustCameraY === null || stats.rustCameraZoom === null) return null;
  return `${stats.rustCameraZoom.toFixed(3)}x @ ${Math.round(stats.rustCameraX)}, ${Math.round(stats.rustCameraY)}`;
}

function rustLastHitValue(stats: RendererStats | null): string | null {
  if (!stats?.rustLastHitKind) return null;
  const target = stats.rustLastHitId ? `${stats.rustLastHitKind}:${stats.rustLastHitId}` : stats.rustLastHitKind;
  const field = stats.rustLastHitField ? `:${stats.rustLastHitField}` : "";
  const port = stats.rustLastHitPort ? `:${stats.rustLastHitPort}` : "";
  const screen =
    stats.rustLastHitScreenX === null || stats.rustLastHitScreenY === null
      ? ""
      : ` @ ${Math.round(stats.rustLastHitScreenX)}, ${Math.round(stats.rustLastHitScreenY)}`;
  return `${target}${field}${port}${screen}`;
}

function selectionValue(selection: SceneSelection): string {
  if (selection.kind === "canvas") return "canvas";
  if (selection.kind === "multi") return `multi:${selection.ids.length} objects`;
  return `${selection.kind}:${selection.id}`;
}
