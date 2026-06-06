import { createReadStream } from "node:fs";
import { mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import initSqlJs from "sql.js";
import { boundsForNodes, defaultSeedNodePositions, seedGroupScene } from "./local";
import {
  artifactSchema,
  boundsSchema,
  createCommentRequestSchema,
  createGroupRequestSchema,
  createTagRequestSchema,
  graphEdgeSchema,
  graphNodeSchema,
  sceneCommentSchema,
  sceneEdgeSchema,
  sceneGroupSchema,
  sceneNodeSchema,
  scenePatchSchema,
  sceneSchema,
  sceneSelectionSchema,
  tagSchema,
  updateCommentRequestSchema,
  updateGroupTagsRequestSchema,
  updateTagRequestSchema,
  type Bounds,
  type CreateGroupRequest,
  type CreateTagRequest,
  type ExportType,
  type Scene,
  type SceneArtifact,
  type SceneComment,
  type SceneEdge,
  type SceneGroup,
  type SceneNode,
  type ScenePatch,
  type SceneSelection,
  type Tag,
  type UpdateTagRequest
} from "../shared/schema";
import { boundsIntersect, expandedBounds, nodeBounds } from "../shared/graph";
import type { OperationEnvelope } from "../shared/operation";

type SqlDatabase = initSqlJs.Database;
type SqlValue = initSqlJs.SqlValue;
type SqlRow = Record<string, SqlValue | undefined>;

export type SceneQuery = {
  tagIds?: string[];
};

export type GroupDetail = {
  group: SceneGroup;
  nodes: SceneNode[];
  edges: SceneEdge[];
  tags: Tag[];
  comments: SceneComment[];
  artifacts: SceneArtifact[];
};

const __filename = fileURLToPath(import.meta.url);
const appRoot = resolve(dirname(__filename), "../..");

export const REPO_ROOT = resolve(process.env.SHAPE_AI_REPO_ROOT ?? appRoot);
export const DATA_ROOT = resolve(process.env.SHAPE_AI_DATA_DIR ?? join(appRoot, ".local"));
export const EXPORTS_DIR = join(DATA_ROOT, "exports");
export const DATABASE_PATH = join(DATA_ROOT, "shape.sqlite");

let databasePromise: Promise<SqlDatabase> | null = null;
let writeQueue: Promise<unknown> = Promise.resolve();

export async function ensureStorage(): Promise<void> {
  await getDb();
}

export async function readScene(query: SceneQuery = {}): Promise<Scene> {
  return withDb((db) => {
    const all = readFullSceneInDb(db);
    const tagIds = query.tagIds?.filter(Boolean) ?? [];
    const groups = all.groups.filter((group) => tagIds.length === 0 || tagIds.every((tagId) => group.tagIds.includes(tagId)));
    const groupIds = new Set(groups.map((group) => group.id));
    const nodes = all.nodes.filter((node) => groupIds.has(node.groupId));
    const nodeIds = new Set(nodes.map((node) => node.id));
    const edges = all.edges.filter((edge) => groupIds.has(edge.groupId) && nodeIds.has(edge.source) && nodeIds.has(edge.target));

    return sceneSchema.parse({
      ...all,
      groups,
      nodes,
      edges
    });
  });
}

export async function readFullScene(): Promise<Scene> {
  return withDb(readFullSceneInDb);
}

export async function listGroups(): Promise<SceneGroup[]> {
  return withDb((db) => readGroupsInDb(db));
}

export async function readGroup(id: string): Promise<GroupDetail | null> {
  return withDb((db) => {
    const scene = readFullSceneInDb(db);
    const group = scene.groups.find((candidate) => candidate.id === id);
    if (!group) return null;
    const nodeIds = new Set(scene.nodes.filter((node) => node.groupId === id).map((node) => node.id));
    return {
      group,
      nodes: scene.nodes.filter((node) => node.groupId === id),
      edges: scene.edges.filter((edge) => edge.groupId === id && nodeIds.has(edge.source) && nodeIds.has(edge.target)),
      tags: scene.tags.filter((tag) => group.tagIds.includes(tag.id)),
      comments: scene.comments.filter((comment) => commentTargetsGroup(comment.target, id, nodeIds)),
      artifacts: scene.artifacts.filter((artifact) => artifact.target.kind === "group" && artifact.target.id === id)
    };
  });
}

export async function createGroup(input: CreateGroupRequest): Promise<{ group: SceneGroup; scene: Scene; message: string }> {
  const body = createGroupRequestSchema.parse(input);
  return withWritableDb((db) => {
    const now = new Date().toISOString();
    const groupId = `group-${randomUUID().slice(0, 8)}`;
    assertTagsExist(db, body.tagIds ?? []);
    const seed = seedGroupScene(body.prompt, {
      groupId,
      now,
      parentGroupId: body.parentGroupId ?? null,
      tagIds: body.tagIds ?? []
    });
    const offset = nextGroupOffset(db, seed.group.bounds);
    const nodes = seed.nodes.map((node) =>
      sceneNodeSchema.parse({
        ...node,
        position: { x: node.position.x + offset.x, y: node.position.y + offset.y }
      })
    );
    const group = sceneGroupSchema.parse({
      ...seed.group,
      title: body.title || seed.title,
      bounds: boundsForNodes(nodes)
    });
    upsertGroupInDb(db, group);
    for (const node of nodes) upsertNodeInDb(db, node);
    for (const edge of seed.edges) upsertEdgeInDb(db, edge);
    setGroupTagsInDb(db, group.id, group.tagIds);
    bumpSceneVersion(db);
    return { group, scene: readFullSceneInDb(db), message: seed.explanation };
  });
}

/** Optional actor context that MCP callers supply to tag their writes. */
export type SaveScenePatchMeta = {
  actorType: "human" | "mcp" | "system";
  actorId: string;
  clientId: string;
  sourceToolCall?: { tool: string; callId?: string };
};

export async function saveScenePatch(input: ScenePatch, meta?: SaveScenePatchMeta): Promise<Scene> {
  const patch = scenePatchSchema.parse(input);
  return withWritableDb((db) => {
    // Detect selection-only patches: no document mutation, just an ephemeral selection write.
    const isSelectionOnly =
      !patch.groups?.length &&
      !patch.nodes?.length &&
      !patch.edges?.length &&
      !patch.translateGroups?.length &&
      !patch.removeGroupIds?.length &&
      !patch.removeNodeIds?.length &&
      !patch.removeEdgeIds?.length &&
      Boolean(patch.selection);

    for (const groupId of patch.removeGroupIds ?? []) removeGroupInDb(db, groupId);
    const removedNodeGroupIds = new Set<string>();
    for (const nodeId of patch.removeNodeIds ?? []) {
      const row = queryOne(db, "SELECT group_id FROM nodes WHERE id = ?", [nodeId]);
      if (row) removedNodeGroupIds.add(stringValue(row, "group_id"));
      removeNodeInDb(db, nodeId);
    }
    for (const edgeId of patch.removeEdgeIds ?? []) db.run("DELETE FROM edges WHERE id = ?", [edgeId]);
    for (const movement of patch.translateGroups ?? []) {
      const group = getGroupInDb(db, movement.groupId);
      if (group) moveGroupBy(db, group, movement.dx, movement.dy);
    }
    for (const group of patch.groups ?? []) upsertGroupInDb(db, group);
    for (const node of patch.nodes ?? []) upsertNodeInDb(db, { ...node, updatedAt: new Date().toISOString() });
    for (const edge of patch.edges ?? []) upsertEdgeInDb(db, { ...edge, updatedAt: new Date().toISOString() });
    if (patch.selection) setMetadata(db, "selection_json", toJson(patch.selection));
    recomputeTouchedGroupBounds(db, patch, removedNodeGroupIds);

    if (!isSelectionOnly) {
      // Document write: capture baseRevision before bumping, then bump.
      const baseRevision = getSceneVersion(db);
      bumpSceneVersion(db);
      const now = new Date().toISOString();
      const targetIds = scenePatchtargetIds(patch);
      const actor = meta ?? { actorType: "human" as const, actorId: "human", clientId: "local-shell" };
      const envelope: OperationEnvelope = {
        operationId: `op-${now}-${Math.random().toString(36).slice(2, 9)}`,
        actorId: actor.actorId,
        actorType: actor.actorType,
        clientId: actor.clientId,
        targetIds,
        timestamp: now,
        baseRevision,
        sourceToolCall: actor.sourceToolCall,
        patch: { kind: "select", selection: patch.selection ?? { kind: "canvas" } }
      };
      appendEventInDb(db, envelope, patch);
    }

    return readFullSceneInDb(db);
  });
}

export async function createTag(input: CreateTagRequest): Promise<{ tag: Tag; scene: Scene }> {
  const body = createTagRequestSchema.parse(input);
  return withWritableDb((db) => {
    const now = new Date().toISOString();
    const tag = tagSchema.parse({
      id: `tag-${slug(body.name)}-${randomUUID().slice(0, 6)}`,
      name: body.name.trim(),
      color: body.color,
      description: body.description ?? "",
      createdAt: now,
      updatedAt: now
    });
    upsertTagInDb(db, tag);
    bumpSceneVersion(db);
    return { tag, scene: readFullSceneInDb(db) };
  });
}

export async function updateTag(id: string, input: UpdateTagRequest): Promise<{ tag: Tag; scene: Scene }> {
  const body = updateTagRequestSchema.parse(input);
  return withWritableDb((db) => {
    const current = getTagInDb(db, id);
    if (!current) throw new Error(`Tag not found: ${id}`);
    const tag = tagSchema.parse({
      ...current,
      name: body.name ?? current.name,
      color: body.color ?? current.color,
      description: body.description ?? current.description,
      updatedAt: new Date().toISOString()
    });
    upsertTagInDb(db, tag);
    bumpSceneVersion(db);
    return { tag, scene: readFullSceneInDb(db) };
  });
}

export async function deleteUnusedTag(id: string): Promise<Scene> {
  return withWritableDb((db) => {
    const uses = numberValue(queryOne(db, "SELECT COUNT(*) AS count FROM group_tags WHERE tag_id = ?", [id]), "count");
    if (uses > 0) throw new Error("Cannot delete a tag that is still attached to groups");
    db.run("DELETE FROM tags WHERE id = ?", [id]);
    bumpSceneVersion(db);
    return readFullSceneInDb(db);
  });
}

export async function updateGroupTags(groupId: string, tagIds: string[]): Promise<{ group: SceneGroup; scene: Scene }> {
  const body = updateGroupTagsRequestSchema.parse({ tagIds });
  return withWritableDb((db) => {
    const group = getGroupInDb(db, groupId);
    if (!group) throw new Error(`Group not found: ${groupId}`);
    assertTagsExist(db, body.tagIds);
    const updated = sceneGroupSchema.parse({
      ...group,
      tagIds: body.tagIds,
      updatedAt: new Date().toISOString()
    });
    upsertGroupInDb(db, updated);
    setGroupTagsInDb(db, updated.id, updated.tagIds);
    bumpSceneVersion(db);
    return { group: updated, scene: readFullSceneInDb(db) };
  });
}

export async function addComment(input: { target: SceneSelection; body: string; author?: string }): Promise<{ comment: SceneComment; scene: Scene }> {
  const body = createCommentRequestSchema.parse(input);
  return withWritableDb((db) => {
    assertTargetExists(db, body.target);
    const now = new Date().toISOString();
    const comment = sceneCommentSchema.parse({
      id: randomUUID(),
      target: body.target,
      body: body.body,
      author: body.author || "human",
      resolved: false,
      createdAt: now,
      updatedAt: now
    });
    db.run(
      "INSERT INTO comments (id, target_json, body, author, resolved, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
      [comment.id, toJson(comment.target), comment.body, comment.author, comment.resolved ? 1 : 0, comment.createdAt, comment.updatedAt]
    );
    bumpSceneVersion(db);
    return { comment, scene: readFullSceneInDb(db) };
  });
}

export async function updateComment(commentId: string, input: { body?: string; resolved?: boolean }): Promise<{ comment: SceneComment; scene: Scene }> {
  const body = updateCommentRequestSchema.parse(input);
  return withWritableDb((db) => {
    const current = getCommentInDb(db, commentId);
    if (!current) throw new Error(`Comment not found: ${commentId}`);
    const updated = sceneCommentSchema.parse({
      ...current,
      body: body.body ?? current.body,
      resolved: body.resolved ?? current.resolved,
      updatedAt: new Date().toISOString()
    });
    db.run("UPDATE comments SET target_json = ?, body = ?, author = ?, resolved = ?, updated_at = ? WHERE id = ?", [
      toJson(updated.target),
      updated.body,
      updated.author,
      updated.resolved ? 1 : 0,
      updated.updatedAt,
      updated.id
    ]);
    bumpSceneVersion(db);
    return { comment: updated, scene: readFullSceneInDb(db) };
  });
}

export async function addArtifact(
  groupId: string,
  artifact: Omit<SceneArtifact, "id" | "createdAt" | "sceneVersion">
): Promise<{ artifact: SceneArtifact; scene: Scene }> {
  return withWritableDb((db) => {
    if (!getGroupInDb(db, groupId)) throw new Error(`Group not found: ${groupId}`);
    const now = new Date().toISOString();
    const nextArtifact = artifactSchema.parse({
      ...artifact,
      id: randomUUID(),
      createdAt: now,
      sceneVersion: getSceneVersion(db)
    });
    db.run("INSERT INTO artifacts (id, target_json, artifact_json, path, created_at) VALUES (?, ?, ?, ?, ?)", [
      nextArtifact.id,
      toJson(nextArtifact.target),
      toJson(nextArtifact),
      nextArtifact.path,
      nextArtifact.createdAt
    ]);
    bumpSceneVersion(db);
    return { artifact: nextArtifact, scene: readFullSceneInDb(db) };
  });
}

export async function writeArtifactContent(input: {
  groupId: string;
  type: ExportType;
  title: string;
  content: string;
  contentType: string;
}): Promise<{ path: string; contentType: string }> {
  await mkdir(join(EXPORTS_DIR, input.groupId), { recursive: true });
  const filename = `${Date.now()}-${input.type}-${input.title.replace(/[^a-z0-9.-]+/gi, "-").slice(0, 80)}.${extensionFor(input.type)}`;
  const absolutePath = join(EXPORTS_DIR, input.groupId, filename);
  await writeFile(absolutePath, input.content, "utf8");
  return { path: absolutePath, contentType: input.contentType };
}

export function isExportPath(path: string): boolean {
  const normalized = resolve(path);
  return normalized === EXPORTS_DIR || normalized.startsWith(`${EXPORTS_DIR}/`);
}

export function artifactReadStream(path: string) {
  return createReadStream(path);
}

/**
 * T5.4: Read recent events for a specific clientId from the events table.
 *
 * Returns up to `limit` rows, most-recent first, matching the given clientId
 * via a json_extract on payload_json.$.clientId.
 */
export async function readClientEvents(
  clientId: string,
  limit = 50
): Promise<Array<{ id: string; type: string; payloadJson: string; createdAt: string }>> {
  return withDb((db) => {
    const rows = queryRows(
      db,
      `SELECT id, type, payload_json, created_at FROM events
       WHERE json_extract(payload_json, '$.clientId') = ?
       ORDER BY created_at DESC LIMIT ?`,
      [clientId, limit]
    );
    return rows.map((row) => ({
      id: stringValue(row, "id"),
      type: stringValue(row, "type"),
      payloadJson: stringValue(row, "payload_json"),
      createdAt: stringValue(row, "created_at")
    }));
  });
}

async function getDb(): Promise<SqlDatabase> {
  if (databasePromise) return databasePromise;
  databasePromise = (async () => {
    await ensureStorageDirs();
    const SQL = await initSqlJs();
    let db: SqlDatabase;
    try {
      const bytes = await readFile(DATABASE_PATH);
      db = new SQL.Database(bytes);
    } catch {
      db = new SQL.Database();
    }
    prepareLegacyTables(db);
    createSchema(db);
    migrateLegacyDesignsIfNeeded(db);
    initializeMetadata(db);
    const previous = currentDbForParse;
    currentDbForParse = db;
    try {
      if (repairSceneLayoutIfNeeded(db)) bumpSceneVersion(db);
    } finally {
      currentDbForParse = previous;
    }
    await persistDb(db);
    return db;
  })();
  return databasePromise;
}

async function ensureStorageDirs(): Promise<void> {
  await mkdir(EXPORTS_DIR, { recursive: true });
}

function createSchema(db: SqlDatabase): void {
  db.run(`
    CREATE TABLE IF NOT EXISTS metadata (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS groups (
      id TEXT PRIMARY KEY,
      parent_group_id TEXT,
      title TEXT NOT NULL,
      summary TEXT NOT NULL,
      bounds_json TEXT NOT NULL,
      z_index REAL NOT NULL,
      collapsed INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS nodes (
      id TEXT PRIMARY KEY,
      group_id TEXT NOT NULL,
      node_json TEXT NOT NULL,
      x REAL NOT NULL,
      y REAL NOT NULL,
      width REAL NOT NULL,
      height REAL NOT NULL,
      z_index REAL NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS edges (
      id TEXT PRIMARY KEY,
      group_id TEXT NOT NULL,
      edge_json TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS tags (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      color TEXT NOT NULL,
      description TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS group_tags (
      group_id TEXT NOT NULL,
      tag_id TEXT NOT NULL,
      PRIMARY KEY (group_id, tag_id)
    );
    CREATE TABLE IF NOT EXISTS comments (
      id TEXT PRIMARY KEY,
      target_json TEXT NOT NULL,
      body TEXT NOT NULL,
      author TEXT NOT NULL,
      resolved INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS artifacts (
      id TEXT PRIMARY KEY,
      target_json TEXT NOT NULL,
      artifact_json TEXT NOT NULL,
      path TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS events (
      id TEXT PRIMARY KEY,
      type TEXT NOT NULL,
      payload_json TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_groups_parent ON groups(parent_group_id);
    CREATE INDEX IF NOT EXISTS idx_nodes_group ON nodes(group_id);
    CREATE INDEX IF NOT EXISTS idx_nodes_bounds ON nodes(x, y, width, height);
    CREATE INDEX IF NOT EXISTS idx_edges_group ON edges(group_id);
    CREATE INDEX IF NOT EXISTS idx_group_tags_tag ON group_tags(tag_id);
  `);
}

function prepareLegacyTables(db: SqlDatabase): void {
  if (tableExists(db, "nodes") && !columnExists(db, "nodes", "group_id")) renameTableIfPossible(db, "nodes", "legacy_nodes");
  if (tableExists(db, "edges") && !columnExists(db, "edges", "group_id")) renameTableIfPossible(db, "edges", "legacy_edges");
  if (tableExists(db, "comments") && columnExists(db, "comments", "design_id")) renameTableIfPossible(db, "comments", "legacy_comments");
  if (tableExists(db, "artifacts") && !columnExists(db, "artifacts", "target_json")) renameTableIfPossible(db, "artifacts", "legacy_artifacts");
  if (tableExists(db, "layouts") && !columnExists(db, "layouts", "updated_at")) renameTableIfPossible(db, "layouts", "legacy_layouts");
}

function renameTableIfPossible(db: SqlDatabase, from: string, to: string): void {
  if (!tableExists(db, from) || tableExists(db, to)) return;
  db.run(`ALTER TABLE ${from} RENAME TO ${to}`);
}

function initializeMetadata(db: SqlDatabase): void {
  if (!getMetadata(db, "scene_version")) setMetadata(db, "scene_version", "0");
  if (!getMetadata(db, "selection_json")) setMetadata(db, "selection_json", toJson({ kind: "canvas" }));
}

function migrateLegacyDesignsIfNeeded(db: SqlDatabase): void {
  if (!tableExists(db, "designs")) return;
  const groupCount = numberValue(queryOne(db, "SELECT COUNT(*) AS count FROM groups"), "count");
  if (groupCount > 0) return;
  const rows = queryRows(db, "SELECT snapshot_json FROM designs ORDER BY updated_at ASC");
  for (const row of rows) {
    migrateLegacyDesign(db, parseJson(stringValue(row, "snapshot_json")));
  }
}

function migrateLegacyDesign(db: SqlDatabase, legacy: unknown): void {
  if (!legacy || typeof legacy !== "object") return;
  const value = legacy as Record<string, unknown>;
  const now = stringOr(value.updatedAt, new Date().toISOString());
  const groupId = stringOr(value.id, `group-${randomUUID().slice(0, 8)}`);
  const graph = value.graph as Record<string, unknown> | undefined;
  const layout = value.layout as Record<string, unknown> | undefined;
  const nodePositions = (layout?.nodePositions as Record<string, { x?: number; y?: number }> | undefined) ?? {};
  const nodeZOrder = (layout?.nodeZOrder as Record<string, number> | undefined) ?? {};
  const idMap = new Map<string, string>();
  const nodes = ((graph?.nodes as unknown[]) ?? []).map((candidate, index) => {
    const parsed = graphNodeSchema.parse(candidate);
    const nextId = `${groupId}-${parsed.id}`;
    idMap.set(parsed.id, nextId);
    const position = nodePositions[parsed.id] ?? { x: 80 + index * 160, y: 120 + index * 120 };
    return sceneNodeSchema.parse({
      ...parsed,
      id: nextId,
      groupId,
      position: { x: Number(position.x ?? 0), y: Number(position.y ?? 0) },
      size: layoutNodeSize,
      zIndex: nodeZOrder[parsed.id] ?? index,
      updatedAt: now
    });
  });
  const edges = ((graph?.edges as unknown[]) ?? []).map((candidate) => {
    const parsed = graphEdgeSchema.parse(candidate);
    return sceneEdgeSchema.parse({
      ...parsed,
      id: `${groupId}-${parsed.id}`,
      source: idMap.get(parsed.source) ?? parsed.source,
      target: idMap.get(parsed.target) ?? parsed.target,
      groupId,
      updatedAt: now
    });
  });
  const group = sceneGroupSchema.parse({
    id: groupId,
    parentGroupId: null,
    title: stringOr(value.title, "Migrated group"),
    summary: stringOr(value.prompt, ""),
    bounds: boundsForNodes(nodes),
    tagIds: [],
    zIndex: numberValueOr(value.zIndex, 0),
    collapsed: false,
    createdAt: stringOr(value.createdAt, now),
    updatedAt: now
  });
  upsertGroupInDb(db, group);
  for (const node of nodes) upsertNodeInDb(db, node);
  for (const edge of edges) upsertEdgeInDb(db, edge);

  for (const commentCandidate of (value.comments as unknown[]) ?? []) {
    const comment = commentCandidate as Record<string, unknown>;
    const target = migrateLegacyTarget(comment.target, groupId, idMap);
    const parsed = sceneCommentSchema.parse({
      id: stringOr(comment.id, randomUUID()),
      target,
      body: stringOr(comment.body, ""),
      author: stringOr(comment.author, "human"),
      resolved: Boolean(comment.resolved),
      createdAt: stringOr(comment.createdAt, now),
      updatedAt: stringOr(comment.updatedAt, now)
    });
    db.run(
      "INSERT OR REPLACE INTO comments (id, target_json, body, author, resolved, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
      [parsed.id, toJson(parsed.target), parsed.body, parsed.author, parsed.resolved ? 1 : 0, parsed.createdAt, parsed.updatedAt]
    );
  }

  for (const artifactCandidate of (value.artifacts as unknown[]) ?? []) {
    const artifact = artifactCandidate as Record<string, unknown>;
    const parsed = artifactSchema.parse({
      id: stringOr(artifact.id, randomUUID()),
      type: artifact.type,
      title: stringOr(artifact.title, "Artifact"),
      target: { kind: "group", id: groupId },
      path: stringOr(artifact.path, ""),
      contentType: stringOr(artifact.contentType, "text/plain; charset=utf-8"),
      createdAt: stringOr(artifact.createdAt, now),
      sceneVersion: numberValueOr(artifact.graphVersion, 0)
    });
    db.run("INSERT OR REPLACE INTO artifacts (id, target_json, artifact_json, path, created_at) VALUES (?, ?, ?, ?, ?)", [
      parsed.id,
      toJson(parsed.target),
      toJson(parsed),
      parsed.path,
      parsed.createdAt
    ]);
  }
}

function readFullSceneInDb(db: SqlDatabase): Scene {
  return sceneSchema.parse({
    version: 1,
    sceneVersion: getSceneVersion(db),
    groups: readGroupsInDb(db),
    nodes: queryRows(db, "SELECT * FROM nodes ORDER BY z_index ASC").map(parseNodeRow),
    edges: queryRows(db, "SELECT * FROM edges ORDER BY id ASC").map(parseEdgeRow),
    tags: readTagsInDb(db),
    comments: queryRows(db, "SELECT * FROM comments ORDER BY updated_at DESC").map(parseCommentRow),
    artifacts: queryRows(db, "SELECT artifact_json FROM artifacts ORDER BY created_at DESC").map((row) =>
      artifactSchema.parse(parseJson(stringValue(row, "artifact_json")))
    ),
    selection: sceneSelectionSchema.parse(parseJson(getMetadata(db, "selection_json") ?? toJson({ kind: "canvas" }))),
    updatedAt: new Date().toISOString()
  });
}

function readGroupsInDb(db: SqlDatabase): SceneGroup[] {
  return queryRows(db, "SELECT * FROM groups ORDER BY z_index ASC, updated_at DESC").map(parseGroupRow);
}

function readTagsInDb(db: SqlDatabase): Tag[] {
  return queryRows(db, "SELECT * FROM tags ORDER BY updated_at DESC, name ASC").map(parseTagRow);
}

function upsertGroupInDb(db: SqlDatabase, groupInput: SceneGroup): void {
  const group = sceneGroupSchema.parse(groupInput);
  db.run(
    `
      INSERT INTO groups (id, parent_group_id, title, summary, bounds_json, z_index, collapsed, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        parent_group_id = excluded.parent_group_id,
        title = excluded.title,
        summary = excluded.summary,
        bounds_json = excluded.bounds_json,
        z_index = excluded.z_index,
        collapsed = excluded.collapsed,
        updated_at = excluded.updated_at
    `,
    [
      group.id,
      group.parentGroupId,
      group.title,
      group.summary,
      toJson(group.bounds),
      group.zIndex,
      group.collapsed ? 1 : 0,
      group.createdAt,
      group.updatedAt
    ]
  );
  setGroupTagsInDb(db, group.id, group.tagIds);
}

function upsertNodeInDb(db: SqlDatabase, nodeInput: SceneNode): void {
  const node = sceneNodeSchema.parse(nodeInput);
  db.run(
    `
      INSERT INTO nodes (id, group_id, node_json, x, y, width, height, z_index, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        group_id = excluded.group_id,
        node_json = excluded.node_json,
        x = excluded.x,
        y = excluded.y,
        width = excluded.width,
        height = excluded.height,
        z_index = excluded.z_index,
        updated_at = excluded.updated_at
    `,
    [
      node.id,
      node.groupId,
      toJson(node),
      node.position.x,
      node.position.y,
      node.size.width,
      node.size.height,
      node.zIndex,
      node.updatedAt ?? new Date().toISOString()
    ]
  );
}

function upsertEdgeInDb(db: SqlDatabase, edgeInput: SceneEdge): void {
  const edge = sceneEdgeSchema.parse(edgeInput);
  db.run(
    `
      INSERT INTO edges (id, group_id, edge_json, updated_at)
      VALUES (?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        group_id = excluded.group_id,
        edge_json = excluded.edge_json,
        updated_at = excluded.updated_at
    `,
    [edge.id, edge.groupId, toJson(edge), edge.updatedAt ?? new Date().toISOString()]
  );
}

function upsertTagInDb(db: SqlDatabase, tagInput: Tag): void {
  const tag = tagSchema.parse(tagInput);
  db.run(
    `
      INSERT INTO tags (id, name, color, description, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        name = excluded.name,
        color = excluded.color,
        description = excluded.description,
        updated_at = excluded.updated_at
    `,
    [tag.id, tag.name, tag.color, tag.description, tag.createdAt, tag.updatedAt]
  );
}

function setGroupTagsInDb(db: SqlDatabase, groupId: string, tagIds: string[]): void {
  db.run("DELETE FROM group_tags WHERE group_id = ?", [groupId]);
  for (const tagId of tagIds) {
    db.run("INSERT OR IGNORE INTO group_tags (group_id, tag_id) VALUES (?, ?)", [groupId, tagId]);
  }
}

function removeGroupInDb(db: SqlDatabase, groupId: string): void {
  const childRows = queryRows(db, "SELECT id FROM groups WHERE parent_group_id = ?", [groupId]);
  for (const row of childRows) removeGroupInDb(db, stringValue(row, "id"));
  db.run("DELETE FROM edges WHERE group_id = ?", [groupId]);
  db.run("DELETE FROM nodes WHERE group_id = ?", [groupId]);
  db.run("DELETE FROM group_tags WHERE group_id = ?", [groupId]);
  db.run("DELETE FROM groups WHERE id = ?", [groupId]);
}

function removeNodeInDb(db: SqlDatabase, nodeId: string): void {
  for (const edge of queryRows(db, "SELECT id, edge_json FROM edges").map(parseEdgeRow)) {
    if (edge.source === nodeId || edge.target === nodeId) {
      db.run("DELETE FROM edges WHERE id = ?", [edge.id]);
    }
  }
  db.run("DELETE FROM nodes WHERE id = ?", [nodeId]);
}

function recomputeTouchedGroupBounds(db: SqlDatabase, patch: ScenePatch, removedNodeGroupIds: Set<string>): void {
  const groupIds = new Set<string>(removedNodeGroupIds);
  for (const group of patch.groups ?? []) groupIds.add(group.id);
  for (const node of patch.nodes ?? []) groupIds.add(node.groupId);
  for (const edge of patch.edges ?? []) groupIds.add(edge.groupId);
  for (const groupId of groupIds) {
    const group = getGroupInDb(db, groupId);
    if (!group) continue;
    const nodes = queryRows(db, "SELECT * FROM nodes WHERE group_id = ?", [groupId]).map(parseNodeRow);
    if (nodes.length === 0) continue;
    upsertGroupInDb(db, { ...group, bounds: boundsForNodes(nodes), updatedAt: new Date().toISOString() });
  }
}

function parseGroupRow(row: SqlRow): SceneGroup {
  const groupId = stringValue(row, "id");
  const tagRows = queryRowsFromCurrentDb("SELECT tag_id FROM group_tags WHERE group_id = ?", [groupId]);
  return sceneGroupSchema.parse({
    id: groupId,
    parentGroupId: row.parent_group_id === null || row.parent_group_id === undefined ? null : String(row.parent_group_id),
    title: stringValue(row, "title"),
    summary: stringValue(row, "summary"),
    bounds: boundsSchema.parse(parseJson(stringValue(row, "bounds_json"))),
    tagIds: tagRows.map((tagRow) => stringValue(tagRow, "tag_id")),
    zIndex: numberValue(row, "z_index"),
    collapsed: numberValue(row, "collapsed") === 1,
    createdAt: stringValue(row, "created_at"),
    updatedAt: stringValue(row, "updated_at")
  });
}

let currentDbForParse: SqlDatabase | null = null;

function queryRowsFromCurrentDb(sql: string, params: SqlValue[] = []): SqlRow[] {
  if (!currentDbForParse) return [];
  return queryRows(currentDbForParse, sql, params);
}

function parseNodeRow(row: SqlRow): SceneNode {
  return sceneNodeSchema.parse(parseJson(stringValue(row, "node_json")));
}

function parseEdgeRow(row: SqlRow): SceneEdge {
  return sceneEdgeSchema.parse(parseJson(stringValue(row, "edge_json")));
}

function parseTagRow(row: SqlRow): Tag {
  return tagSchema.parse({
    id: stringValue(row, "id"),
    name: stringValue(row, "name"),
    color: stringValue(row, "color"),
    description: stringValue(row, "description"),
    createdAt: stringValue(row, "created_at"),
    updatedAt: stringValue(row, "updated_at")
  });
}

function parseCommentRow(row: SqlRow): SceneComment {
  return sceneCommentSchema.parse({
    id: stringValue(row, "id"),
    target: parseJson(stringValue(row, "target_json")),
    body: stringValue(row, "body"),
    author: stringValue(row, "author"),
    resolved: numberValue(row, "resolved") === 1,
    createdAt: stringValue(row, "created_at"),
    updatedAt: stringValue(row, "updated_at")
  });
}

function getGroupInDb(db: SqlDatabase, id: string): SceneGroup | null {
  const previous = currentDbForParse;
  currentDbForParse = db;
  try {
    const row = queryOne(db, "SELECT * FROM groups WHERE id = ?", [id]);
    return row ? parseGroupRow(row) : null;
  } finally {
    currentDbForParse = previous;
  }
}

function getTagInDb(db: SqlDatabase, id: string): Tag | null {
  const row = queryOne(db, "SELECT * FROM tags WHERE id = ?", [id]);
  return row ? parseTagRow(row) : null;
}

function getCommentInDb(db: SqlDatabase, id: string): SceneComment | null {
  const row = queryOne(db, "SELECT * FROM comments WHERE id = ?", [id]);
  return row ? parseCommentRow(row) : null;
}

function assertTagsExist(db: SqlDatabase, tagIds: string[]): void {
  for (const tagId of tagIds) {
    if (!getTagInDb(db, tagId)) throw new Error(`Tag not found: ${tagId}`);
  }
}

function assertTargetExists(db: SqlDatabase, target: SceneSelection): void {
  if (target.kind === "canvas") return;
  if (target.kind === "multi") {
    for (const id of target.ids) {
      if (!queryOne(db, "SELECT id FROM nodes WHERE id = ?", [id])) throw new Error(`Target not found: ${id}`);
    }
    return;
  }
  if (target.kind === "group" && getGroupInDb(db, target.id)) return;
  if (target.kind === "node" && queryOne(db, "SELECT id FROM nodes WHERE id = ?", [target.id])) return;
  if (target.kind === "edge" && queryOne(db, "SELECT id FROM edges WHERE id = ?", [target.id])) return;
  throw new Error(`Target not found: ${target.id}`);
}

function commentTargetsGroup(target: SceneSelection, groupId: string, nodeIds: Set<string>): boolean {
  if (target.kind === "group") return target.id === groupId;
  if (target.kind === "node") return nodeIds.has(target.id);
  return false;
}

async function withDb<T>(fn: (db: SqlDatabase) => T): Promise<T> {
  const db = await getDb();
  const previous = currentDbForParse;
  currentDbForParse = db;
  try {
    return fn(db);
  } finally {
    currentDbForParse = previous;
  }
}

async function withWritableDb<T>(fn: (db: SqlDatabase) => T): Promise<T> {
  const run = async () => {
    const db = await getDb();
    const previous = currentDbForParse;
    currentDbForParse = db;
    try {
      const result = fn(db);
      await persistDb(db);
      return result;
    } catch (error) {
      databasePromise = null;
      throw error;
    } finally {
      currentDbForParse = previous;
    }
  };
  const next = writeQueue.then(run, run);
  writeQueue = next.then(
    () => undefined,
    () => undefined
  );
  return next;
}

async function persistDb(db: SqlDatabase): Promise<void> {
  await mkdir(dirname(DATABASE_PATH), { recursive: true });
  const tempPath = `${DATABASE_PATH}.${process.pid}.${Date.now()}.tmp`;
  try {
    await writeFile(tempPath, Buffer.from(db.export()));
    await rename(tempPath, DATABASE_PATH);
  } catch (error) {
    await rm(tempPath, { force: true }).catch(() => undefined);
    throw error;
  }
}

function getSceneVersion(db: SqlDatabase): number {
  return Number(getMetadata(db, "scene_version") ?? "0");
}

function bumpSceneVersion(db: SqlDatabase): void {
  setMetadata(db, "scene_version", String(getSceneVersion(db) + 1));
}

function getMetadata(db: SqlDatabase, key: string): string | undefined {
  const row = queryOne(db, "SELECT value FROM metadata WHERE key = ?", [key]);
  return row ? stringValue(row, "value") : undefined;
}

function setMetadata(db: SqlDatabase, key: string, value: string): void {
  db.run(
    "INSERT INTO metadata (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    [key, value]
  );
}

/**
 * Append one operation envelope row to the events table.
 * Called inside the same withWritableDb block as the document write so the
 * log row is atomic with the patch application and sceneVersion bump.
 *
 * id          = envelope.operationId
 * type        = patch.kind (the verb)
 * payload_json= full envelope JSON (metadata + targetIds + inner patch)
 * created_at  = envelope.timestamp
 */
function appendEventInDb(db: SqlDatabase, envelope: OperationEnvelope, patch: ScenePatch): void {
  // Derive the event type from the patch fields — first document-mutation verb wins.
  const type = scenePatchtargetVerb(patch);
  db.run(
    "INSERT INTO events (id, type, payload_json, created_at) VALUES (?, ?, ?, ?)",
    [envelope.operationId, type, toJson(envelope), envelope.timestamp]
  );
}

/**
 * Derive a human-readable verb for the events.type column from a ScenePatch.
 * Mirrors the RenderScenePatch kind taxonomy so the events table stays consistent.
 */
function scenePatchtargetVerb(patch: ScenePatch): string {
  if (patch.removeGroupIds?.length) return "delete-group";
  if (patch.removeNodeIds?.length) return "delete-card";
  if (patch.removeEdgeIds?.length) return "delete-edge";
  if (patch.translateGroups?.length) return "move-group";
  if (patch.groups?.length) return "upsert-group";
  if (patch.nodes?.length) return "upsert-card";
  if (patch.edges?.length) return "upsert-edge";
  return "patch-scene";
}

/**
 * Derive the set of targetIds that a ScenePatch touches.
 * Used as the fallback when no envelope was supplied by the caller.
 */
function scenePatchtargetIds(patch: ScenePatch): string[] {
  const ids: string[] = [];
  for (const gid of patch.removeGroupIds ?? []) ids.push(gid);
  for (const nid of patch.removeNodeIds ?? []) ids.push(nid);
  for (const eid of patch.removeEdgeIds ?? []) ids.push(eid);
  for (const t of patch.translateGroups ?? []) ids.push(t.groupId);
  for (const g of patch.groups ?? []) ids.push(g.id);
  for (const n of patch.nodes ?? []) ids.push(n.id);
  for (const e of patch.edges ?? []) ids.push(e.id);
  return ids;
}

const layoutRepairVersion = "11";
const groupGap = 140;
const collisionPadding = 60;
const layoutNodeSize = { width: 270, height: 178 };
const legacySeedNodePositions: Record<string, { x: number; y: number }> = {
  "n-proposition": { x: 0, y: 0 },
  "n-decision-points": { x: 460, y: 120 },
  "n-option-graph": { x: 920, y: 0 },
  "n-option-freeform": { x: 920, y: 460 },
  "n-evidence": { x: 1380, y: 120 },
  "n-tradeoff": { x: 1380, y: 580 },
  "n-blocker": { x: 1380, y: 1040 },
  "n-subdecision": { x: 1840, y: 0 },
  "n-task": { x: 2300, y: 120 },
  "n-artifact": { x: 2300, y: 580 }
};
const wideSeedNodePositions: Record<string, { x: number; y: number }> = {
  "n-proposition": { x: 0, y: 360 },
  "n-decision-points": { x: 520, y: 360 },
  "n-option-graph": { x: 1040, y: 0 },
  "n-option-freeform": { x: 1040, y: 480 },
  "n-evidence": { x: 1560, y: 0 },
  "n-tradeoff": { x: 1560, y: 480 },
  "n-blocker": { x: 1560, y: 960 },
  "n-subdecision": { x: 2080, y: 0 },
  "n-task": { x: 2080, y: 480 },
  "n-artifact": { x: 2080, y: 960 }
};
const previousSeedNodePositions: Record<string, { x: number; y: number }> = {
  "n-proposition": { x: 0, y: 510 },
  "n-decision-points": { x: 450, y: 510 },
  "n-option-graph": { x: 900, y: 80 },
  "n-option-freeform": { x: 900, y: 510 },
  "n-evidence": { x: 1350, y: 80 },
  "n-tradeoff": { x: 1350, y: 510 },
  "n-blocker": { x: 1350, y: 940 },
  "n-subdecision": { x: 1800, y: 80 },
  "n-task": { x: 1800, y: 510 },
  "n-artifact": { x: 1800, y: 940 }
};
const compactSeedNodePositions = normalizeSeedPositions(defaultSeedNodePositions);

function repairSceneLayoutIfNeeded(db: SqlDatabase): boolean {
  if (getMetadata(db, "scene_layout_version") === layoutRepairVersion) return false;
  let changed = repairLegacySeedNodeLayouts(db);
  if (repairTopLevelGroupPacking(db, true)) changed = true;
  setMetadata(db, "scene_layout_version", layoutRepairVersion);
  return changed;
}

function repairLegacySeedNodeLayouts(db: SqlDatabase): boolean {
  let changed = false;
  const groups = readGroupsInDb(db);
  for (const group of groups) {
    const nodes = queryRows(db, "SELECT * FROM nodes WHERE group_id = ?", [group.id]).map(parseNodeRow);
    if (!isLegacySeedLayout(group, nodes)) continue;
    const anchor = { x: group.bounds.x + 160, y: group.bounds.y + 160 };
    const updatedNodes: SceneNode[] = [];
    const extraNodes: SceneNode[] = [];
    for (const node of nodes) {
      const seedKey = seedKeyForNode(group.id, node.id);
      const position = seedKey ? compactSeedNodePositions[seedKey] : undefined;
      if (!position) {
        extraNodes.push(node);
        continue;
      }
      updatedNodes.push(sceneNodeSchema.parse({
        ...node,
        position: { x: anchor.x + position.x, y: anchor.y + position.y },
        size: layoutNodeSize,
        updatedAt: new Date().toISOString()
      }));
    }
    const seedMaxY = Math.max(...Object.values(compactSeedNodePositions).map((position) => position.y + layoutNodeSize.height));
    const extraColumns = 4;
    for (const [index, node] of extraNodes.sort((a, b) => a.zIndex - b.zIndex).entries()) {
      const preferred = {
        x: anchor.x + (index % extraColumns) * 360,
        y: anchor.y + seedMaxY + 120 + Math.floor(index / extraColumns) * 260
      };
      updatedNodes.push(
        sceneNodeSchema.parse({
          ...node,
          position: openNodePosition(updatedNodes, preferred, layoutNodeSize),
          size: layoutNodeSize,
          updatedAt: new Date().toISOString()
        })
      );
    }
    for (const node of updatedNodes) upsertNodeInDb(db, node);
    upsertGroupInDb(db, {
      ...group,
      bounds: boundsForNodes(updatedNodes),
      updatedAt: new Date().toISOString()
    });
    changed = true;
  }
  return changed;
}

function isLegacySeedLayout(group: SceneGroup, nodes: SceneNode[]): boolean {
  return (
    matchesSeedLayout(group, nodes, legacySeedNodePositions) ||
    matchesSeedLayout(group, nodes, wideSeedNodePositions) ||
    matchesSeedLayout(group, nodes, previousSeedNodePositions)
  );
}

function matchesSeedLayout(group: SceneGroup, nodes: SceneNode[], expectedPositions: Record<string, { x: number; y: number }>): boolean {
  const anchor = { x: group.bounds.x + 160, y: group.bounds.y + 160 };
  const seedKeys = Object.keys(defaultSeedNodePositions);
  for (const seedKey of seedKeys) {
    const node = nodes.find((candidate) => candidate.id === `${group.id}-${seedKey}`);
    if (!node) return false;
    const expected = expectedPositions[seedKey];
    if (!expected) return false;
    if (Math.abs(node.position.x - (anchor.x + expected.x)) > 6) return false;
    if (Math.abs(node.position.y - (anchor.y + expected.y)) > 6) return false;
  }
  return true;
}

function normalizeSeedPositions(positions: Record<string, { x: number; y: number }>): Record<string, { x: number; y: number }> {
  const minX = Math.min(...Object.values(positions).map((position) => position.x));
  const minY = Math.min(...Object.values(positions).map((position) => position.y));
  return Object.fromEntries(Object.entries(positions).map(([key, position]) => [key, { x: position.x - minX, y: position.y - minY }]));
}

function openNodePosition(placedNodes: SceneNode[], preferred: { x: number; y: number }, size: { width: number; height: number }): { x: number; y: number } {
  const occupied = placedNodes.map(nodeBounds);
  const candidateFree = (position: { x: number; y: number }) => {
    const candidate = expandLayoutBounds({ x: position.x, y: position.y, width: size.width, height: size.height }, 54);
    return occupied.every((bounds) => !boundsIntersect(candidate, bounds));
  };
  if (candidateFree(preferred)) return preferred;
  const stepX = 450;
  const stepY = 430;
  for (let radius = 1; radius <= 8; radius += 1) {
    for (let x = -radius; x <= radius; x += 1) {
      for (let y = -radius; y <= radius; y += 1) {
        if (Math.abs(x) !== radius && Math.abs(y) !== radius) continue;
        const candidate = { x: preferred.x + x * stepX, y: preferred.y + y * stepY };
        if (candidateFree(candidate)) return candidate;
      }
    }
  }
  return preferred;
}

function repairTopLevelGroupPacking(db: SqlDatabase, force: boolean): boolean {
  const groups = readGroupsInDb(db).filter((group) => group.parentGroupId === null);
  if (groups.length < 2 || (!force && !hasSignificantGroupOverlap(groups))) return false;
  packTopLevelGroups(db, groups);
  return true;
}

function hasSignificantGroupOverlap(groups: SceneGroup[]): boolean {
  let overlaps = 0;
  for (let index = 0; index < groups.length; index += 1) {
    for (let next = index + 1; next < groups.length; next += 1) {
      if (boundsOverlapArea(groups[index].bounds, groups[next].bounds) > Math.min(boundsArea(groups[index].bounds), boundsArea(groups[next].bounds)) * 0.18) {
        overlaps += 1;
      }
    }
  }
  return overlaps >= Math.max(2, Math.ceil(groups.length * 0.18));
}

function packTopLevelGroups(db: SqlDatabase, groups: SceneGroup[]): void {
  const ordered = [...groups].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  const columns = Math.max(3, Math.ceil(Math.sqrt(ordered.length)));
  let cursorY = 0;
  for (let rowStart = 0; rowStart < ordered.length; rowStart += columns) {
    let cursorX = 0;
    let rowHeight = 0;
    for (const group of ordered.slice(rowStart, rowStart + columns)) {
      moveGroupBy(db, group, cursorX - group.bounds.x, cursorY - group.bounds.y);
      cursorX += group.bounds.width + groupGap;
      rowHeight = Math.max(rowHeight, group.bounds.height);
    }
    cursorY += rowHeight + groupGap;
  }
}

function nextGroupOffset(db: SqlDatabase, desiredBounds?: Bounds): { x: number; y: number } {
  const desired = desiredBounds ?? { x: -120, y: -120, width: 1900, height: 1100 };
  const groups = readGroupsInDb(db).filter((group) => group.parentGroupId === null);
  const cellWidth = Math.max(1900, desired.width + groupGap);
  const cellHeight = Math.max(1200, desired.height + groupGap);
  const columns = Math.max(3, Math.ceil(Math.sqrt(groups.length + 1)));
  for (let index = 0; index < Math.max(256, (groups.length + 1) * 4); index += 1) {
    const bounds = {
      x: (index % columns) * cellWidth,
      y: Math.floor(index / columns) * cellHeight,
      width: desired.width,
      height: desired.height
    };
    if (groups.every((group) => boundsOverlapArea(expandLayoutBounds(bounds, collisionPadding), expandLayoutBounds(group.bounds, collisionPadding)) === 0)) {
      return { x: bounds.x - desired.x, y: bounds.y - desired.y };
    }
  }
  const fallbackIndex = groups.length;
  return {
    x: (fallbackIndex % columns) * cellWidth - desired.x,
    y: Math.floor(fallbackIndex / columns) * cellHeight - desired.y
  };
}

function moveGroupBy(db: SqlDatabase, group: SceneGroup, dx: number, dy: number): void {
  if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
  const now = new Date().toISOString();
  const nodes = queryRows(db, "SELECT * FROM nodes WHERE group_id = ?", [group.id]).map(parseNodeRow);
  for (const node of nodes) {
    upsertNodeInDb(db, {
      ...node,
      position: { x: node.position.x + dx, y: node.position.y + dy },
      updatedAt: now
    });
  }
  upsertGroupInDb(db, {
    ...group,
    bounds: { ...group.bounds, x: group.bounds.x + dx, y: group.bounds.y + dy },
    updatedAt: now
  });
}

function seedKeyForNode(groupId: string, nodeId: string): string | undefined {
  const prefix = `${groupId}-`;
  if (!nodeId.startsWith(prefix)) return undefined;
  const key = nodeId.slice(prefix.length);
  return key in defaultSeedNodePositions ? key : undefined;
}

function boundsArea(bounds: Bounds): number {
  return Math.max(0, bounds.width) * Math.max(0, bounds.height);
}

function boundsOverlapArea(a: Bounds, b: Bounds): number {
  const x = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x));
  const y = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));
  return x * y;
}

