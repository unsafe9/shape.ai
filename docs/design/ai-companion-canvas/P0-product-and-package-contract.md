# Phase P0: Product And Package Contract

> Reviewer-facing phase rollup for P0 of the AI Companion Canvas plan
> (`docs/ai-companion-canvas-task-breakdown.md:106`). Design-only: no production
> code is written, moved, or scaffolded in this phase.
> Tasks: T0.1, T0.2, T0.3. Full detail lives in `tasks/<ID>.md`.

## Goal / Why now

Lock the universal object **vocabulary** and the package/layout **direction**
before implementation fans out, so the canvas core, template model, MCP activity
tracker, and future native shells share one contract instead of pulling the repo
in five directions.

## Decisions at a glance

The key contracts, vocabulary, and migration rules across P0, deduped, with
pointers to the owning task for full detail.

| Decision | Summary | Detail |
| --- | --- | --- |
| **Primitive vs template seam** | The canvas draws/edits ~7 universal primitive kinds; every product surface (todo/wiki/ADR/server-arch/presentation) is a *recipe* over those primitives, never a private renderer/object model. | `tasks/T0.1.md` §1–2, §6 |
| **Primitive kinds** | `shape`, `text`, `edge`, `frame` (group), `image_artifact`, `comment_marker` (document) + `actor_marker` (ephemeral). Extends existing `RenderObjectKind = group\|card\|edge\|port\|text`. | `tasks/T0.1.md` §2 |
| **Evolve, don't replace** | Build on existing `Scene`/`SceneSnapshot`/`RenderScenePatch` contracts. ADD a `PrimitiveKind` dimension + generic `text`/`label` capability; DEMOTE decision-graph identity (`nodeType`/`edgeType`/`nodeStatus`/`confidence`/`evidenceRefs`/`childDecisionIds`/`rationale`) into template-owned `meta` + `styleKey`. KEEP geometry/grouping/z-order/tags/comments/artifacts/selection/style-tokens. | `tasks/T0.1.md` §1, §4–5 |
| **Document vs ephemeral split** | DOCUMENT (persisted, op-tracked): `shape`/`text`/`edge`/`frame`/`image_artifact`/`comment_marker` + tags + artifacts + saved selection. EPHEMERAL (never persisted): `actor_marker` presence, camera/viewport, hover, active tool, follow mode, read cursor, companion animation. `CameraState` already lives only render-side. | `tasks/T0.1.md` §3 |
| **No structural render change** | `RenderCard.type`/`status` are already opaque strings used only for `styleKey`; `excludedBusinessFields()` already lists the demoted fields. Demotion makes "opaque, template-owned" the *intended* contract, not a schema rewrite. | `tasks/T0.1.md` §4 |
| **Target package boundaries (names)** | Seven boundaries: platform-neutral **canvas-core**, **web-adapter**, **native/metal-adapter**, **web product shell**, **future macOS shell**, **server/API/MCP**, **shared contracts**. Mapping below in Review gate. | `tasks/T0.2.md` (NOT AUTHORED — see audit) |
| **Migration rule** | Current `src/` stays exactly where it is until the target layout is approved; this phase produces a placement *plan*, not a move. No placeholder directory is committed architecture; no build config imports a future path. | breakdown T0.2/T0.3 deliverable; `tasks/T0.3.md` (NOT AUTHORED) |

## Per-task deliverable summaries

### T0.1 Define Product Object Vocabulary — AUTHORED (`tasks/T0.1.md`)

Core contract: a stable universal vocabulary that separates **primitive objects**
(what the canvas draws/edits/hit-tests) from **semantic templates** (named
recipes over primitives), expressed as an *evolution* of `Scene` /
`SceneSnapshot` / `RenderScenePatch` — not a parallel model. Seven primitive
kinds (`shape`, `text`, `edge`, `frame`, `image_artifact`, `comment_marker`,
ephemeral `actor_marker`) plus two cross-cutting capabilities (tags/labels;
text-on-everything via the single core `text.rs` shaper). Decision-graph
semantics DEMOTE into a template-namespaced `meta` bag + `styleKey`; geometry,
grouping, comments, artifacts, selection, and style tokens are KEPT verbatim.
All five product surfaces are expanded as primitive-only recipes (§6), so no
template needs a renderer object the primitive set lacks. The render contract
needs no structural change because `excludedBusinessFields()` already encodes the
seam.

