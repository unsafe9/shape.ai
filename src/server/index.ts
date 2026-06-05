import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { join } from "node:path";
import { stat } from "node:fs/promises";
import fastifyStatic from "@fastify/static";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import Fastify, { type FastifyInstance, type FastifyReply } from "fastify";
import { listClients, registerClient, setClientDockState, removeClient } from "./mcpClients";
import {
  createCommentRequestSchema,
  createGroupRequestSchema,
  createTagRequestSchema,
  exportRequestSchema,
  scenePatchSchema,
  updateCommentRequestSchema,
  updateGroupTagsRequestSchema,
  updateTagRequestSchema,
  type ExportType
} from "../shared/schema";
import { sceneGraphForGroup } from "../shared/graph";
import { shapeSceneToRenderSnapshot } from "../shared/renderScene";
import { generateLocalExport } from "./local";
import {
  addArtifact,
  addComment,
  artifactReadStream,
  createGroup,
  createTag,
  DATA_ROOT,
  deleteUnusedTag,
  ensureStorage,
  isExportPath,
  readFullScene,
  readGroup,
  readScene,
  saveScenePatch,
  updateComment,
  updateGroupTags,
  updateTag,
  writeArtifactContent
} from "./storage";
import { createSceneMcpServer } from "./mcp";

const port = Number(process.env.SHAPE_AI_PORT ?? 8787);
const host = process.env.SHAPE_AI_HOST ?? "127.0.0.1";

export async function buildServer() {
  const app = Fastify({
    logger: {
      level: process.env.LOG_LEVEL ?? "info"
    }
  });

  app.setErrorHandler((error, _request, reply) => {
    app.log.error(error);
    const message = error instanceof Error ? error.message : "Unknown error";
    reply.status(500).send({
      error: "internal_error",
      message
    });
  });

  app.get("/api/health", async () => ({
    ok: true,
    dataRoot: DATA_ROOT,
    mcp: {
      command: "npm run mcp",
      transport: "streamable_http",
      stdioTransport: "stdio",
      remoteTransport: "streamable_http",
      url: `http://${browserHost(host)}:${port}/mcp`
    }
  }));

  // Stateful MCP transport: session IDs give each connected client a stable identity.
  // Each session gets its own McpServer instance; the registry (mcpClients.ts) maps
  // sessionId → McpClientIdentity so the dock can track who is doing what.
  const mcpTransports = new Map<string, StreamableHTTPServerTransport>();

  app.all("/mcp", async (request, reply) => {
    const server = createSceneMcpServer();
    const sessionId = randomUUID();
    const transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: () => sessionId
    });

    // Wire identity: after initialize we know the client's Implementation.
    server.server.oninitialized = () => {
      const impl = server.server.getClientVersion();
      if (impl) {
        registerClient(impl, "http", sessionId);
      }
    };

    reply.hijack();
    reply.raw.on("close", () => {
      mcpTransports.delete(sessionId);
      setClientDockState(sessionId, "disconnected");
      // Grace: after a brief delay remove the entry so reconnects within the window can reattach.
      setTimeout(() => removeClient(sessionId), 5_000);
      void transport.close();
      void server.close();
    });

    mcpTransports.set(sessionId, transport);

    try {
      await server.connect(transport);
      await transport.handleRequest(request.raw, reply.raw, request.body);
    } catch (error) {
      app.log.error(error);
      if (!reply.raw.headersSent) {
        reply.raw.writeHead(500, { "content-type": "application/json" });
      }
      if (!reply.raw.writableEnded) {
        reply.raw.end(
          JSON.stringify({
            jsonrpc: "2.0",
            error: {
              code: -32603,
              message: error instanceof Error ? error.message : "Internal server error"
            },
            id: null
          })
        );
      }
      mcpTransports.delete(sessionId);
      await transport.close();
      await server.close();
    }
  });

  // Read path: shell/dock reads the live companion identity list.
  app.get("/api/mcp/clients", async () => ({ clients: listClients() }));

  registerSceneRoutes(app);
  await registerClientIfBuilt(app);
  return app;
}

