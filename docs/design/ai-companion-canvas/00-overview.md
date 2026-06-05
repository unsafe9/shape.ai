# AI Companion Canvas — Design Buildout Overview

> This design set turns shape.ai from an engineering-specific decision-graph
> editor into a universal, local-first, Figma-like infinite canvas for AI coding
> agents: ~7 universal primitives that every product surface (todo, wiki, ADR,
> server-architecture, presentation, investigation map) composes as a template
> recipe; a platform-neutral Rust/WebGPU core behind core/web-adapter/Metal-adapter
> boundaries that target web, macOS, and future iOS; good (identity-preserving) LOD
> for unbounded scenes; an MCP companion activity tracker that makes agent reads
> and writes observable on the canvas; and a gated React→Svelte web-shell migration.
> It is a **planning pass only**: no production code under `src/` or `poc/` is
> moved, scaffolded, or rewritten. The output is 28 subagent-ready task cards
> (`tasks/T0.1`–`tasks/T6.5`), one authored phase rollup (P0), and two adversarial
> decision reviews (D1, D2), both of which land at **approve_with_conditions** —
> the direction is confirmed, with a short list of owned follow-up tasks and two
> human-decision gates standing between this plan and implementation.

## Status by phase

The phase docs requested for P1–P6 were **not authored as rollups**; their design
detail lives entirely in the per-task `tasks/T*.md` fragments. Only P0 has a
reviewer-facing phase rollup. This absence is itself a recorded condition (D2 C9).
Gate column = the gate the breakdown assigns each phase; gate status reflects the
authored artifact.

| Phase | Doc | Review gate | Gate status |
| --- | --- | --- | --- |
| P0 Product & Package Contract | `P0-product-and-package-contract.md` | `human-decision`: target layout + boundary names | **Open** — awaiting human approve/choose (rollup authored; T0.2/T0.3 fragments are reconstructions) |
| P1 Portable Canvas Core Boundary | _(no rollup; `tasks/T1.1`–`T1.4`)_ | `human-decision`: core/adapter split | **Open** — awaiting human approval |
| P2 Universal Primitive Editing Slice | _(no rollup; `tasks/T2.1`–`T2.5`)_ | → D1 Primitive Sufficiency Review | **approve_with_conditions** (D1) |
| D1 Primitive Sufficiency Review | `D1-primitive-sufficiency-review.md` | decision review | **approve_with_conditions** |
| P3 Good LOD & Infinite Performance | _(no rollup; `tasks/T3.1`–`T3.4`)_ | verify-or-evaluate (no formal gate) | Design complete; perf evidence pending T3.4 |
| P4 Template System On Primitives | _(no rollup; `tasks/T4.1`–`T4.5`)_ | verify-or-evaluate (no formal gate) | Design complete; carries D1 conditions C1–C4 |
| P5 MCP Companion Activity Tracker | _(no rollup; `tasks/T5.1`–`T5.4`)_ | verify-or-evaluate (no formal gate) | Design complete; gated on MCP stateful transport (D2 C6) |
| P6 Integration / Svelte / Rust Migration | _(no rollup; `tasks/T6.1`–`T6.5`)_ | → D2 Direction Review | **approve_with_conditions** (D2) |
| D2 Direction Review | `D2-direction-review.md` | decision review | **approve_with_conditions** |

## Human decisions required now

Two `human-decision` gates block any physical work. Both are choices about names
and shape, not authorizations to move code.

1. **P0 — Approve the target layout and package boundary names.**
   - **Recommended:** boundary names `canvas-core` · `web-adapter` ·
     `native-adapter` · `web-shell` · `macos-shell` · `server` · `shared`; layout
     **Option B (reorganize within `src/`)** — rename subtrees to the boundary
     names, leave `native-adapter`/`macos-shell` as documented-but-empty target
     slots, re-point only the two known build couplings (`renderer:wasm:build`
     out-dir and `tsconfig.server.json` includes).
   - **Choose against:** Option A (keep `src/` as-is, names in docs only — lowest
     churn, tree never self-documents the seam) or Option C (top-level
     `packages/*` with a workspace manager — strongest isolation, heaviest
     migration, trips the T0.3 package-manager/bundler stop-condition).
   - The gate must also confirm **Svelte is the `web-shell` target with React/
     React-DOM removed** (no dual shell) and **`canvas-core` stays
     platform-neutral** (no DOM/business/MCP/export semantics).
   - **Caveat:** T0.2/T0.3 were never authored, so Option B is recommended but
     **uncosted** — D2 C2 requires a real path-by-path migration map before any
     file moves.

