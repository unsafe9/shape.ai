import { mkdir, readFile, readdir, rename, rm, writeFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import initSqlJs from "sql.js";
import { applyGraphPatch, graphTextDigest } from "../shared/graph";
import {
  artifactSchema,
  designSchema,
  graphCommentSchema,
  graphPatchSchema,
  proposalCommentSchema,
  proposalPatchSchema,
  proposalSchema,
  type DecisionGraph,
  type Design,
  type DesignArtifact,
  type ExportType,
  type GraphPatch,
  type Proposal,
  type ProposalComment,
  type ProposalPatch,
  type ProposalValidationStatus
} from "../shared/schema";

type SqlDatabase = initSqlJs.Database;
type SqlValue = initSqlJs.SqlValue;
type SqlRow = Record<string, SqlValue | undefined>;

export type ProposalValidation = {
  status: ProposalValidationStatus;
  messages: string[];
  baseGraphVersion: number;
  currentGraphVersion: number;
  baseGraph?: DecisionGraph;
  previewGraph?: DecisionGraph;
};

export type ProposalDiff = {
  proposal: Proposal;
  patches: ProposalPatch[];
  comments: ProposalComment[];
  validation: ProposalValidation;
  beforeDigest: string;
  afterDigest?: string;
};

const __filename = fileURLToPath(import.meta.url);
const appRoot = resolve(dirname(__filename), "../..");

export const REPO_ROOT = resolve(process.env.SHAPE_AI_REPO_ROOT ?? appRoot);
export const DATA_ROOT = resolve(process.env.SHAPE_AI_DATA_DIR ?? join(appRoot, ".local"));
export const DESIGNS_DIR = join(DATA_ROOT, "designs");
export const EXPORTS_DIR = join(DATA_ROOT, "exports");
export const DATABASE_PATH = join(DATA_ROOT, "shape.sqlite");

let databasePromise: Promise<SqlDatabase> | null = null;
let writeQueue: Promise<unknown> = Promise.resolve();

export async function ensureStorage(): Promise<void> {
  await getDb();
}

export async function listDesigns(): Promise<Design[]> {
  return withDb((db) =>
    queryRows(db, "SELECT snapshot_json FROM designs ORDER BY updated_at DESC").map((row) =>
      designSchema.parse(parseJson(stringValue(row, "snapshot_json")))
    )
  );
}

export async function readDesign(id: string): Promise<Design | null> {
  return withDb((db) => getDesignInDb(db, id));
}

export async function saveDesign(design: Design): Promise<Design> {
  const parsed = designSchema.parse(design);
  return withWritableDb((db) => upsertDesign(db, parsed));
}

export async function createDesign(input: {
  title: string;
  prompt: string;
  graph: DecisionGraph;
}): Promise<Design> {
  const now = new Date().toISOString();
  return saveDesign({
    id: randomUUID(),
    title: input.title,
    prompt: input.prompt,
    createdAt: now,
    updatedAt: now,
    graphVersion: 0,
    graph: input.graph,
    layout: { nodePositions: {} },
    selection: { kind: "graph" },
    comments: [],
    artifacts: []
  });
}

export async function updateDesignGraph(design: Design, graph: DecisionGraph): Promise<Design> {
  return saveDesign({
    ...design,
    graph,
    graphVersion: design.graphVersion + 1,
    updatedAt: new Date().toISOString()
  });
}

export async function addArtifact(
  design: Design,
  artifact: Omit<DesignArtifact, "id" | "createdAt" | "graphVersion">
): Promise<Design> {
  const nextArtifact = artifactSchema.parse({
    ...artifact,
    id: randomUUID(),
    createdAt: new Date().toISOString(),
    graphVersion: design.graphVersion
  });
  return saveDesign({
    ...design,
    artifacts: [nextArtifact, ...design.artifacts],
    updatedAt: new Date().toISOString()
  });
}

export async function addComment(
  design: Design,
  input: { target: Design["selection"]; body: string; author?: string }
): Promise<Design> {
  const now = new Date().toISOString();
  const comment = graphCommentSchema.parse({
    id: randomUUID(),
    target: input.target,
    body: input.body,
    author: input.author || "human",
    resolved: false,
    createdAt: now,
    updatedAt: now
  });
  return saveDesign({
    ...design,
    comments: [comment, ...design.comments],
    updatedAt: now
  });
}

export async function updateComment(
  design: Design,
  commentId: string,
  input: { body?: string; resolved?: boolean }
): Promise<Design> {
  if (!design.comments.some((comment) => comment.id === commentId)) {
    throw new Error(`Comment not found: ${commentId}`);
  }
  const now = new Date().toISOString();
  return saveDesign({
    ...design,
    comments: design.comments.map((comment) =>
      comment.id === commentId
        ? {
            ...comment,
            body: input.body ?? comment.body,
            resolved: input.resolved ?? comment.resolved,
            updatedAt: now
          }
        : comment
    ),
    updatedAt: now
  });
}

export async function listOpenProposals(designId?: string): Promise<Proposal[]> {
  return withDb((db) => {
    const sql = designId
      ? "SELECT * FROM proposals WHERE design_id = ? AND status IN ('open', 'changes_requested') ORDER BY updated_at DESC"
      : "SELECT * FROM proposals WHERE status IN ('open', 'changes_requested') ORDER BY updated_at DESC";
    return queryRows(db, sql, designId ? [designId] : []).map(parseProposalRow);
  });
}

export async function createProposal(input: {
  designId: string;
  title: string;
  description?: string;
  baseGraphVersion?: number;
  createdBy?: string;
}): Promise<Proposal> {
  return withWritableDb((db) => {
    const design = getDesignInDb(db, input.designId);
    if (!design) {
      throw new Error(`Design not found: ${input.designId}`);
    }
    const baseGraphVersion = input.baseGraphVersion ?? design.graphVersion;
    if (!getGraphVersionInDb(db, design.id, baseGraphVersion)) {
      throw new Error(`Graph version not found: ${design.id}@${baseGraphVersion}`);
    }
    const now = new Date().toISOString();
    const proposal = proposalSchema.parse({
      id: randomUUID(),
      designId: design.id,
      title: input.title,
      description: input.description ?? "",
      baseGraphVersion,
      status: "open",
      validationStatus: baseGraphVersion === design.graphVersion ? "clean" : "needs_rebase",
      createdBy: input.createdBy ?? "agent",
      createdAt: now,
      updatedAt: now
    });
    insertProposalInDb(db, proposal);
    insertEvent(db, {
      designId: design.id,
      proposalId: proposal.id,
      type: "proposal_created",
      payload: { title: proposal.title, baseGraphVersion }
    });
    return proposal;
  });
}

export async function appendProposalPatch(input: {
  proposalId: string;
  patch: GraphPatch;
}): Promise<{ proposal: Proposal; patch: ProposalPatch; validation: ProposalValidation }> {
  const patch = graphPatchSchema.parse(input.patch);
  return withWritableDb((db) => {
    const proposal = getProposalOrThrowInDb(db, input.proposalId);
    assertProposalWritable(proposal);

    const sequence = nextPatchSequence(db, proposal.id);
    const now = new Date().toISOString();
    const proposalPatch = proposalPatchSchema.parse({
      id: randomUUID(),
      proposalId: proposal.id,
      sequence,
      patch,
      validationStatus: "clean",
      createdAt: now
    });
    insertProposalPatchInDb(db, proposalPatch);

    const validation = validateProposalInDb(db, proposal.id);
    updateProposalValidationInDb(db, proposal.id, validation.status);
    db.run("UPDATE proposal_patches SET validation_status = ? WHERE proposal_id = ?", [validation.status, proposal.id]);
    insertEvent(db, {
      designId: proposal.designId,
      proposalId: proposal.id,
      type: "proposal_patch_appended",
      payload: { patchId: proposalPatch.id, sequence, validationStatus: validation.status }
    });

    return {
      proposal: getProposalOrThrowInDb(db, proposal.id),
      patch: { ...proposalPatch, validationStatus: validation.status },
      validation
    };
  });
}

export async function validateProposal(
  proposalId: string
): Promise<{ proposal: Proposal; validation: ProposalValidation }> {
  return withWritableDb((db) => {
    const validation = validateProposalInDb(db, proposalId);
    updateProposalValidationInDb(db, proposalId, validation.status);
    db.run("UPDATE proposal_patches SET validation_status = ? WHERE proposal_id = ?", [validation.status, proposalId]);
    return {
      proposal: getProposalOrThrowInDb(db, proposalId),
      validation
    };
  });
}

export async function getProposalDiff(proposalId: string): Promise<ProposalDiff> {
  await validateProposal(proposalId);
  return withDb((db) => {
    const proposal = getProposalOrThrowInDb(db, proposalId);
    const patches = getProposalPatchesInDb(db, proposalId);
    const comments = getProposalCommentsInDb(db, proposalId);
    const validation = validateProposalInDb(db, proposalId);
    const baseGraph = validation.baseGraph ?? { version: 1, nodes: [], edges: [] };
    return {
      proposal,
      patches,
      comments,
      validation,
      beforeDigest: graphTextDigest(baseGraph),
      afterDigest: validation.previewGraph ? graphTextDigest(validation.previewGraph) : undefined
    };
  });
}

export async function commentOnProposal(input: {
  proposalId: string;
  body: string;
  author?: string;
}): Promise<{ proposal: Proposal; comment: ProposalComment }> {
  return withWritableDb((db) => {
    const proposal = getProposalOrThrowInDb(db, input.proposalId);
    const comment = insertProposalCommentInDb(db, {
      proposalId: proposal.id,
      body: input.body,
      author: input.author ?? "human"
    });
    insertEvent(db, {
      designId: proposal.designId,
      proposalId: proposal.id,
      type: "proposal_comment_added",
      payload: { commentId: comment.id }
    });
    return { proposal: getProposalOrThrowInDb(db, proposal.id), comment };
  });
}

export async function requestProposalChanges(input: {
  proposalId: string;
  body?: string;
  author?: string;
}): Promise<{ proposal: Proposal; comment?: ProposalComment }> {
  return withWritableDb((db) => {
    const proposal = getProposalOrThrowInDb(db, input.proposalId);
    assertProposalWritable(proposal);
    const now = new Date().toISOString();
    let comment: ProposalComment | undefined;
    if (input.body) {
      comment = insertProposalCommentInDb(db, {
        proposalId: proposal.id,
        body: input.body,
        author: input.author ?? "human"
      });
    }
    db.run("UPDATE proposals SET status = 'changes_requested', updated_at = ? WHERE id = ?", [now, proposal.id]);
    insertEvent(db, {
      designId: proposal.designId,
      proposalId: proposal.id,
      type: "proposal_changes_requested",
      payload: { commentId: comment?.id }
    });
    return { proposal: getProposalOrThrowInDb(db, proposal.id), comment };
  });
}

export async function approveProposal(
  proposalId: string
): Promise<{ proposal: Proposal; design: Design; validation: ProposalValidation }> {
  return withWritableDb((db) => {
    const proposal = getProposalOrThrowInDb(db, proposalId);
    assertProposalWritable(proposal);

    const validation = validateProposalInDb(db, proposal.id);
    updateProposalValidationInDb(db, proposal.id, validation.status);
    if (validation.status !== "clean" || !validation.previewGraph) {
      throw new Error(`Proposal is not approvable: ${validation.status}`);
    }

    const design = getDesignOrThrowInDb(db, proposal.designId);
    const updated = designSchema.parse({
      ...design,
      graph: validation.previewGraph,
      graphVersion: design.graphVersion + 1,
      updatedAt: new Date().toISOString()
    });
    upsertDesign(db, updated, "proposal_approved");
    db.run("UPDATE proposals SET status = 'approved', validation_status = 'clean', updated_at = ? WHERE id = ?", [
      updated.updatedAt,
      proposal.id
    ]);
    insertEvent(db, {
      designId: design.id,
      proposalId: proposal.id,
      type: "proposal_approved",
      payload: { graphVersion: updated.graphVersion }
    });
    return {
      proposal: getProposalOrThrowInDb(db, proposal.id),
      design: updated,
      validation
    };
  });
}

export async function rejectProposal(proposalId: string): Promise<{ proposal: Proposal }> {
  return withWritableDb((db) => {
    const proposal = getProposalOrThrowInDb(db, proposalId);
    assertProposalWritable(proposal);
    const now = new Date().toISOString();
    db.run("UPDATE proposals SET status = 'rejected', updated_at = ? WHERE id = ?", [now, proposal.id]);
    insertEvent(db, {
      designId: proposal.designId,
      proposalId: proposal.id,
      type: "proposal_rejected",
      payload: {}
    });
    return { proposal: getProposalOrThrowInDb(db, proposal.id) };
  });
}

export async function writeArtifactContent(input: {
  designId: string;
  type: ExportType;
  title: string;
  content: string | Buffer;
  contentType: string;
}): Promise<{ path: string; contentType: string }> {
  const ext = extensionFor(input.type, input.contentType);
  const filename = `${Date.now()}-${slug(input.title)}${ext}`;
  const absolutePath = join(EXPORTS_DIR, input.designId, filename);
  await mkdir(dirname(absolutePath), { recursive: true });
  await writeFile(absolutePath, input.content);
  return { path: absolutePath, contentType: input.contentType };
}

export function isExportPath(path: string): boolean {
  const absolutePath = resolve(path);
  return absolutePath.startsWith(resolve(EXPORTS_DIR));
}

async function getDb(): Promise<SqlDatabase> {
  if (!databasePromise) {
    const pending = openDatabase();
    pending.catch(() => {
      if (databasePromise === pending) databasePromise = null;
    });
    databasePromise = pending;
  }
  return databasePromise;
}

async function openDatabase(): Promise<SqlDatabase> {
  await ensureStorageDirs();
  const SQL = await initSqlJs();
  let db: SqlDatabase;
  try {
    db = new SQL.Database(await readFile(DATABASE_PATH));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
      throw error;
    }
    db = new SQL.Database();
  }
  await createSchema(db);
  await migrateJsonDesignsIfNeeded(db);
  await persistDb(db);
  return db;
}

