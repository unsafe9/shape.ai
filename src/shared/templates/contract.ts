/**
 * T4.1 — Template Contract
 *
 * A TemplateContract is a *recipe over existing primitives*: it declares the
 * objects/edges/groups/tags to create (as the existing create-op types), default
 * layout, allowed exports, suggested tags, and optional AI prompt hints.
 *
 * Applying a template lowers entirely to existing RenderScenePatch ops
 * (create-group, create-card, create-edge) run through applyRenderPatchToShapeScene.
 * The output is indistinguishable from hand-drawn objects — only meta.templateKind
 * marks their origin, and that is an ordinary meta string.
 *
 * Real symbols used: SceneGroup, SceneNode, SceneEdge, Tag (schema.ts),
 * RenderScenePatch / applyRenderPatchToShapeScene (renderPatch.ts),
 * ExportType / exportTypeSchema (schema.ts).
 */

import type { ExportType, Scene, SceneEdge, SceneGroup, SceneNode, Tag } from "../schema";
import { applyRenderPatchToShapeScene } from "../renderPatch";
import type { RenderCard, RenderGroup, WorldPoint } from "../renderScene";

// ---------------------------------------------------------------------------
// §1 Template metadata
// ---------------------------------------------------------------------------

export type TemplateId = string;

export type TemplateMetadata = {
  id: TemplateId;
  title: string;
  description: string;
  category: "planning" | "knowledge" | "engineering" | "presentation" | "general";
  icon?: string;
  /** Value written to created objects' meta.templateKind (T0.1 §3 / T2.1 §2 convention). */
  templateKind: string;
};

// ---------------------------------------------------------------------------
// §2 Primitive recipe
// ---------------------------------------------------------------------------

export type RecipeFrame = {
  localId: string;
  title: string;
  summary?: string;
  parentLocalId?: string;
  tagLocalIds?: string[];
  meta?: Record<string, unknown>;
};

export type RecipeShape = {
  localId: string;
  frameLocalId: string;
  title?: string;
  summary?: string;
  detail?: string;
  /** Chosen by the template, not derived from a demoted nodeType (D1 C3). */
  styleKey?: string;
  tagLocalIds?: string[];
  meta?: Record<string, unknown>;
  position: WorldPoint;
  size?: { width: number; height: number };
};

export type RecipeEdge = {
  localId: string;
  frameLocalId: string;
  sourceLocalId: string;
  targetLocalId: string;
  label?: string;
  styleKey?: string;
  meta?: Record<string, unknown>;
};

export type TemplateRecipe = {
  frames: RecipeFrame[];
  shapes: RecipeShape[];
  edges: RecipeEdge[];
};

// ---------------------------------------------------------------------------
// §3 Default layout
// ---------------------------------------------------------------------------

export type RecipeLayout = {
  origin?: WorldPoint;
  defaultShapeSize?: { width: number; height: number };
};

// ---------------------------------------------------------------------------
// §4 Allowed exports
// ---------------------------------------------------------------------------

export type TemplateExports = {
  allowed: ExportType[];
  default?: ExportType;
};

// ---------------------------------------------------------------------------
// §5 Suggested tags
// ---------------------------------------------------------------------------

export type SuggestedTag = {
  localId: string;
  name: string;
  color: string;
  description?: string;
};

export type TemplateTags = {
  suggested: SuggestedTag[];
};

// ---------------------------------------------------------------------------
// §6 Optional AI prompt hints
// ---------------------------------------------------------------------------

export type TemplatePromptHints = {
  systemHint?: string;
  fieldHints?: Record<string, string>;
  suggestedOperations?: string[];
};

// ---------------------------------------------------------------------------
// §7 Full contract
// ---------------------------------------------------------------------------

export type TemplateContract = {
  metadata: TemplateMetadata;
  recipe: TemplateRecipe;
  layout: RecipeLayout;
  exports: TemplateExports;
  tags: TemplateTags;
  promptHints?: TemplatePromptHints;
};

// ---------------------------------------------------------------------------
// Application result
// ---------------------------------------------------------------------------

