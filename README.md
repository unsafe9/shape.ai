# shape.ai

Visual group canvas for humans and AI agents — an infinite, local-first canvas
that agents can review and extend through MCP.

shape.ai keeps every canvas behind a single native Rust server. The server owns
one running actor per canvas, persists scene objects per-object in SQLite, and
serves the Web UI, a WebSocket sync transport, and a remote Streamable HTTP MCP
endpoint from the same process. The web client is a Svelte shell over a
Rust/WASM/WebGPU canvas; canvas logic (op-apply, layout, hit testing, LWW,
fractional indexing, templates) lives in the shared `scene-core` crate and is
never reimplemented in TypeScript or in the server.

## What It Does

- Creates a group from a proposition, architecture concern, or implementation plan.
- Stores scene objects as `Group`, `Node`, and `Edge` records with scene-space
  bounds and z-index ordering.
- Drives a Rust/WASM/WebGPU canvas with smooth pan/zoom, pinch zoom,
  renderer-owned culling, inline note editing, copy/paste, comments, and z-order
  actions.
- Keeps group tags in a global registry with create, rename, recolor,
  delete-unused, attach, detach, and filter flows.
- Exports group, node, edge, or selection scope as MADR Markdown, YADR YAML,
  Mermaid, and image-generation prompts.
- Exposes MCP tools so AI agents can query the scene, inspect groups, update
  group tags, patch scene objects, add comments, and export group content.

## Architecture

The stack is one Rust workspace plus a thin Svelte shell:

- `crates/scene-core` (`shape_scene_core`) — the pure canvas core: scene model,
  op enum, op-apply, per-property LWW, fractional index, templates, wire serde,
  and the command catalog. Compiles for native and `wasm32`; no time/rng/thread/IO.
  The web client builds it to WASM (`--features wasm`) and uses it as the
  client-side op-apply.
- `crates/storage-core` (`shape_storage_core`) — store-neutral persistence: a
  `StorageAdapter` trait, a spatial region index, a portable sharded bundle
  format, and Memory/File/SQLite adapters. SQLite is native-only; `wasm32` keeps
  the model, the trait, and the in-memory adapter.
- `crates/coordination` (`shape_coordination`) — the scale-out seam: a
  `Coordinator` trait giving single-writer canvas leases, pub/sub, and presence.
  Ships an in-memory impl (single-process dev) and a file impl (single-host
  multi-process). No canvas logic lives here.
- `crates/server` (`shape_server`) — the native (tokio/axum) platform layer. It
  orchestrates transport, persistence, and fan-out only:
  - **Canvas actor.** One tokio task per canvas serializes every edit through an
    mpsc channel, calls scene-core for op-apply, persists a per-object checkpoint
    plus a durable journal entry per op, and fans applied patches out over a
    broadcast channel.
  - **WebSocket transport (`/ws`).** Two logical channels over one socket
    (sync + presence). A client `hello` yields a windowed `welcome`, then ops are
    acked with monotonic server seqs; opId dedup makes re-apply idempotent.
  - **Sync engine.** Server-authoritative: opId dedup, journal-tail recovery past
    the last checkpoint, a per-property LWW store for concurrent property writes,
    and additive fractional order keys stamped into created objects.
  - **Per-object storage + windowing.** Each object is its own region-indexed
    SQLite record. Region reads answer a windowed subscriber from the actor's live
    in-memory scene filtered by bbox.
  - **Registry / leases.** The registry maps `canvasId` to its actor, acquires a
    single-writer lease before spawning, renews it on an interval, evicts idle
    canvases (flush + checkpoint), and releases leases on graceful shutdown so a
    successor recovers with no data loss.
  - **MCP (`/mcp`).** Streamable HTTP MCP (rmcp) mounted on the same router, plus
    a companion dock (`/api/mcp/clients`, `/api/mcp/trace`).
- `crates/renderer-core` (`shape_renderer_core`) — the pure CPU renderer core:
  tessellation, stroke expansion, curve LOD, text layout, hit testing, and the
  render model. No `wgpu`, no `web_sys`.
- `crates/renderer-wgpu` (`shape_canvas_core`) — the WebGPU half: the `wgpu`
  pipelines, web surface lifecycle, and the `#[wasm_bindgen]` renderer surface,
  built to WASM for the web canvas. Depends on `renderer-core` by path.
- `platforms/web` — the Svelte shell. It owns product UI and orchestration and
  talks to the canvas only through a narrow imperative handle plus an event
  stream. The client uses scene-core-WASM for optimistic op-apply and reaches the
  server over `/ws` and the `/api/*` routes.