async function ensureStorageDirs(): Promise<void> {
  await mkdir(DESIGNS_DIR, { recursive: true });
  await mkdir(EXPORTS_DIR, { recursive: true });
}

async function createSchema(db: SqlDatabase): Promise<void> {
  db.run(`
    CREATE TABLE IF NOT EXISTS designs (
      id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      prompt TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      graph_version INTEGER NOT NULL,
      selection_json TEXT NOT NULL,
      snapshot_json TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS graph_versions (
      design_id TEXT NOT NULL,
      version INTEGER NOT NULL,
      graph_json TEXT NOT NULL,
      layout_json TEXT NOT NULL,
      created_at TEXT NOT NULL,
      source TEXT NOT NULL,
      PRIMARY KEY (design_id, version)
    );
    CREATE TABLE IF NOT EXISTS nodes (
      design_id TEXT NOT NULL,
      graph_version INTEGER NOT NULL,
      node_id TEXT NOT NULL,
      node_json TEXT NOT NULL,
      PRIMARY KEY (design_id, graph_version, node_id)
    );
    CREATE TABLE IF NOT EXISTS edges (
      design_id TEXT NOT NULL,
      graph_version INTEGER NOT NULL,
      edge_id TEXT NOT NULL,
      edge_json TEXT NOT NULL,
      PRIMARY KEY (design_id, graph_version, edge_id)
    );
    CREATE TABLE IF NOT EXISTS layouts (
      design_id TEXT NOT NULL,
      graph_version INTEGER NOT NULL,
      layout_json TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      PRIMARY KEY (design_id, graph_version)
    );
    CREATE TABLE IF NOT EXISTS comments (
      id TEXT PRIMARY KEY,
      design_id TEXT NOT NULL,
      target_json TEXT NOT NULL,
      body TEXT NOT NULL,
      author TEXT NOT NULL,
      resolved INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS artifacts (
      id TEXT PRIMARY KEY,
      design_id TEXT NOT NULL,
      artifact_json TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS proposals (
      id TEXT PRIMARY KEY,
      design_id TEXT NOT NULL,
      title TEXT NOT NULL,
      description TEXT NOT NULL,
      base_graph_version INTEGER NOT NULL,
      status TEXT NOT NULL,
      validation_status TEXT NOT NULL,
      created_by TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS proposal_patches (
      id TEXT PRIMARY KEY,
      proposal_id TEXT NOT NULL,
      sequence INTEGER NOT NULL,
      patch_json TEXT NOT NULL,
      validation_status TEXT NOT NULL,
      created_at TEXT NOT NULL,
      UNIQUE (proposal_id, sequence)
    );
    CREATE TABLE IF NOT EXISTS proposal_comments (
      id TEXT PRIMARY KEY,
      proposal_id TEXT NOT NULL,
      body TEXT NOT NULL,
      author TEXT NOT NULL,
      resolved INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS events (
      id TEXT PRIMARY KEY,
      design_id TEXT,
      proposal_id TEXT,
      type TEXT NOT NULL,
      payload_json TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
  `);
}

