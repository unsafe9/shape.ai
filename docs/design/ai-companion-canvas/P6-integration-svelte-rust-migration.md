# Phase P6: Product Integration, Svelte Migration, And Rust Boundary Plan

> Reviewer-facing phase rollup for P6 of the AI Companion Canvas plan
> (`docs/ai-companion-canvas-task-breakdown.md:774`). Design-only: no production
> code is written, moved, or scaffolded in this phase.
> Tasks: T6.1, T6.2, T6.3, T6.4, T6.5 (all authored). Full detail lives in
> `tasks/<ID>.md`.

## Goal / Why now

Decide how the universal canvas, templates, companion activity tracker, Svelte
web shell, and Rust-owned canvas boundary reach production **without disrupting
the canvas-replacement work already in flight**. The repo already has a promoted
Rust WebGPU renderer (`870e158`) and a React shell with drag decoupled from React
sync (`5d3dc31`). P6 exists to keep those two tracks from colliding: it separates
the **product/framework shell migration** (React→Svelte) from the
**canvas/performance migration** (Rust-owned boundary) and from the **canonical
model migration** (evolve-in-place schema), sequences them behind explicit
acceptance gates, and ends with a single direction-review call before broad
implementation fans out.

The throughline every task converges on: this is **evolution, not rebuild**. The
canonical model (`Scene`/`SceneSnapshot`), the op vocabulary (`RenderScenePatch`),
the collaboration-ready log (`events` table), and the GPU renderer
(`ShapeWebGpuRenderer`) already live in `src/` and already sit on the right side
of the primitive/business line. P6 demotes the decision-graph specifics off those
seams; it does not replace them.

## Decisions at a glance

The key migration contracts, gates, and sequencing rules across P6, deduped, with
pointers to the owning task for full detail.

| Decision | Summary | Detail |
| --- | --- | --- |
| **Current-model compatibility: evolve in place** | Evolve the existing `Scene` schema (additive `meta` + `proposals` + first use of the existing `events` log), **not** a parallel primitive layer and **not** a new canonical model. The demote/generalize work is a backward-compatible JSON reshape — no `ALTER TABLE` — because decision-graph identity already lives only in two Zod enums + JSON-blob business fields, never in geometry, tables, or the render/core contract. | `tasks/T6.1.md` §1 |
| **SQLite migration is gated, idempotent, recoverable** | Op metadata/actor/base-revision ride the T2.5 `OperationEnvelope` into the existing four-column `events` table; no actor table is added; selection becomes authoritative-ephemeral (`selection_json` stops bumping `scene_version`). The reshape reuses the repo's own `scene_layout_version`/`repairSceneLayoutIfNeeded` precedent, takes a one-time `*.pre-meta.bak` file backup, and tolerates both blob shapes during a transition window. | `tasks/T6.1.md` §2, §5 |
| **Old Shape/project + structured-ADR demoted, never revived** | Project→ordinary `frame`; ADR nodes/edges→`shape`/`edge` + `meta.semanticType`; ADR business fields→`meta.*`; ADR exports→export presets reading primitives; the de-facto `seedGroupScene` ADR graph→the first registered `TemplateContract`. No project/ADR object family survives in the primitive/write path; `graph.ts` helpers kept only as export/migration shims. | `tasks/T6.1.md` §3, §4 |
| **React→Svelte: clean cut, in place, wrapping the narrow bridge** | Replace the React shell with a Svelte shell that wraps the already-narrow `RendererCanvasHostHandle` + `EngineEvent` bridge **verbatim**. Verified scope: exactly 10 `.tsx` files import `react`/`lucide-react`; `renderer/*`, `lib/*`, `shared/*`, `server/*`, and all of `tests/*` import zero React. So this is a shell rewrite + dependency removal, not an engine rewrite. No directory move this pass (deferred to T6.4). | `tasks/T6.2.md` §1–§3 |
| **Svelte↔canvas bridge contract** | Extract `RendererCanvasHost.tsx` into a framework-neutral `lib/canvasHost.ts` exposing the existing handle commands (load scene / apply op batch / input batch / overlay requests / diagnostics snapshots / event callbacks) + `EngineEvent`. Svelte owns only the three DOM nodes + store wiring; no engine method, event variant, or batching path is added/removed/rewritten. | `tasks/T6.2.md` §4 |
| **Rust performance boundary is mostly ratification** | Of 12 canvas-adjacent responsibilities, 9 are already Rust/core-owned (camera math, hit, selection/hull geometry, membership visuals, LOD seed, culling, text layout) and 2 sit correctly on the adapter seam (input/patch batching, diagnostics). The only behavioral moves: demote/delete the `scene.ts` JS parity layer, fix the camera two-way write-back to a one-way echo, relocate `App.tsx` canvas clipboard/keyboard to the adapter, and one `#[cfg]` removal to promote core types into the default build. | `tasks/T6.3.md` §1–§2 |
| **Boundary enforced by framework-neutral contract tests** | Tests drive `ShapeCanvasEngine` directly (no React/Svelte harness) and assert the shell calls narrow `RustWebGpuRenderer` APIs rather than deriving canvas state through reactivity: camera→`input_batch`, selection→`apply_patch_batch` without `load_scene`, production `src/client` imports only *types* from `scene.ts`, overlay geometry core-sourced. LOD threshold logic is forbidden from Svelte, reserved core-side. | `tasks/T6.3.md` §3, §4 |
| **Production sequence: three gated steps on one branch** | One feature branch off `main`: **S1** land the schema/SQLite/op-envelope migration (server/shared/core-enum, no `.tsx`), **S2** land the Rust boundary (GATE-R), **S3** swap React→Svelte and delete React (GATE-X). Order is load-bearing: S2 before S3 so the camera fix lands once in the React file and carries into the extraction; GATE-R before GATE-X so the boundary is certified pre-Svelte. `build`/`test`/`typecheck` green at every commit. | `tasks/T6.4.md` §2, §3 |
| **No directory move, no repo/package-manager change this pass** | The Rust core is already at its production path (`src/renderer/core`, crate `shape_canvas_core`); there is no `poc/` to promote and no top-level `apps/`/`canvas/`. The Svelte shell replaces React in place at `src/client/`. Every build/test script *name and shape* is preserved; only the Vite/Vitest plugin + entry/jsx internals change, and only at S3. | `tasks/T6.4.md` §1, §5 |
| **Final direction review against Decision D2** | Consolidate every phase's evidence so a reviewer makes one clean call — approve broad implementation, split the P6 migration into its own gated track, or redirect positioning. All five D2 Approve-if criteria satisfied; all three Redirect-if triggers evaluated and not tripped. | `tasks/T6.5.md` §1–§10 |

