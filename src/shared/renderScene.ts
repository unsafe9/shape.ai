import type { Scene, SceneEdge, SceneGroup, SceneNode, SceneSelection } from "./schema";

export type RenderObjectKind = "group" | "card" | "edge" | "port" | "text";

export type WorldPoint = {
  x: number;
  y: number;
};

export type WorldRect = WorldPoint & {
  width: number;
  height: number;
};

export type CameraState = {
  x: number;
  y: number;
  zoom: number;
};

export type SceneShadowLayerToken = {
  offsetX: number;
  offsetY: number;
  blur: number;
  spread?: number;
  color: string;
  alpha: number;
};

export type SceneRadiusToken = {
  group?: number;
  groupSelected?: number;
  card?: number;
  cardSelected?: number;
  badge?: number;
  edgeLabel?: number;
  port?: number;
  focusRing?: number;
};

export type SceneStrokeWidthToken = {
  group?: number;
  groupSelected?: number;
  card?: number;
  cardSelected?: number;
  inner?: number;
  focusRing?: number;
  edge?: number;
  edgeCompact?: number;
  edgeSelected?: number;
  separator?: number;
  port?: number;
};

export type SceneTypographyToken = {
  groupTitleSize?: number;
  groupSummarySize?: number;
  cardTitleSize?: number;
  cardSelectedTitleSize?: number;
  cardSummarySize?: number;
  badgeSize?: number;
  edgeLabelSize?: number;
};

export type SceneSpacingToken = {
  groupPaddingX?: number;
  groupPaddingY?: number;
  cardPadding?: number;
  cardGap?: number;
  badgePaddingX?: number;
  badgeHeight?: number;
  labelPaddingX?: number;
  edgeLabelHeight?: number;
  portRadius?: number;
  separatorInset?: number;
};

export type SceneGradientToken = {
  surfaceTopAlpha?: number;
  pastelBottomAlpha?: number;
  accentStartAlpha?: number;
  accentEndAlpha?: number;
};

export type SceneStateVariantToken = {
  fillAlpha?: number;
  strokeAlpha?: number;
  focusAlpha?: number;
  shadowAlpha?: number;
  glowAlpha?: number;
};

export type SceneBadgeToken = {
  fillAlpha?: number;
  strokeAlpha?: number;
  textAlpha?: number;
  minWidth?: number;
};

export type SceneEdgeStyleToken = {
  strokeAlpha?: number;
  selectedStrokeAlpha?: number;
  compactStrokeAlpha?: number;
  labelFillAlpha?: number;
  labelStrokeAlpha?: number;
  labelTextAlpha?: number;
};

export type ScenePortStyleToken = {
  fillAlpha?: number;
  strokeAlpha?: number;
  selectedFillAlpha?: number;
  selectedStrokeAlpha?: number;
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
  radius?: SceneRadiusToken;
  strokeWidths?: SceneStrokeWidthToken;
  typography?: SceneTypographyToken;
  spacing?: SceneSpacingToken;
  shadow?: SceneShadowLayerToken[];
  selectedShadow?: SceneShadowLayerToken[];
  glow?: SceneShadowLayerToken[];
  gradient?: SceneGradientToken;
  states?: {
    default?: SceneStateVariantToken;
    selected?: SceneStateVariantToken;
    compact?: SceneStateVariantToken;
  };
  badge?: SceneBadgeToken;
  edge?: SceneEdgeStyleToken;
  port?: ScenePortStyleToken;
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

export type RenderSnapshotOptions = {
  camera?: CameraState;
  generatedAt?: string;
  sceneId?: string;
};

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

export function shapeSceneToRenderSnapshot(scene: Scene, options: RenderSnapshotOptions = {}): SceneSnapshot {
  return {
    version: 1,
    sceneId: options.sceneId ?? `shape-scene-v${scene.sceneVersion}`,
    camera: options.camera ?? { x: 140, y: 120, zoom: 0.28 },
    groups: scene.groups.map(groupToFrame),
    cards: scene.nodes.map(nodeToCard),
    edges: scene.edges.map(edgeToRenderEdge),
    styles: defaultStyles,
    selection: scene.selection,
    metadata: {
      source: "shape-scene-adapter",
      generatedAt: options.generatedAt ?? scene.updatedAt,
      notes: [
        "Adapter includes only renderable group, card, edge, text, z-order, tag id, and selection fields.",
        "Comments, artifacts, export state, confidence, evidence refs, and MCP/proposal semantics stay in the TypeScript app layer."
      ]
    }
  };
}

export function excludedBusinessFields(): string[] {
  return [
    "Scene.tags.name/color registry",
    "Scene.comments",
    "Scene.artifacts",
    "Node.confidence",
    "Node.evidenceRefs",
    "Node.childDecisionIds",
    "Edge.confidence",
    "Edge.rationale",
    "Export/proposal/MCP workflow state"
  ];
}

function groupToFrame(group: SceneGroup): RenderGroup {
  return {
    id: group.id,
    title: group.title,
    summary: group.summary,
    bounds: group.bounds,
    tagIds: group.tagIds,
    zIndex: group.zIndex,
    styleKey: "default"
  };
}

function nodeToCard(node: SceneNode): RenderCard {
  return {
    id: node.id,
    groupId: node.groupId,
    title: node.title,
    summary: node.summary,
    detail: node.detail,
    status: node.status,
    type: node.type,
    bounds: {
      x: node.position.x,
      y: node.position.y,
      width: node.size.width,
      height: node.size.height
    },
    zIndex: node.zIndex,
    styleKey: node.type,
    accessibilityLabel: `${node.type} ${node.title}. ${node.summary}`
  };
}

function edgeToRenderEdge(edge: SceneEdge): RenderEdge {
  return {
    id: edge.id,
    groupId: edge.groupId,
    source: edge.source,
    target: edge.target,
    label: edge.label,
    type: edge.type,
    zIndex: 0,
    styleKey: edge.type === "blocks" || edge.type === "trades_off_with" ? "risk" : "default"
  };
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
