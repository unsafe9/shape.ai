# shape.ai

Visual group canvas for humans and AI agents.

shape.ai stores one infinite `Scene` in SQLite. The only organization unit is a `Group`; nodes and edges live inside groups, and registered tags are attached to groups for filtering, overview tinting, and MCP queries. The same Fastify process serves the Web UI and remote Streamable HTTP MCP at `/mcp`.

## What It Does

- Creates a group from a proposition, architecture concern, or implementation plan.
- Stores scene objects as `Group`, `Node`, and `Edge` records with scene-space bounds and z-index ordering.
- Supports a Rust/WASM/WebGPU canvas UI with smooth pan/zoom, pinch zoom, renderer-owned culling, inline note editing, copy/paste, comments, and z-order actions.
- Keeps group tags in a global registry with create, rename, recolor, delete-unused, attach, detach, and filter flows.
- Exports group, node, edge, or selection scope as MADR Markdown, YADR YAML, Mermaid, and image-generation prompts.
- Exposes MCP tools so AI agents can query the scene, inspect groups, update group tags, patch scene objects, add comments, and export group content.

## Local Development

```bash
npm install
npm start
```

`npm start` builds the client, starts the Fastify server, and opens `http://127.0.0.1:8787`.

For iterative frontend/backend development, use:

```bash
npm run dev
```

The Fastify backend listens on `http://127.0.0.1:8787` by default. Canonical scene data is stored in `.local/shape.sqlite`, and exported artifacts are stored under `.local/exports/`. Legacy snapshots are migrated into top-level groups when an old local database is detected. The `.local/` directory is intentionally ignored by git.

Useful environment variables:

- `SHAPE_AI_HOST`: backend host, default `127.0.0.1`
- `SHAPE_AI_PORT`: backend port, default `8787`
- `SHAPE_AI_CLIENT_PORT`: Vite client port, default `5173`
- `SHAPE_AI_DATA_DIR`: local storage root, default `.local`
- `SHAPE_AI_REPO_ROOT`: repository root used when validating local export paths
- `SHAPE_AI_OPEN_BROWSER`: set to `1` to open the browser when running the server directly

## API

Primary HTTP routes:

- `GET /api/scene?tags`: query canonical scene objects with an optional tag filter. Viewport and zoom culling are renderer-owned.
- `PATCH /api/scene`: patch groups, nodes, edges, or the current selection.
- `POST /api/groups`: create a group with seeded nodes and optional tags.
- `GET /api/groups/:id`: read a group subgraph.
- `PATCH /api/groups/:id/tags`: replace the tag IDs attached to a group.
- `POST /api/tags`, `PATCH /api/tags/:id`, `DELETE /api/tags/:id`: manage the tag registry.
- `POST /api/groups/:id/export`: export group content with optional `scope`.
- `POST /api/comments`, `PATCH /api/comments/:commentId`: add or resolve comments.

## MCP Usage

Remote Streamable HTTP MCP is served by the main server:

```text
http://127.0.0.1:8787/mcp
```

The stdio MCP server is also available for hosts that need process-based MCP:

```bash
npm run mcp
```

Example MCP host command:

```json
{
  "command": "npm",
  "args": ["run", "mcp"],
  "cwd": "/absolute/path/to/shape.ai"
}
```

Available tools:

- `query_scene`
- `list_groups`
- `get_group`
- `create_group`
- `patch_scene`
- `create_tag`
- `update_group_tags`
- `set_selection`
- `add_comment`
- `export_group`

`export_group` accepts either `type` for one format or `types` for several formats, and returns generated content in preview fields alongside persisted artifact metadata. MCP clients can pass `markdown` as an alias for `madr`.

## Exports

Exports are generated locally and deterministically from the stored group subgraph:

- `madr`: Markdown Architectural Decision Record, based on the `adr/madr` template.
- `yadr`: YAML Architectural Decision Record, based on the `adr/yadr` template.
- `mermaid`: Mermaid flowchart text.
- `image_prompt`: Prompt text for an MCP client with its own image-generation capability.

Exports can target `group`, `node`, `edge`, or `selection` scope.

## Performance Model

- The server returns canonical scene data with business filters; viewport and zoom culling happen in the Rust/WASM/WebGPU renderer.
- The client mounts a renderer host and delegates retained scene rendering, camera updates, culling, hit testing, text layout, and GPU buffer/cache work to the Rust/WASM/WebGPU renderer.
- DOM remains for product panels, floating controls, diagnostics, and the active native input overlay while editing.
- Generated WASM glue is built into `src/client/renderer/wasm/` before production builds and copied into the client bundle.

## Constraints

- SQLite is the canonical store.
- `Scene`, `Group`, `Node`, `Edge`, and `Tag` are the canonical model.
- The Web UI does not embed an OpenAI client, AI chat, shell execution, or code-editing tool.
- AI integration is via MCP, with remote Streamable HTTP at `/mcp` and stdio as a compatibility transport.
- Runtime data and exports under `.local/` should stay out of git.

## Verification

```bash
npm run typecheck
npm run test:unit
npm run renderer:test
npm run renderer:rust:test
npm run build
```
