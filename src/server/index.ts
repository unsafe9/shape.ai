import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { spawn } from "node:child_process";
import { join } from "node:path";
import fastifyStatic from "@fastify/static";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import Fastify, { type FastifyInstance, type FastifyReply } from "fastify";
import {
  createCommentRequestSchema,
  createDesignRequestSchema,
  decisionGraphSchema,
  exportRequestSchema,
  graphEditRequestSchema,
  updateCommentRequestSchema
} from "../shared/schema";
import { generateLocalExport, seedDesignGraph } from "./local";
import {
  addArtifact,
  addComment,
  createDesign,
  DATA_ROOT,
  ensureStorage,
  isExportPath,
  listDesigns,
  readDesign,
  saveDesign,
  updateComment,
  writeArtifactContent
} from "./storage";
import { createShapeMcpServer } from "./mcp";

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

  app.all("/mcp", async (request, reply) => {
    const server = createShapeMcpServer();
    const transport = new StreamableHTTPServerTransport({
      sessionIdGenerator: undefined
    });
    reply.hijack();
    reply.raw.on("close", () => {
      void transport.close();
      void server.close();
    });

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
      await transport.close();
      await server.close();
    }
  });

  app.get("/api/designs", async () => ({ designs: await listDesigns() }));

  app.post("/api/designs", async (request, reply) => {
    const body = createDesignRequestSchema.parse(request.body);
    const seed = seedDesignGraph(body.prompt);
    const design = await createDesign({
      title: body.title || seed.title,
      prompt: body.prompt,
      graph: decisionGraphSchema.parse(seed.graph)
    });
    reply.status(201).send({ design, message: seed.explanation });
  });

  app.get<{ Params: { id: string } }>("/api/designs/:id", async (request, reply) => {
    const design = await loadDesignOr404(request.params.id, reply);
    if (!design) return;
    return { design };
  });

  app.patch<{ Params: { id: string } }>("/api/designs/:id/graph", async (request, reply) => {
    const design = await loadDesignOr404(request.params.id, reply);
    if (!design) return;
    const body = graphEditRequestSchema.parse(request.body ?? {});
    const graphChanged = Boolean(body.graph);
    const updated = await saveDesign({
      ...design,
      graph: body.graph ?? design.graph,
      layout: body.layout ?? design.layout,
      selection: body.selection ?? design.selection,
      graphVersion: graphChanged ? design.graphVersion + 1 : design.graphVersion,
      updatedAt: new Date().toISOString()
    });
    return { design: updated };
  });

  app.post<{ Params: { id: string } }>("/api/designs/:id/comments", async (request, reply) => {
    const design = await loadDesignOr404(request.params.id, reply);
    if (!design) return;
    const body = createCommentRequestSchema.parse(request.body ?? {});
    const updated = await addComment(design, body);
    return { design: updated, comment: updated.comments[0] };
  });

  app.patch<{ Params: { id: string; commentId: string } }>(
    "/api/designs/:id/comments/:commentId",
    async (request, reply) => {
      const design = await loadDesignOr404(request.params.id, reply);
      if (!design) return;
      const body = updateCommentRequestSchema.parse(request.body ?? {});
      if (!design.comments.some((candidate) => candidate.id === request.params.commentId)) {
        reply.status(404).send({ error: "not_found", message: "Comment not found" });
        return;
      }
      const updated = await updateComment(design, request.params.commentId, body);
      const comment = updated.comments.find((candidate) => candidate.id === request.params.commentId);
      return { design: updated, comment };
    }
  );

  app.post<{ Params: { id: string } }>("/api/designs/:id/export", async (request, reply) => {
    const design = await loadDesignOr404(request.params.id, reply);
    if (!design) return;
    const body = exportRequestSchema.parse(request.body ?? {});
    const generated = generateLocalExport(design.graph, body, design.title);
    const persisted = await writeArtifactContent({
      designId: design.id,
      type: body.type,
      title: generated.title,
      content: generated.content,
      contentType: contentTypeFor(body.type)
    });
    const updated = await addArtifact(design, {
      type: body.type,
      title: generated.title,
      scope: body.scope.kind === "whole_graph" ? "whole_graph" : `${body.scope.kind}:${body.scope.id ?? ""}`,
      path: persisted.path,
      contentType: persisted.contentType
    });
    const artifact = updated.artifacts[0];
    return { design: updated, artifact };
  });

  app.get<{ Params: { id: string; artifactId: string } }>(
    "/api/designs/:id/artifacts/:artifactId",
    async (request, reply) => {
      const design = await loadDesignOr404(request.params.id, reply);
      if (!design) return;
      const artifact = design.artifacts.find((candidate) => candidate.id === request.params.artifactId);
      if (!artifact) {
        reply.status(404).send({ error: "not_found", message: "Artifact not found" });
        return;
      }
      if (!isExportPath(artifact.path)) {
        reply.status(403).send({ error: "forbidden", message: "Artifact path is outside export directory" });
        return;
      }
      reply.header("Content-Disposition", `attachment; filename="${artifact.title.replace(/[^a-z0-9.-]+/gi, "-")}"`);
      reply.type(artifact.contentType);
      return reply.send(createReadStream(artifact.path));
    }
  );

  await registerClientIfBuilt(app);
  return app;
}

async function loadDesignOr404(id: string, reply: FastifyReply) {
  const design = await readDesign(id);
  if (!design) {
    reply.status(404).send({ error: "not_found", message: "Design not found" });
    return null;
  }
  return design;
}

function contentTypeFor(type: string): string {
  if (type === "architecture_image") return "image/svg+xml";
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

if (import.meta.url === `file://${process.argv[1]}`) {
  await ensureStorage();
  const app = await buildServer();
  await app.listen({ host, port });
  const url = `http://${browserHost(host)}:${port}/`;
  if (shouldOpenBrowser()) {
    openBrowser(url);
  }
}

function shouldOpenBrowser(): boolean {
  return process.argv.includes("--open") || process.env.SHAPE_AI_OPEN_BROWSER === "1";
}

function browserHost(value: string): string {
  if (value === "0.0.0.0" || value === "::") return "127.0.0.1";
  return value;
}

function openBrowser(url: string): void {
  const [command, args] =
    process.platform === "darwin"
      ? ["open", [url]]
      : process.platform === "win32"
        ? ["cmd", ["/c", "start", "", url]]
        : ["xdg-open", [url]];
  const child = spawn(command, args, {
    detached: true,
    stdio: "ignore"
  });
  child.unref();
}