async function migrateJsonDesignsIfNeeded(db: SqlDatabase): Promise<void> {
  const existing = numberValue(queryOne(db, "SELECT COUNT(*) AS count FROM designs") ?? { count: 0 }, "count");
  if (existing > 0) return;

  const entries = await readdir(DESIGNS_DIR).catch((error) => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  });
  for (const entry of entries.filter((candidate) => candidate.endsWith(".json"))) {
    const raw = await readFile(join(DESIGNS_DIR, entry), "utf8");
    const design = designSchema.parse(JSON.parse(raw));
    upsertDesign(db, design, "legacy_json_import");
  }
}

async function withDb<T>(fn: (db: SqlDatabase) => T): Promise<T> {
  const db = await getDb();
  return fn(db);
}

async function withWritableDb<T>(fn: (db: SqlDatabase) => T): Promise<T> {
  const job = writeQueue.then(async () => {
    const db = await getDb();
    db.run("BEGIN");
    let result: T;
    try {
      result = fn(db);
    } catch (error) {
      db.run("ROLLBACK");
      throw error;
    }
    db.run("COMMIT");
    try {
      await persistDb(db);
    } catch (error) {
      resetDatabase(db);
      throw error;
    }
    return result;
  });
  writeQueue = job.catch(() => undefined);
  return job;
}

