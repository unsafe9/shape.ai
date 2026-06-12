// Renderer adapter scene types (object-native).
//
// The canonical canvas model is the object substrate (`platforms/web/shared/object.ts`,
// D1). The renderer's object draw entry consumes a `RenderObjectScene` — the
// object-substrate render view that mirrors the renderer-core
// `render_object::RenderObjectScene` serde shape (camelCase keys; `geometryD` for
// the path-string; bare 3x3 transform). The host (`lib/canvasHost.ts`) projects an
// `ObjectScene` into this shape before handing it to the renderer.
//
// Camera/coordinate primitives live in `platforms/web/shared/geometry.ts` (render-time
// camera/coords, not domain). The screen<->world helpers here are pure projection.

import type { CameraState, WorldPoint, WorldRect } from "../shared/geometry";

export type { CameraState, WorldPoint, WorldRect };

// ---------------------------------------------------------------------------
// Object render feed — the object-substrate render view the renderer draws.
// Mirrors `render_object::RenderObjectScene` (camelCase serde shape).
// ---------------------------------------------------------------------------

/** Bare 3x3 row-major transform `[[a,b,c],[d,e,f],[g,h,i]]`. */
export type RenderTransform3x3 = [
  [number, number, number],
  [number, number, number],
  [number, number, number]
];

export type RenderPaint =
  | { kind: "solid"; color: string }
  | { kind: "gradient"; stops: { offset: number; color: string }[]; angle: number }
  | { kind: "image"; contentRef: string };

export type RenderFill = {
  paint: RenderPaint;
  opacity?: number;
};

export type RenderStroke = {
  paint: RenderPaint;
  width: number;
  opacity?: number;
  dash?: number[];
  cap?: "butt" | "round" | "square";
  join?: "miter" | "round" | "bevel";
};

export type RenderTextRun = {
  text: string;
  color: string;
  size: number;
  bold?: boolean;
  italic?: boolean;
  font?: string;
};

export type RenderText = {
  runs: RenderTextRun[];
  align?: "start" | "center" | "end" | "justify";
  valign?: "top" | "middle" | "bottom";
};

/** One object as the renderer draws it (D1): path-string geometry + inline style. */
export type RenderObject = {
  id: string;
  parent: string | null;
  order: string;
  transform: RenderTransform3x3;
  geometryD: string;
  fill: RenderFill | null;
  stroke: RenderStroke | null;
  text: RenderText | null;
  clip: boolean;
};

/** The renderer-core object scene view (the renderer's object draw input). */
export type RenderObjectScene = {
  sceneId: string;
  camera: CameraState;
  objects: RenderObject[];
  /** Persisted single-anchor selection: id of the selected object, if any. */
  selection: string | null;
  /** Transient multi-select set (never persisted). */
  multiSelect: string[];
};

// ---------------------------------------------------------------------------
// Hit / overlay / frame stats — the adapter↔core call boundary value types.
// ---------------------------------------------------------------------------

export type RenderObjectKind = "group" | "card" | "edge" | "port" | "text";

export type HitResult = {
  kind: RenderObjectKind;
  id: string;
  groupId?: string;
  field?: "title" | "summary" | "detail";
  port?: "source" | "target";
  world: WorldPoint;
  screen: WorldPoint;
};

export type DomOverlayRequest = {
  target: {
    kind: "card-text";
    id: string;
    field: "title" | "summary" | "detail";
  };
  value: string;
  worldRect: WorldRect;
  screenRect: WorldRect;
  style: {
    fontFamily: string;
    fontSize: number;
    fontWeight: number;
    lineHeight: number;
    letterSpacing: number;
    paddingX: number;
    paddingY: number;
    textColor: string;
    backgroundColor: string;
    borderColor: string;
    borderWidth: number;
    borderRadius: number;
    focusRingColor: string;
    focusRingWidth: number;
    boxShadow: string;
    caretColor: string;
    accentColor: string;
    selectionBackgroundColor: string;
    maxLines: number;
    overflowX: "hidden";
    overflowY: "hidden" | "auto";
    state: "default" | "selected";
  };
};

export type FrameStats = {
  frameMs: number;
  renderMs: number;
  totalGroups: number;
  totalCards: number;
  totalEdges: number;
  visibleGroups: number;
  visibleCards: number;
  visibleEdges: number;
  cacheHits: number;
  cacheMisses: number;
  boundaryCalls: number;
  inputBatchSize: number;
  memoryBytes: number | null;
  backend: string;
  drawBackend: "rust-wgpu-visible";
  webGpuRendererAvailable: boolean;
  rustBoundaryCalls: number;
  rustFrameCards: number | null;
  rustFrameEdges: number | null;
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
  rustObjectCount: number | null;
  rustObjectFillIndices: number | null;
  rustObjectStrokeVertices: number | null;
  rustObjectDraws: number | null;
  rustCameraX: number | null;
  rustCameraY: number | null;
  rustCameraZoom: number | null;
  rustSelectionKind: string | null;
  rustSelectionId: string | null;
  rustLastHitKind: string | null;
  rustLastHitId: string | null;
  rustLastHitField: string | null;
  rustLastHitPort: string | null;
  rustLastHitScreenX: number | null;
  rustLastHitScreenY: number | null;
};

