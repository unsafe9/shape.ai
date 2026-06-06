import type { ShapeCanvasEngine } from "./engine";
import { sceneWorldBounds, type FrameStats, type SceneSnapshot, type WorldRect } from "./scene";

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

// -- T3.4 interaction-latency harness --
//
// `runScriptedPanZoom` measures steady-state *frame time*. Interaction quality is
// a discrete-event *latency* question: when the user presses to select, drags a
// card, or starts typing, how long until the result is committed and painted.
// This harness times input -> commit -> paint per scripted interaction, reusing
// the same `FrameStats` the engine already returns from every input batch / frame
// (`runScriptedPanZoom` and `BenchmarkResult` are unchanged). It scripts events
// through the real engine seams; it does not move logic across the boundary.

export type InteractionKind = "pan" | "zoom" | "select" | "drag" | "text-edit" | "follow";

export type InteractionLatencyResult = {
  kind: InteractionKind;
  samples: number; // discrete interactions scripted
  // input -> commit(engine seam returns) -> paint(stats in hand) round trip, ms
  p50LatencyMs: number;
  p95LatencyMs: number;
  maxLatencyMs: number;
  // frames whose FrameStats.frameMs exceeded `frameBudgetMs` during the interaction
  droppedFrames: number;
  // load context captured from the FrameStats the engine already returns
  totalCards: number; // FrameStats.totalCards (scene scale under test)
  averageVisibleCards: number; // FrameStats.visibleCards (post-cull working set)
  memoryBytes: number | null; // FrameStats.memoryBytes
  // working-set ratio = visible / total: the TS-visible degradation signal. Under
  // load this falls (more objects cull/degrade) while latency must stay bounded —
  // the operational "degrade-don't-stall" read. (Per-tier counts live in the Rust
  // WebGpuFrameStats and are asserted in the core; they are not surfaced to TS.)
  workingSetRatio: number;
};

// Seed per-flow latency budgets (input->paint p95), anchored to a 60fps frame
// budget. Tunable by measurement against mixed-5k/mixed-workspace, exactly like
// the T3.1 tier thresholds and the T3.2 cache caps. The contract is the *shape*
// (per-flow p95 <= budget, bounded drops, degrade-don't-stall); the milliseconds
// are the first calibration point.
export const FRAME_BUDGET_MS = 1000 / 60; // 16.7ms

export const INTERACTION_LATENCY_BUDGET_MS: Record<InteractionKind, number> = {
  pan: FRAME_BUDGET_MS,
  zoom: 33, // a tier-crossing frame may rebuild caches/density once
  select: 50, // a click feels instant under ~50ms incl. far-zoom hit-test
  drag: FRAME_BUDGET_MS, // continuous pointer tracking; every move is a frame
  "text-edit": 50, // mount -> first paint
  follow: FRAME_BUDGET_MS // user input under follow must hold its own budget
};

type LatencySample = { latencyMs: number; stats: FrameStats; dropped: boolean };

/**
 * Time one frame of `step()` (which drives a real engine seam and returns the
 * `FrameStats` in hand). The latency is the input->commit->paint round trip; the
 * frame is "dropped" if `FrameStats.frameMs` exceeded the frame budget.
 */
function timeInteraction(step: () => FrameStats, frameBudgetMs: number): LatencySample {
  const start = performance.now();
  const stats = step();
  const latencyMs = performance.now() - start;
  return { latencyMs, stats, dropped: stats.frameMs > frameBudgetMs };
}

function summarizeInteraction(kind: InteractionKind, samples: LatencySample[], frameBudgetMs: number): InteractionLatencyResult {
  const latencies = samples.map((sample) => sample.latencyMs).sort((a, b) => a - b);
  const last = samples.at(-1)?.stats ?? null;
  const totalCards = last?.totalCards ?? 0;
  const averageVisibleCards = average(samples.map((sample) => sample.stats.visibleCards));
  return {
    kind,
    samples: samples.length,
    p50LatencyMs: percentile(latencies, 0.5),
    p95LatencyMs: percentile(latencies, 0.95),
    maxLatencyMs: latencies.length > 0 ? Math.max(...latencies) : 0,
    droppedFrames: samples.filter((sample) => sample.dropped).length,
    totalCards,
    averageVisibleCards,
    memoryBytes: last?.memoryBytes ?? null,
    workingSetRatio: totalCards > 0 ? averageVisibleCards / totalCards : 0
  };
}

/**
 * Drive a discrete-interaction flow through the real engine seams and report
 * per-interaction latency. Each flow scripts the same call a real pointer/wheel
 * event would take (`setCamera`/`wheelAtScreen`/`applyPatchBatch`/...), times the
 * input->paint round trip, and reads the `FrameStats` the engine already returns.
 *
 * `steps` is the number of discrete interactions to script. `frameBudgetMs`
 * defaults to one 60fps frame; a flow's seed budget lives in
 * `INTERACTION_LATENCY_BUDGET_MS`.
 */
