# shape.ai

Visual decision design for humans and AI agents.

shape.ai is a design-decision workspace that agents can read, review, and extend through MCP. It stores typed design graphs in SQLite, exposes a React canvas for humans, and serves both a Web UI and a remote Streamable HTTP MCP endpoint from one Fastify server.

## What It Does

- Creates a `Design` from a proposition, architecture concern, or implementation plan.
- Represents each design as a typed decision graph with nodes for propositions, decision points, options, evidence, tradeoffs, blockers, subdecisions, tasks, and artifacts.
- Supports graph editing in the Web UI: node dragging, layout persistence, node and edge inspection, inline field edits, connected-node creation, edge creation, and deletion.
- Stores comments on the whole graph, a selected node, or a selected edge.
- Exports the whole graph or selected subgraph as deterministic local artifacts: AI task plan Markdown, human design doc Markdown, Confluence HTML, Mermaid, and an architecture image SVG.
- Exposes MCP tools so AI agents can list/read designs, propose graph changes, validate diffs, leave review comments, approve/reject proposals, and export designs.

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

The Fastify backend listens on `http://127.0.0.1:8787` by default. Canonical design data is stored in `.local/shape.sqlite`, legacy `.local/designs/*.json` files are imported when the database is empty, and exported artifacts are stored under `.local/exports/`. The `.local/` directory is intentionally ignored by git.

Useful environment variables:

- `SHAPE_AI_HOST`: backend host, default `127.0.0.1`
- `SHAPE_AI_PORT`: backend port, default `8787`
- `SHAPE_AI_CLIENT_PORT`: Vite client port, default `5173`
- `SHAPE_AI_DATA_DIR`: local storage root, default `.local`
- `SHAPE_AI_REPO_ROOT`: repository root used when validating local export paths
- `SHAPE_AI_OPEN_BROWSER`: set to `1` to open the browser when running the server directly

## MCP Usage

Remote Streamable HTTP MCP is served by the main server:

```text
http://127.0.0.1:8787/mcp
```

The stdio MCP server is still available for hosts that need process-based MCP:

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
- `get_selection`
- `list_open_proposals`
- `create_proposal`
- `append_proposal_patch`
- `validate_proposal`
- `get_proposal_diff`
- `comment_on_proposal`
- `request_proposal_changes`
- `approve_proposal`
- `reject_proposal`
- `export_design`

The compatibility tools `create_design`, `save_layout`, `set_selection`, `add_comment`, and `update_comment` remain available. Direct graph mutation through MCP is intentionally not exposed; graph content changes should go through proposals.

## Exports

Exports are generated locally and deterministically from the stored graph:

- `ai_task_plan`: Markdown task plan for agents.
- `human_design_doc`: Markdown design document for human review.
- `confluence_html`: Confluence-ready HTML.
- `mermaid`: Mermaid flowchart text.
- `architecture_image`: SVG architecture image.

Exports can target the whole graph, a selected node subgraph, or a selected edge subgraph.

## Constraints

- shape.ai uses SQLite as the canonical store.
- APIs are under `/api/designs`.
- The Web UI does not embed an OpenAI client, AI chat, shell execution, or code-editing tool.
- AI integration is intentionally via MCP, with remote Streamable HTTP at `/mcp` and stdio as a compatibility transport.
- Runtime data and exports under `.local/` should stay out of git.

## Verification

```bash
npm run typecheck
npm run test:unit
npm run build
npm run test:e2e
```
