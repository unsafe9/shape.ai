import type { ShapeCanvasEngine } from "./engine";
import { sceneWorldBounds, type FrameStats } from "./scene";

export type BenchmarkResult = {
  frames: number;
  averageFrameMs: number;
  p95FrameMs: number;
  maxFrameMs: number;
  averageVisibleCards: number;
  memoryBytes: number | null;
  backend: string;
  drawBackend: FrameStats["drawBackend"];
  webGpuRendererAvailable: boolean;
  rustBoundaryCalls: number;
  rustFrameCards: number | null;
  rustGpuVertices: number | null;
  rustDrawnVertices: number | null;
  rustDrawRanges: number | null;
  rustTextGlyphs: number | null;
  rustFallbackGlyphs: number | null;
  rustCjkGlyphs: number | null;
  rustFontFallbackRuns: number | null;
  rustMissingGlyphs: number | null;
  rustTextAtlasOverflowGlyphs: number | null;
  rustTextMissingRasterGlyphs: number | null;
  rustTextAtlasGlyphs: number | null;
  rustTextRasterCacheHits: number | null;
  rustTextRasterCacheMisses: number | null;
  rustTextLayoutCacheHits: number | null;
  rustTextLayoutCacheMisses: number | null;
  rustStyleTokens: number | null;
  rustPatchUpdates: number | null;
  rustDirtyWrites: number | null;
  rustFullRebuilds: number | null;
  rustVertexTruncations: number | null;
  rustTruncatedVertices: number | null;
  rustEdgeCapacityGrows: number | null;
  rustEdgeCompactions: number | null;
  rustEdgeSlots: number | null;
  rustFreeEdgeSlots: number | null;
  rustCardCapacityGrows: number | null;
  rustCardCompactions: number | null;
  rustCardSlots: number | null;
  rustFreeCardSlots: number | null;
  rustGroupCapacityGrows: number | null;
  rustGroupCompactions: number | null;
  rustGroupSlots: number | null;
  rustFreeGroupSlots: number | null;
};

export async function runScriptedPanZoom(engine: ShapeCanvasEngine, frames = 180): Promise<BenchmarkResult> {
  const snapshot = engine.getSnapshot();
  if (!snapshot) throw new Error("No scene loaded");
  const bounds = sceneWorldBounds(snapshot);
  const samples: FrameStats[] = [];
  for (let index = 0; index < frames; index += 1) {
    const t = index / Math.max(1, frames - 1);
    const zoom = 0.08 + Math.sin(t * Math.PI) * 0.72;
    const worldX = bounds.x + bounds.width * t;
    const worldY = bounds.y + bounds.height * (0.35 + Math.sin(t * Math.PI * 2) * 0.18);
    engine.setCamera({
      zoom,
      x: 620 - worldX * zoom,
      y: 380 - worldY * zoom
    });
    samples.push(engine.renderFrame(performance.now()));
    await nextFrame();
  }
  const frameTimes = samples.map((sample) => sample.frameMs).sort((a, b) => a - b);
  const averageFrameMs = average(frameTimes);
  return {
    frames,
    averageFrameMs,
    p95FrameMs: percentile(frameTimes, 0.95),
    maxFrameMs: Math.max(...frameTimes),
    averageVisibleCards: average(samples.map((sample) => sample.visibleCards)),
    memoryBytes: samples.at(-1)?.memoryBytes ?? null,
    backend: samples.at(-1)?.backend ?? "unknown",
    drawBackend: samples.at(-1)?.drawBackend ?? "rust-wgpu-visible",
    webGpuRendererAvailable: samples.at(-1)?.webGpuRendererAvailable ?? false,
    rustBoundaryCalls: samples.at(-1)?.rustBoundaryCalls ?? 0,
    rustFrameCards: samples.at(-1)?.rustFrameCards ?? null,
    rustGpuVertices: samples.at(-1)?.rustGpuVertices ?? null,
    rustDrawnVertices: samples.at(-1)?.rustDrawnVertices ?? null,
    rustDrawRanges: samples.at(-1)?.rustDrawRanges ?? null,
    rustTextGlyphs: samples.at(-1)?.rustTextGlyphs ?? null,
    rustFallbackGlyphs: samples.at(-1)?.rustFallbackGlyphs ?? null,
    rustCjkGlyphs: samples.at(-1)?.rustCjkGlyphs ?? null,
    rustFontFallbackRuns: samples.at(-1)?.rustFontFallbackRuns ?? null,
    rustMissingGlyphs: samples.at(-1)?.rustMissingGlyphs ?? null,
    rustTextAtlasOverflowGlyphs: samples.at(-1)?.rustTextAtlasOverflowGlyphs ?? null,
    rustTextMissingRasterGlyphs: samples.at(-1)?.rustTextMissingRasterGlyphs ?? null,
    rustTextAtlasGlyphs: samples.at(-1)?.rustTextAtlasGlyphs ?? null,
    rustTextRasterCacheHits: samples.at(-1)?.rustTextRasterCacheHits ?? null,
    rustTextRasterCacheMisses: samples.at(-1)?.rustTextRasterCacheMisses ?? null,
    rustTextLayoutCacheHits: samples.at(-1)?.rustTextLayoutCacheHits ?? null,
    rustTextLayoutCacheMisses: samples.at(-1)?.rustTextLayoutCacheMisses ?? null,
    rustStyleTokens: samples.at(-1)?.rustStyleTokens ?? null,
    rustPatchUpdates: samples.at(-1)?.rustPatchUpdates ?? null,
    rustDirtyWrites: samples.at(-1)?.rustDirtyWrites ?? null,
    rustFullRebuilds: samples.at(-1)?.rustFullRebuilds ?? null,
    rustVertexTruncations: samples.at(-1)?.rustVertexTruncations ?? null,
    rustTruncatedVertices: samples.at(-1)?.rustTruncatedVertices ?? null,
    rustEdgeCapacityGrows: samples.at(-1)?.rustEdgeCapacityGrows ?? null,
    rustEdgeCompactions: samples.at(-1)?.rustEdgeCompactions ?? null,
    rustEdgeSlots: samples.at(-1)?.rustEdgeSlots ?? null,
    rustFreeEdgeSlots: samples.at(-1)?.rustFreeEdgeSlots ?? null,
    rustCardCapacityGrows: samples.at(-1)?.rustCardCapacityGrows ?? null,
    rustCardCompactions: samples.at(-1)?.rustCardCompactions ?? null,
    rustCardSlots: samples.at(-1)?.rustCardSlots ?? null,
    rustFreeCardSlots: samples.at(-1)?.rustFreeCardSlots ?? null,
    rustGroupCapacityGrows: samples.at(-1)?.rustGroupCapacityGrows ?? null,
    rustGroupCompactions: samples.at(-1)?.rustGroupCompactions ?? null,
    rustGroupSlots: samples.at(-1)?.rustGroupSlots ?? null,
    rustFreeGroupSlots: samples.at(-1)?.rustFreeGroupSlots ?? null
  };
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

function average(values: number[]): number {
  return values.reduce((sum, value) => sum + value, 0) / Math.max(1, values.length);
}

function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const index = Math.min(values.length - 1, Math.floor(values.length * p));
  return values[index];
}
