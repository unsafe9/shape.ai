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

export const graphLayoutSchema = z.object({
  nodePositions: z.record(
    z.string(),
    z.object({
      x: z.number(),
      y: z.number()
    })
  ).default({}),
  nodeZOrder: z.record(z.string(), z.number()).default({})
});

export const graphSelectionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("graph") }),
  z.object({ kind: z.literal("node"), id: z.string().min(1) }),
  z.object({ kind: z.literal("edge"), id: z.string().min(1) })
]);

export const graphCommentSchema = z.object({
  id: z.string().min(1),
  target: graphSelectionSchema,
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
  scope: z.string().default("whole_graph"),
  path: z.string().min(1),
  contentType: z.string().min(1),
  createdAt: z.string().min(1),
  graphVersion: z.number().int().nonnegative()
});

export const designSchema = z.object({
  id: z.string().min(1),
  title: z.string().min(1),
  prompt: z.string().default(""),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1),
  graphVersion: z.number().int().nonnegative().default(0),
  graph: decisionGraphSchema,
  layout: graphLayoutSchema.default({ nodePositions: {}, nodeZOrder: {} }),
  selection: graphSelectionSchema.default({ kind: "graph" }),
  comments: z.array(graphCommentSchema).default([]),
  artifacts: z.array(artifactSchema).default([])
});

export const graphPatchSchema = z.object({
  addNodes: z.array(graphNodeSchema).default([]),
  updateNodes: z.array(graphNodeSchema).default([]),
  removeNodeIds: z.array(z.string()).default([]),
  addEdges: z.array(graphEdgeSchema).default([]),
  updateEdges: z.array(graphEdgeSchema).default([]),
  removeEdgeIds: z.array(z.string()).default([])
});

export const proposalStatusSchema = z.enum(["open", "changes_requested", "approved", "rejected"]);

export const proposalValidationStatusSchema = z.enum(["clean", "needs_rebase", "conflict", "invalid"]);

export const proposalSchema = z.object({
  id: z.string().min(1),
  designId: z.string().min(1),
  title: z.string().min(1),
  description: z.string().default(""),
  baseGraphVersion: z.number().int().nonnegative(),
  status: proposalStatusSchema.default("open"),
  validationStatus: proposalValidationStatusSchema.default("clean"),
  createdBy: z.string().min(1).default("agent"),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1)
});

export const proposalPatchSchema = z.object({
  id: z.string().min(1),
  proposalId: z.string().min(1),
  sequence: z.number().int().positive(),
  patch: graphPatchSchema,
  validationStatus: proposalValidationStatusSchema.default("clean"),
  createdAt: z.string().min(1)
});

export const proposalCommentSchema = z.object({
  id: z.string().min(1),
  proposalId: z.string().min(1),
  body: z.string().min(1),
  author: z.string().min(1).default("human"),
  resolved: z.boolean().default(false),
  createdAt: z.string().min(1),
  updatedAt: z.string().min(1)
});

export const designSeedOutputSchema = z.object({
  title: z.string().min(1),
  explanation: z.string().min(1),
  graph: decisionGraphSchema
});

export const exportOutputSchema = z.object({
  title: z.string().min(1),
  content: z.string().min(1),
  imagePrompt: z.string().optional()
});

export const createDesignRequestSchema = z.object({
  prompt: z.string().min(1),
  title: z.string().optional()
});

export const exportRequestSchema = z.object({
  type: exportTypeSchema,
  scope: z
    .object({
      kind: z.enum(["whole_graph", "node", "edge"]),
      id: z.string().optional()
    })
    .default({ kind: "whole_graph" })
});

export const graphEditRequestSchema = z.object({
  graph: decisionGraphSchema.optional(),
  layout: graphLayoutSchema.optional(),
  selection: graphSelectionSchema.optional()
});

export const createCommentRequestSchema = z.object({
  target: graphSelectionSchema,
  body: z.string().min(1),
  author: z.string().min(1).optional()
});

export const updateCommentRequestSchema = z.object({
  body: z.string().min(1).optional(),
  resolved: z.boolean().optional()
});

export type NodeType = z.infer<typeof nodeTypeSchema>;
export type EdgeType = z.infer<typeof edgeTypeSchema>;
export type NodeStatus = z.infer<typeof nodeStatusSchema>;
export type ExportType = z.infer<typeof exportTypeSchema>;
export type GraphNode = z.infer<typeof graphNodeSchema>;
export type GraphEdge = z.infer<typeof graphEdgeSchema>;
export type DecisionGraph = z.infer<typeof decisionGraphSchema>;
export type GraphLayout = z.infer<typeof graphLayoutSchema>;
export type GraphSelection = z.infer<typeof graphSelectionSchema>;
export type GraphComment = z.infer<typeof graphCommentSchema>;
export type DesignArtifact = z.infer<typeof artifactSchema>;
export type Design = z.infer<typeof designSchema>;
export type Shape = Design;
export type ShapeArtifact = DesignArtifact;
export type GraphPatch = z.infer<typeof graphPatchSchema>;
export type ProposalStatus = z.infer<typeof proposalStatusSchema>;
export type ProposalValidationStatus = z.infer<typeof proposalValidationStatusSchema>;
export type Proposal = z.infer<typeof proposalSchema>;
export type ProposalPatch = z.infer<typeof proposalPatchSchema>;
export type ProposalComment = z.infer<typeof proposalCommentSchema>;
export type DesignSeedOutput = z.infer<typeof designSeedOutputSchema>;
export type ShapeSeedOutput = DesignSeedOutput;
export type ExportOutput = z.infer<typeof exportOutputSchema>;
export type CreateDesignRequest = z.infer<typeof createDesignRequestSchema>;
export type CreateShapeRequest = CreateDesignRequest;
export type ExportRequest = z.infer<typeof exportRequestSchema>;
export type GraphEditRequest = z.infer<typeof graphEditRequestSchema>;
export type CreateCommentRequest = z.infer<typeof createCommentRequestSchema>;
export type UpdateCommentRequest = z.infer<typeof updateCommentRequestSchema>;
