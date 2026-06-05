# Phase P5: MCP Companion Activity Tracker

> Reviewer-facing phase document. Design-only: no production code is written, moved, or scaffolded.
> Sources: `docs/ai-companion-canvas-task-breakdown.md` (P5 intro) and task fragments `tasks/T5.1.md`, `tasks/T5.2.md`, `tasks/T5.3.md`, `tasks/T5.4.md`. Ground truth read in `src/server/mcp.ts` (10 registered tools + `jsonResponse`, stdio `main`) and `src/server/index.ts` (`app.all("/mcp")` Streamable HTTP transport).

## Goal / Why now

Make MCP clients visible as **agent companions** that show what they read, write, and produce on the canvas. This is the AI-native differentiator: it turns headless MCP tool calls into observable canvas activity.

The Locked Decision is binding here — *the MCP companion tracker is observability, not decoration*. Every behavior in this phase must answer "who is this client, where is it acting, and what did it just do?" without inventing permanent on-canvas clutter, without touching the canonical `Scene` schema, and without moving canvas-performance work into the shell.

## Decisions at a glance

| Concern | Decision | Pointer |
| --- | --- | --- |
| **Client identity & dock** | Each connected MCP client gets an **ephemeral** `McpClientIdentity` (server-minted `clientId`, `label`/`color`/`iconRef` seeded from the SDK `Implementation`) rendered as a dock chip with an idle/active/error/disconnected/muted state machine. The active "jumping" cue is a bounded, low-amplitude dock-chip CSS animation off the GPU frame budget, honoring `prefers-reduced-motion` + `muted`. Identity lives in an in-memory registry, never in `Scene`/SQLite. | `tasks/T5.1.md` |
| **Tool-call → canvas-target mapping** | Every tool call resolves to one ephemeral `CanvasTarget` (`canvas`/`group`/`node`/`edge`/`selection`/`viewport`/`artifact`). Write tools inherit `targetIds` from the already-built T2.5 `OperationEnvelope`; read tools resolve from input args. Geometry reuses the existing Rust core `selection_world_rect`; the `actor_marker` rides the existing `CoreOverlayRequest` ephemeral overlay path. Mapping is op-kind driven, so future template ops need zero new code. | `tasks/T5.2.md` |
| **Spectator / follow mode** | A single-followee, shell-only `FollowState` (`off`/`pinned`/`paused`) drives the camera **only** through the existing `focusBounds`/`setCamera` handle. Click-to-follow, pinned follow, pause/resume, jump-to-current, recent trail, and handoff are all ephemeral. A "never steal control" guard hard-interlocks follow to the engine's existing `gesture{active}` signal — any user grab demotes `pinned → paused`, and follow emits zero camera commands while a gesture is live. | `tasks/T5.3.md` |
| **Operation trace & write preview** | The trace is **derived**, not stored: writes/comments/exports/proposals are projected from the T2.5 append-only `events` log; reads/pre-commit errors live in a small in-memory ring. Surfaced as transient on-canvas pulses + a per-client activity-trail drawer (modeled on `RendererDiagnosticsDrawer`). Risky writes (`delete-*`, wide multi-target) stage as T2.5 proposals and render a non-committing **ghost preview** via a `CoreOverlayStyle.state:"ghost"` variant; accept/reject route through `accept-proposal`/`reject-proposal`. | `tasks/T5.4.md` |

## Per-task deliverable summaries

### T5.1 MCP Client Identity And Dock (Wave C)

- **`McpClientIdentity` model.** Ephemeral, app-layer record seeded from the SDK `Implementation` (`name`/`title` → `label`, `icons[0].src` → `iconRef`) with a deterministic `color = hash(clientId)` (hashed on `clientId`, **not** `name`, so two windows of the same tool differ). Reuses T0.1's `actor_marker` fields; never added to `Scene`/`SceneSnapshot`/core.
- **Dock state machine.** idle / active / error / disconnected / muted, with a per-state visual + behavior table and an ordering rule (`active > error > idle > muted > disconnected`, then `lastActivityAt` desc).
- **Active motion rule.** Bounded, low-amplitude dock-chip bob (~1 Hz, few px), shell/CSS-rendered off the WebGPU frame budget, self-terminating with the in-flight tool call, degrading to a steady pulse on long calls; respects `prefers-reduced-motion` + `muted`.
- **Activity hooks.** Dock state changes are driven by the tool-call lifecycle at the `jsonResponse` choke point in `mcp.ts` (connect → idle, call start → active, success → idle, throw → error, transport close → disconnected, user toggle → muted).
- **Server change required.** A stable per-connection `clientId` over the stateless `/mcp` transport — see **Resolved decisions**.
- **No new canvas surface.** The dock reads MCP state over a `/api/mcp/clients` read path and commands the canvas only through the existing `focusBounds`/`setCamera` handle; it does not ride the `EngineEvent` stream.

