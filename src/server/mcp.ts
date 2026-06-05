import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { graphTextDigest, sceneGraphForGroup } from "../shared/graph";
import {
  createGroupRequestSchema,
  createTagRequestSchema,
  exportRequestSchema,
  exportTypeSchema,
  scenePatchSchema,
  sceneSelectionSchema,
  updateGroupTagsRequestSchema,
  type ExportType,
  type Scene
} from "../shared/schema";
import { generateLocalExport } from "./local";
import {
  addArtifact,
  addComment,
  createGroup,
  createTag,
  ensureStorage,
  readFullScene,
  readGroup,
  readScene,
  saveScenePatch,
  updateGroupTags,
  writeArtifactContent
} from "./storage";

const mcpExportTypeSchema = z.enum([
  "madr",
  "markdown",
  "yadr",
  "image_prompt",
  "ai_plan_md",
  "design_doc_md",
  "confluence_html",
  "mermaid",
  "architecture_image"
]);

const exportScopeInputSchema = z
  .object({
    kind: z.enum(["group", "node", "edge", "selection"]),
    id: z.string().min(1).optional()
  })
  .optional();

type McpExportType = z.infer<typeof mcpExportTypeSchema>;

export function createSceneMcpServer(): McpServer {
  const server = new McpServer({
    name: "shape.ai",
    version: "0.1.0"
  });

  server.registerTool(
    "query_scene",
    {
      description: "Query canonical scene data with optional group tag filters. Renderer-side culling owns viewport and zoom behavior.",
      inputSchema: {
        tagIds: z.array(z.string()).optional()
      }
    },
    async ({ tagIds }) => jsonResponse({ scene: await readScene({ tagIds }) })
  );

  server.registerTool(
    "list_groups",
    {
      description: "List top-level and nested groups on the scene canvas.",
      inputSchema: {}
    },
    async () => {
      const scene = await readFullScene();
      return jsonResponse({
        groups: scene.groups.map((group) => ({
          id: group.id,
          parentGroupId: group.parentGroupId,
          title: group.title,
          summary: group.summary,
          bounds: group.bounds,
          tagIds: group.tagIds,
          nodes: scene.nodes.filter((node) => node.groupId === group.id).length,
          edges: scene.edges.filter((edge) => edge.groupId === group.id).length,
          artifacts: scene.artifacts.filter((artifact) => artifact.target.kind === "group" && artifact.target.id === group.id).length
        })),
        tags: scene.tags
      });
    }
  );

  server.registerTool(
    "get_group",
    {
      description: "Read one group with its nodes, edges, tags, comments, artifacts, and graph digest.",
      inputSchema: {
        groupId: z.string().min(1)
      }
    },
    async ({ groupId }) => {
      const detail = await readGroup(groupId);
      if (!detail) throw new Error(`Group not found: ${groupId}`);
      const scene = await readFullScene();
      const graph = sceneGraphForGroup(scene, groupId);
      return jsonResponse({
        ...detail,
        digest: graphTextDigest(graph)
      });
    }
  );

  server.registerTool(
    "create_group",
    {
      description: "Create a new group on the infinite scene canvas from a prompt.",
      inputSchema: createGroupRequestSchema.shape
    },
    async (input) => jsonResponse(await createGroup(input))
  );

  server.registerTool(
    "patch_scene",
    {
      description: "Patch groups, nodes, edges, removals, or selection on the scene canvas.",
      inputSchema: scenePatchSchema.shape
    },
    async (input) => jsonResponse({ scene: await saveScenePatch(input) })
  );

  server.registerTool(
    "create_tag",
    {
      description: "Create a registered group tag with color and description.",
      inputSchema: createTagRequestSchema.shape
    },
    async (input) => jsonResponse(await createTag(input))
  );

  server.registerTool(
    "update_group_tags",
    {
      description: "Replace the registered tag ids attached to one group.",
      inputSchema: {
        groupId: z.string().min(1),
        ...updateGroupTagsRequestSchema.shape
      }
    },
    async ({ groupId, tagIds }) => jsonResponse(await updateGroupTags(groupId, tagIds))
  );

  server.registerTool(
    "set_selection",
    {
      description: "Set the current Web UI scene selection.",
      inputSchema: {
        selection: sceneSelectionSchema
      }
    },
    async ({ selection }) => jsonResponse({ scene: await saveScenePatch({ selection }) })
  );

  server.registerTool(
    "add_comment",
    {
      description: "Add a comment to the canvas, a group, a node, or an edge.",
      inputSchema: {
        target: sceneSelectionSchema,
        body: z.string().min(1),
        author: z.string().min(1).optional()
      }
    },
    async (input) => jsonResponse(await addComment({ target: input.target, body: input.body, author: input.author ?? "agent" }))
  );

  server.registerTool(
    "export_group",
    {
      description: "Generate local export artifacts from a group. Pass type for one format or types for multiple formats; content is returned in preview fields.",
      inputSchema: {
        groupId: z.string().min(1),
        type: mcpExportTypeSchema.optional(),
        types: z.array(mcpExportTypeSchema).min(1).optional(),
        scope: exportScopeInputSchema
      }
    },
    async ({ groupId, type, types, scope }) => {
      const result = await exportGroupContent(groupId, exportTypesFromInput({ type, types }), scope);
      return jsonResponse({
        group: result.group,
        artifact: result.exports[0]?.artifact,
        preview: result.exports[0]?.preview,
        exports: result.exports,
        imagePrompt: result.exports[0]?.preview.imagePrompt
      });
    }
  );

  return server;
}

function exportTypesFromInput(input: { type?: McpExportType; types?: McpExportType[] }): ExportType[] {
  const values = [...(input.type ? [input.type] : []), ...(input.types ?? [])].map(normalizeExportType);
  const unique = Array.from(new Set(values));
  if (unique.length === 0) throw new Error("type or types is required");
  return unique;
}

function normalizeExportType(type: McpExportType): ExportType {
  return type === "markdown" ? "madr" : type;
}

async function exportGroupContent(groupId: string, types: ExportType[], scope: unknown) {
  const detail = await readGroup(groupId);
  if (!detail) throw new Error(`Group not found: ${groupId}`);
  let scene: Scene = await readFullScene();
  const exports = [];

  for (const type of types) {
    const request = exportRequestSchema.parse({ type, scope: normalizeScope(scope, groupId) });
    const graph = sceneGraphForGroup(scene, groupId);
    const generated = generateLocalExport(graph, request, detail.group.title);
    const persisted = await writeArtifactContent({
      groupId,
      type,
      title: generated.title,
      content: generated.content,
      contentType: contentTypeFor(type)
    });
    const stored = await addArtifact(groupId, {
      type,
      title: generated.title,
      target: { kind: "group", id: groupId },
      path: persisted.path,
      contentType: persisted.contentType
    });
    scene = stored.scene;
    exports.push({
      artifact: stored.artifact,
      preview: {
        type,
        title: generated.title,
        content: generated.content,
        contentType: persisted.contentType,
        imagePrompt: generated.imagePrompt
      }
    });
  }

  return { group: detail.group, scene, exports };
}

function normalizeScope(scope: unknown, groupId: string) {
  const parsed = exportScopeInputSchema.parse(scope);
  return parsed ?? { kind: "group" as const, id: groupId };
}

function contentTypeFor(type: ExportType): string {
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
  const server = createSceneMcpServer();
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