async function persistDb(db: SqlDatabase): Promise<void> {
  await ensureStorageDirs();
  const tempPath = join(DATA_ROOT, `.shape-${process.pid}-${Date.now()}.sqlite.tmp`);
  try {
    await writeFile(tempPath, Buffer.from(db.export()));
    await rename(tempPath, DATABASE_PATH);
  } catch (error) {
    await rm(tempPath, { force: true }).catch(() => undefined);
    throw error;
  }
}

function resetDatabase(db: SqlDatabase): void {
  try {
    db.close();
  } catch {
    // Ignore close failures while recovering from a failed persistence write.
  }
  databasePromise = null;
}

function upsertDesign(db: SqlDatabase, design: Design, source = "canonical_save"): Design {
  const parsed = designSchema.parse(design);
  db.run(
    `
      INSERT INTO designs (
        id, title, prompt, created_at, updated_at, graph_version, selection_json, snapshot_json
      )
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET
        title = excluded.title,
        prompt = excluded.prompt,
        updated_at = excluded.updated_at,
        graph_version = excluded.graph_version,
        selection_json = excluded.selection_json,
        snapshot_json = excluded.snapshot_json
    `,
    [
      parsed.id,
      parsed.title,
      parsed.prompt,
      parsed.createdAt,
      parsed.updatedAt,
      parsed.graphVersion,
      toJson(parsed.selection),
      toJson(parsed)
    ]
  );
  syncGraphVersion(db, parsed, source);
  syncComments(db, parsed);
  syncArtifacts(db, parsed);
  return parsed;
}