2. **P1 — Approve the core/adapter split before production migration starts.**
   - **Recommended:** approve the **shared-core + platform adapter** split
     (`SceneSnapshot`/`RenderScenePatch`/`CanvasInputEvent`/`CoreHitResult`/
     `CoreOverlayRequest` as plain serde payloads the core ingests/emits; GPU
     surface, input source, text-overlay widget, display scale isolated in the
     adapter; `wgpu` already vendors a Metal backend).
   - **Choose against:** a separate native reimplementation (the T0.2
     stop-condition) — not recommended; it forks the semantic scene model.
   - **Caveat:** the platform-neutral core is **not yet buildable** — it is all
     `#[cfg(feature="wgpu-probe")]`-gated with non-optional `web-sys`/`wasm-bindgen`,
     so iOS-readiness is design-only until D2 C1's ungating task ships. Approving
     the split commits to that follow-up task.

There is also a **binding human decision inside P5** (not a phase gate):
**make `/mcp` stateful** (`sessionIdGenerator` + `Map<sessionId, identity>`,
registry kept ephemeral/out of SQLite). Without it, two concurrent identical HTTP
clients collapse to one dock entry and MCP client identity is undeliverable
(D2 C6).

## Done-criteria coverage

Each Done Criteria bullet from the breakdown mapped to the owning phase/task.
"addressed-in-design" = a task card specifies it; "open" = a condition or gap
must close first.

| Done Criteria bullet | Owner | Status |
| --- | --- | --- |
| Package boundary explains web, macOS, iOS | P0 T0.2/T0.3; P1 T1.1–T1.4 | **open** — boundary design sound, but core not yet buildable (D2 C1) and layout uncosted (D2 C2) |
| Scene/core model separates primitive vs semantic template | P0 T0.1; P2 T2.1 | addressed-in-design (D1 confirms sufficiency) |
| Web UI is a general primitive editor, not an engineering node editor | P2 T2.1–T2.4; P4 T4.1 | addressed-in-design |
| Minimal Figma-like editing (select/multi-select/move/resize/text/shape/edge/group/nested group/hull/z-order/align-snap/label-tag/comment) | P2 T2.2, T2.3, T2.4 | addressed-in-design; box-less `text` hit/selection rule still **open** (D1 promotion 3 / D2 C5) |
| Old Shape/project + ADR component removed from primitive layer | P0 T0.1; P2 T2.1; P6 T6.1 | addressed-in-design (demotes to `frame` + `meta` + export preset; D1 confirmed) |
| Todo/wiki/ADR-design/server-arch/presentation templates from primitive composition | P4 T4.2–T4.5 (+ investigation map T4.4 §6) | addressed-in-design **except image previews** — `image_artifact` has no render substrate (D1 C1 / D2 C3, **open**) |
| Good LOD: stable identity + stable pan/zoom at scale | P3 T3.1–T3.4 | addressed-in-design; "performant" evidence **open** pending T3.4 latency measurement |
| Web shell migrated to Svelte; React/React-DOM removed from production | P6 T6.2, T6.4 | addressed-in-design (single-branch S0–S4, GATE-X removes `react()` from vite **and** vitest) |
| Performance-sensitive canvas logic in Rust/core, not Svelte | P6 T6.3 | addressed-in-design (12 responsibilities audited; camera write-back removed) |
| MCP companion icon/dock/animation/click-to-follow/spectator/trace | P5 T5.1–T5.4 | addressed-in-design; identity **open** on stateful-transport decision (D2 C6) |
| Human + AI/MCP writes record actor/target/operation/diff/timestamp/base revision | P2 T2.5; P5 T5.4 | addressed-in-design (typed op envelope over the existing unused `events` table) |
| Document vs ephemeral (viewport/follow/hover) state separated | P0 T0.1 §3; P2 T2.5 | addressed-in-design; selection-as-ephemeral back-compat check **open** (D1 C7) |
| Existing MCP/API/export/comment semantics preserved on new model | P6 T6.1 | addressed-in-design; gated by a round-trip migration fixture (D1 C6) and core badge migration (D1 C5 / D2 C4) |

## Open questions & deferrals

**Load-bearing open questions / conditions (must close before dependent work):**

- **Image/texture-quad substrate (D1 C1 / D2 C3).** The single load-bearing
  missing primitive. The wgpu core has one pipeline and one texture (the glyph
  atlas); `image_artifact` renders a captioned empty box. Promote it once as a
  shared primitive (second bind group + image-sampling pipeline, decode/upload
  path, image-quad emit, `SceneSnapshot` image array) before presentation,
  wiki-evidence, or server-arch-image templates claim preview coverage.
- **Buildable platform-neutral core (D2 C1).** Ungate the `SceneRenderer`/op/stats
  types from `wgpu-probe`, make `web-sys`/`wgpu` optional, split WASM binding from
  the engine, prove a non-`web-sys` compile. Until then iOS-readiness is asserted,
  not demonstrated. The "pure `#[cfg]` removal" framing is understated.
