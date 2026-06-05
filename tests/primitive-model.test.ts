import { describe, expect, it } from "vitest";
import {
  objectMetaSchema,
  sceneNodeSchema,
  sceneEdgeSchema,
  sceneGroupSchema,
  sceneCommentSchema,
  artifactSchema,
  primitiveKind,
  type SceneNode,
  type SceneEdge,
  type SceneGroup,
  type SceneComment,
  type SceneArtifact
} from "../src/shared/schema";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const baseNode: SceneNode = {
  id: "n1",
  groupId: "g1",
  type: "decision_point",
  title: "Adopt sql.js",
  summary: "Use sql.js for local-first storage",
  detail: "Detailed rationale here",
  status: "viable",
  confidence: 0.8,
  evidenceRefs: [],
  childDecisionIds: [],
  tagIds: [],
  position: { x: 100, y: 200 },
  size: { width: 390, height: 390 },
  zIndex: 0,
  updatedAt: "2024-01-01T00:00:00Z"
};

const baseEdge: SceneEdge = {
  id: "e1",
  groupId: "g1",
  type: "supports",
  source: "n1",
  target: "n2",
  label: "supports decision",
  rationale: "because of XYZ",
  confidence: 0.7,
  tagIds: [],
  updatedAt: "2024-01-01T00:00:00Z"
};

const baseGroup: SceneGroup = {
  id: "g1",
  parentGroupId: null,
  title: "ADR Frame",
  summary: "Architecture Decision Record",
  bounds: { x: 0, y: 0, width: 800, height: 600 },
  tagIds: [],
  zIndex: 0,
  collapsed: false,
  createdAt: "2024-01-01T00:00:00Z",
  updatedAt: "2024-01-01T00:00:00Z"
};

const baseComment: SceneComment = {
  id: "c1",
  target: { kind: "node", id: "n1" },
  body: "This is a comment",
  author: "human",
  resolved: false,
  createdAt: "2024-01-01T00:00:00Z",
  updatedAt: "2024-01-01T00:00:00Z"
};

const baseArtifact: SceneArtifact = {
  id: "a1",
  type: "architecture_image",
  title: "Architecture diagram",
  target: { kind: "canvas" },
  path: "/artifacts/a1.png",
  contentType: "image/png",
  createdAt: "2024-01-01T00:00:00Z",
  sceneVersion: 1
};

// ---------------------------------------------------------------------------
// objectMetaSchema round-trip
// ---------------------------------------------------------------------------