### T5.2 Tool Call To Canvas Target Mapping (Wave G)

- **One `CanvasTarget` type, four resolvers.** `canvas` / `group` / `node` / `edge` / `selection` / `viewport` / `artifact`. The first four are exactly `SceneSelection`; the rest are the super-selection cases T5.1's `lastTarget` anticipated. Recommends widening `lastTarget` to `CanvasTarget | null` as the one shared shape.
- **Targets are not re-parsed.** Write tools consume the T2.5 `OperationEnvelope.targetIds` + `sourceToolCall.tool`; read tools (`query_scene`/`list_groups`/`get_group`) resolve from input args.
- **Per-tool mapping table** for all 10 registered tools, each with a real arg/op source, a resolved `CanvasTarget`, and a marker behavior (read sweep vs write flash vs produce pulse).
- **Future template ops** inherit targets automatically through the shared `patch_scene` funnel — mapping is op-kind driven, not tool-name driven, so no per-template code.
- **Fallback** for three no-target cases: registry/non-spatial write → `canvas`; whole-scene read → scene-fit viewport; empty/failed resolution → keep prior target (never jump to origin, never flicker).
- **Geometry reuse.** `CanvasTarget → WorldRect` via the existing core `selection_world_rect`/union; the `actor_marker` is emitted as an ephemeral `CoreOverlayRequest` overlay, the same path the text-edit overlay uses. T5.2 never auto-pans on its own.

### T5.3 Spectator And Follow Mode (Wave E)

- **One ephemeral controller.** Shell-only `FollowState` (`off`/`pinned`/`paused`) with a single followee; adds nothing to `Scene`/`SceneSnapshot`/`EngineEvent`/core.
- **Six behaviors.** Click-to-follow (one framing jump on entry), pinned follow (per-target re-frame), pause/resume (freeze camera, keep followee + trail), jump-to-current (one-shot frame of the live target), recent trail (bounded `TrailEntry[]` ring + optional faint on-canvas breadcrumb over the shared overlay path), and handoff (soft → `paused`, hard → `off`, both leaving the camera in place).
- **Animation lives in the shell.** Core applies `FocusBounds`/`SetCamera` instantly; smoothing is a shell-side RAF tween emitting short `setCamera` batches, interruptible by design, with camera math/clamping still in core. Respects `prefers-reduced-motion`.
- **"Never steal control" guard.** Hard-interlocked to the engine's existing `gesture{active}` signal — while a gesture is live the controller emits zero camera commands and cancels any tween; a user grab during `pinned` demotes to `paused`; the controller tags its own `setCamera` calls so they are not misread as a grab.

### T5.4 Operation Trace And Write Preview (Wave E)

- **Trace is derived, not a new store.** Writes/comments/exports/proposals are a filtered read of the T2.5 `events` log; reads and pre-commit errors live in a small per-client in-memory ring (reads are not document mutations, so they are never logged).
- **`TraceEvent` projection** covering all eight named categories (reading, writing, commenting, exporting, proposal creation/acceptance/rejection, errors), each mapped to a real source symbol, a `CanvasTarget`, a transient on-canvas cue, and an activity-trail line.
- **Ghost write preview.** Risky writes (`delete-*`, wide multi-target `patch_scene`, agent-flagged) stage as T2.5 proposals; the ghost is the diff between the current scene and a preview snapshot built by the pure `applyRenderPatchToShapeScene`, rendered via a new `CoreOverlayStyle.state:"ghost"` variant. No commit, no `sceneVersion` bump, until accept.
- **Bounded visibility, complete history.** Transient on-canvas pulses (self-terminating, off the document layer, suppressed for `muted`) keep the canvas clean; a bounded, recent-first, per-client activity-trail **drawer** keeps history fully inspectable.

## Phase verify-or-evaluate audit

The P5 intro lists three "Verify or evaluate" bullets. Each is restated below with a MET / PARTIAL / GAP judgement against the four task designs.

| Phase verify bullet | Status | Basis |
| --- | --- | --- |
| Each connected MCP client has a visible identity. | **MET** (conditional on the resolved T5.1 server change) | T5.1 §1 derives a per-client `label`/`color`/`iconRef` from the SDK `Implementation` and renders it as a dock chip. Distinguishing **two concurrent identical clients** depends on a stable per-connection `clientId`, which the stateless `/mcp` transport could not provide — resolved below by making the transport stateful. With that change, identity is fully stable; the dock projection and state machine are unblocked either way. |
| Active clients move to the real target location for reads/writes. | **MET** | T5.2 resolves every tool call to a concrete `CanvasTarget` and positions the `actor_marker` via the existing core `selection_world_rect` + overlay path; the per-tool table maps all 10 tools (reads → viewport/group sweep, writes → object/union flash, export → artifact preview). T5.4 §2 differentiates the cue per verb. No destination is hand-waved; each has a real geometry source. |
| Users can click an icon to inspect or follow the agent's current work. | **MET** | T5.3 makes the dock chip clickable to enter `pinned` follow through the existing `focusBounds`/`setCamera` handle, with jump-to-current and a recent trail; T5.4's activity-trail drawer answers "what did this agent just read or change?" with click-to-jump per entry. The follow "never steal control" guard (T5.3 §8) is MET against its own verify bullet. |