### T0.2 Confirm Package And App Boundaries — NOT AUTHORED (reconstructed from breakdown)

Intended core contract: take the **current** `src` structure as ground truth,
inventory it, and name the future package boundaries without moving code. The
boundary note must place platform-neutral canvas core, web canvas adapter,
native/Metal adapter, web product shell, future macOS shell, server/API/MCP, and
shared contracts; present layout options (keep `src` / reorganize within `src` /
top-level packages) with tradeoffs; and state the rule that `src/` stays put
until a target is approved. Required property: the boundary must support a future
iOS native app **without** product business logic leaking into the renderer
adapter. Observed current inventory (the inputs this task would consume):

| Current path | Contents | Maps to boundary |
| --- | --- | --- |
| `src/renderer/core/` (Rust: `lib.rs`, `model.rs`, `text.rs`, `webgpu.rs`, `stats.rs`) | scene model, hit/geometry, text shaping, wgpu render, debug stats | platform-neutral **canvas-core** |
| `src/client/renderer/` (`adapter.ts`, `engine.ts`, `scene.ts`, `wasmLoader.ts`, `wasm/`) | WASM load, surface bridge, scene marshalling | **web-adapter** |
| `src/client/` (`App.tsx`, `main.tsx`, `components/`, `lib/`, `styles.css`) | product UI shell (currently React/React-DOM) | **web product shell** (Svelte is the migration target; React removed) |
| `src/server/` (`index.ts`, `local.ts`, `mcp.ts`, `storage.ts`) | HTTP/API, local persistence, MCP server, storage/migration | **server/API/MCP** |
| `src/shared/` (`schema.ts`, `renderScene.ts`, `renderPatch.ts`, `graph.ts`) | canonical schema, render snapshot, op vocabulary, export derivation | **shared contracts** |
| `poc/` (empty), no native dir yet | — | **native/Metal adapter** + **future macOS/iOS shell**: target-only, no current code |

### T0.3 Plan Target Layout Placement — NOT AUTHORED (reconstructed from breakdown)

Intended core contract: turn the T0.2 boundary decision into a concrete
placement plan **without moving files**. Deliver a proposed final directory
layout for web shell / renderer core / web adapter / native adapter / shared
contracts / server-API-MCP / tests / scripts / docs; a path-by-path migration map
(move vs stay, with reason); an import/build/test impact note for the selected
layout; and the rule for when an empty placeholder directory is useful vs when it
must be avoided. Required property: a reviewer can see exactly which path moves,
which stays, and why — and no build config (`vite.config.ts`, `tsconfig*.json`,
`Cargo.toml`, `scripts/renderer-toolchain.sh`) imports a future path before the
migration task that owns it. Note the build coupling this task must respect:
`renderer:wasm:build` writes `src/renderer/core/wasm` into
`../../client/renderer/wasm`, and `tsconfig.server.json` includes
`src/server` + `src/shared` — any move re-points these.

## Phase verify-or-evaluate audit

Each P0 phase "Verify or evaluate" bullet, restated, with MET / PARTIAL / GAP.

| Phase verify bullet | Status | Basis |
| --- | --- | --- |
| A reviewer can explain what belongs in app shell, canvas core, platform adapter, semantic template, and MCP activity tracker. | **PARTIAL** | The primitive-vs-template and document-vs-ephemeral seams are fully argued in `tasks/T0.1.md` §1–3, and the boundary names + current-path mapping are reconstructed above. But the boundary-note tradeoff analysis that T0.2 owns was never authored, so "platform adapter vs app shell" rests on reconstruction, not a reviewed task design. |
| Current `src` responsibilities are inventoried before any directory scaffold is treated as architecture. | **PARTIAL** | A live inventory of all five `src` subtrees is captured in the T0.2 summary above from the actual tree. T0.2 (the task that formally owns this inventory and its tradeoffs) was not authored, so this is an orchestrator-supplied inventory rather than the task deliverable. |
| Existing `src/` and `poc/` code remains unmoved. | **MET** | Design-only phase. No source file was read-modified, moved, or scaffolded; `poc/` is empty and untouched. T0.1 §Verify confirms this for its scope; T0.2/T0.3 are reconstructed as notes only. |
| Target layout is chosen from the current repo shape and documented before files move. | **GAP** | T0.3 (Plan Target Layout Placement) — the task that produces the proposed final layout, the path-by-path migration map, and the build-impact note — was **not authored**. The Review gate below proposes a concrete layout to unblock the decision, but the full migration map and import/build/test impact note remain to be written before any move. |

