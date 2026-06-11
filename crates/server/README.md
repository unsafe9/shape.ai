# shape_server

The native (tokio/axum) platform seam for shape.ai: it orchestrates **transport,
persistence, and fan-out** in front of the pure scene core. Agent rules for this
crate live in `CLAUDE.md`.

## Running

```sh
# from the workspace root, always via the toolchain wrapper:
scripts/renderer-toolchain.sh cargo run -p shape_server
```

Configuration is read from the environment (`Config::from_env`):

| Env var               | Default                          | Meaning                                            |
| --------------------- | -------------------------------- | -------------------------------------------------- |
| `SHAPE_AI_HOST`       | `127.0.0.1`                      | Listener host.                                     |
| `SHAPE_AI_PORT`       | `8787`                           | Listener port.                                     |
| `SHAPE_AI_CLIENT_DIR` | `<crate>/../../dist/client`      | Pre-built SPA dir; static hosting skipped if absent.|
| `SHAPE_AI_DATA_DIR`   | `.local`                         | Data dir; the canvas sqlite db is `<dir>/shape.sqlite`. |
| `RUST_LOG`            | `info`                           | `tracing-subscriber` env filter.                   |

Routes mounted by `serve` → `build_router_with_mcp`:

- `GET  /api/health` — liveness + build identity.
- `GET  /api/ready`  — readiness.
- `GET  /api/mcp/clients` — live MCP companion identity list (dock).
- `GET  /api/mcp/trace?clientId=..&limit=..` — one companion's recent trace ring.
- `POST/GET/DELETE /mcp` — the streamable-HTTP MCP transport.
- static SPA under `client_dir` with `index.html` fallback, when present.

## Module layout

| File              | Responsibility                                                                 |
| ----------------- | ------------------------------------------------------------------------------ |
| `lib.rs`          | `serve` entry point; constructs shared state and the router; re-exports.        |
| `main.rs`         | Thin binary: init tracing, `serve(Config::from_env())`.                         |
| `config.rs`       | `Config` from env (host/port/client_dir).                                       |
| `app.rs`          | Router assembly: `build_router`, `build_router_with_mcp`, dock HTTP, MCP mount.  |
| `canvas_actor.rs` | `CanvasActor` task + `ActorHandle`; per-canvas serialized edits, persist, fan-out.|
| `registry.rs`     | `CanvasRegistry`: `canvasId → ActorHandle`, spawn-on-demand, idle eviction.     |
| `mcp.rs`          | `SceneMcp`: the 11 rmcp tools, wired through the registry into scene-core.       |
| `mcp_clients.rs`  | `ClientRegistry`: in-memory companion identities + per-client trace ring (dock). |

## CanvasActor / Registry / ActorHandle public API

One `CanvasActor` is a tokio task that **owns one canvas's `Scene`** and serializes
every edit through an mpsc command channel, so the scene is never touched
concurrently. The actor calls `shape_scene_core::apply_render_patch_to_shape_scene`
(and `add_shape_scene_comment`) and only orchestrates persistence + fan-out.

### `CanvasRegistry` (clone-able, shared state)

```rust
CanvasRegistry::open(path) -> anyhow::Result<Self>     // on-disk sqlite
CanvasRegistry::open_in_memory() -> anyhow::Result<Self>// tests
CanvasRegistry::new(SqliteAdapter) -> Self
fn get_or_spawn(&self, &CanvasId) -> ActorHandle        // spawn + load durable state on first use
fn contains(&self, &CanvasId) -> bool
fn len(&self) -> usize ; fn is_empty(&self) -> bool
async fn evict(&self, &CanvasId)                        // remove + shutdown (flush+checkpoint)
async fn evict_idle(&self, Duration) -> Vec<CanvasId>   // evict canvases idle >= duration
```

A single `SqliteAdapter` behind a `Mutex` (`SharedStore = Arc<Mutex<SqliteAdapter>>`)
backs every canvas; the registry hands each actor a clone of that shared store.

### `ActorHandle` (clone-able)

```rust
async fn apply_patch(&self, RenderScenePatch, user_id: &str) -> ApplyResult
async fn add_comment(&self, SceneSelection, body: &str, user_id: &str) -> CommentResult
async fn get_scene(&self) -> Scene
fn      subscribe(&self) -> broadcast::Receiver<PatchBroadcast>
async fn shutdown(&self)                                 // flush + checkpoint, stop the task
```