No bullet is PARTIAL or GAP once the T5.1 transport decision (below) is taken. The single open dependency was the identity-uniqueness gap; it is resolved in the next section.

## Resolved decisions

### T5.1 — make the `/mcp` HTTP transport STATEFUL (recommended, chosen)

**Problem.** `src/server/index.ts` runs `app.all("/mcp")` with a fresh `createSceneMcpServer()` + `StreamableHTTPServerTransport({ sessionIdGenerator: undefined })` **per HTTP request** (stateless). In stateless mode `RequestHandlerExtra.sessionId` is `undefined`, and `server.getClientVersion()` returns the same `Implementation` for two concurrent identical clients (e.g. two Cursor windows). The SDK therefore gives **no stable per-connection identity** for the HTTP path, which the dock identity model requires. (stdio is fine: one process = one client.)

**Decision.** Turn `/mcp` **stateful**:

- Pass `sessionIdGenerator: () => randomUUID()` to `StreamableHTTPServerTransport`.
- Keep an in-memory `Map<sessionId, McpClientIdentity>` registry alongside the per-session server instance.
- `RequestHandlerExtra.sessionId` is then populated and equals the dock `clientId`.

**Why this one.** It is the SDK-sanctioned path (`streamableHttp.d.ts` stateful mode) and the **smallest correct fix**. The prior stateless `StreamableHTTPServerTransport({ sessionIdGenerator: undefined })` could not disambiguate two concurrent identical clients; the only added cost is honoring the MCP session header, which compliant clients already do. The registry stays **ephemeral app/server state** (in-memory `Map`, lost on restart) exposed to the shell over `/api/mcp/clients`; it never touches `Scene`/SQLite, preserving the T1.4 boundary.

**Rejected alternative (T5.1 §4 option 2).** Keep stateless and derive a soft `clientId` from `Implementation` + a per-stream nonce. Weaker: two identical concurrent clients remain indistinguishable. Not chosen.

**Implementation note.** The current `reply.hijack()` + per-request `createSceneMcpServer()` shape must keep one server per **session** instead of per request, and the `disconnected` state must key off session expiry/heartbeat rather than individual request-stream close (Streamable HTTP closes the stream between calls, so naive "stream close = disconnected" would flap). Mechanics belong to the P5 build; the boundary (registry in-memory ephemeral, server still owns persistence) is fixed here.

## Review gate

**None.** P5 has no `human-decision` review gate in the breakdown. The standing acceptance bar is the Locked Decision: the companion tracker **must be observability, not decoration**. Every surface in this phase is held to that bar — transient pulses self-terminate, the marker moves rather than accumulates, ghosts exist only while a proposal is pending, and history lives in an off-canvas drawer backed by the operation log. The D2 direction review later re-checks that the tracker "still provides useful agent observability instead of only decorative animation."

## Stop-conditions encountered

| Task | Stop-or-ask condition | Status | Resolution |
| --- | --- | --- | --- |
| T5.1 | MCP transport cannot provide stable client/session identity. | **TRIGGERED (HTTP path), now resolved** | stdio is fine; the stateless HTTP transport could not disambiguate concurrent identical clients. Resolved by the stateful-transport decision above. Everything else in T5.1 was unblocked regardless. |
| T5.2 | Tool payloads lack enough target information to visualize intent. | **CLEAR** | T2.5 makes `targetIds` a required envelope field with per-kind derivation; read tools carry their target in input args; the only payload-poor calls (`query_scene` no-filter, `list_groups`, `create_tag`) are genuinely whole-scene/non-spatial and have a defined fallback. |
| T5.3 | Spectator mode conflicts with local direct manipulation. | **CLEAR** | Follow is purely ephemeral and shares the same core `apply_input_event` batch loop as user input; the §8 guard hard-interlocks follow to `gesture{active}` and demotes `pinned → paused` on any user grab, so direct manipulation structurally preempts follow. |
| T5.4 | Trace events become permanent visual clutter instead of inspectable history. | **CLEAR** | On-canvas cues are transient/self-terminating and off the document layer; ghosts are bounded by pending-proposal count; durable history is the `events` log surfaced as an off-canvas activity-trail drawer. No trace event becomes a permanent canvas mark. |