function syncGraphVersion(db: SqlDatabase, design: Design, source: string): void {
  db.run(
    `
      INSERT INTO graph_versions (design_id, version, graph_json, layout_json, created_at, source)
      VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT(design_id, version) DO UPDATE SET
        graph_json = excluded.graph_json,
        layout_json = excluded.layout_json,
        source = excluded.source
    `,
    [design.id, design.graphVersion, toJson(design.graph), toJson(design.layout), design.updatedAt, source]
  );

  db.run("DELETE FROM nodes WHERE design_id = ? AND graph_version = ?", [design.id, design.graphVersion]);
  for (const node of design.graph.nodes) {
    db.run("INSERT INTO nodes (design_id, graph_version, node_id, node_json) VALUES (?, ?, ?, ?)", [
      design.id,
      design.graphVersion,
      node.id,
      toJson(node)
    ]);
  }

  db.run("DELETE FROM edges WHERE design_id = ? AND graph_version = ?", [design.id, design.graphVersion]);
  for (const edge of design.graph.edges) {
    db.run("INSERT INTO edges (design_id, graph_version, edge_id, edge_json) VALUES (?, ?, ?, ?)", [
      design.id,
      design.graphVersion,
      edge.id,
      toJson(edge)
    ]);
  }

  db.run(
    `
      INSERT INTO layouts (design_id, graph_version, layout_json, updated_at)
      VALUES (?, ?, ?, ?)
      ON CONFLICT(design_id, graph_version) DO UPDATE SET
        layout_json = excluded.layout_json,
        updated_at = excluded.updated_at
    `,
    [design.id, design.graphVersion, toJson(design.layout), design.updatedAt]
  );
}

