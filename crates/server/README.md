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
| `SHAPE_AI_DATA_DIR`   | `.local`                         | Data dir; the canvas redb db is `<dir>/shape.redb`.    |
| `RUST_LOG`            | `info`                           | `tracing-subscriber` env filter.                   |

Routes mounted by `serve` → `build_router_with_mcp`:

- `GET  /api/health` — liveness + build identity.
- `GET  /api/ready`  — readiness.
- `GET  /ws` — the WebSocket transport (two logical channels over one socket).
- `GET/POST/DELETE /api/canvases` — canvas CRUD (list / create / delete).
- `GET  /api/templates` — the built-in object-template catalog.
- `POST/GET/DELETE /mcp` — the streamable-HTTP MCP transport.
- static SPA under `client_dir` with `index.html` fallback, when present.

## Module layout

| File               | Responsibility                                                                      |
| ------------------ | ----------------------------------------------------------------------------------- |
| `lib.rs`           | `serve` entry point; constructs shared state and the router; re-exports.            |
| `main.rs`          | Thin binary: init tracing, `serve(Config::from_env())`.                             |
| `config.rs`        | `Config` from env (host/port/client_dir).                                           |
| `app.rs`           | Router assembly: `build_router`, `build_router_with_mcp`, WS + canvas API + MCP.   |
| `canvas_actor.rs`  | `CanvasActor` task + `ActorHandle`; per-canvas serialized edits, persist, fan-out.  |
| `canvas_index.rs`  | Durable canvas index: list of canvases that survive restart (one store `Record`).   |
| `registry.rs`      | `CanvasRegistry`: `canvasId → ActorHandle`, lease-guarded spawn, routing, eviction. |
| `mcp.rs`           | `SceneMcp`: the 9 object-native rmcp tools, wired through the registry.             |
| `object_mcp.rs`    | Pure tool bodies over `&ObjectScene`: list/get/create/patch/query/export (no IO).   |
| `object_feature.rs`| Feature channel handlers: request/response RPC lowered to `ObjectOp`s.             |
| `object_store.rs`  | Object-native persistence: per-object `Record`s, region index, apply layer.        |
| `sync.rs`          | Server-authoritative sync primitives: op-id dedup, journal, checkpoint recovery.   |
| `ws.rs`            | WebSocket transport: `reliable_ordered` + `ephemeral_besteffort` logical channels. |

## CanvasActor / Registry / ActorHandle public API

One `CanvasActor` is a tokio task that **owns one canvas's `ObjectScene`** and
serializes every edit through an mpsc command channel, so the scene is never
touched concurrently. The actor drives `ObjectStore` which calls scene-core's
`apply_object_op_lww`; the actor only orchestrates the server sequence, durable
journaling, idempotent dedup, and fan-out to subscribers.

### `CanvasRegistry` (clone-able, shared state)

```rust
CanvasRegistry::open(path) -> anyhow::Result<Self>              // on-disk redb
CanvasRegistry::open_in_memory() -> anyhow::Result<Self>         // tests
CanvasRegistry::new(RedbAdapter) -> Self
async fn get_or_spawn(&self, &CanvasId) -> Result<ActorHandle, SpawnError>  // lease-guarded spawn
fn contains(&self, &CanvasId) -> bool
fn len(&self) -> usize ; fn is_empty(&self) -> bool
async fn evict(&self, &CanvasId)                                 // remove + shutdown (flush+checkpoint)
async fn evict_idle(&self, Duration) -> Vec<CanvasId>            // evict canvases idle >= duration
fn create_canvas(&self, title: &str) -> anyhow::Result<CanvasSummary>
async fn delete_canvas(&self, &CanvasId) -> anyhow::Result<bool>
fn list_canvases(&self) -> Vec<CanvasSummary>
async fn shutdown(&self)                                         // drain all actors (graceful stop)
```

A single `RedbAdapter` behind a `Mutex` (`SharedStore = Arc<Mutex<RedbAdapter>>`)
backs every canvas; the registry hands each actor a clone of that shared store.

### `ActorHandle` (clone-able)

```rust
async fn apply_op(&self, ObjectOp, user_id: &str) -> ApplyResult        // bare op (MCP / tests)
async fn apply_envelope(&self, OpEnvelope, user_id: &str) -> ApplyResult // dedup envelope (WS)
async fn feature(&self, FeatureRequest, user_id: &str) -> FeatureResponse
async fn get_scene(&self) -> ObjectScene
async fn get_scene_region(&self, Option<RegionWindow>) -> ObjectScene
fn      subscribe(&self) -> broadcast::Receiver<PatchBroadcast>
async fn shutdown(&self)                                                  // flush + stop the task
```