## Per-task deliverable summaries

### T6.1 Current Model Compatibility Review — AUTHORED (`tasks/T6.1.md`)

Verdict: **evolve the existing `Scene` schema in place.** A three-option
comparison (evolve / parallel primitive layer / new canonical model) is forced by
one fact — decision-graph identity lives only in two Zod enums and a handful of
JSON-blob business fields, never in the geometry, tables, or render/core contract
(`nodeToCard`/`edgeToRenderEdge` already consume `type`/`status` opaquely;
`excludedBusinessFields()` already lists the demoted fields). So "make the model
universal" is a **demotion into a `meta` bag**, not a structural rebuild. The
SQLite migration note maps operation metadata/base-revisions/actor records into
the T2.5 `OperationEnvelope` written to the **already-existing-but-unused `events`
table** (no actor table; no `ALTER TABLE`), and pins the document-vs-ephemeral
boundary — including the one behavioral correction that selection stops bumping
`scene_version`. The demotion note removes Shape/project and structured-ADR as
privileged primitives (→ `frame`/`shape`/`edge` + `meta` + export presets). A
per-flow table proves all 10 MCP tools + mirrored REST routes keep their wire
contract. The reshape is gated/idempotent on a `metadata` version string (the
`scene_layout_version` precedent), file-backed before its first write, and
tolerant of both blob shapes — so the Stop-or-ask ("break local SQLite without a
recovery path") stays clear.

### T6.2 React To Svelte Web Shell Migration — AUTHORED (`tasks/T6.2.md`)

Replace the React shell with a Svelte shell that **wraps the already-narrow
bridge unchanged**, deleting `react`/`react-dom`/`@vitejs/plugin-react`/
`lucide-react`/`@types/react*`, the 10 `.tsx` files, and the JSX build
assumptions, while the canvas adapter (`renderer/*`), `lib/*`, `shared/*`,
server, and the entire (already node-environment, React-free) `tests/` suite
carry over without touching canvas behavior. Every named UI surface (app chrome,
panels, inspector, template picker [P4 slot], export drawer, MCP dock [P5 slot],
spectator controls [P5 slot], active-text-overlay host, diagnostics host,
API/MCP orchestration) maps 1:1 to a `.svelte` file; shell state partitions into
`lib/stores/` along the T6.1 document/ephemeral boundary, and the debounced
patch-save extracts verbatim into a framework-neutral `lib/patchSaver.ts`. The
bridge contract extracts `RendererCanvasHost.tsx` into `lib/canvasHost.ts`
exposing the existing handle commands + `EngineEvent` verbatim. No
React-rendering tests exist (verified), so the test plan is additive (bridge
contract + orchestration unit tests; optional per-file jsdom-pragma smoke tests).
Both Stop-or-asks (React compat layer / dual shell; Svelte rewriting the engine)
are clear.

### T6.3 Rust-Owned Canvas Performance Boundary — AUTHORED (`tasks/T6.3.md`)

Audit every canvas-adjacent responsibility, prove the performance-critical set
**already lives in Rust/core** (`ShapeWebGpuRenderer`), and name the small TS
residue to demote *before* the Svelte shell can inherit it as component state. Of
12 responsibilities, 9 are Rust/core-owned (ratify against `screen_to_world`/
`hit_scene_at_screen`/`selection_world_rect`/`build_draw_list`/`text.rs`/
`SceneStateTokens.compact`) and input/patch batching + diagnostics sit correctly
on the `engine.ts` adapter seam. The only behavioral moves: (a) demote/delete the
`scene.ts` JS parity layer (`applyScenePatch` shadow-apply, `rectsIntersect`/
`screenToWorld`/`worldToScreen` geometry dups, `validateScenePatch`,
`truncateText`) leaving only type re-exports so core is the single mutation
authority; (b) remove the `useEffect([camera])` write-back so camera is
core-authoritative and the shell holds a one-way echo; (c) relocate `App.tsx`
canvas clipboard/keyboard to adapter ops (business markdown stays shell); (d)
one `#[cfg(feature="wgpu-probe")]` removal to promote core input/op types into
the default `wasm-pack` build. Boundary contract tests (framework-neutral) lock
it; all eight named flows (pan/zoom/select/drag/group/label/follow/trace) are
walked as shell-event → adapter → core → echo routes. Both Stop-or-asks (Svelte
as performance source of truth; business rules pulled into the renderer) are
clear.

### T6.4 Production Migration Sequence — AUTHORED (`tasks/T6.4.md`)

Sequence the cutover as **three independently-runnable, gate-bounded steps on one
feature branch** off `main`, with `build`/`test`/`typecheck` green at every
commit. Two ground-truth facts shrink the sequence: there is no POC/engine
promotion and no directory move (the Rust core is already at `src/renderer/core`,
no `poc/`, no top-level packages), and the three migration contents are nearly
file-independent, ordered by blast radius. Steps: **S1** schema/SQLite/op-envelope
(server/shared/core-enum, no `.tsx`) — first because it is framework-agnostic and
the system-of-record change must be proven against the live React shell; **S2**
Rust boundary — second so the camera and JS-parity fixes land in the React files
once and carry into the S3 extraction, and the boundary tests certify the
pre-Svelte ownership (**GATE-R** at S2 exit); **S3** React→Svelte swap + React
deletion — last, highest-churn/lowest-logic, replacement built before deletion
(**GATE-X** at S3 exit). The two gates are ordered (R strictly before X) and
neither passes implicitly. Rollback posture is structural: each step is
independently revertible, the SQLite reshape is gated/idempotent/file-backed, and
the branch is the macro-rollback unit (nothing substantial on `main`). The
coordination plan preserves every script name/shape and changes only the
Vite/Vitest plugin + entry/jsx internals at S3 — and catches that
`vitest.config.ts` *also* loads `react()`, an extra removal point T6.2
under-counted. Stop-or-ask (repo/package-manager change beyond scope) is clear.

### T6.5 Final Direction Review — AUTHORED (`tasks/T6.5.md`)

The terminal synthesis: a single **review brief** covering nine named topics
(product scope, core architecture, primitive sufficiency, LOD evidence, template
coverage, MCP activity-tracker behavior, Svelte shell migration, Rust canvas
boundary, remaining deferrals) consolidated against **Decision D2**, so a
reviewer makes one clean call. It asserts nothing new; it reports the state
upstream tasks established, citing real symbols and owning tasks. All five D2
Approve-if criteria are satisfied (web/macOS/iOS-portable layering; six templates
as primitive recipes, contingent on D1 C1 image substrate; stable-identity LOD
ladder as design+harness; MCP observability from ephemeral + `events` data;
Svelte-shell + Rust-core split). All three Redirect-if triggers are evaluated and
not tripped. **Recommendation: approve broad implementation**, with two binding
gates the reviewer should attach — gate image-preview templates on D1 C1 (the one
open product-capability gap), and treat P3 LOD as the load-bearing performance
proof to validate early (design+harness, not yet measured). A **split** (run the
P6 migration as a separately-gated track while P2–P5 product work proceeds) is a
first-class, pre-designed option, not a fallback. Both Stop-or-asks
(direction shifts back to engineering-only) are clear.

## Phase verify-or-evaluate audit

Each P6 phase "Verify or evaluate" bullet, restated, with MET / PARTIAL / GAP.

| Phase verify bullet | Status | Basis |
| --- | --- | --- |
| The plan explains how current `src/`, existing renderer paths, any retained `poc/` evidence, and the chosen target layout fit together. | **MET** | T6.1 §1–§4 anchors the current `src/server`/`src/shared` + render/core contract and the evolve-in-place verdict; T6.4 §1 records the two ground-truth facts — the Rust core is already at its production path (`src/renderer/core`), the adapter at `src/client/renderer`, **there is no `poc/`** (it is empty/absent), and the "chosen target layout" *is* the current `src/` layout with **no directory move** this pass. T6.5 §0 ties the existing seams (primitive/business, render contract, op vocabulary, `events` log, GPU renderer) to what the plan does to each. |
| The plan explains how React is removed and Svelte becomes the web shell. | **MET** | T6.2 §1–§3 names every Svelte-owned surface with its target `.svelte` file and lists the complete removed React surface (10 `.tsx` files, `react`/`react-dom`/`@vitejs/plugin-react`/`lucide-react`/`@types/react*`, the `react()` plugin, `"jsx": "react-jsx"`, the `index.html` entry — verified to be the full set). T6.4 §2 S3 + §3 GATE-X sequence the removal and gate it; T6.4 §5 catches the second `react()` load in `vitest.config.ts`. |
| The plan explains which canvas-adjacent responsibilities move to Rust/core rather than Svelte. | **MET** | T6.3 §1 audits all 12 responsibilities to a real owner (9 Rust/core, 2 adapter, 1 shell leak to relocate); §2 states the four behavioral moves (demote `scene.ts` JS parity, fix camera write-back, relocate clipboard/keyboard, promote feature-gated core types) with TS/Svelte retaining only product semantics + API + DOM-overlay mounting + shell event routing; §3 boundary tests + §4 flow routes enforce that Svelte never authors geometry/visibility/hit/LOD/text/caches. |
| No current production behavior is lost without an acceptance gate. | **MET** | T6.4 §2 makes `build && test && typecheck` green a step-exit criterion for every step and §4 guarantees it structurally (independently revertible steps, replacement-before-deletion in S3, gated/idempotent/file-backed SQLite reshape, node-env React-free suite). The two mandatory gates (§3: GATE-R Rust boundary at S2 exit, GATE-X React removal at S3 exit) are ordered and never implicit; T6.1 §4 proves every MCP/API/export/comment flow round-trips with no break, and §5 makes the SQLite migration non-destructive and recoverable. No behavior is removed except behind a gate or a no-break compatibility path. |

## Review gate

`human-decision`: two binding acceptance gates inside the single migration branch,
plus the overarching invariant that **no current production behavior is lost
without passing one of them.** Both gates are checkpoints *within*
`feature/p6-production-migration`, not separate branches, and neither passes
implicitly (T6.4 §3).

### GATE-R — Rust performance boundary (S2 exit, before the Svelte swap begins)

All required:

1. Boundary contract tests green: camera moves route through `input_batch`;
   `syncSelection` routes through `apply_patch_batch` **without** `load_scene`;
   production `src/client` imports only *types* (not `applyScenePatch`/
   `screenToWorld`/`rectsIntersect`/`validateScenePatch`) from `scene.ts`;
   overlay geometry comes from `overlay_request`.
2. `CanvasInputEvent`/op types build in the **default** (non-`wgpu-probe`)
   `wasm-pack` output.
3. Camera is a one-way echo — no `[camera]`→`setCamera` write-back.
4. `renderer:rust:test` + `renderer:test` green.

Owner of criteria: T6.3. **Purpose:** certify the canvas-performance ownership on
the *pre-Svelte* shell, so the framework swap cannot silently move canvas state
into Svelte — the already-green, framework-neutral tests carry into S3 unchanged.

### GATE-X — React removal (S3 exit, before branch acceptance/merge)

All required:

1. `react`, `react-dom`, `@vitejs/plugin-react`, `lucide-react`, `@types/react`,
   `@types/react-dom` absent from `package.json`; no source file imports
   `react`/`react-dom`/`lucide-react` (grep clean — today exactly 10 files do).
2. No dual shell: no co-existing `App.tsx` *and* `App.svelte` in the production
   path; `index.html` entry is `/src/client/main.ts`; `react()` removed from
   **both** `vite.config.ts` and `vitest.config.ts`; no `"jsx": "react-jsx"`.
3. `npm run build` (Svelte bundle + `copyRendererWasm` + server `tsc`),
   `npm test` (node-env, framework-neutral), `npm run typecheck`, and the added
   `svelte-check` all green.
4. No React compatibility layer / fallback retained (Explicit Non-Goal).

Owner of criteria: T6.2. **Purpose:** certify the framework cutover on top of an
already-locked Rust boundary.

### The gate-ordering invariant

GATE-R must pass **strictly before** GATE-X. The Rust boundary is certified on the
pre-Svelte shell; the React removal is certified on top of a locked boundary. The
boundary tests must outlive S3 unchanged (framework-neutral, driving
`ShapeCanvasEngine` directly) — they are the proof the swap did not move canvas
state into Svelte and must not be "ported" to a Svelte harness and weakened.

### What the gate also confirms

- **No current production behavior lost without a gate.** T6.1 §4 gives a no-break
  compatibility path for every MCP tool + REST route; the one behavioral change
  (selection no longer bumps `scene_version`) is server-internal and preserves
  the client round-trip; the SQLite reshape is gated/idempotent/file-backed.
- **No directory move and no repo/package-manager change this pass** (T6.4 §1, §5)
  — the Svelte shell replaces React in place at `src/client/`; a dedicated
  web-shell relocation, if ever wanted, is a separate churn-justified task.

## Stop-conditions encountered

- **T6.1:** "Migration would break existing local SQLite data without a recovery
  path." — **CLEAR (not triggered).** The reshape is strictly additive at the
  table level (one optional `meta`, one optional `proposals` array, first use of
  the existing `events` table) plus a JSON reshape within `node_json`/`edge_json`:
  no `ALTER TABLE`, no `DROP`, no index/geometry-column change. Gated/idempotent
  on a `metadata` version string, file-backed before the first write, with legacy
  parse retained and both blob shapes tolerated. Failure mode is "fields not yet
  relocated," not "data lost" (`tasks/T6.1.md` §5).
- **T6.2:** "A React compatibility layer or dual production shell becomes
  necessary" / "Svelte migration starts rewriting the canvas engine instead of
  wrapping the canvas adapter." — **both CLEAR.** Clean cut: React removed
  outright with no fallback, 1:1 file replacement behind the already
  framework-neutral bridge; `renderer/*` is untouched and the only canvas-adjacent
  code that *moves* is the React glue → `lib/canvasHost.ts`, with zero
  engine-method/event/batching changes (`tasks/T6.2.md` §Stop-or-ask).
- **T6.3:** "A canvas feature cannot be implemented without making Svelte state
  the performance-critical source of truth" / "Moving a responsibility to Rust
  would pull product business rules into the renderer." — **both CLEAR.** Every
  canvas-state need maps to a core/adapter owner; the two TS-resident risks (the
  `scene.ts` JS mirror and the camera write-back) are removed, not promoted to
  Svelte; the business half of clipboard stays in the shell while only the
  canvas-scoped half moves to the adapter (`tasks/T6.3.md` §Stop-or-ask).
- **T6.4:** "The migration needs a repo/package manager change beyond the approved
  scope." — **CLEAR.** Stays within the single-package npm + Vite 7 + `tsc`
  project-reference setup; no directory move, no monorepo/workspaces, no bundler
  replacement — only in-place add/remove of Svelte-for-React packages in the same
  `package.json` (`tasks/T6.4.md` §Stop-or-ask).
- **T6.5:** "The product direction shifts back to a narrow engineering-only
  architecture canvas." — **CLEAR.** The consolidated evidence points the other
  way (decision-graph identity demoted to `meta`/templates; six universal
  templates; MCP observability as a first-class pillar). A reviewer *may* still
  redirect, but that would be a deliberate call against this evidence, not a
  direction the design drifts toward (`tasks/T6.5.md` §Stop-or-ask).
- **Phase-level:** all five P6 task fragments are authored (unlike P0's T0.2/T0.3
  gaps), so the audit above rests on reviewed task designs, not reconstruction.
  One known open *product-capability* gap is carried, not a process gap: **D1 C1
  (image/texture-quad render substrate)** must land before any image-preview
  template, else templates ship the documented captioned-box placeholder; and
  **P3 LOD is design + harness, not yet measured** — both surfaced by T6.5 §10 as
  conditions on the "approve" call, not P6 blockers.