function syncComments(db: SqlDatabase, design: Design): void {
  db.run("DELETE FROM comments WHERE design_id = ?", [design.id]);
  for (const comment of design.comments) {
    db.run(
      `
        INSERT INTO comments (
          id, design_id, target_json, body, author, resolved, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      `,
      [
        comment.id,
        design.id,
        toJson(comment.target),
        comment.body,
        comment.author,
        comment.resolved ? 1 : 0,
        comment.createdAt,
        comment.updatedAt
      ]
    );
  }
}

function syncArtifacts(db: SqlDatabase, design: Design): void {
  db.run("DELETE FROM artifacts WHERE design_id = ?", [design.id]);
  for (const artifact of design.artifacts) {
    db.run("INSERT INTO artifacts (id, design_id, artifact_json, created_at) VALUES (?, ?, ?, ?)", [
      artifact.id,
      design.id,
      toJson(artifact),
      artifact.createdAt
    ]);
  }
}

function getDesignInDb(db: SqlDatabase, id: string): Design | null {
  const row = queryOne(db, "SELECT snapshot_json FROM designs WHERE id = ?", [id]);
  if (!row) return null;
  return designSchema.parse(parseJson(stringValue(row, "snapshot_json")));
}

function getDesignOrThrowInDb(db: SqlDatabase, id: string): Design {
  const design = getDesignInDb(db, id);
  if (!design) {
    throw new Error(`Design not found: ${id}`);
  }
  return design;
}

function getGraphVersionInDb(db: SqlDatabase, designId: string, graphVersion: number): DecisionGraph | null {
  const row = queryOne(db, "SELECT graph_json FROM graph_versions WHERE design_id = ? AND version = ?", [
    designId,
    graphVersion
  ]);
  if (!row) return null;
  return designSchema.shape.graph.parse(parseJson(stringValue(row, "graph_json")));
}

function insertProposalInDb(db: SqlDatabase, proposal: Proposal): void {
  db.run(
    `
      INSERT INTO proposals (
        id, design_id, title, description, base_graph_version, status, validation_status,
        created_by, created_at, updated_at
      )
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `,
    [
      proposal.id,
      proposal.designId,
      proposal.title,
      proposal.description,
      proposal.baseGraphVersion,
      proposal.status,
      proposal.validationStatus,
      proposal.createdBy,
      proposal.createdAt,
      proposal.updatedAt
    ]
  );
}

function getProposalOrThrowInDb(db: SqlDatabase, proposalId: string): Proposal {
  const row = queryOne(db, "SELECT * FROM proposals WHERE id = ?", [proposalId]);
  if (!row) {
    throw new Error(`Proposal not found: ${proposalId}`);
  }
  return parseProposalRow(row);
}

function insertProposalPatchInDb(db: SqlDatabase, patch: ProposalPatch): void {
  db.run(
    `
      INSERT INTO proposal_patches (id, proposal_id, sequence, patch_json, validation_status, created_at)
      VALUES (?, ?, ?, ?, ?, ?)
    `,
    [patch.id, patch.proposalId, patch.sequence, toJson(patch.patch), patch.validationStatus, patch.createdAt]
  );
}

function nextPatchSequence(db: SqlDatabase, proposalId: string): number {
  const row = queryOne(db, "SELECT COALESCE(MAX(sequence), 0) + 1 AS next FROM proposal_patches WHERE proposal_id = ?", [
    proposalId
  ]);
  return numberValue(row ?? { next: 1 }, "next");
}