export type AppliedTemplate = {
  group: SceneGroup;
  nodes: SceneNode[];
  edges: SceneEdge[];
  /** Tags that were newly created by the template (not pre-existing). */
  newTags: Tag[];
  errors: string[];
};

// ---------------------------------------------------------------------------
// §7 applyTemplate — lowers recipe to normal canvas objects
// ---------------------------------------------------------------------------

const DEFAULT_SHAPE_SIZE = { width: 390, height: 390 };
const NOW = "2026-06-05T00:00:00.000Z";

/**
 * Apply a TemplateContract anchored at `anchor` world position, producing
 * normal SceneGroup / SceneNode / SceneEdge objects.
 *
 * The output objects are indistinguishable from hand-drawn ones except for
 * meta.templateKind. No template runtime or private mode is left behind.
 *
 * @param template  The contract to apply.
 * @param anchor    World-space origin for the template (replaces RecipeLayout.origin).
 * @param idPrefix  Optional prefix for generated ids (default: "tpl").
 * @param now       ISO timestamp string (default: stable fixture value).
 */
export function applyTemplate(
  template: TemplateContract,
  anchor: WorldPoint,
  idPrefix = "tpl",
  now = NOW
): AppliedTemplate {
  const templateKind = template.metadata.templateKind;
  const shapeSize = template.layout.defaultShapeSize ?? DEFAULT_SHAPE_SIZE;

  // 1. Build local→scene id map for frames, shapes, edges, and tags.
  const idMap = new Map<string, string>();
  let seq = 0;
  function sceneId(localId: string): string {
    if (!idMap.has(localId)) {
      idMap.set(localId, `${idPrefix}-${localId}-${seq++}`);
    }
    return idMap.get(localId)!;
  }

  // 2. Resolve suggested tags — produce Tag objects (no DB; pure objects).
  const newTags: Tag[] = template.tags.suggested.map((st) => {
    const tagId = sceneId(st.localId);
    return {
      id: tagId,
      name: st.name,
      color: st.color,
      description: st.description ?? "",
      createdAt: now,
      updatedAt: now
    };
  });
  const tagLocalToId = new Map<string, string>(
    template.tags.suggested.map((st) => [st.localId, sceneId(st.localId)])
  );

  // 3. Build a minimal Scene to thread through applyRenderPatchToShapeScene.
  //    Start with an empty scene; add tags so tag-lookup can work if needed.
  let scene: Scene = {
    version: 1,
    sceneVersion: 0,
    groups: [],
    nodes: [],
    edges: [],
    tags: newTags,
    comments: [],
    artifacts: [],
    selection: { kind: "canvas" },
    updatedAt: now
  };

  const errors: string[] = [];

  // 4a. Apply frames (create-group) — parents before children.
  const frameOrder = orderFrames(template.recipe.frames);
  for (const frame of frameOrder) {
    const groupId = sceneId(frame.localId);
    const parentGroupId = frame.parentLocalId ? (idMap.get(frame.parentLocalId) ?? null) : null;
    const tagIds = (frame.tagLocalIds ?? []).map((lid) => tagLocalToId.get(lid) ?? lid);

    // Compute rough initial bounds from the shapes belonging to this frame.
    const members = template.recipe.shapes.filter((s) => s.frameLocalId === frame.localId);
    const bounds = boundsForShapes(members, anchor, shapeSize);

    const renderGroup: RenderGroup = {
      id: groupId,
      title: frame.title,
      summary: frame.summary ?? "",
      bounds,
      tagIds,
      zIndex: 0,
      styleKey: "default"
    };

    const result = applyRenderPatchToShapeScene(scene, { kind: "create-group", group: renderGroup }, now);
    if (result.errors.length > 0) {
      errors.push(...result.errors);
      continue;
    }
    // Patch parentGroupId in after creation (renderGroupToSceneGroup hardcodes null).
    scene = {
      ...result.scene,
      groups: result.scene.groups.map((g) =>
        g.id === groupId && parentGroupId ? { ...g, parentGroupId } : g
      )
    };
  }

  // 4b. Apply shapes (create-card).
  for (const shape of template.recipe.shapes) {
    const nodeId = sceneId(shape.localId);
    const groupId = sceneId(shape.frameLocalId);
    const size = shape.size ?? shapeSize;
    const pos: WorldPoint = { x: anchor.x + shape.position.x, y: anchor.y + shape.position.y };

    const meta: Record<string, unknown> = {
      templateKind,
      ...(shape.meta ?? {})
    };

    const card: RenderCard = {
      id: nodeId,
      groupId,
      title: shape.title ?? "Untitled",
      summary: shape.summary ?? "",
      detail: shape.detail ?? "",
      status: "draft",
      type: "task",
      bounds: { x: pos.x, y: pos.y, width: size.width, height: size.height },
      zIndex: 0,
      styleKey: shape.styleKey ?? "default",
      accessibilityLabel: shape.title ?? "Untitled"
    };

    const result = applyRenderPatchToShapeScene(scene, { kind: "create-card", card }, now);
    if (result.errors.length > 0) {
      errors.push(...result.errors);
      continue;
    }
    // Attach meta to the node after creation.
    scene = {
      ...result.scene,
      nodes: result.scene.nodes.map((n) => (n.id === nodeId ? { ...n, meta } : n))
    };
  }

  // 4c. Apply edges (create-edge).
  for (const recipeEdge of template.recipe.edges) {
    const edgeId = sceneId(recipeEdge.localId);
    const groupId = sceneId(recipeEdge.frameLocalId);
    const sourceId = sceneId(recipeEdge.sourceLocalId);
    const targetId = sceneId(recipeEdge.targetLocalId);

    const result = applyRenderPatchToShapeScene(
      scene,
      {
        kind: "create-edge",
        groupId,
        source: sourceId,
        target: targetId,
        edgeId,
        label: recipeEdge.label
      },
      now
    );
    if (result.errors.length > 0) {
      errors.push(...result.errors);
      continue;
    }
    // Attach meta to the edge after creation.
    if (recipeEdge.meta || recipeEdge.styleKey) {
      const meta: Record<string, unknown> = {
        templateKind,
        ...(recipeEdge.meta ?? {}),
        ...(recipeEdge.styleKey ? { styleKey: recipeEdge.styleKey } : {})
      };
      scene = {
        ...result.scene,
        edges: result.scene.edges.map((e) => (e.id === edgeId ? { ...e, meta } : e))
      };
    } else {
      scene = result.scene;
    }
  }

  return {
    group: scene.groups[0] ?? (null as unknown as SceneGroup),
    nodes: scene.nodes,
    edges: scene.edges,
    newTags,
    errors
  };
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/** Topological sort of frames so parents are created before children. */
function orderFrames(frames: RecipeFrame[]): RecipeFrame[] {
  const byLocalId = new Map(frames.map((f) => [f.localId, f]));
  const visited = new Set<string>();
  const result: RecipeFrame[] = [];

  function visit(f: RecipeFrame) {
    if (visited.has(f.localId)) return;
    if (f.parentLocalId && byLocalId.has(f.parentLocalId)) {
      visit(byLocalId.get(f.parentLocalId)!);
    }
    visited.add(f.localId);
    result.push(f);
  }

  for (const f of frames) visit(f);
  return result;
}

/** Derive padded bounds from recipe shapes placed at anchor. */
function boundsForShapes(
  shapes: RecipeShape[],
  anchor: WorldPoint,
  defaultSize: { width: number; height: number }
): { x: number; y: number; width: number; height: number } {
  if (shapes.length === 0) {
    return { x: anchor.x, y: anchor.y, width: defaultSize.width + 80, height: defaultSize.height + 80 };
  }
  const padding = 40;
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const s of shapes) {
    const size = s.size ?? defaultSize;
    const x = anchor.x + s.position.x;
    const y = anchor.y + s.position.y;
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (x + size.width > maxX) maxX = x + size.width;
    if (y + size.height > maxY) maxY = y + size.height;
  }
  return {
    x: minX - padding,
    y: minY - padding,
    width: maxX - minX + padding * 2,
    height: maxY - minY + padding * 2
  };
}
