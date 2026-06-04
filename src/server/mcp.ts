import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { graphTextDigest } from "../shared/graph";
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
  appendProposalPatch,
  approveProposal,
  commentOnProposal,
  createDesign,
  createProposal,
  ensureStorage,
  getProposalDiff,
  listDesigns,
  listOpenProposals,
  readDesign,
  rejectProposal,
  requestProposalChanges,
  saveDesign,
  updateComment,
  validateProposal,
  writeArtifactContent
} from "./storage";

export function createShapeMcpServer(): McpServer {
  const server = new McpServer({
    name: "shape.ai",
    version: "0.1.0"
  });

  server.registerTool(
    "list_designs",
    {
      description: "List shape.ai designs with graph, selection, comment, proposal, and export counts.",
      inputSchema: {}
    },
    async () => {
      const designs = await listDesigns();
      const openProposals = await listOpenProposals();
      return jsonResponse({
        designs: designs.map((design) => ({
          id: design.id,
          title: design.title,
          updatedAt: design.updatedAt,
          graphVersion: design.graphVersion,
          selection: design.selection,
          nodes: design.graph.nodes.length,
          edges: design.graph.edges.length,
          openProposals: openProposals.filter((proposal) => proposal.designId === design.id).length,
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
    "get_selection",
    {
      description: "Read the current Web UI selection for a design.",
      inputSchema: {
        designId: z.string().min(1)
      }
    },
    async ({ designId }) => {
      const design = await readDesignOrThrow(designId);
      const selection = design.selection;
      const selected =
        selection.kind === "node"
          ? design.graph.nodes.find((node) => node.id === selection.id)
          : selection.kind === "edge"
            ? design.graph.edges.find((edge) => edge.id === selection.id)
            : undefined;
      return jsonResponse({ selection: design.selection, selected });
    }
  );

  server.registerTool(
    "list_open_proposals",
    {
      description: "List open or changes-requested proposals, optionally scoped to one design.",
      inputSchema: {
        designId: z.string().min(1).optional()
      }
    },
    async ({ designId }) => jsonResponse({ proposals: await listOpenProposals(designId) })
  );

  server.registerTool(
    "create_proposal",
    {
      description: "Create a proposal against a base graph version. This does not mutate the canonical graph.",
      inputSchema: {
        designId: z.string().min(1),
        title: z.string().min(1),
        description: z.string().default("").optional(),
        baseGraphVersion: z.number().int().nonnegative().optional(),
        createdBy: z.string().min(1).optional()
      }
    },
    async ({ designId, title, description, baseGraphVersion, createdBy }) => {
      const proposal = await createProposal({ designId, title, description, baseGraphVersion, createdBy });
      return jsonResponse({ proposal });
    }
  );

  server.registerTool(
    "append_proposal_patch",
    {
      description: "Append typed node and edge additions, updates, or removals to a proposal.",
      inputSchema: {
        proposalId: z.string().min(1),
        patch: graphPatchSchema
      }
    },
    async ({ proposalId, patch }) => {
      const result = await appendProposalPatch({ proposalId, patch });
      return jsonResponse(result);
    }
  );

  server.registerTool(
    "validate_proposal",
    {
      description: "Validate whether a proposal can apply to the current graph version.",
      inputSchema: {
        proposalId: z.string().min(1)
      }
    },
    async ({ proposalId }) => jsonResponse(await validateProposal(proposalId))
  );

  server.registerTool(
    "get_proposal_diff",
    {
      description: "Read proposal patches, comments, validation status, and graph digests before/after the proposal.",
      inputSchema: {
        proposalId: z.string().min(1)
      }
    },
    async ({ proposalId }) => jsonResponse(await getProposalDiff(proposalId))
  );

  server.registerTool(
    "comment_on_proposal",
    {
      description: "Add a review comment to a proposal.",
      inputSchema: {
        proposalId: z.string().min(1),
        body: z.string().min(1),
        author: z.string().min(1).optional()
      }
    },
    async ({ proposalId, body, author }) => jsonResponse(await commentOnProposal({ proposalId, body, author }))
  );

  server.registerTool(
    "request_proposal_changes",
    {
      description: "Mark a proposal as needing changes, optionally adding a review comment.",
      inputSchema: {
        proposalId: z.string().min(1),
        body: z.string().min(1).optional(),
        author: z.string().min(1).optional()
      }
    },
    async ({ proposalId, body, author }) => jsonResponse(await requestProposalChanges({ proposalId, body, author }))
  );

  server.registerTool(
    "approve_proposal",
    {
      description: "Apply a clean proposal to the canonical design graph and create a new graph version.",
      inputSchema: {
        proposalId: z.string().min(1)
      }
    },
    async ({ proposalId }) => {
      const result = await approveProposal(proposalId);
      return jsonResponse({
        ...result,
        digest: graphTextDigest(result.design.graph)
      });
    }
  );

  server.registerTool(
    "reject_proposal",
    {
      description: "Reject a proposal without mutating the canonical graph.",
      inputSchema: {
        proposalId: z.string().min(1)
      }
    },
    async ({ proposalId }) => jsonResponse(await rejectProposal(proposalId))
  );

  server.registerTool(
    "create_design",
    {
      description: "Create a design graph from a prompt. This does not call an embedded model.",
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
    "save_layout",
    {
      description: "Update graph node positions for the Web UI canvas. This does not change graph content.",
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
      description: "Update graph comment text or resolution state.",
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
      return jsonResponse({ design: updated, artifact: updated.artifacts[0], imagePrompt: generated.imagePrompt });
    }
  );

  return server;
}

async function readDesignOrThrow(designId: string) {
  const design = await readDesign(designId);
  if (!design) {
    throw new Error(`Design not found: ${designId}`);
  }
  return design;
}

function contentTypeFor(type: string): string {
  if (type === "yadr") return "application/yaml; charset=utf-8";
  if (type === "image_prompt" || type === "architecture_image") return "text/markdown; charset=utf-8";
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
  const server = createShapeMcpServer();
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("shape.ai MCP server running on stdio");
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    console.error("Fatal MCP server error:", error);
    process.exit(1);
  });
}