function getProposalPatchesInDb(db: SqlDatabase, proposalId: string): ProposalPatch[] {
  return queryRows(db, "SELECT * FROM proposal_patches WHERE proposal_id = ? ORDER BY sequence ASC", [proposalId]).map(
    parseProposalPatchRow
  );
}

function insertProposalCommentInDb(
  db: SqlDatabase,
  input: { proposalId: string; body: string; author: string }
): ProposalComment {
  const now = new Date().toISOString();
  const comment = proposalCommentSchema.parse({
    id: randomUUID(),
    proposalId: input.proposalId,
    body: input.body,
    author: input.author,
    resolved: false,
    createdAt: now,
    updatedAt: now
  });
  db.run(
    `
      INSERT INTO proposal_comments (id, proposal_id, body, author, resolved, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `,
    [comment.id, comment.proposalId, comment.body, comment.author, 0, comment.createdAt, comment.updatedAt]
  );
  return comment;
}

function getProposalCommentsInDb(db: SqlDatabase, proposalId: string): ProposalComment[] {
  return queryRows(db, "SELECT * FROM proposal_comments WHERE proposal_id = ? ORDER BY created_at ASC", [proposalId]).map(
    parseProposalCommentRow
  );
}

function validateProposalInDb(db: SqlDatabase, proposalId: string): ProposalValidation {
  const proposal = getProposalOrThrowInDb(db, proposalId);
  const design = getDesignInDb(db, proposal.designId);
  if (!design) {
    return {
      status: "invalid",
      messages: [`Design not found: ${proposal.designId}`],
      baseGraphVersion: proposal.baseGraphVersion,
      currentGraphVersion: proposal.baseGraphVersion
    };
  }

  const baseGraph = getGraphVersionInDb(db, proposal.designId, proposal.baseGraphVersion);
  if (!baseGraph) {
    return {
      status: "invalid",
      messages: [`Base graph version not found: ${proposal.designId}@${proposal.baseGraphVersion}`],
      baseGraphVersion: proposal.baseGraphVersion,
      currentGraphVersion: design.graphVersion
    };
  }

  let previewGraph = baseGraph;
  for (const patch of getProposalPatchesInDb(db, proposalId)) {
    const errors = validatePatchReferences(previewGraph, patch.patch);
    if (errors.length > 0) {
      return {
        status: "conflict",
        messages: errors,
        baseGraphVersion: proposal.baseGraphVersion,
        currentGraphVersion: design.graphVersion,
        baseGraph
      };
    }
    previewGraph = applyGraphPatch(previewGraph, patch.patch);
  }

  if (proposal.baseGraphVersion !== design.graphVersion) {
    return {
      status: "needs_rebase",
      messages: [`Base graph version ${proposal.baseGraphVersion} is behind current version ${design.graphVersion}.`],
      baseGraphVersion: proposal.baseGraphVersion,
      currentGraphVersion: design.graphVersion,
      baseGraph,
      previewGraph
    };
  }

  return {
    status: "clean",
    messages: [],
    baseGraphVersion: proposal.baseGraphVersion,
    currentGraphVersion: design.graphVersion,
    baseGraph,
    previewGraph
  };
}

function validatePatchReferences(graph: DecisionGraph, patch: GraphPatch): string[] {
  const nodeIds = new Set(graph.nodes.map((node) => node.id));
  const edgeIds = new Set(graph.edges.map((edge) => edge.id));
  const errors: string[] = [];

  for (const node of patch.addNodes) {
    if (nodeIds.has(node.id)) errors.push(`Node already exists: ${node.id}`);
  }
  for (const node of patch.updateNodes) {
    if (!nodeIds.has(node.id)) errors.push(`Node update target not found: ${node.id}`);
  }
  for (const nodeId of patch.removeNodeIds) {
    if (!nodeIds.has(nodeId)) errors.push(`Node remove target not found: ${nodeId}`);
  }
  for (const edge of patch.addEdges) {
    if (edgeIds.has(edge.id)) errors.push(`Edge already exists: ${edge.id}`);
  }
  for (const edge of patch.updateEdges) {
    if (!edgeIds.has(edge.id)) errors.push(`Edge update target not found: ${edge.id}`);
  }
  for (const edgeId of patch.removeEdgeIds) {
    if (!edgeIds.has(edgeId)) errors.push(`Edge remove target not found: ${edgeId}`);
  }

  const nextNodeIds = new Set(nodeIds);
  for (const nodeId of patch.removeNodeIds) nextNodeIds.delete(nodeId);
  for (const node of patch.addNodes) nextNodeIds.add(node.id);

  for (const edge of [...patch.addEdges, ...patch.updateEdges]) {
    if (!nextNodeIds.has(edge.source)) errors.push(`Edge source node not found: ${edge.id} -> ${edge.source}`);
    if (!nextNodeIds.has(edge.target)) errors.push(`Edge target node not found: ${edge.id} -> ${edge.target}`);
  }

  return [...new Set(errors)];
}

