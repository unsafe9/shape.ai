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

export type SceneStyleToken = {
  id: string;
  fill: string;
  stroke: string;
  text: string;
  mutedText: string;
  accent: string;
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
  {
    id: "default",
    fill: "#ffffff",
    stroke: "#7b8794",
    text: "#172026",
    mutedText: "#65717b",
    accent: "#158f83"
  },
  {
    id: "decision",
    fill: "#f7fbff",
    stroke: "#2f7ee6",
    text: "#102033",
    mutedText: "#5a7188",
    accent: "#2f7ee6"
  },
  {
    id: "evidence",
    fill: "#f4fbf7",
    stroke: "#1aa269",
    text: "#11251a",
    mutedText: "#5a7563",
    accent: "#1aa269"
  },
  {
    id: "risk",
    fill: "#fff8f1",
    stroke: "#c67914",
    text: "#2a1b0b",
    mutedText: "#80684c",
    accent: "#c67914"
  }
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
    styleKey: node.type === "evidence" ? "evidence" : node.type === "blocker" || node.type === "tradeoff" ? "risk" : "decision",
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