## Review gate

`human-decision`: **Approve the target layout and package boundary NAMES before
any code moves into them.** Nothing below is implemented; the human is choosing
names and a shape, not authorizing a move.

### Recommended boundary names

Seven boundaries, named to be host-neutral and to keep product logic out of the
renderer adapter:

`canvas-core` (Rust, platform-neutral) · `web-adapter` · `native-adapter`
(Metal/iOS, target-only) · `web-shell` (Svelte product shell) · `macos-shell`
(target-only) · `server` (API + MCP + storage) · `shared` (canonical contracts).

### Recommended layout — Option B: reorganize within `src`

Keep a single `src/` root (one workspace, one toolchain) and rename subtrees to
the boundary names, leaving native/macOS as documented-but-empty target slots
created only when their migration task runs:

```text
src/
  canvas-core/     <- src/renderer/core           (Rust, unchanged contents)
  web-adapter/     <- src/client/renderer
  web-shell/       <- src/client                  (Svelte; React removed in P-shell task)
  server/          <- src/server                  (API + MCP + storage)
  shared/          <- src/shared                  (schema/renderScene/renderPatch/graph)
  native-adapter/  <- (target-only; no files until its migration task)
  macos-shell/     <- (target-only; no files until its shell task)
tests/  scripts/  docs/                            (stay)
```

Why B: it makes the boundary names visible in the tree, keeps one build graph
(no premature monorepo tooling — honors T0.3's stop-condition on package-manager
/ bundler / project-reference changes), and re-points only two known build
couplings (the `renderer:wasm:build` out-dir and `tsconfig.server.json` includes).

### The 1–2 alternatives the human must choose between

- **Option A — Keep `src` as-is, name boundaries by convention only.** Zero
  moves, lowest risk; boundary names live only in docs, so the tree never
  self-documents the core/adapter/shell split and the iOS-readiness seam stays
  implicit. Choose if minimizing churn outweighs legibility.
- **Option C — Promote to top-level packages (`packages/canvas-core`, …) with a
  workspace manager.** Strongest isolation and the cleanest future-native story,
  but triggers the T0.3 stop-condition (package-manager / bundler / TS
  project-reference changes beyond approved scope) and is the heaviest migration.
  Choose only if a near-term native target justifies workspace tooling now.

The gate must also confirm: **Svelte is the `web-shell` target and React/
React-DOM are removed** (no dual/compat shell), and **`canvas-core` stays
platform-neutral** (no DOM/business/MCP/export semantics absorbed).

## Stop-conditions encountered

- **T0.1:** "A required object would force a full Figma-like vector editor or
  browser CSS layout engine." — **CLEAR (not triggered).** All seven kinds are
  bounded boxes, connectors, frames/hulls, text runs, image placements, and
  markers, renderable by the existing wgpu core + single text shaper; no
  path/boolean/auto-layout or DOM/CSS layout (`tasks/T0.1.md` §Stop-or-ask).
- **T0.2:** "The desired native target changes from shared-core-plus-native-
  adapter to separate native reimplementation." — Not evaluated (task not
  authored); the recommended layout assumes shared-core + native adapter and
  surfaces this for the gate.
- **T0.3:** "The selected layout requires package-manager, bundler, or
  TypeScript project-reference changes beyond approved scope." — Would be
  **triggered by Option C**; the recommended Option B is chosen specifically to
  avoid it.
- **Phase-level:** T0.2 and T0.3 task fragments were not authored before this
  rollup was assembled (see audit GAP/PARTIAL). This is a process gap, not a
  vocabulary or layout blocker, but the T0.3 migration map and build-impact note
  must be written before any file actually moves.
