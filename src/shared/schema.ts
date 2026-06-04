import { z } from "zod";

export const nodeTypeSchema = z.enum([
  "proposition",
  "decision_point",
  "option",
  "evidence",
  "tradeoff",
  "blocker",
  "subdecision",
  "task",
  "artifact"
]);

export const edgeTypeSchema = z.enum([
  "depends_on",
  "supports",
  "blocks",
  "trades_off_with",
  "chooses_between",
  "decomposes_to",
  "produces"
]);

export const nodeStatusSchema = z.enum([
  "draft",
  "viable",
  "conditional",
  "infeasible",
  "unknown",
  "selected",
  "deferred",
  "complete"
]);

export const exportTypeSchema = z.enum([
  "madr",
  "yadr",
  "image_prompt",
  "ai_plan_md",
  "design_doc_md",
  "confluence_html",
  "mermaid",
  "architecture_image"
]);

export const pointSchema = z.object({
  x: z.number(),
  y: z.number()
});

export const sizeSchema = z.object({
  width: z.number().positive(),
  height: z.number().positive()
});

export const boundsSchema = pointSchema.extend({
  width: z.number().positive(),
  height: z.number().positive()
});

export const graphNodeSchema = z.object({
  id: z.string().min(1),
  type: nodeTypeSchema,
  title: z.string().min(1),
  summary: z.string().default(""),
  detail: z.string().default(""),
  status: nodeStatusSchema.default("draft"),
  confidence: z.number().min(0).max(1).default(0.5),
  evidenceRefs: z.array(z.string()).default([]),
  childDecisionIds: z.array(z.string()).default([])
});

export const graphEdgeSchema = z.object({
  id: z.string().min(1),
  type: edgeTypeSchema,
  source: z.string().min(1),
  target: z.string().min(1),
  label: z.string().default(""),
  rationale: z.string().default(""),
  confidence: z.number().min(0).max(1).default(0.5)
});

export const decisionGraphSchema = z.object({
  version: z.literal(1).default(1),
  nodes: z.array(graphNodeSchema).default([]),
  edges: z.array(graphEdgeSchema).default([])
});

export const tagSchema = z.object({
  id: z.string().min(1),
  name: z.string().min(1),
  color: z.string().min(1),
  description: z.string().default(""),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1)
});

export const sceneGroupSchema = z.object({
  id: z.string().min(1),
  parentGroupId: z.string().min(1).nullable().default(null),
  title: z.string().min(1),
  summary: z.string().default(""),
  bounds: boundsSchema,
  tagIds: z.array(z.string()).default([]),
  zIndex: z.number().default(0),
  collapsed: z.boolean().default(false),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1)
});

export const sceneNodeSchema = graphNodeSchema.extend({
  groupId: z.string().min(1),
  position: pointSchema,
  size: sizeSchema.default({ width: 390, height: 390 }),
  zIndex: z.number().default(0),
  updatedAt: z.string().min(1).optional()
});

export const sceneEdgeSchema = graphEdgeSchema.extend({
  groupId: z.string().min(1),
  updatedAt: z.string().min(1).optional()
});

export const sceneSelectionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("canvas") }),
  z.object({ kind: z.literal("group"), id: z.string().min(1) }),
  z.object({ kind: z.literal("node"), id: z.string().min(1) }),
  z.object({ kind: z.literal("edge"), id: z.string().min(1) })
]);

export const sceneCommentSchema = z.object({
  id: z.string().min(1),
  target: sceneSelectionSchema,
  body: z.string().min(1),
  author: z.string().min(1).default("human"),
  resolved: z.boolean().default(false),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1)
});

export const artifactSchema = z.object({
  id: z.string().min(1),
  type: exportTypeSchema,
  title: z.string().min(1),
  target: sceneSelectionSchema,
  path: z.string().min(1),
  contentType: z.string().min(1),
  createdAt: z.string().min(1),
  sceneVersion: z.number().int().nonnegative()
});

export const sceneSchema = z.object({
  version: z.literal(1).default(1),
  sceneVersion: z.number().int().nonnegative().default(0),
  groups: z.array(sceneGroupSchema).default([]),
  nodes: z.array(sceneNodeSchema).default([]),
  edges: z.array(sceneEdgeSchema).default([]),
  tags: z.array(tagSchema).default([]),
  comments: z.array(sceneCommentSchema).default([]),
  artifacts: z.array(artifactSchema).default([]),
  selection: sceneSelectionSchema.default({ kind: "canvas" }),
  updatedAt: z.string().min(1)
});