```rust
enum ApplyResult {
    Applied { seq: i64, revision: i64 },   // seq = actor's monotonic server seq; revision = scene version
    Rejected { errors: Vec<String> },      // scene-core rejected; nothing persisted or broadcast
}

struct PatchBroadcast {                     // what subscribers receive on each apply
    seq: i64,                               // server seq assigned to this apply
    op: ObjectOp,                           // the applied op
    scene: ObjectScene,                     // full scene after apply (whole-scene fan-out)
    author: Option<String>,                 // originating userId/clientId; WS fan-out skips the originator
}
```

`CanvasActor::spawn(CanvasId, SharedStore) -> ActorHandle` is also public so tests
can spawn a bare actor on a shared store (used to verify checkpoint/respawn).

## MCP tool list + transport

The MCP server (`SceneMcp`) is built on the official Rust SDK (`rmcp` v1.7) and
served over the **streamable-HTTP** transport mounted at `/mcp` (`StreamableHttpService`
as a `tower::Service` via `nest_service`; coexists with axum 0.7, both on http 1.x).
The transport's service factory mints a fresh `SceneMcp` per session; every scene
mutation goes through `ActorHandle::apply_op`.

Handler bodies are plain `async fn`s on `SceneMcp`, so tests call them directly
without standing up the transport. `SceneMcp::tool_definitions()` enumerates the
registered tools (the `#[tool_router]` macro's own `tool_router()` is private).

9 object-native tools (pure over `ObjectScene`; write tools lower to `ObjectOp`
and push through the actor):

| Tool            | Kind    | Notes                                                                        |
| --------------- | ------- | ---------------------------------------------------------------------------- |
| `list_objects`  | read    | every object as a summary: id, descriptive kind label, bounds, tags.         |
| `get_object`    | read    | one full object: geometry, style, text, anchors, comments, tags.             |
| `create_object` | write   | insert object from a rect/text spec; lowers to one insert-object op.         |
| `patch_object`  | write   | update geometry/style/text/anchors on one object; lowers to a patch op.      |
| `tag_object`    | write   | replace the tag set on one object (set-tags op).                             |
| `add_comment`   | comment | append a comment to an object, optionally anchored to a geometry node.       |
| `query`         | read    | find object ids by tag, by anchor connection neighbors, and/or by region.    |
| `export`        | export  | AI-readable connection-graph digest (mermaid or plain text) for a scope.     |
| `set_selection` | write   | set the persisted scene selection.                                           |

### Known deferrals

- `export` returns `graph_text_digest` (text formats) / `make_mermaid`
  (mermaid); it does **not** yet produce full MADR/YADR/confluence_html/ai_plan_md
  bodies, nor persist artifacts to disk. Wire during the MG-7 Node decommission.
- `get_scene_region` loads from the region-indexed store; per-object `Record`
  eviction from the in-memory working set is a PC10 follow-up (MG-5/MG-9).
- `now()` in `canvas_actor.rs` is a fixed RFC3339 stub; MG-4 sources a real clock
  from the transport boundary so scene-core stays ambient-time-free.

## Storage layout

A single redb db (`<SHAPE_AI_DATA_DIR>/shape.redb`) backs every canvas through one
shared `RedbAdapter` behind a `Mutex`. The actor persists under one storage lock per
apply and **namespaces every `Record` id by `canvasId`** so ids stay globally unique:

- **Per-object record** — `Record { id: "{canvasId}:object:{objId}", kind: "object" }`.
  Written through on every op that touches the object; carries a `RegionKey` (world-space
  AABB) for spatial queries.
- **Canvas-meta record** — `Record { id: "{canvasId}:canvas", kind: "canvas" }`.
  Holds scene-level fields that belong to no single object (`sceneVersion`, `selection`,
  `tags`, `updatedAt`).
- **Journal entry** — `Record { id: "{canvasId}:journal:{seq}", kind: "journal" }`,
  one durable `OpEnvelope` per applied op (for crash recovery of the in-flight tail).
- **Canvas index** — `Record { id: "canvas-index" }` (no `canvasId` prefix),
  the ordered list of `CanvasSummary` entries (all canvases that survive restart).

On `spawn` the actor replays the journal tail on top of the last written object state
to recover the in-flight tail after a crash.
