// Renderer adapter scene types (object-native).
//
// The canonical canvas model is the object substrate (`src/shared/object.ts`,
// D1). The renderer's object draw entry consumes a `RenderObjectScene` — the
// object-substrate render view that mirrors the renderer-core
// `render_object::RenderObjectScene` serde shape (camelCase keys; `geometryD` for
// the path-string; bare 3x3 transform). The host (`lib/canvasHost.ts`) projects an
// `ObjectScene` into this shape before handing it to the renderer.
//
// Camera/coordinate primitives live in `src/shared/geometry.ts` (render-time
// camera/coords, not domain). The screen<->world helpers here are pure projection.
//
// The legacy 2D render-primitive types (`SceneSnapshot`/`ScenePatch` + friends)
// and the in-memory `applyScenePatch` mirror that remain below are the legacy
// `ShapeCanvasEngine` 2D harness substrate, kept self-contained here for the
// engine/benchmark/fixture path and its tests. They carry NO domain op-apply (the
// object op-apply is the scene-core wasm).

import type { SceneSelection } from "../../shared/schema";
import type { CameraState, WorldPoint, WorldRect } from "../../shared/geometry";

export type { CameraState, WorldPoint, WorldRect, SceneSelection };

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
  align?: "left" | "center" | "right";
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
  rustCameraX: number | null;
  rustCameraY: number | null;
  rustCameraZoom: number | null;
  rustSelectionKind: SceneSelection["kind"] | null;
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

// ===========================================================================
// Legacy 2D harness — `SceneSnapshot`/`ScenePatch` render primitives.
//
// These feed the `ShapeCanvasEngine` 2D path (engine/benchmark/fixtures) and its
// tests. They are NOT the object render feed above and carry NO domain op-apply
// (the object op-apply is the scene-core wasm). Self-contained here so the legacy
// `renderScene.ts`/`renderPatch.ts` modules can be deleted.
// ===========================================================================

export type SceneShadowLayerToken = {
  offsetX: number;
  offsetY: number;
  blur: number;
  spread?: number;
  color: string;
  alpha: number;
};

export type SceneStyleToken = {
  id: string;
  fill: string;
  stroke: string;
  text: string;
  mutedText: string;
  accent: string;
  surface?: string;
  surface2?: string;
  surface3?: string;
  pastel?: string;
  line?: string;
  lineStrong?: string;
  focus?: string;
  radius?: Record<string, number | undefined>;
  strokeWidths?: Record<string, number | undefined>;
  typography?: Record<string, number | undefined>;
  spacing?: Record<string, number | undefined>;
  shadow?: SceneShadowLayerToken[];
  selectedShadow?: SceneShadowLayerToken[];
  glow?: SceneShadowLayerToken[];
  gradient?: Record<string, number | undefined>;
  states?: {
    default?: Record<string, number | undefined>;
    selected?: Record<string, number | undefined>;
    compact?: Record<string, number | undefined>;
  };
  badge?: Record<string, number | undefined>;
  edge?: Record<string, number | undefined>;
  port?: Record<string, number | undefined>;
};

export type RenderGroup = {
  id: string;
  title: string;
  summary: string;
  bounds: WorldRect;
  tagIds: string[];
  zIndex: number;
  styleKey: string;
};

export type RenderCard = {
  id: string;
  groupId: string;
  title: string;
  summary: string;
  detail: string;
  status: string;
  type: string;
  bounds: WorldRect;
  zIndex: number;
  styleKey: string;
  accessibilityLabel: string;
};

export type RenderEdge = {
  id: string;
  groupId: string;
  source: string;
  target: string;
  label: string;
  type: string;
  zIndex: number;
  styleKey: string;
};

export type SceneSnapshot = {
  version: 1;
  sceneId: string;
  camera: CameraState;
  groups: RenderGroup[];
  cards: RenderCard[];
  edges: RenderEdge[];
  styles: SceneStyleToken[];
  selection: SceneSelection;
  metadata: {
    source: "fixture" | "shape-scene-adapter";
    generatedAt: string;
    fixtureSeed?: number;
    notes: string[];
  };
};

export type ScenePatch =
  | { kind: "create-group"; group: RenderGroup }
  | { kind: "delete-group"; id: string }
  | { kind: "move-group"; id: string; delta: WorldPoint }
  | { kind: "move-card"; id: string; position: WorldPoint }
  | { kind: "set-card-z-index"; id: string; zIndex: number }
  | { kind: "edit-card-text"; id: string; field: "title" | "summary" | "detail"; value: string }
  | { kind: "create-card"; card: RenderCard }
  | { kind: "delete-card"; id: string }
  | { kind: "create-edge"; groupId: string; source: string; target: string; edgeId: string; label?: string }
  | { kind: "delete-edge"; id: string }
  | { kind: "select"; selection: SceneSelection };