export const createGroupRequestSchema = z.object({
  prompt: z.string().min(1),
  title: z.string().optional(),
  parentGroupId: z.string().min(1).nullable().optional(),
  tagIds: z.array(z.string()).optional()
});

export const createTagRequestSchema = z.object({
  name: z.string().min(1),
  color: z.string().min(1),
  description: z.string().optional()
});

export const updateTagRequestSchema = z.object({
  name: z.string().min(1).optional(),
  color: z.string().min(1).optional(),
  description: z.string().optional()
});

export const updateGroupTagsRequestSchema = z.object({
  tagIds: z.array(z.string())
});

export const exportScopeSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("group"), id: z.string().min(1) }),
  z.object({ kind: z.literal("node"), id: z.string().min(1) }),
  z.object({ kind: z.literal("edge"), id: z.string().min(1) }),
  z.object({ kind: z.literal("selection") })
]);

export const exportRequestSchema = z.object({
  type: exportTypeSchema,
  scope: exportScopeSchema.optional()
});

export const scenePatchSchema = z.object({
  groups: z.array(sceneGroupSchema).optional(),
  nodes: z.array(sceneNodeSchema).optional(),
  edges: z.array(sceneEdgeSchema).optional(),
  translateGroups: z.array(z.object({ groupId: z.string().min(1), dx: z.number(), dy: z.number() })).optional(),
  removeGroupIds: z.array(z.string()).optional(),
  removeNodeIds: z.array(z.string()).optional(),
  removeEdgeIds: z.array(z.string()).optional(),
  selection: sceneSelectionSchema.optional()
});

export const createCommentRequestSchema = z.object({
  target: sceneSelectionSchema,
  body: z.string().min(1),
  author: z.string().min(1).optional()
});

export const updateCommentRequestSchema = z.object({
  body: z.string().min(1).optional(),
  resolved: z.boolean().optional()
});

export const exportOutputSchema = z.object({
  title: z.string().min(1),
  content: z.string().min(1),
  imagePrompt: z.string().optional()
});

export const graphPatchSchema = z.object({
  addNodes: z.array(graphNodeSchema).default([]),
  updateNodes: z.array(graphNodeSchema).default([]),
  removeNodeIds: z.array(z.string()).default([]),
  addEdges: z.array(graphEdgeSchema).default([]),
  updateEdges: z.array(graphEdgeSchema).default([]),
  removeEdgeIds: z.array(z.string()).default([])
});

export type NodeType = z.infer<typeof nodeTypeSchema>;
export type EdgeType = z.infer<typeof edgeTypeSchema>;
export type NodeStatus = z.infer<typeof nodeStatusSchema>;
export type ExportType = z.infer<typeof exportTypeSchema>;
export type Point = z.infer<typeof pointSchema>;
export type Size = z.infer<typeof sizeSchema>;
export type Bounds = z.infer<typeof boundsSchema>;
export type GraphNode = z.infer<typeof graphNodeSchema>;
export type GraphEdge = z.infer<typeof graphEdgeSchema>;
export type DecisionGraph = z.infer<typeof decisionGraphSchema>;
export type Tag = z.infer<typeof tagSchema>;
export type SceneGroup = z.infer<typeof sceneGroupSchema>;
export type SceneNode = z.infer<typeof sceneNodeSchema>;
export type SceneEdge = z.infer<typeof sceneEdgeSchema>;
export type SceneSelection = z.infer<typeof sceneSelectionSchema>;
export type SceneComment = z.infer<typeof sceneCommentSchema>;
export type SceneArtifact = z.infer<typeof artifactSchema>;
export type Scene = z.infer<typeof sceneSchema>;
export type GraphComment = SceneComment;
export type CreateGroupRequest = z.infer<typeof createGroupRequestSchema>;
export type CreateTagRequest = z.infer<typeof createTagRequestSchema>;
export type UpdateTagRequest = z.infer<typeof updateTagRequestSchema>;
export type UpdateGroupTagsRequest = z.infer<typeof updateGroupTagsRequestSchema>;
export type ExportRequest = z.infer<typeof exportRequestSchema>;
export type ScenePatch = z.infer<typeof scenePatchSchema>;
export type CreateCommentRequest = z.infer<typeof createCommentRequestSchema>;
export type UpdateCommentRequest = z.infer<typeof updateCommentRequestSchema>;
export type ExportOutput = z.infer<typeof exportOutputSchema>;
export type GraphPatch = z.infer<typeof graphPatchSchema>;

export type GroupSeedOutput = {
  title: string;
  explanation: string;
  group: SceneGroup;
  nodes: SceneNode[];
  edges: SceneEdge[];
};