describe("objectMetaSchema", () => {
  it("accepts undefined (field is optional)", () => {
    const result = objectMetaSchema.safeParse(undefined);
    expect(result.success).toBe(true);
  });

  it("accepts an empty object", () => {
    const result = objectMetaSchema.safeParse({});
    expect(result.success).toBe(true);
    expect(result.data).toEqual({});
  });

  it("round-trips arbitrary key/value pairs", () => {
    const meta = {
      semanticType: "decision_point",
      status: "viable",
      confidence: 0.7,
      evidenceRefs: ["ref1", "ref2"],
      templateKind: "adr",
      nested: { a: 1 }
    };
    const result = objectMetaSchema.safeParse(meta);
    expect(result.success).toBe(true);
    expect(result.data).toEqual(meta);
  });

  it("rejects non-string keys (only allows record<string, unknown>)", () => {
    // z.record(z.string(), z.unknown()) requires the input to be a plain object.
    // Arrays are not plain records.
    const result = objectMetaSchema.safeParse([1, 2, 3]);
    expect(result.success).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// sceneNodeSchema meta field round-trip
// ---------------------------------------------------------------------------

describe("sceneNodeSchema meta field", () => {
  it("parses a node without meta (backward compat)", () => {
    const result = sceneNodeSchema.safeParse(baseNode);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toBeUndefined();
  });

  it("parses a node with a populated meta bag", () => {
    const node = {
      ...baseNode,
      meta: {
        templateKind: "adr",
        semanticType: "decision_point",
        status: "viable",
        confidence: 0.7,
        evidenceRefs: ["ref1"]
      }
    };
    const result = sceneNodeSchema.safeParse(node);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toEqual(node.meta);
  });

  it("preserves all existing node fields alongside meta", () => {
    const node = { ...baseNode, meta: { x: 42 } };
    const result = sceneNodeSchema.safeParse(node);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.type).toBe("decision_point");
    expect(result.data.status).toBe("viable");
    expect(result.data.confidence).toBe(0.8);
    expect(result.data.evidenceRefs).toEqual([]);
    expect(result.data.childDecisionIds).toEqual([]);
    expect(result.data.meta).toEqual({ x: 42 });
  });
});

// ---------------------------------------------------------------------------
// sceneEdgeSchema meta field round-trip
// ---------------------------------------------------------------------------

describe("sceneEdgeSchema meta field", () => {
  it("parses an edge without meta (backward compat)", () => {
    const result = sceneEdgeSchema.safeParse(baseEdge);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toBeUndefined();
  });

  it("parses an edge with a populated meta bag", () => {
    const edge = {
      ...baseEdge,
      meta: { semanticType: "supports", rationale: "because of XYZ" }
    };
    const result = sceneEdgeSchema.safeParse(edge);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toEqual(edge.meta);
  });

  it("preserves existing edge enum fields alongside meta", () => {
    const edge = { ...baseEdge, meta: { custom: true } };
    const result = sceneEdgeSchema.safeParse(edge);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.type).toBe("supports");
    expect(result.data.rationale).toBe("because of XYZ");
    expect(result.data.confidence).toBe(0.7);
    expect(result.data.meta).toEqual({ custom: true });
  });
});

// ---------------------------------------------------------------------------
// sceneGroupSchema meta field round-trip
// ---------------------------------------------------------------------------

describe("sceneGroupSchema meta field", () => {
  it("parses a group without meta (backward compat)", () => {
    const result = sceneGroupSchema.safeParse(baseGroup);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toBeUndefined();
  });

  it("parses a group with a populated meta bag", () => {
    const group = {
      ...baseGroup,
      meta: { templateKind: "adr", projectName: "shape.ai" }
    };
    const result = sceneGroupSchema.safeParse(group);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.meta).toEqual(group.meta);
  });

  it("preserves existing group fields alongside meta", () => {
    const group = { ...baseGroup, meta: { slideIndex: 3 } };
    const result = sceneGroupSchema.safeParse(group);
    expect(result.success).toBe(true);
    if (!result.success) throw result.error;
    expect(result.data.title).toBe("ADR Frame");
    expect(result.data.collapsed).toBe(false);
    expect(result.data.meta).toEqual({ slideIndex: 3 });
  });
});

// ---------------------------------------------------------------------------
// primitiveKind helper
// ---------------------------------------------------------------------------

describe("primitiveKind helper", () => {
  it('maps SceneGroup → "frame"', () => {
    expect(primitiveKind(baseGroup)).toBe("frame");
  });

  it('maps SceneNode → "shape"', () => {
    expect(primitiveKind(baseNode)).toBe("shape");
  });

  it('maps SceneEdge → "edge"', () => {
    expect(primitiveKind(baseEdge)).toBe("edge");
  });

  it('maps SceneComment → "comment_marker"', () => {
    expect(primitiveKind(baseComment)).toBe("comment_marker");
  });

  it('maps SceneArtifact → "image_artifact"', () => {
    expect(primitiveKind(baseArtifact)).toBe("image_artifact");
  });

  it("works for a node with meta (shape is still shape)", () => {
    const nodeWithMeta: SceneNode = { ...baseNode, meta: { semanticType: "decision_point" } };
    expect(primitiveKind(nodeWithMeta)).toBe("shape");
  });

  it("works for a group with meta (frame is still frame)", () => {
    const groupWithMeta: SceneGroup = { ...baseGroup, meta: { templateKind: "adr" } };
    expect(primitiveKind(groupWithMeta)).toBe("frame");
  });

  it("works for an edge with meta (edge is still edge)", () => {
    const edgeWithMeta: SceneEdge = { ...baseEdge, meta: { semanticType: "supports" } };
    expect(primitiveKind(edgeWithMeta)).toBe("edge");
  });
});
