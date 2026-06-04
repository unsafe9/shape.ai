# shape.ai

Visual decision design for humans and AI agents.

shape.ai is a local design-decision workspace that agents can read, edit, and extend through MCP. It stores typed design graphs on disk, exposes a React canvas for humans, and provides a stdio MCP server for AI agents that need to inspect or update the same local state.

## What It Does

- Creates a `Design` from a proposition, architecture concern, or implementation plan.
- Represents each design as a typed decision graph with nodes for propositions, decision points, options, evidence, tradeoffs, blockers, subdecisions, tasks, and artifacts.
- Supports graph editing in the Web UI: node dragging, layout persistence, node and edge inspection, inline field edits, connected-node creation, edge creation, and deletion.
- Stores comments on the whole graph, a selected node, or a selected edge.
- Exports the whole graph or selected subgraph as deterministic local artifacts: AI task plan Markdown, human design doc Markdown, Confluence HTML, Mermaid, and an architecture image SVG.
- Exposes MCP tools so local AI agents can list, read, create, patch, comment on, lay out, select, and export designs.

## Local Development

```bash
npm install
npm run dev
```

Open `http://127.0.0.1:5173`.

The Fastify backend listens on `http://127.0.0.1:8787` by default. Local design data is stored under `.local/designs/`, and exported artifacts are stored under `.local/exports/`. The `.local/` directory is intentionally ignored by git.

Useful environment variables:

- `SHAPE_AI_HOST`: backend host, default `127.0.0.1`
- `SHAPE_AI_PORT`: backend port, default `8787`
- `SHAPE_AI_CLIENT_PORT`: Vite client port, default `5173`
- `SHAPE_AI_DATA_DIR`: local storage root, default `.local`
- `SHAPE_AI_REPO_ROOT`: repository root used when validating local export paths

## MCP Usage

Run the stdio MCP server from this repository:

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

- `list_designs`
- `get_design`
- `create_design`
- `apply_graph_patch`
- `save_layout`
- `set_selection`
- `add_comment`
- `update_comment`
- `export_design`

## Exports

Exports are generated locally and deterministically from the stored graph:

- `ai_task_plan`: Markdown task plan for agents.
- `human_design_doc`: Markdown design document for human review.
- `confluence_html`: Confluence-ready HTML.
- `mermaid`: Mermaid flowchart text.
- `architecture_image`: SVG architecture image.

Exports can target the whole graph, a selected node subgraph, or a selected edge subgraph.

## Constraints

- shape.ai is local-first and file-backed.
- APIs are under `/api/designs`.
- The Web UI does not embed an OpenAI client, AI chat, shell execution, or code-editing tool.
- AI integration is intentionally via the stdio MCP server.
- Runtime data and exports under `.local/` should stay out of git.

## Verification

```bash
npm run typecheck
npm run test:unit
npm run build
npm run test:e2e
```