function expandLayoutBounds(bounds: Bounds, padding: number): Bounds {
  return {
    x: bounds.x - padding,
    y: bounds.y - padding,
    width: bounds.width + padding * 2,
    height: bounds.height + padding * 2
  };
}

function tableExists(db: SqlDatabase, name: string): boolean {
  return Boolean(queryOne(db, "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?", [name]));
}

function columnExists(db: SqlDatabase, table: string, column: string): boolean {
  return queryRows(db, `PRAGMA table_info(${table})`).some((row) => row.name === column);
}

function queryRows(db: SqlDatabase, sql: string, params: SqlValue[] = []): SqlRow[] {
  const statement = db.prepare(sql, params);
  const rows: SqlRow[] = [];
  try {
    while (statement.step()) rows.push(statement.getAsObject() as SqlRow);
  } finally {
    statement.free();
  }
  return rows;
}

function queryOne(db: SqlDatabase, sql: string, params: SqlValue[] = []): SqlRow | null {
  return queryRows(db, sql, params)[0] ?? null;
}

function toJson(value: unknown): string {
  return JSON.stringify(value);
}

function parseJson(value: string): unknown {
  return JSON.parse(value);
}

function stringValue(row: SqlRow | null, key: string): string {
  const value = row?.[key];
  if (typeof value !== "string") throw new Error(`Expected string column ${key}`);
  return value;
}

function numberValue(row: SqlRow | null, key: string): number {
  const value = row?.[key];
  if (typeof value !== "number") throw new Error(`Expected number column ${key}`);
  return value;
}

function numberValueOr(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function stringOr(value: unknown, fallback: string): string {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function slug(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "tag";
}

function extensionFor(type: ExportType): string {
  if (type === "yadr") return "yaml";
  if (type === "confluence_html") return "html";
  if (type === "mermaid") return "mmd";
  return "md";
}

function migrateLegacyTarget(target: unknown, groupId: string, idMap: Map<string, string>): SceneSelection {
  if (!target || typeof target !== "object") return { kind: "group", id: groupId };
  const value = target as Record<string, unknown>;
  const kind = value.kind;
  const id = typeof value.id === "string" ? value.id : undefined;
  if (kind === "node" && id) return { kind: "node", id: idMap.get(id) ?? id };
  if (kind === "edge" && id) return { kind: "edge", id: `${groupId}-${id}` };
  return { kind: "group", id: groupId };
}

export async function listLegacyJsonFiles(): Promise<string[]> {
  try {
    return (await readdir(DATA_ROOT)).filter((entry) => entry.endsWith(".json"));
  } catch {
    return [];
  }
}
