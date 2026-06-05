# Phase P3: Good LOD And Infinite Object Performance

> Reviewer-facing phase rollup for P3 of the AI Companion Canvas plan
> (`docs/ai-companion-canvas-task-breakdown.md:465`). Design-only: no production
> code is written, moved, or scaffolded in this phase.
> Tasks: T3.1, T3.2, T3.3, T3.4. Full detail lives in `tasks/<ID>.md`.

## Goal / Why now

Render and navigate an effectively unbounded canvas **without changing object
identity during zoom**. The product only works as a visual filesystem if large
amounts of heterogeneous data stay spatially explorable — so P3 fixes the LOD
*policy* (what degrades and what stays stable), the cache/streaming *plan* (how
unbounded data enters the viewport), the heterogeneous *fixtures* (what real load
looks like), and the interaction-under-load *evaluation* (proof that direct
manipulation survives the load) — all as design over the existing wgpu core, not
a new renderer.

## Decisions at a glance

The key contracts across P3, deduped, with pointers to the owning task. Every
decision is additive over named existing symbols; no schema, render-contract, or
parallel-renderer change is introduced anywhere in the phase.

| Decision | Summary | Detail |
| --- | --- | --- |
| **LOD policy = paint-only tiers with stable identity** | A fixed five-tier ladder (`Full`/`Compact`/`ShapeOnly`/`Density`/`Minimap`) selected per-object-per-frame by `apparent_px = max(bounds.w, bounds.h) * camera.zoom`. Tiers only add/remove *paint* of the same object; id, position, `bounds`, selection geometry, and hit-test identity are binding invariants at every tier. `Density`/`Minimap` are bounded overview-only paint, never a dot↔card swap or clustering super-object. | `tasks/T3.1.md` §1–§4 |
| **LOD lives in core, geometry-only** | Tier resolution is a cheap extension of the existing `build_draw_list` cull loop (`webgpu.rs:1613`); no second scene pass, no business field (`status`/`node_type`/`confidence`) participates, no TS/Svelte tier compute. Hysteresis (±10% dead-band, per-slot `previous` tier) prevents flicker on pan/zoom. | `tasks/T3.1.md` §3 |
| **Group hull LOD = outside-in, containment never degrades** | Hulls simplify their *paint* (outline → tint) at distance while `groupId`/`parentGroupId` are document fields LOD never reads or mutates. Nested hulls collapse innermost-first so the top-level grouping survives longest. Freeform hull shape (T2.4) is a geometry-source-agnostic input, not a fork. | `tasks/T3.1.md` §5 |
| **Cache/streaming keyed to LOD tier** | A core-owned uniform-grid `SpatialGrid` over `bounds` drives a viewport+zoom-aware query at the storage seam; nine cache structures (spatial index, viewport query, tier-keyed residency levels, text/glyph/edge caches, density texture, prefetch ring, memory budget) layer additively on `readScene`/`load_scene`/`build_draw_list`/`VertexRanges`/`TextLayoutCache`. Residency level = the cell's coarsest `LodTier`, so detail demand and residency demand are one axis. | `tasks/T3.2.md` §1–§10 |
| **Stream, never mount-all / load-all** | `SceneQuery` gains optional `viewport`/`zoom`/`prefetchPadding` (omitted = today's whole-scene, back-compat); SQL bounds filter rides the existing `idx_nodes_bounds`; zoom-trims text payload for sub-`Compact` objects (payload subtraction, never identity change); `merge_scene` upserts streamed tiles into the resident snapshot. No object is ever a DOM node — only a GPU slot or density-tint contribution under one `<canvas>`. Named budgets + LRU eviction reach a steady state. | `tasks/T3.2.md` §2–§9 |
| **Heterogeneous benchmark fixtures** | Extend the deterministic seeded generator from one homogeneous decision-graph grid into a composable mixed-workspace generator (notes, shapes, edges, frames, todos, wiki cards, slide frames, architecture diagrams + comment/artifact/actor-marker sidecars). New profiles `mixed-1k`/`mixed-5k`/`mixed-workspace`; existing `createBenchmarkFixture()`/`createSmallFixture()` kept verbatim as the regression baseline. All families compose from the three existing render kinds + `defaultStyles`; comments/artifacts/markers ride a sidecar, not `SceneSnapshot`. | `tasks/T3.3.md` §2–§4 |
| **Interaction-under-load evaluation** | Add a per-interaction **latency** metric (`InteractionLatencyResult`: input→commit→paint p50/p95/max, `droppedFrames`, `tierCounts`) over the existing input-batch boundary — the missing axis next to frame-time. Script all seven flows (pan/zoom/select/drag/text-edit/follow/operation-trace) through the real engine seams against T3.3 fixtures under T3.2 streaming, each bound to a per-flow seed budget anchored to a 60fps frame. "Graceful degradation" = `tierCounts` may shift toward `Density`/`Minimap` while p95 stays in budget (degrade-don't-stall). | `tasks/T3.4.md` §1–§5 |
| **Follow-vs-input contention designed out** | Follow/spectator animation and user input share the **single** `sendInputBatch` boundary — serialized, not raced; the existing `inputGestureActive`/`deferredScene` guard already yields ephemeral camera work to active gestures. Priority rule: user input > follow animation > operation-trace marker updates. A future follow RAF writing camera state outside `sendInputBatch` is the named boundary that would re-introduce the race. | `tasks/T3.4.md` §4 |

## Per-task deliverable summaries

### T3.1 Good LOD Policy — AUTHORED (`tasks/T3.1.md`)

Defines the allowed LOD transitions as a fixed ladder of **visual-only** detail
tiers selected by a single core-owned function of apparent on-screen size, where
every tier preserves object id, position, `bounds`, selection identity, and
hit-test identity. The repo already has viewport *culling* and a `states.compact`
token (wired to label-presence, not zoom) but **no tiering pass** — T3.1 closes
that with: the five-tier ladder (`Full ≥220px` / `Compact 120–220` / `ShapeOnly
40–120` / `Density 8–40` / `Minimap <8`, half-open bands with hysteresis); a
per-primitive degradation mapping that reuses existing `SceneStyleToken` fields
(no new render model); five stability invariants tied to real symbols
(`vertex_ranges` id keys, `bounds`-derived `selection_world_rect`/`CoreHitResult`);
an outside-in group-hull degradation rule that keeps containment legible; and
per-tier counts added to `WebGpuFrameStats` so LOD is observable. A `Minimap`-tier
card is still hoverable, selectable, and an MCP target — the bright line between
degradation (removes paint) and replacement (removes/swaps identity). No semantic
replacement / clustering, no streaming design (T3.2), no freeform hull math (T2.4).

### T3.2 Renderer Cache And Streaming Plan — AUTHORED (`tasks/T3.2.md`)

Plans how an unbounded scene enters the viewport without mounting every object as
DOM or loading it at full detail. The renderer already *culls* a fully-resident
scene and *caches* text; it does not *stream*, has no spatial index/tile structure,
no edge/density cache, no prefetch, no eviction. T3.2 specifies all nine
spec-named concerns as additive structures keyed to T3.1's `LodTier`: a core-owned
uniform-grid `SpatialGrid` that `build_draw_list` queries instead of the full
vector; a viewport+zoom-aware `SceneQuery` extension (SQL bounds filter on the
existing `idx_nodes_bounds`, zoom-trimmed text payload, `merge_scene` partial
ingest) — the one part touching storage and the Stop-or-ask probe; tier-keyed
residency levels (`Hot`/`Warm`/`Cool`/`Cold`); LRU-bounded text-layout and glyph
caches (eviction over silent atlas overflow); a new dirty-invalidated edge-route
cache; a per-group density texture so far regions cost one textured quad; a
prefetch ring distinct from the render cull padding; and named per-cache memory
budgets with an eviction order that never drops visible `Hot` residency. Every
level transition is paint/residency only, never identity. No schema change, no
raster image tiles/pyramid, no realtime/CRDT, no benchmark run.

### T3.3 Benchmark Fixtures For Heterogeneous Work Objects — AUTHORED (`tasks/T3.3.md`)

Extends the deterministic fixture generator from a single homogeneous
decision-graph grid into a composable heterogeneous-workspace generator emitting
the full T2.1 primitive mix at the existing 1k+ scale and beyond. The harness
(seeded `lcg` PRNG, `runScriptedPanZoom`, `BenchmarkResult`) already exists; only
the fixture *content* is engineering-homogeneous. T3.3 adds named profiles
(`mixed-1k` / `mixed-5k` / `mixed-workspace`) and a per-family composition table
where every family (note, shape, edge, frame, todo, wiki, slide, architecture,
artifact-preview) is built from the three existing render kinds + `defaultStyles`
with varied geometry/text profiles — so a single camera zoom puts different
families in different T3.1 tiers and makes the per-tier counts non-degenerate. An
additive `createHeterogeneousFixture(profile)` returns a valid `SceneSnapshot`
plus a **sidecar** for comments/artifacts/actor-markers (which the render contract
deliberately excludes via `excludedBusinessFields()`), preserving the
document/ephemeral split even in fixtures. Anti-synthetic guardrails: family
distribution mirrors the real first template set, realistic geometry/text ranges
(CJK + Latin + no-space tokens, labeled/label-less edges), clustered placement for
realistic culling/prefetch locality, retained homogeneous baseline. Existing
exports unchanged; no renderer/schema/LOD change.

### T3.4 Interaction Under Load — AUTHORED (`tasks/T3.4.md`)

A concrete test/evaluation plan driving every direct-manipulation flow — pan,
zoom, select, drag, text-edit overlay, follow-agent, operation-trace — through the
existing input-batch/camera/overlay seams against T3.3 fixtures at T3.2 streaming
load. The harness measures *throughput* (frame-time percentiles) but never
*responsiveness of a discrete action*; T3.4 adds an additive
`InteractionLatencyResult` (input→commit→paint p50/p95/max latency, `droppedFrames`,
load context, `tierCounts`) timed by the harness around the unmodified engine
boundary — no core stat is required for v1. Each of the seven flows is scripted
over the real seams with a stated load stressor and pass signal, bound to a
per-flow seed latency budget anchored to a 60fps frame (pan ≤16.7ms, zoom ≤33ms on
a tier-crossing frame, select ≤50ms commit, drag ≤16.7ms/move, text-edit ≤50ms
mount). "Graceful degradation" is operationalized as `tierCounts` allowed to shift
toward `Density`/`Minimap` while p95 stays in budget. Follow and operation-trace
are tested at their **performance seam** (`focus-bounds` animation, the patch/op
stream), not the full P5 feature. The follow-vs-input contention contract turns the
existing gesture-deferral guard into a measured invariant. No new renderer/core
structure, no schema change, no follow/trace feature build.

## Phase verify-or-evaluate audit

Each P3 phase "Verify or evaluate" bullet, restated, with MET / PARTIAL / GAP.
Note the phase requires **performance evidence to include frame time, memory,
object count, text/cache stats, and interaction latency** — this phase supplies
the *observable hooks and the harness shape* for all five; the actual measured run
is downstream execution, not this design pass.

| Phase verify bullet | Status | Basis |
| --- | --- | --- |
| Large scenes keep **stable object identity during zoom**. | **MET** | `tasks/T3.1.md` §4 makes id/position/`bounds`/selection/hit-test stability binding invariants over real symbols: tier changes never touch slot identity (`vertex_ranges` id keys), `bounds`, `selection_world_rect`, or `CoreHitResult`, and compose with the existing dirty-range machinery without reassigning slots. `tasks/T3.2.md` §3 preserves the same invariant across residency-level transitions (`Hot→Cool` keeps the slot key; re-entry repopulates), and `tasks/T3.4.md` §2 scripts identity-stable selection of a tiny far object at `Density`/`Minimap` tier as a pass signal. |
| Far objects **degrade visually instead of becoming unrelated semantic replacements**. | **MET** | `tasks/T3.1.md` §1–§2 degrade by removing paint (text → badges → shadow → stroke → individual draw) while the object keeps the same id/`bounds`; §7 + strategy §3 bound `Density`/`Minimap` to overview-only paint with no dot↔card swap and no clustering super-object. `tasks/T3.2.md` §7 makes the density texture *paint, not an object model* (members keep live slots/hit-test), and the aggregate-first Stop-or-ask stays CLEAR across both tasks. The corollary "a `Minimap`-tier card is still selectable / hit-testable / MCP-targetable" is the bright line, enforced in fixtures (T3.3 §4 keeps markers/comments as sidecars) and proven under load (T3.4 §2 select-at-far-zoom). |
| Performance evidence includes **frame time, memory, object count, text/cache stats, and interaction latency**. | **PARTIAL** | The first four already flow through `BenchmarkResult`/`FrameStats` (`p95FrameMs`/`maxFrameMs`, `memoryBytes`, `totalCards`/`visibleCards`, the full Rust text/layout/raster/atlas cache counters). `tasks/T3.1.md` §6 adds per-tier counts to `WebGpuFrameStats`; `tasks/T3.2.md` §4–§9 add eviction/density/edge-route counters so memory pressure and tier distribution are observable; `tasks/T3.3.md` §5 makes those counters non-degenerate under a real heterogeneous mix; `tasks/T3.4.md` §1/§5 supply the **fifth** axis — interaction latency — and bind all five into one `InteractionLatencyResult` per flow. **PARTIAL, not MET**, because each owning task is itself PARTIAL on this bullet by design: the design supplies the complete *observable surface and harness shape*, but the actual frame-time/memory/latency **measurement run** (and the calibration of the seed budgets/thresholds it would tune) is downstream execution, not this design pass. |

## Review gate

**None — this phase is evidence-gated, not decision-gated.** No P3 task carries a
`human-decision` review gate; the open choices (LOD band edges, grid cell size,
prefetch radius, per-cache memory budgets, per-flow latency budgets, fixture
object-count weights) are all **seed constants explicitly tunable by
measurement**, not contracts requiring human approval before fan-out. They are
calibrated against the T3.3 fixtures and T3.4 latency runs during execution, and
they live in ephemeral core/harness state touching no schema. The phase advances
on benchmark evidence (the five-axis metric set above), not on a gate.

## Stop-conditions encountered

- **T3.1:** "Product direction changes to aggregate-first graph analytics." —
  **CLEAR (not triggered).** Continuous per-object vector paint stays the default;
  aggregation appears only at `Density`/`Minimap` as bounded overview paint that
  never replaces or clusters objects, and every object stays individually
  selectable/hit-testable at all tiers. Would trigger only if overview tiers became
  primary and re-identified objects (`tasks/T3.1.md` §Stop-or-ask).
- **T3.2:** "SQLite/API query shape cannot support spatial paging and zoom-aware
  fetches." — **CLEAR (not triggered).** `nodes` already stores decomposed
  `x,y,width,height` columns with `idx_nodes_bounds`, `graph.ts` ships
  `boundsIntersect`/`pointInBounds`/`expandedBounds`, and `SceneQuery` /
  `/api/scene/render-snapshot` / `query_scene` already take a query object, so the
  added `viewport`/`zoom`/`prefetchPadding` are additive and back-compatible.
  Residual risks that would flip it: groups store `bounds_json` (not SQL-indexable —
  mitigated by group count ≪ node count); and sql.js synchronous paging is a T3.4
  measurement question, not a query-shape blocker (`tasks/T3.2.md` §Stop-or-ask).
- **T3.3:** "Benchmark data becomes too synthetic to reflect real workspaces." —
  **CLEAR (not triggered).** §6 guardrails: family distribution mirrors the actual
  first template set (todo board / wiki cluster / slide deck / architecture
  diagram), realistic geometry/text ranges, clustered placement for realistic
  locality, comment/artifact/marker density as first-class families, and a retained
  homogeneous baseline for comparable regression. Would trigger only if a profile
  abandoned workspace-shaped regions for pure stress noise (`tasks/T3.3.md`
  §Stop-or-ask).
- **T3.4:** "Follow/spectator animation competes with editor input responsiveness."
  — **CLEAR (not triggered).** Follow animation and user input share the single
  `sendInputBatch` boundary and are serialized; the existing
  `inputGestureActive`/`deferredScene`/`flushDeferredSceneSoon` guard yields
  ephemeral camera work to active gestures, and T3.4 turns that into a measured
  invariant (user-input p95 under follow == no-follow p95 ±tol; follow yields on
  gestures; follow never steals the active text-edit overlay). Would trigger only if
  a future follow RAF wrote camera state outside `sendInputBatch` — the named
  boundary the implementer must not cross (`tasks/T3.4.md` §Stop-or-ask).
- **Phase-level:** No blocking stop-condition encountered. All four task fragments
  are authored, and all four Stop-or-ask conditions are CLEAR — the phase is
  unblocked for execution, gated only on producing the five-axis performance
  evidence (and on calibrating the seed budgets/thresholds against it).
