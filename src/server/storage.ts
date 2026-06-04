import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import {
  artifactSchema,
  graphCommentSchema,
  designSchema,
  type DecisionGraph,
  type Design,
  type DesignArtifact,
  type ExportType
} from "../shared/schema";

const __filename = fileURLToPath(import.meta.url);
const appRoot = resolve(dirname(__filename), "../..");
const repoRoot = resolve(appRoot, "../..");

export const REPO_ROOT = resolve(process.env.CHARRETTE_REPO_ROOT ?? repoRoot);
export const DATA_ROOT = resolve(process.env.CHARRETTE_DATA_DIR ?? join(appRoot, ".local"));
export const DESIGNS_DIR = join(DATA_ROOT, "designs");
export const EXPORTS_DIR = join(DATA_ROOT, "exports");

export async function ensureStorage(): Promise<void> {
  await mkdir(DESIGNS_DIR, { recursive: true });
  await mkdir(EXPORTS_DIR, { recursive: true });
}

export async function listDesigns(): Promise<Design[]> {
  await ensureStorage();
  const entries = await readdir(DESIGNS_DIR);
  const designs = await Promise.all(
    entries
      .filter((entry) => entry.endsWith(".json"))
      .map(async (entry) => readDesign(entry.replace(/\.json$/, "")))
  );
  return designs
    .filter((design): design is Design => Boolean(design))
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

export async function readDesign(id: string): Promise<Design | null> {
  await ensureStorage();
  try {
    const raw = await readFile(designPath(id), "utf8");
    return designSchema.parse(JSON.parse(raw));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      return null;
    }
    throw error;
  }
}

export async function saveDesign(design: Design): Promise<Design> {
  await ensureStorage();
  const parsed = designSchema.parse(design);
  await writeJson(designPath(parsed.id), parsed);
  return parsed;
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

function designPath(id: string): string {
  return join(DESIGNS_DIR, `${id}.json`);
}

async function writeJson(path: string, value: unknown): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function extensionFor(type: ExportType, contentType: string): string {
  if (type === "architecture_image" && contentType === "image/png") return ".png";
  if (type === "architecture_image") return ".svg";
  if (type === "confluence_html") return ".html";
  if (type === "mermaid") return ".mmd";
  if (contentType.includes("html")) return ".html";
  const known = extname(contentType);
  return known || ".md";
}

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 60) || "artifact";
}