function updateProposalValidationInDb(
  db: SqlDatabase,
  proposalId: string,
  validationStatus: ProposalValidationStatus
): void {
  db.run("UPDATE proposals SET validation_status = ?, updated_at = ? WHERE id = ?", [
    validationStatus,
    new Date().toISOString(),
    proposalId
  ]);
}

function assertProposalWritable(proposal: Proposal): void {
  if (proposal.status === "approved" || proposal.status === "rejected") {
    throw new Error(`Proposal is already ${proposal.status}: ${proposal.id}`);
  }
}

function insertEvent(
  db: SqlDatabase,
  input: { designId?: string; proposalId?: string; type: string; payload: unknown }
): void {
  db.run(
    "INSERT INTO events (id, design_id, proposal_id, type, payload_json, created_at) VALUES (?, ?, ?, ?, ?, ?)",
    [
      randomUUID(),
      input.designId ?? null,
      input.proposalId ?? null,
      input.type,
      toJson(input.payload),
      new Date().toISOString()
    ]
  );
}

function parseProposalRow(row: SqlRow): Proposal {
  return proposalSchema.parse({
    id: stringValue(row, "id"),
    designId: stringValue(row, "design_id"),
    title: stringValue(row, "title"),
    description: stringValue(row, "description"),
    baseGraphVersion: numberValue(row, "base_graph_version"),
    status: stringValue(row, "status"),
    validationStatus: stringValue(row, "validation_status"),
    createdBy: stringValue(row, "created_by"),
    createdAt: stringValue(row, "created_at"),
    updatedAt: stringValue(row, "updated_at")
  });
}

function parseProposalPatchRow(row: SqlRow): ProposalPatch {
  return proposalPatchSchema.parse({
    id: stringValue(row, "id"),
    proposalId: stringValue(row, "proposal_id"),
    sequence: numberValue(row, "sequence"),
    patch: parseJson(stringValue(row, "patch_json")),
    validationStatus: stringValue(row, "validation_status"),
    createdAt: stringValue(row, "created_at")
  });
}

function parseProposalCommentRow(row: SqlRow): ProposalComment {
  return proposalCommentSchema.parse({
    id: stringValue(row, "id"),
    proposalId: stringValue(row, "proposal_id"),
    body: stringValue(row, "body"),
    author: stringValue(row, "author"),
    resolved: numberValue(row, "resolved") === 1,
    createdAt: stringValue(row, "created_at"),
    updatedAt: stringValue(row, "updated_at")
  });
}

function queryRows(db: SqlDatabase, sql: string, params: SqlValue[] = []): SqlRow[] {
  const result = db.exec(sql, params);
  const first = result[0];
  if (!first) return [];
  return first.values.map((values) =>
    Object.fromEntries(first.columns.map((column, index) => [column, values[index]]))
  );
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

function stringValue(row: SqlRow, key: string): string {
  const value = row[key];
  if (typeof value !== "string") {
    throw new Error(`Expected string column: ${key}`);
  }
  return value;
}

function numberValue(row: SqlRow, key: string): number {
  const value = row[key];
  if (typeof value !== "number") {
    throw new Error(`Expected number column: ${key}`);
  }
  return value;
}

function extensionFor(type: ExportType, contentType: string): string {
  if (type === "yadr") return ".yaml";
  if (type === "image_prompt" || type === "architecture_image") return ".md";
  if (type === "confluence_html") return ".html";
  if (type === "mermaid") return ".mmd";
  if (contentType.includes("html")) return ".html";
  const known = extname(contentType);
  return known || ".md";
}

function slug(value: string): string {
  return (
    value
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "")
      .slice(0, 60) || "artifact"
  );
}