export function runInteractionLatency(
  engine: ShapeCanvasEngine,
  kind: InteractionKind,
  steps = 60,
  frameBudgetMs = FRAME_BUDGET_MS
): InteractionLatencyResult {
  const snapshot = engine.getSnapshot();
  if (!snapshot) throw new Error("No scene loaded");
  const bounds = sceneWorldBounds(snapshot);
  const samples: LatencySample[] = [];

  for (let index = 0; index < steps; index += 1) {
    const t = index / Math.max(1, steps - 1);
    const sample = timeInteraction(() => stepInteraction(engine, kind, snapshot, bounds, t), frameBudgetMs);
    samples.push(sample);
  }

  return summarizeInteraction(kind, samples, frameBudgetMs);
}

/**
 * Run a flow's latency while a follow loop animates `focusBounds` between targets
 * (the P5/T5.3 seam). Proves the §4 contention contract: scripted user-input
 * latency under follow must stay within the flow's no-follow budget, because both
 * ride the single `sendInputBatch` boundary and are serialized, not raced.
 */
export function runInteractionLatencyUnderFollow(
  engine: ShapeCanvasEngine,
  kind: InteractionKind,
  steps = 60,
  frameBudgetMs = FRAME_BUDGET_MS
): InteractionLatencyResult {
  const snapshot = engine.getSnapshot();
  if (!snapshot) throw new Error("No scene loaded");
  const bounds = sceneWorldBounds(snapshot);
  const samples: LatencySample[] = [];
  const followTargets = followTargetBounds(snapshot, bounds);

  for (let index = 0; index < steps; index += 1) {
    const t = index / Math.max(1, steps - 1);
    // The follow loop animates a camera focus before the user input each frame.
    // It rides the same boundary, so it is serialized with the timed input rather
    // than racing it — the spectator camera never steals input responsiveness.
    engine.focusBounds(followTargets[index % followTargets.length], { screen: { x: 620, y: 380 }, zoom: 0.5 });
    const sample = timeInteraction(() => stepInteraction(engine, kind, snapshot, bounds, t), frameBudgetMs);
    samples.push(sample);
  }

  return summarizeInteraction(kind, samples, frameBudgetMs);
}

function stepInteraction(
  engine: ShapeCanvasEngine,
  kind: InteractionKind,
  snapshot: SceneSnapshot,
  bounds: WorldRect,
  t: number
): FrameStats {
  // The seam call is the input -> commit half (it routes through the engine's
  // input-batch boundary internally); `renderFrame` is the paint half. Timing the
  // whole `stepInteraction` captures the input->paint round trip, mirroring how
  // `runScriptedPanZoom` reads `engine.renderFrame(performance.now())` per step.
  switch (kind) {
    case "pan":
    case "follow":
    case "text-edit": {
      // Fixed-velocity sweep across the fixture bounds at a mid zoom band. For
      // text-edit the overlay is mounted by the caller and re-projects each frame
      // via the same pan path (`updateOverlayPosition`).
      const zoom = 0.5;
      const worldX = bounds.x + bounds.width * t;
      const worldY = bounds.y + bounds.height * (0.35 + Math.sin(t * Math.PI * 2) * 0.18);
      engine.setCamera({ zoom, x: 620 - worldX * zoom, y: 380 - worldY * zoom });
      break;
    }
    case "zoom": {
      // Discrete zoom steps that cross the T3.1 tier thresholds.
      const zoom = 0.08 + Math.sin(t * Math.PI) * 0.72;
      engine.wheelAtScreen({ x: 620, y: 380 }, zoom > 0.4 ? -120 : 120);
      break;
    }
    case "select": {
      // Scripted selection of a known card across the scene via a select patch
      // (the same patch the core's `apply_input_event` returns from a pointer-down).
      const card = snapshot.cards[Math.floor(t * (snapshot.cards.length - 1))];
      engine.applyPatchBatch([{ kind: "select", selection: { kind: "node", id: card.id } }]);
      break;
    }
    case "drag": {
      // pointer-move equivalent: a `move-card` patch per step over one card.
      const card = snapshot.cards[0];
      const dx = Math.round(Math.sin(t * Math.PI * 2) * 40);
      const dy = Math.round(Math.cos(t * Math.PI * 2) * 40);
      engine.applyPatchBatch([
        { kind: "move-card", id: card.id, position: { x: card.bounds.x + dx, y: card.bounds.y + dy } }
      ]);
      break;
    }
  }
  return engine.renderFrame(performance.now());
}

/** A handful of card bounds to use as follow-loop targets (the companion jump). */
function followTargetBounds(snapshot: SceneSnapshot, fallback: WorldRect): WorldRect[] {
  const cards = snapshot.cards;
  if (cards.length === 0) return [fallback];
  const stride = Math.max(1, Math.floor(cards.length / 8));
  const targets: WorldRect[] = [];
  for (let index = 0; index < cards.length; index += stride) targets.push(cards[index].bounds);
  return targets.length > 0 ? targets : [fallback];
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