```rust
enum ApplyResult {
    Applied { seq: i64, revision: i64 },   // seq = actor's monotonic server seq; revision = scene.scene_version
    Rejected { errors: Vec<String> },      // scene-core rejected; nothing persisted or broadcast
}
enum CommentResult { Added { comment: SceneComment }, Rejected { errors: Vec<String> } }

struct PatchBroadcast {                     // what subscribers receive on each apply
    seq: i64,                               // server seq assigned to this apply
    patch: RenderScenePatch,                // the applied op
    scene: Scene,                           // full scene after apply (whole-scene fan-out for MG-2)
}
```

`CanvasActor::spawn(CanvasId, SharedStore) -> ActorHandle` is also public so tests
can spawn a bare actor on a shared store (used to verify checkpoint/respawn).

## MCP tool list + transport

The MCP server (`SceneMcp`) is built on the official Rust SDK (`rmcp` v1.7) and
served over the **streamable-HTTP** transport mounted at `/mcp` (`StreamableHttpService`
as a `tower::Service` via `nest_service`; coexists with axum 0.7, both on http 1.x).
The transport's service factory mints a fresh `SceneMcp` per session with a stable
`http-session-{n}` client id; the session's identity is registered in `initialize()`
from the client's advertised `Implementation` (mirrors the Node `oninitialized` hook).

Handler bodies are plain `async fn`s on `SceneMcp`, so tests call them directly
without standing up the transport. `SceneMcp::tool_definitions()` enumerates the
registered tools (the `#[tool_router]` macro's own `tool_router()` is private).

11 tools (each a behavioural port of the Node `src/server/mcp.ts` tool of the same
name; every scene mutation goes through `ActorHandle`, every digest/export through
scene-core `graph` helpers):

| Tool                | Kind    | Notes                                                                |
| ------------------- | ------- | ------------------------------------------------------------------- |
| `query_scene`       | read    | optional `tagIds` group filter.                                      |
| `list_groups`       | read    | groups + per-group node/edge/artifact counts + tags.                |
| `get_group`         | read    | one group with nodes/edges/tags/comments/artifacts + graph digest.  |
| `create_group`      | write   | from a prompt; fixed default frame + prompt-derived title.          |
| `patch_scene`       | write   | arbitrary `RenderScenePatch` (the shell wire shape).                |
| `create_tag`        | write   | registered group tag.                                               |
| `update_group_tags` | write   | replace a group's tag set.                                          |
| `set_selection`     | write   | set the Web UI scene selection.                                     |
| `add_comment`       | comment | canvas/group/node/edge comment.                                     |
| `export_group`      | export  | text digest / mermaid from the group's decision graph, inline.      |
| `get_client_trace`  | read    | one companion's in-memory trace ring.                               |

### Known deferrals

- `export_group` returns the scene-core `graph_text_digest` (text formats) /
  `make_mermaid` (mermaid/architecture_image); it does **not** yet produce the full
  MADR/YADR/confluence_html/ai_plan_md bodies, nor persist artifacts to disk / add a
  `SceneArtifact` (the Node `generateLocalExport` + `addArtifact`). Wire during the
  MG-7 Node decommission.
- `create_group` uses a fixed default frame `(0,0,1900x1100)` and a prompt-derived
  title; it does not run the Node `seedGroupScene` (seed nodes/edges/layout). Port
  when CC/MG-9 needs it.
- `get_client_trace` reads only the in-memory ring; the persisted journal is not yet
  projected into the trace.
- `ClientRegistry::mark_disconnected()` exists but has no caller; MG-6/MG-8 should
  hook it on session close.

## Storage layout

A single sqlite db (`<SHAPE_AI_DATA_DIR>/shape.sqlite`) backs every canvas through one
shared `SqliteAdapter`. The actor persists under one storage lock per apply and
**namespaces every `Record` id by `canvasId`** so ids stay globally unique:

- **Scene checkpoint** — `Record { id: "{canvasId}:scene", kind: "canvas", version: seq }`.
  Rewritten on every applied op (checkpoint-on-apply *is* the flush) and re-affirmed
  on shutdown.
- **Journal entry** — `Record { id: "{canvasId}:journal:{seq}", kind: "journal", version: seq }`,
  one durable `OperationEnvelope` per applied op (the op log).

On `(get_or_spawn | spawn)` a canvas loads `(scene, seq)` from its `:scene` checkpoint
(or a fresh empty scene at seq 0). Per-object `Record`s + region-backed eviction via
`SpatialStore` is a PC10 follow-up for MG-5/MG-9.

`now()` in `canvas_actor.rs` is a fixed RFC3339 stub; MG-4 sources a real clock from
the transport boundary so scene-core stays ambient-time-free.