export const defaultStyles: SceneStyleToken[] = [
  shapeStyleToken("default", "#ffffff", "#7b8794", "#172026", "#65717b", "#158f83", "#f7f9fb"),
  shapeStyleToken("decision", "#f7fbff", "#2f7ee6", "#102033", "#5a7188", "#2f7ee6", "#ebf4ff"),
  shapeStyleToken("risk", "#fff8f1", "#c67914", "#2a1b0b", "#80684c", "#c67914", "#fdf2de"),
  shapeStyleToken("proposition", "#f4fbf9", "#19917f", "#10231f", "#56736e", "#19917f", "#e8f9f5"),
  shapeStyleToken("decision_point", "#f7fbff", "#2f7ee6", "#102033", "#5a7188", "#2f7ee6", "#ebf4ff"),
  shapeStyleToken("option", "#f4fbf6", "#26965e", "#10251a", "#5b7464", "#26965e", "#eaf9ef"),
  shapeStyleToken("evidence", "#f4fbff", "#228bb8", "#102432", "#5a7180", "#228bb8", "#e8f7fc"),
  shapeStyleToken("tradeoff", "#fff8f1", "#c17518", "#2a1b0b", "#80684c", "#c17518", "#fdf2de"),
  shapeStyleToken("blocker", "#fff7f8", "#d14c58", "#2c1014", "#84545a", "#d14c58", "#fdebed"),
  shapeStyleToken("subdecision", "#f8f7ff", "#7a68ce", "#1d1833", "#675f85", "#7a68ce", "#f1effd"),
  shapeStyleToken("task", "#f7faff", "#5371b3", "#111c33", "#5d6b87", "#5371b3", "#eef3fc"),
  shapeStyleToken("artifact", "#f7fafb", "#617a85", "#142027", "#65747a", "#617a85", "#eff5f6")
];

export function applyScenePatch(snapshot: SceneSnapshot, patch: ScenePatch): SceneSnapshot {
  if (patch.kind === "create-group") {
    if (snapshot.groups.some((group) => group.id === patch.group.id)) return snapshot;
    if (patch.group.bounds.width <= 0 || patch.group.bounds.height <= 0) return snapshot;
    return {
      ...snapshot,
      groups: [...snapshot.groups, patch.group],
      selection: { kind: "group", id: patch.group.id }
    };
  }
  if (patch.kind === "delete-group") {
    const deletedGroupIds = new Set([patch.id]);
    const removedCardIds = new Set(snapshot.cards.filter((card) => deletedGroupIds.has(card.groupId)).map((card) => card.id));
    const removedEdgeIds = new Set(
      snapshot.edges
        .filter((edge) => deletedGroupIds.has(edge.groupId) || removedCardIds.has(edge.source) || removedCardIds.has(edge.target))
        .map((edge) => edge.id)
    );
    return {
      ...snapshot,
      groups: snapshot.groups.filter((group) => !deletedGroupIds.has(group.id)),
      cards: snapshot.cards.filter((card) => !removedCardIds.has(card.id)),
      edges: snapshot.edges.filter((edge) => !removedEdgeIds.has(edge.id)),
      selection: selectionAfterGroupDelete(snapshot.selection, deletedGroupIds, removedCardIds, removedEdgeIds)
    };
  }
  if (patch.kind === "move-group") {
    return {
      ...snapshot,
      groups: snapshot.groups.map((group) =>
        group.id === patch.id ? { ...group, bounds: { ...group.bounds, x: group.bounds.x + patch.delta.x, y: group.bounds.y + patch.delta.y } } : group
      ),
      cards: snapshot.cards.map((card) =>
        card.groupId === patch.id ? { ...card, bounds: { ...card.bounds, x: card.bounds.x + patch.delta.x, y: card.bounds.y + patch.delta.y } } : card
      )
    };
  }
  if (patch.kind === "move-card") {
    return {
      ...snapshot,
      cards: snapshot.cards.map((card) =>
        card.id === patch.id ? { ...card, bounds: { ...card.bounds, x: patch.position.x, y: patch.position.y } } : card
      )
    };
  }
  if (patch.kind === "set-card-z-index") {
    return {
      ...snapshot,
      cards: snapshot.cards.map((card) => (card.id === patch.id ? { ...card, zIndex: patch.zIndex } : card)),
      selection: { kind: "node", id: patch.id }
    };
  }
  if (patch.kind === "edit-card-text") {
    return {
      ...snapshot,
      cards: snapshot.cards.map((card) => (card.id === patch.id ? { ...card, [patch.field]: patch.value } : card))
    };
  }
  if (patch.kind === "create-card") {
    if (snapshot.cards.some((card) => card.id === patch.card.id)) return snapshot;
    if (!snapshot.groups.some((group) => group.id === patch.card.groupId)) return snapshot;
    return {
      ...snapshot,
      cards: [...snapshot.cards, patch.card],
      selection: { kind: "node", id: patch.card.id }
    };
  }
  if (patch.kind === "delete-card") {
    const incidentEdgeIds = new Set(snapshot.edges.filter((edge) => edge.source === patch.id || edge.target === patch.id).map((edge) => edge.id));
    return {
      ...snapshot,
      cards: snapshot.cards.filter((card) => card.id !== patch.id),
      edges: snapshot.edges.filter((edge) => !incidentEdgeIds.has(edge.id)),
      selection: selectionAfterCardDelete(snapshot.selection, patch.id, incidentEdgeIds)
    };
  }
  if (patch.kind === "create-edge") {
    if (patch.source === patch.target) return snapshot;
    if (!snapshot.cards.some((card) => card.id === patch.source) || !snapshot.cards.some((card) => card.id === patch.target)) return snapshot;
    if (snapshot.edges.some((edge) => edge.id === patch.edgeId)) return snapshot;
    return {
      ...snapshot,
      edges: [
        ...snapshot.edges,
        {
          id: patch.edgeId,
          groupId: patch.groupId,
          source: patch.source,
          target: patch.target,
          label: patch.label ?? "relates",
          type: "supports",
          zIndex: snapshot.edges.length,
          styleKey: "default"
        }
      ]
    };
  }
  if (patch.kind === "delete-edge") {
    return { ...snapshot, edges: snapshot.edges.filter((edge) => edge.id !== patch.id) };
  }
  if (patch.kind === "select") {
    return { ...snapshot, selection: patch.selection };
  }
  return snapshot;
}