## Local Development

The web shell owns its build toolchain under `platforms/web/` (`package.json`,
vite/svelte/vitest, `tsconfig.json`, `tests/`); Rust stays pure cargo. Build the
client (Rust → WASM for scene-core and the renderer, then the Vite bundle), then
run the native server:

```bash
cd platforms/web && npm install && npm run build && cd -
make build                          # same, via the root Makefile
cargo run -p shape_server
```

`npm run build` runs `scene:wasm:build` + `renderer:wasm:build` + `vite build`,
emitting the SPA to `platforms/web/dist/client/`, which is the server's default
client dir — `cargo run -p shape_server` serves it on `http://127.0.0.1:8787`
with no extra config.

For iterative frontend work, run the Vite dev server against a running backend:

```bash
make serve                          # backend on :8787  (cargo run -p shape_server)
make web                            # Vite client on :5173  (cd platforms/web && npm run dev)
```

Canonical scene data is stored per-object in `.local/shape.sqlite`, and exported
artifacts under `.local/exports/`. The `.local/` directory is ignored by git.

Useful environment variables:

- `SHAPE_AI_HOST`: server host, default `127.0.0.1`
- `SHAPE_AI_PORT`: server port, default `8787`
- `SHAPE_AI_DATA_DIR`: local storage root, default `.local`
- `SHAPE_AI_CLIENT_DIR`: pre-built client assets to serve, default
  `platforms/web/dist/client` (the Vite build output)

## API

Primary HTTP routes (served by `shape_server` alongside `/ws` and `/mcp`):

- `GET /api/health`, `GET /api/ready`: liveness and readiness probes.
- `GET /api/scene?tags`: query canonical scene objects with an optional tag
  filter. Viewport and zoom culling are renderer-owned.
- `PATCH /api/scene`: patch groups, nodes, edges, or the current selection.
- `POST /api/groups`: create a group with seeded nodes and optional tags.
- `PATCH /api/groups/:id/tags`: replace the tag IDs attached to a group.
- `POST /api/groups/:id/export`: export group content with optional `scope`.
- `POST /api/tags`, `PATCH /api/tags/:id`, `DELETE /api/tags/:id`: manage the
  tag registry.
- `POST /api/comments`, `PATCH /api/comments/:id`: add or resolve comments.
- `GET /api/canvases`, `POST /api/canvases`, `DELETE /api/canvases/:id`:
  multi-canvas CRUD.
- `GET /api/templates`, `POST /api/templates`, `DELETE /api/templates/:id`:
  template library.

## MCP Usage

Remote Streamable HTTP MCP is served by the same server:

```text
http://127.0.0.1:8787/mcp
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

`export_group` accepts either `type` for one format or `types` for several
formats, and returns generated content in preview fields alongside persisted
artifact metadata. MCP clients can pass `markdown` as an alias for `madr`.

## Exports

Exports are generated deterministically from the stored group subgraph:

- `madr`: Markdown Architectural Decision Record (`adr/madr` template).
- `yadr`: YAML Architectural Decision Record (`adr/yadr` template).
- `mermaid`: Mermaid flowchart text.
- `image_prompt`: prompt text for an MCP client with its own image generation.

Exports can target `group`, `node`, `edge`, or `selection` scope.

## Constraints

- SQLite is the canonical store; every object is its own region-indexed record.
- `Scene`, `Group`, `Node`, `Edge`, and `Tag` are the canonical model.
- Canvas logic stays in `shape_scene_core`. The server and the client shell are
  thin platform layers.
- The Web UI does not embed an OpenAI client, AI chat, shell execution, or
  code-editing tool. AI integration is via MCP.
- Identity is `userId`-only with no auth yet.
- Runtime data and exports under `.local/` stay out of git.

## Verification

```bash
scripts/renderer-toolchain.sh cargo test --workspace
scripts/renderer-toolchain.sh cargo check -p shape_scene_core --target wasm32-unknown-unknown
scripts/renderer-toolchain.sh cargo check -p shape_storage_core --target wasm32-unknown-unknown
scripts/renderer-toolchain.sh cargo check -p shape_client_runtime --target wasm32-unknown-unknown
cd platforms/web && npm run typecheck && npm run test:unit && npm run build
```

The root `Makefile` bundles these: `make test` (cargo + web unit), `make
check-wasm` (the three wasm32 checks), `make build`, `make test-renderer`.
