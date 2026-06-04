import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { applyGraphPatch, graphTextDigest } from "../shared/graph";
import {
  exportRequestSchema,
  exportTypeSchema,
  graphLayoutSchema,
  graphPatchSchema,
  graphSelectionSchema
} from "../shared/schema";
import { generateLocalExport, seedDesignGraph } from "./local";
import {
  addArtifact,
  addComment,
  createDesign,
  ensureStorage,
  listDesigns,
  readDesign,
  saveDesign,
  updateComment,
  updateDesignGraph,
  writeArtifactContent
} from "./storage";

const server = new McpServer({
  name: "charrette",
  version: "0.1.0"
});

server.registerTool(
  "list_designs",
  {
    description: "List local Charrette designs with graph, selection, comment, and export counts.",
    inputSchema: {}
  },
  async () => {
    const designs = await listDesigns();
    return jsonResponse({
      designs: designs.map((design) => ({
        id: design.id,
        title: design.title,
        updatedAt: design.updatedAt,
        graphVersion: design.graphVersion,
        selection: design.selection,
        nodes: design.graph.nodes.length,
        edges: design.graph.edges.length,
        unresolvedComments: design.comments.filter((comment) => !comment.resolved).length,
        artifacts: design.artifacts.length
      }))
    });
  }
);

server.registerTool(
  "get_design",
  {
    description: "Read one design, including graph, Web UI selection, comments, layout, and artifact metadata.",
    inputSchema: {
      designId: z.string().min(1)
    }
  },
  async ({ designId }) => {
    const design = await readDesignOrThrow(designId);
    return jsonResponse({
      design,
      digest: graphTextDigest(design.graph)
    });
  }
);

server.registerTool(
  "create_design",
  {
    description: "Create a local design graph from a prompt. This does not call an embedded model.",
    inputSchema: {
      prompt: z.string().min(1),
      title: z.string().min(1).optional()
    }
  },
  async ({ prompt, title }) => {
    const seed = seedDesignGraph(prompt);
    const design = await createDesign({
      title: title || seed.title,
      prompt,
      graph: seed.graph
    });
    return jsonResponse({ design, message: seed.explanation });
  }
);

server.registerTool(
  "apply_graph_patch",
  {
    description: "Apply typed node and edge additions, updates, or removals to a design graph.",
    inputSchema: {
      designId: z.string().min(1),
      patch: graphPatchSchema
    }
  },
  async ({ designId, patch }) => {
    const design = await readDesignOrThrow(designId);
    const graph = applyGraphPatch(design.graph, patch);
    const updated = await updateDesignGraph(design, graph);
    return jsonResponse({ design: updated, digest: graphTextDigest(updated.graph) });
  }
);

server.registerTool(
  "save_layout",
  {
    description: "Update graph node positions for the Web UI canvas.",
    inputSchema: {
      designId: z.string().min(1),
      layout: graphLayoutSchema
    }
  },
  async ({ designId, layout }) => {
    const design = await readDesignOrThrow(designId);
    const updated = await saveDesign({
      ...design,
      layout,
      updatedAt: new Date().toISOString()
    });
    return jsonResponse({ design: updated });
  }
);

server.registerTool(
  "set_selection",
  {
    description: "Set the current Web UI selection that the user or agent is discussing.",
    inputSchema: {
      designId: z.string().min(1),
      selection: graphSelectionSchema
    }
  },
  async ({ designId, selection }) => {
    const design = await readDesignOrThrow(designId);
    const updated = await saveDesign({
      ...design,
      selection,
      updatedAt: new Date().toISOString()
    });
    return jsonResponse({ design: updated });
  }
);

server.registerTool(
  "add_comment",
  {
    description: "Add a comment to the whole graph, a node, or an edge.",
    inputSchema: {
      designId: z.string().min(1),
      target: graphSelectionSchema,
      body: z.string().min(1),
      author: z.string().min(1).optional()
    }
  },
  async ({ designId, target, body, author }) => {
    const design = await readDesignOrThrow(designId);
    const updated = await addComment(design, { target, body, author: author || "agent" });
    return jsonResponse({ design: updated, comment: updated.comments[0] });
  }
);

server.registerTool(
  "update_comment",
  {
    description: "Update comment text or resolution state.",
    inputSchema: {
      designId: z.string().min(1),
      commentId: z.string().min(1),
      body: z.string().min(1).optional(),
      resolved: z.boolean().optional()
    }
  },
  async ({ designId, commentId, body, resolved }) => {
    const design = await readDesignOrThrow(designId);
    if (!design.comments.some((comment) => comment.id === commentId)) {
      throw new Error(`Comment not found: ${commentId}`);
    }
    const updated = await updateComment(design, commentId, { body, resolved });
    return jsonResponse({
      design: updated,
      comment: updated.comments.find((comment) => comment.id === commentId)
    });
  }
);

server.registerTool(
  "export_design",
  {
    description: "Generate a local export artifact from a whole design graph or selected subgraph.",
    inputSchema: {
      designId: z.string().min(1),
      type: exportTypeSchema,
      scope: z
        .object({
          kind: z.enum(["whole_graph", "node", "edge"]),
          id: z.string().min(1).optional()
        })
        .optional()
    }
  },
  async ({ designId, type, scope }) => {
    const design = await readDesignOrThrow(designId);
    const request = exportRequestSchema.parse({ type, scope });
    const generated = generateLocalExport(design.graph, request, design.title);
    const persisted = await writeArtifactContent({
      designId: design.id,
      type,
      title: generated.title,
      content: generated.content,
      contentType: contentTypeFor(type)
    });
    const updated = await addArtifact(design, {
      type,
      title: generated.title,
      scope: request.scope.kind === "whole_graph" ? "whole_graph" : `${request.scope.kind}:${request.scope.id ?? ""}`,
      path: persisted.path,
      contentType: persisted.contentType
    });
    return jsonResponse({ design: updated, artifact: updated.artifacts[0] });
  }
);

async function readDesignOrThrow(designId: string) {
  const design = await readDesign(designId);
  if (!design) {
    throw new Error(`Design not found: ${designId}`);
  }
  return design;
}

function contentTypeFor(type: string): string {
  if (type === "architecture_image") return "image/svg+xml";
  if (type === "confluence_html") return "text/html; charset=utf-8";
  if (type === "mermaid") return "text/plain; charset=utf-8";
  return "text/markdown; charset=utf-8";
}

function jsonResponse(value: unknown) {
  return {
    content: [
      {
        type: "text" as const,
        text: JSON.stringify(value, null, 2)
      }
    ]
  };
}

async function main() {
  await ensureStorage();
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("Charrette MCP server running on stdio");
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Fatal MCP server error:", error);
    process.exit(1);
  });
}