- **Core badge/style decoupling (D1 C5 / D2 C4).** `node_type_label()`
  (`webgpu.rs:4696`) hardcodes the ADR enum and renders a per-`nodeType` badge;
  `defaultStyles`/`edgeToRenderEdge` select style by enum. The "demotion is a
  byte-identical no-op for core" premise is **disproven** and currently owned by
  nobody; needs an owned task or a documented `'Node'` fallback.
- **Layout migration map (D2 C2).** T0.2/T0.3 unauthored → recommended Option B is
  uncosted. Author the path-by-path map + build/import/test impact before any move.
- **MCP stateful `/mcp` transport (D2 C6).** Binding human decision (above).
- **Per-template `meta` validation + box-less `text` rule (D1 C4 / D2 C5).**
  Demotion to a `meta` bag drops Zod enum integrity; free-standing `text`
  hit/selection/resize is unsettled.
- **Investigation-map recipe** — was undemonstrated in D1, now shown in T4.4 §6
  (D2 records it resolved).
- **Implicit-root-frame question (D1 C8)** — required `SceneNode.groupId` means no
  canonical "object on the bare canvas"; decide before templates assume
  free-floating objects.
- **HTTP disconnect detection + events-log retention/compaction (D2 C7).**
- **LOD thresholds/memory budgets are provisional seeds** pending T3.4 measurement.
- **Phase rollups P1/P3/P5/P6 absent (D2 C9)** — D2's evidence base is the task
  fragments, not phase-level reviewed rollups.

**Explicit deferrals (out of this planning pass):**

- **iOS app scaffold** — defer until the Metal adapter contract is approved and the
  macOS shell direction is clearer.
- **Realtime remote collaboration** — out of scope (no CRDT/Yjs, transport, remote
  auth, conflict-resolution UI, remote cursors). The op model stays
  collaboration-ready, but realtime is revisited only after the local-first canvas,
  trace, and companion tracker are stable.
- **Full document / presentation / vector design editors** — deferred unless a
  primitive/template gap proves one is actually required. No path editing, boolean
  geometry, auto-layout, or component system.
- **Moving existing `src/` production code** — not done in this pass.

## Recommended next wave

Sequence the binding-condition work first (D2's recommended wave), so the AT-RISK
criteria become MET before the template and MCP build waves run. This refines the
breakdown's Wave A–L ordering, it does not replace it.

1. **Layout decision (Wave A / D2 C2).** Author T0.2 + T0.3 with a costed
   path-by-path migration map and the two build-coupling re-points; take the **P0
   human-decision gate** (Option B recommended). Do not move files until the map
   exists. This is the current "Next Unblocked" front and the single most important
   step.
2. **Buildable portable core + core badge/style decoupling (D2 C1 + C4).** One owned
   refactor; take the **P1 human-decision gate** for the core/adapter split. Can run
   alongside Wave B (T1.x boundary contracts).
3. **Image/texture-quad primitive (D2 C3).** Before any image-dependent template
   (before Wave H).
4. **MCP stateful-transport decision (D2 C6) + envelope foundation.** Then
   T2.5 → T5.1 → T5.2 before the P5 companion build.
5. **LOD calibration (T3.4 vs T3.3 fixtures)** to replace provisional thresholds
   with measured values (Wave F).
6. **React→Svelte migration under the gates (D2 C8).** T6.4 single-branch S0–S4,
   GATE-R before GATE-X, with T6.3 before T6.2 and the vitest `react()` removal in
   GATE-X (Waves I–L).

Per-template `meta` validation (C5) and the missing phase rollups (C9) ride
alongside their owning waves.

## Index

**Phase rollups**

- P0 — `P0-product-and-package-contract.md`
- P1–P6 — no rollup authored; see task fragments below (recorded gap, D2 C9)

**Decision reviews**

- D1 — `D1-primitive-sufficiency-review.md` (approve_with_conditions)
- D2 — `D2-direction-review.md` (approve_with_conditions)

**Source plan**

- Task breakdown — `../../ai-companion-canvas-task-breakdown.md`

**Task fragments** (`tasks/<ID>.md`)

- P0: `tasks/T0.1.md`
- P1: `tasks/T1.1.md` · `tasks/T1.2.md` · `tasks/T1.3.md` · `tasks/T1.4.md`
- P2: `tasks/T2.1.md` · `tasks/T2.2.md` · `tasks/T2.3.md` · `tasks/T2.4.md` · `tasks/T2.5.md`
- P3: `tasks/T3.1.md` · `tasks/T3.2.md` · `tasks/T3.3.md` · `tasks/T3.4.md`
- P4: `tasks/T4.1.md` · `tasks/T4.2.md` · `tasks/T4.3.md` · `tasks/T4.4.md` · `tasks/T4.5.md`
- P5: `tasks/T5.1.md` · `tasks/T5.2.md` · `tasks/T5.3.md` · `tasks/T5.4.md`
- P6: `tasks/T6.1.md` · `tasks/T6.2.md` · `tasks/T6.3.md` · `tasks/T6.4.md` · `tasks/T6.5.md`
