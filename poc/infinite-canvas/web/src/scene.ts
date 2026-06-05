import type { SceneSelection } from "../../../../src/shared/schema";
import { defaultStyles } from "../../../../src/shared/renderScene";
import type {
  CameraState,
  RenderCard,
  RenderEdge,
  RenderGroup,
  RenderObjectKind,
  SceneSnapshot,
  SceneStyleToken,
  WorldPoint,
  WorldRect
} from "../../../../src/shared/renderScene";

export { defaultStyles };
export type {
  CameraState,
  RenderCard,
  RenderEdge,
  RenderGroup,
  RenderObjectKind,
  SceneSnapshot,
  SceneSelection,
  SceneStyleToken,
  WorldPoint,
  WorldRect
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
  drawBackend: "typescript-canvas2d" | "rust-wasm-debug" | "rust-wgpu-visible";
  rustCoreAvailable: boolean;
  rustBoundaryCalls: number;
  rustFrameCards: number | null;
  rustFrameEdges: number | null;
  rustGpuVertices: number | null;
  rustTextGlyphs: number | null;
  rustFallbackGlyphs: number | null;
  rustCjkGlyphs: number | null;
  rustStyleTokens: number | null;
  rustPatchUpdates: number | null;
  rustDirtyWrites: number | null;
  rustFullRebuilds: number | null;
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

export type SpatialIndex = {
  cellSize: number;
  cells: Map<string, string[]>;
  boundsById: Map<string, WorldRect>;
};

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
  return { ...snapshot, selection: patch.selection };
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

export function buildSpatialIndex(cards: RenderCard[], cellSize = 640): SpatialIndex {
  const cells = new Map<string, string[]>();
  const boundsById = new Map<string, WorldRect>();
  for (const card of cards) {
    boundsById.set(card.id, card.bounds);
    for (const key of cellKeysForRect(card.bounds, cellSize)) {
      const bucket = cells.get(key) ?? [];
      bucket.push(card.id);
      cells.set(key, bucket);
    }
  }
  return { cellSize, cells, boundsById };
}

export function querySpatialIndex(index: SpatialIndex, viewport: WorldRect): Set<string> {
  const ids = new Set<string>();
  for (const key of cellKeysForRect(viewport, index.cellSize)) {
    for (const id of index.cells.get(key) ?? []) ids.add(id);
  }
  return ids;
}

export function sceneWorldBounds(snapshot: SceneSnapshot): WorldRect {
  const rects = [
    ...snapshot.groups.map((group) => group.bounds),
    ...snapshot.cards.map((card) => card.bounds)
  ];
  if (rects.length === 0) return { x: 0, y: 0, width: 1200, height: 800 };
  return unionRects(rects);
}

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

export function viewportToWorld(width: number, height: number, camera: CameraState): WorldRect {
  const topLeft = screenToWorld({ x: 0, y: 0 }, camera);
  const bottomRight = screenToWorld({ x: width, y: height }, camera);
  return {
    x: topLeft.x,
    y: topLeft.y,
    width: bottomRight.x - topLeft.x,
    height: bottomRight.y - topLeft.y
  };
}

export function fitCameraToBounds(bounds: WorldRect, viewportWidth: number, viewportHeight: number, padding = 96): CameraState {
  const usableWidth = Math.max(120, viewportWidth - padding * 2);
  const usableHeight = Math.max(120, viewportHeight - padding * 2);
  const zoom = clamp(Math.min(usableWidth / bounds.width, usableHeight / bounds.height), 0.04, 1.6);
  return {
    zoom,
    x: viewportWidth / 2 - (bounds.x + bounds.width / 2) * zoom,
    y: viewportHeight / 2 - (bounds.y + bounds.height / 2) * zoom
  };
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

export function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, Math.max(0, maxLength - 1))}…`;
}

function unionRects(rects: WorldRect[]): WorldRect {
  const minX = Math.min(...rects.map((rect) => rect.x));
  const minY = Math.min(...rects.map((rect) => rect.y));
  const maxX = Math.max(...rects.map((rect) => rect.x + rect.width));
  const maxY = Math.max(...rects.map((rect) => rect.y + rect.height));
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}

function cellKeysForRect(rect: WorldRect, cellSize: number): string[] {
  const minX = Math.floor(rect.x / cellSize);
  const minY = Math.floor(rect.y / cellSize);
  const maxX = Math.floor((rect.x + rect.width) / cellSize);
  const maxY = Math.floor((rect.y + rect.height) / cellSize);
  const keys: string[] = [];
  for (let y = minY; y <= maxY; y += 1) {
    for (let x = minX; x <= maxX; x += 1) keys.push(`${x}:${y}`);
  }
  return keys;
}