// ---------------------------------------------------------------------------
// Screen <-> world projection (pure; camera/coords only).
// ---------------------------------------------------------------------------

export function rectsIntersect(a: WorldRect, b: WorldRect): boolean {
  return a.x <= b.x + b.width && a.x + a.width >= b.x && a.y <= b.y + b.height && a.y + a.height >= b.y;
}

export function pointInRect(point: WorldPoint, rect: WorldRect): boolean {
  return point.x >= rect.x && point.x <= rect.x + rect.width && point.y >= rect.y && point.y <= rect.y + rect.height;
}

export function screenToWorld(point: WorldPoint, camera: CameraState): WorldPoint {
  return {
    x: (point.x - camera.x) / camera.zoom,
    y: (point.y - camera.y) / camera.zoom
  };
}

export function worldToScreen(point: WorldPoint, camera: CameraState): WorldPoint {
  return {
    x: point.x * camera.zoom + camera.x,
    y: point.y * camera.zoom + camera.y
  };
}

export function worldRectToScreen(rect: WorldRect, camera: CameraState): WorldRect {
  const topLeft = worldToScreen(rect, camera);
  return {
    x: topLeft.x,
    y: topLeft.y,
    width: rect.width * camera.zoom,
    height: rect.height * camera.zoom
  };
}

export function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, Math.max(0, maxLength - 1))}…`;
}

// ---------------------------------------------------------------------------
// Theme (AP4 #12c) — light/dark mode shell driver.
//
// The C1 contract owns the canonical token set (scene-core `object::theme`,
// renderer `object_theme`); the kebab names below mirror it so the shell can
// reference default object styles by token and drive the renderer theme-bit.
// `applyDocumentTheme` is the single toggle entry: it flips a root attribute
// (CSS chrome), persists the choice, and drives RB1's renderer theme-bit
// (`setObjectTheme`) so the canvas and chrome flip together.
// ---------------------------------------------------------------------------

export type Theme = "light" | "dark";

/** The C1 kebab token names (mirrors `object::theme::ALL_TOKENS`). */
export const THEME_TOKEN_NAMES = [
  "canvas-bg",
  "surface",
  "surface-muted",
  "default-fill",
  "default-stroke",
  "text",
  "shadow",
  "selection-ring"
] as const;

export type ThemeTokenName = (typeof THEME_TOKEN_NAMES)[number];

/** Default object style as C1 token refs (resolved renderer-side by RB1). */
export const DEFAULT_OBJECT_STYLE_TOKENS: {
  fill: ThemeTokenName;
  stroke: ThemeTokenName;
  text: ThemeTokenName;
} = {
  fill: "default-fill",
  stroke: "default-stroke",
  text: "text"
};

export const THEME_STORAGE_KEY = "shape-ai-theme";
export const THEME_ROOT_ATTRIBUTE = "data-theme";

/** Minimal injectable surfaces so the toggle stays unit-testable (no globals). */
export type ThemeRoot = { setAttribute(name: string, value: string): void };
export type ThemeStorage = { getItem(key: string): string | null; setItem(key: string, value: string): void };

export function isTheme(value: string | null): value is Theme {
  return value === "light" || value === "dark";
}

/** Read the persisted theme, defaulting to light when absent/invalid. */
export function readStoredTheme(storage: ThemeStorage): Theme {
  const stored = storage.getItem(THEME_STORAGE_KEY);
  return isTheme(stored) ? stored : "light";
}

/**
 * Apply `theme` across the shell: flip the root `data-theme` attribute (drives
 * the dark-mode CSS variables), persist it, and drive RB1's renderer theme-bit.
 * `setRendererTheme(dark)` is the feature-detected `ShapeWebGpuRenderer.setObjectTheme`
 * hook — called with the resolved dark bit so the canvas flips with the chrome.
 */
export function applyDocumentTheme(
  theme: Theme,
  opts: { root: ThemeRoot; storage: ThemeStorage; setRendererTheme?: (dark: boolean) => void }
): void {
  const dark = theme === "dark";
  opts.root.setAttribute(THEME_ROOT_ATTRIBUTE, theme);
  opts.storage.setItem(THEME_STORAGE_KEY, theme);
  opts.setRendererTheme?.(dark);
}