export function validateScenePatch(snapshot: SceneSnapshot, patch: ScenePatch): string[] {
  const errors: string[] = [];
  if (patch.kind === "create-group") {
    if (snapshot.groups.some((group) => group.id === patch.group.id)) errors.push(`Duplicate group id: ${patch.group.id}`);
    if (patch.group.bounds.width <= 0 || patch.group.bounds.height <= 0) errors.push("Group bounds must be positive");
  }
  if (patch.kind === "delete-group" && !snapshot.groups.some((group) => group.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
  if (patch.kind === "move-group" && !snapshot.groups.some((group) => group.id === patch.id)) errors.push(`Unknown group id: ${patch.id}`);
  if (patch.kind === "move-card" || patch.kind === "set-card-z-index" || patch.kind === "edit-card-text") {
    if (!snapshot.cards.some((card) => card.id === patch.id)) errors.push(`Unknown card id: ${patch.id}`);
  }
  if (patch.kind === "create-card") {
    if (snapshot.cards.some((card) => card.id === patch.card.id)) errors.push(`Duplicate card id: ${patch.card.id}`);
    if (!snapshot.groups.some((group) => group.id === patch.card.groupId)) errors.push(`Unknown group id: ${patch.card.groupId}`);
    if (patch.card.bounds.width <= 0 || patch.card.bounds.height <= 0) errors.push("Card bounds must be positive");
  }
  if (patch.kind === "delete-card" && !snapshot.cards.some((card) => card.id === patch.id)) errors.push(`Unknown card id: ${patch.id}`);
  if (patch.kind === "create-edge") {
    if (patch.source === patch.target) errors.push("Edge source and target must differ");
    if (!snapshot.cards.some((card) => card.id === patch.source)) errors.push(`Unknown source card id: ${patch.source}`);
    if (!snapshot.cards.some((card) => card.id === patch.target)) errors.push(`Unknown target card id: ${patch.target}`);
    if (!snapshot.groups.some((group) => group.id === patch.groupId)) errors.push(`Unknown group id: ${patch.groupId}`);
  }
  return errors;
}

function selectionAfterCardDelete(selection: SceneSelection, cardId: string, incidentEdgeIds: Set<string>): SceneSelection {
  if (selection.kind === "node" && selection.id === cardId) return { kind: "canvas" };
  if (selection.kind === "edge" && incidentEdgeIds.has(selection.id)) return { kind: "canvas" };
  return selection;
}

function selectionAfterGroupDelete(
  selection: SceneSelection,
  deletedGroupIds: Set<string>,
  removedCardIds: Set<string>,
  removedEdgeIds: Set<string>
): SceneSelection {
  if (selection.kind === "group" && deletedGroupIds.has(selection.id)) return { kind: "canvas" };
  if (selection.kind === "node" && removedCardIds.has(selection.id)) return { kind: "canvas" };
  if (selection.kind === "edge" && removedEdgeIds.has(selection.id)) return { kind: "canvas" };
  return selection;
}

export function sceneWorldBounds(snapshot: SceneSnapshot): WorldRect {
  const rects = [
    ...snapshot.groups.map((group) => group.bounds),
    ...snapshot.cards.map((card) => card.bounds)
  ];
  if (rects.length === 0) return { x: 0, y: 0, width: 1200, height: 800 };
  return unionRects(rects);
}

function unionRects(rects: WorldRect[]): WorldRect {
  const minX = Math.min(...rects.map((rect) => rect.x));
  const minY = Math.min(...rects.map((rect) => rect.y));
  const maxX = Math.max(...rects.map((rect) => rect.x + rect.width));
  const maxY = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

function shapeStyleToken(
  id: string,
  fill: string,
  stroke: string,
  text: string,
  mutedText: string,
  accent: string,
  pastel: string
): SceneStyleToken {
  return {
    id,
    fill,
    stroke,
    text,
    mutedText,
    accent,
    surface: "#ffffff",
    surface2: "#f7f9fb",
    surface3: "#fcfdfe",
    pastel,
    line: "#283644",
    lineStrong: "#1e2d3a",
    focus: "#2f7ee6",
    radius: {
      group: 34,
      groupSelected: 34,
      card: 16,
      cardSelected: 18,
      badge: 7,
      edgeLabel: 9,
      port: 8,
      focusRing: 20
    },
    strokeWidths: {
      group: 2,
      groupSelected: 2,
      card: 1,
      cardSelected: 1,
      inner: 1,
      focusRing: 4,
      edge: 3,
      edgeCompact: 2.2,
      edgeSelected: 5,
      separator: 1,
      port: 2
    },
    typography: {
      groupTitleSize: 38,
      groupSummarySize: 18,
      cardTitleSize: 19,
      cardSelectedTitleSize: 22,
      cardSummarySize: 13,
      badgeSize: 10,
      edgeLabelSize: 18
    },
    spacing: {
      groupPaddingX: 28,
      groupPaddingY: 24,
      cardPadding: 14,
      cardGap: 9,
      badgePaddingX: 7,
      badgeHeight: 20,
      labelPaddingX: 8,
      edgeLabelHeight: 24,
      portRadius: 7,
      separatorInset: 18
    },
    shadow: [
      { offsetX: 0, offsetY: 18, blur: 36, spread: 0, color: "#192430", alpha: 0.1 },
      { offsetX: 0, offsetY: 2, blur: 7, spread: 0, color: "#192430", alpha: 0.06 }
    ],
    selectedShadow: [
      { offsetX: 0, offsetY: 30, blur: 64, spread: 0, color: accent, alpha: 0.14 },
      { offsetX: 0, offsetY: 10, blur: 24, spread: 0, color: "#192430", alpha: 0.1 }
    ],
    glow: [{ offsetX: 0, offsetY: 0, blur: 0, spread: 4, color: accent, alpha: 0.12 }],
    gradient: {
      surfaceTopAlpha: 0.98,
      pastelBottomAlpha: 0.78,
      accentStartAlpha: 0.48,
      accentEndAlpha: 0.22
    },
    states: {
      default: { fillAlpha: 0.96, strokeAlpha: 0.16, shadowAlpha: 1 },
      selected: { fillAlpha: 0.98, strokeAlpha: 0.52, focusAlpha: 0.12, glowAlpha: 1 },
      compact: { strokeAlpha: 0.26 }
    },
    badge: {
      fillAlpha: 0.1,
      strokeAlpha: 0.16,
      textAlpha: 0.94,
      minWidth: 54
    },
    edge: {
      strokeAlpha: 0.42,
      selectedStrokeAlpha: 0.82,
      compactStrokeAlpha: 0.26,
      labelFillAlpha: 0.9,
      labelStrokeAlpha: 0.26,
      labelTextAlpha: 0.72
    },
    port: {
      fillAlpha: 0.94,
      strokeAlpha: 0.46,
      selectedFillAlpha: 0.18,
      selectedStrokeAlpha: 0.82
    }
  };
}