function registerSceneRoutes(app: FastifyInstance) {
  app.get("/api/scene", async (request) => {
    const query = request.query as Record<string, string | undefined>;
    return { scene: await readScene(parseSceneQuery(query)) };
  });

  app.get("/api/scene/render-snapshot", async (request) => {
    const query = request.query as Record<string, string | undefined>;
    const scene = await readScene(parseSceneQuery(query));
    return { snapshot: shapeSceneToRenderSnapshot(scene) };
  });

  app.patch("/api/scene", async (request) => {
    const patch = scenePatchSchema.parse(request.body ?? {});
    return { scene: await saveScenePatch(patch) };
  });

  app.post("/api/groups", async (request, reply) => {
    const body = createGroupRequestSchema.parse(request.body ?? {});
    const result = await createGroup(body);
    reply.status(201).send(result);
  });

  app.get<{ Params: { id: string } }>("/api/groups/:id", async (request, reply) => {
    const group = await readGroup(request.params.id);
    if (!group) {
      reply.status(404).send({ error: "not_found", message: "Group not found" });
      return;
    }
    return group;
  });

  app.patch<{ Params: { id: string } }>("/api/groups/:id/tags", async (request) => {
    const body = updateGroupTagsRequestSchema.parse(request.body ?? {});
    return updateGroupTags(request.params.id, body.tagIds);
  });

  app.post("/api/tags", async (request, reply) => {
    const body = createTagRequestSchema.parse(request.body ?? {});
    const result = await createTag(body);
    reply.status(201).send(result);
  });

  app.patch<{ Params: { id: string } }>("/api/tags/:id", async (request) => {
    const body = updateTagRequestSchema.parse(request.body ?? {});
    return updateTag(request.params.id, body);
  });

  app.delete<{ Params: { id: string } }>("/api/tags/:id", async (request) => ({
    scene: await deleteUnusedTag(request.params.id)
  }));

  app.post("/api/comments", async (request) => {
    const body = createCommentRequestSchema.parse(request.body ?? {});
    return addComment(body);
  });

  app.patch<{ Params: { commentId: string } }>("/api/comments/:commentId", async (request) => {
    const body = updateCommentRequestSchema.parse(request.body ?? {});
    return updateComment(request.params.commentId, body);
  });

  app.post<{ Params: { id: string } }>("/api/groups/:id/export", async (request, reply) => {
    const groupDetail = await readGroup(request.params.id);
    if (!groupDetail) {
      reply.status(404).send({ error: "not_found", message: "Group not found" });
      return;
    }
    const body = exportRequestSchema.parse(request.body ?? {});
    const scene = await readFullScene();
    const scope = body.scope ?? { kind: "group" as const, id: request.params.id };
    const graph = sceneGraphForGroup(scene, request.params.id);
    const generated = generateLocalExport(graph, { type: body.type, scope }, groupDetail.group.title);
    const persisted = await writeArtifactContent({
      groupId: request.params.id,
      type: body.type,
      title: generated.title,
      content: generated.content,
      contentType: contentTypeFor(body.type)
    });
    const result = await addArtifact(request.params.id, {
      type: body.type,
      title: generated.title,
      target: { kind: "group", id: request.params.id },
      path: persisted.path,
      contentType: persisted.contentType
    });
    return {
      scene: result.scene,
      group: groupDetail.group,
      artifact: result.artifact,
      preview: {
        type: body.type,
        title: generated.title,
        content: generated.content,
        contentType: persisted.contentType,
        imagePrompt: generated.imagePrompt
      }
    };
  });

  app.get<{ Params: { groupId: string; artifactId: string } }>("/api/groups/:groupId/artifacts/:artifactId", async (request, reply) => {
    const scene = await readFullScene();
    const artifact = scene.artifacts.find((candidate) => candidate.id === request.params.artifactId);
    if (!artifact || artifact.target.kind !== "group" || artifact.target.id !== request.params.groupId) {
      reply.status(404).send({ error: "not_found", message: "Artifact not found" });
      return;
    }
    if (!isExportPath(artifact.path)) {
      reply.status(403).send({ error: "forbidden", message: "Artifact path is outside export directory" });
      return;
    }
    reply.header("Content-Disposition", `attachment; filename="${artifact.title.replace(/[^a-z0-9.-]+/gi, "-")}"`);
    reply.type(artifact.contentType);
    return reply.send(artifactReadStream(artifact.path));
  });
}

function parseSceneQuery(query: Record<string, string | undefined>) {
  return {
    tagIds: query.tags?.split(",").map((tag) => tag.trim()).filter(Boolean)
  };
}

function contentTypeFor(type: ExportType): string {
  if (type === "yadr") return "application/yaml; charset=utf-8";
  if (type === "image_prompt" || type === "architecture_image") return "text/markdown; charset=utf-8";
  if (type === "confluence_html") return "text/html; charset=utf-8";
  if (type === "mermaid") return "text/plain; charset=utf-8";
  return "text/markdown; charset=utf-8";
}

async function registerClientIfBuilt(app: FastifyInstance): Promise<void> {
  const clientDist = join(process.cwd(), "dist/client");
  try {
    const info = await stat(clientDist);
    if (!info.isDirectory()) return;
  } catch {
    return;
  }

  await app.register(fastifyStatic, {
    root: clientDist,
    prefix: "/"
  });

  app.setNotFoundHandler((_request, reply) => {
    reply.sendFile("index.html");
  });
}

function browserHost(value: string): string {
  return value === "0.0.0.0" ? "127.0.0.1" : value;
}

function openBrowser(url: string) {
  if (process.env.SHAPE_AI_OPEN_BROWSER === "0") return;
  if (process.platform === "darwin") {
    spawn("open", [url], { stdio: "ignore", detached: true }).unref();
    return;
  }
  if (process.platform === "win32") {
    spawn("cmd", ["/c", "start", "", url], { stdio: "ignore", detached: true }).unref();
    return;
  }
  spawn("xdg-open", [url], { stdio: "ignore", detached: true }).unref();
}

if (import.meta.url === `file://${process.argv[1]}`) {
  await ensureStorage();
  const app = await buildServer();
  await app.listen({ host, port });
  openBrowser(`http://${browserHost(host)}:${port}/`);
}
