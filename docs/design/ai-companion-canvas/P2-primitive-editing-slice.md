# Phase P2: Universal Primitive Editing Slice

> Reviewer-facing phase summary. Design-only: no production code is written, moved, or scaffolded in P2.
> Sources: `docs/ai-companion-canvas-task-breakdown.md` (P2 intro, T2.1–T2.5, D1); task fragments `tasks/T2.1.md`–`tasks/T2.5.md`; decision doc `D1-primitive-sufficiency-review.md`.

## Goal / Why now

Replace engineering-only frontend assumptions with a small but general set of canvas primitives and direct-editing operations, so that templates (P4) compose from real primitives instead of becoming special cases.

**Why now:** Templates must be built from real primitives. If the primitive set is too narrow, every template (todo, wiki, ADR, server diagram, presentation, investigation map) degrades into a private renderer or private object model. P2 proves the universal primitive layer is sufficient before P4 builds on it. P2 runs in parallel with P3 (LOD) and P5 (MCP tracker) once P1 fixed stable contracts; P4 depends on enough primitive coverage from P2 plus the D1 gate below.

## Decisions at a glance

Each decision below is deduped across the five tasks; the owning task fragment is the authoritative pointer.

### Primitive model — `tasks/T2.1.md`

- Canonical model carries **six persisted primitive kinds + one ephemeral kind**, each a *generalization of an existing symbol*, not a new parallel type: `shape` (← `SceneNode`), `text` (box-less `shape`, `styleKey:"text-plain"`), `edge` (← `SceneEdge`), `frame`/group (← `SceneGroup`), `image_artifact` (placement referencing `SceneArtifact`), `comment_marker` (← `SceneComment`); ephemeral `actor_marker` (P5/T5.x owns the full model, never persisted).
- Engineering enums/business fields are **DEMOTED** into an additive free-form `meta` bag: `nodeType`/`edgeType`/`status`/`confidence`/`evidenceRefs`/`childDecisionIds`/`rationale` → `meta.*` + `styleKey`. Schema change is **purely additive** (`meta` defaults `{}`, enums retained through P2), so old `Scene`s parse and round-trip; SQLite migration is a `node_json`/`edge_json` JSON reshape with **no `ALTER TABLE`**.
- Old Shape/project and structured-ADR components are **removed from the primitive layer**: project → ordinary top-level `frame`; ADR structure → `frame`+`shape`+`edge`+`comment_marker` template recipe; ADR exports (MADR/YADR/mermaid) → export presets that *read* primitives. No revived canonical project/ADR component.

### Editing tools — `tasks/T2.2.md`

- Minimum direct-manipulation tool set specified as **additive extensions to the existing `CanvasInputEvent` input vocabulary and `RenderScenePatch` op vocabulary**: select / pan / zoom / create-shape / create-text / create-edge / frame / resize / align-distribute / z-order / duplicate / copy-paste / comment / delete. **Rotate is deliberately not built** (breakdown "only if required"; no primitive needs it).
- Op vocabulary grows by exactly **six additive entries** (`resize-card`, `resize-group`, `align-cards`, `distribute-cards`, `duplicate-objects`, `batch`); input vocabulary by modifier flags + two drag states (`Marquee`, `Resize`) + ephemeral active-tool.
- **Transient multi-select** (marquee + modifier-click) is an **ephemeral core selection set** (recommended option B) producing **no `SceneGroup`**; batch move mutates each object's own geometry and the set evaporates on click-away. Persistent grouping is deferred to T2.4.

### Text-on-everything — `tasks/T2.3.md`

- Text is a **named field slot on a primitive**, not a primitive kind: `shape` (`title`/`summary`/`detail`), `edge` (`label`), `frame` (`title`/`summary`), `comment_marker` (`body`), `image_artifact` (caption via the `shape` placement). All five funnel through the **single** `text.rs` shaping engine; **no per-primitive text engine**.
- The gap is editing, not rendering: edge labels and frame titles already render through the engine but are not hit-as-text. T2.3 generalizes the card-only edit path (`hit field → CoreOverlayRequest → edit-card-text op`) to a **target-kinded** path: `edit-card-text` generalizes to `edit-text{target: card|edge|frame, field}` with `edit-card-text` kept as a back-compat alias. Net: 0 new schema fields, 0 new tokens, 0 new shaping code.

### Grouping / labeling — `tasks/T2.4.md`

- Grouping is generic: `group-objects` / `ungroup` / `set-object-group` ops on the existing `SceneGroup` (`frame`) container with `parentGroupId` nesting and **no type field**. `ungroup` is the non-destructive counterpart to the existing cascade-destructive `delete-group`; member ids/geometry/text/tags/edges are preserved.
- **Freeform hull** is a **render-only derived `pad(union(member_bounds))` polygon** (visualization + hit-test only); canonical `bounds` stays the `WorldRect` AABB. No boolean/vector editing. Hull eases toward its settled target as **ephemeral animation**; only the settled `bounds` persists.
- Labels reuse the **single existing `Tag` registry** + a `tagIds[]` generalization from frames onto all objects (`set-object-tags`, `create-tag`). `Tag.color` covers color labels; no second taxonomy, no label object kind. Labels participate in visual-degradation LOD (name → swatch → hidden), never semantic-replacement LOD.

### Op-metadata envelope — `tasks/T2.5.md`

- Every canonical edit (human + MCP) is wrapped in **one operation envelope**: `operationId` / `actorId` / `actorType` / `clientId` / `targetIds` / `timestamp` / `baseRevision` + optional `sourceToolCall`. The 17 verbs are the existing/planned `RenderScenePatch` members; the verb is derived from `patch.kind`, not stored twice.
- The append-only operation log is the **already-existing-but-unused `events(id,type,payload_json,created_at)` table** (zero `INSERT INTO events` today); each committed envelope appends one row atomically with the document write — **no `ALTER TABLE`**. Undo/redo and MCP trace read from the log, not raw snapshots. `targetIds` is the structural anti-"raw blob" guarantee.
- **Document vs ephemeral split** enforced: document = objects/text/style/comments/artifacts/exports/proposals/accepted-ops/`sceneVersion`; ephemeral = viewport/hover/active-tool/follow/companion-animation/read-cursor/**selection**. The one behavioral correction: `select`/`multi-select` stops bumping `sceneVersion` and writes only the ephemeral `selection_json` metadata key. No CRDT/transport/auth/cursor is added; `baseRevision` is the unused future-conflict hook.

## Per-task deliverable summaries

| Task | Wave | Deliverable summary |
| --- | --- | --- |
| **T2.1 Primitive Object Model** | C | Evolves the canonical `Scene` model into six persisted primitive kinds + one ephemeral kind by *generalizing* `SceneGroup`/`SceneNode`/`SceneEdge` and demoting engineering enums/business fields into an additive `meta` bag. Field-by-field KEEP/DEMOTE/GENERALIZE mapping for `Group`/`Node`/`Edge`/`Tag`/`Comment`/`Artifact`; Shape/project + ADR removal path; universality table proving plain note / box-and-arrow / slide / current workflow / ADR all compose from primitives. Render/core projection unchanged; migration is a JSON reshape with no `ALTER TABLE`. |
| **T2.2 Minimal Figma-Like Editing Tools** | D | Per-tool gesture→op→home plan for the minimum direct-manipulation set, plus a transient multi-select plan (marquee, modifier-click, select-all-in-frame, selection bounds, batch move/resize/z-order/delete). Adds six op-union entries + input modifier flags + two drag states + active-tool, all shaped so a T2.5 envelope drops on cleanly. Rotate excluded. Core-vs-shell placement keeps hit/marquee/selection/camera geometry in Rust core; clipboard and active-tool ephemeral in the shell. |
| **T2.3 Text On Everything** | D | Text-attachment table (five sites, one engine) + editing behavior generalizing the card-only edit pipeline to a target-kinded path. Generalizes `edit-card-text` → `edit-text{target,field}` (alias retained), adds three core hit/overlay helpers for edge/frame, extends `overlay_max_lines`. Single-engine invariant means the renderer-plan P4 text-quality gates (Korean, mixed, multiline, no-space, edge/shape labels) apply uniformly with no new shaping code. |
| **T2.4 Grouping And Labeling Foundation** | E | Group/ungroup/membership ops + group-level transforms on the existing `frame` container; render-only freeform hull with smooth shrink/expand animation; group-vs-member selection via ephemeral active-group focus + deepest-enclosing-frame hit rule; drag-in/out membership on unambiguous gestures; `tagIds[]` generalized onto all objects via the one `Tag` registry; zoom-band label thinning; ADR/project label migration into `frame.title`/`tagIds`/`meta`. Five additive ops + `parentGroupId` passthrough fix; no schema break. |
| **T2.5 Collaboration-Ready Operation Model** | F | Names the 17-verb taxonomy over existing/planned `RenderScenePatch` members; defines the seven-field operation envelope + optional `sourceToolCall`; routes every committed envelope into the existing `events` table as the primary edit unit; defines document-state and ephemeral-state boundaries (incl. the selection correction and a minimal `proposals` slot for accept/reject); specifies undo/redo/trace/audit/future-broadcast all driven from the op log. Additive contract/persistence delta only; no CRDT/transport/auth/cursor. |

## Phase verify-or-evaluate audit

The P2 phase intro lists three "Verify or evaluate" bullets. Each is restated and marked against the assembled task designs.

| Phase verify-or-evaluate bullet | Status | Basis |
| --- | --- | --- |
| A user can draw, multi-select, label, connect, group, arrange, and annotate simple objects without choosing a specialized template. | **MET** | T2.2 covers draw/multi-select/connect/arrange/annotate via primitive tools (no template field; slide/todo/wiki/box-diagram assembled by hand in its Verify); T2.4 adds label + persistent group. All seven verbs land across T2.1–T2.4 on shared primitives. |
| Grouping, adding objects to a group, removing objects from a group, nested grouping, and ungrouping feel direct and reversible. | **MET** | T2.4 §1–§6 make membership change only on unambiguous gestures via `set-object-group` with `groupId` as the single source of truth; `ungroup` preserves member identity; nesting via `parentGroupId`/deepest-enclosing-frame; each interaction is exactly one operation with `targetIds`, so T2.5's op log drives reversible undo/redo. T2.4 Verify marks all five group-interaction bullets MET. |
| Primitive operations are actor-tracked and serializable. | **PARTIAL → MET when T2.5 lands** | Every T2.1–T2.4 op is a serializable `RenderScenePatch` member shaped to carry the envelope, but the actor-tracking fields (`operationId`/`actorId`/`actorType`/`clientId`/`targetIds`/`timestamp`/`baseRevision`) and the `events`-log funnel are added in **T2.5** (T2.2 marks this line "PARTIAL by design"). Because T2.5 is in P2 (Wave F), the bullet is **MET at phase exit**: the envelope + `events` append close the gap. Flagged PARTIAL at the per-task level only to record that the metadata does not exist before T2.5. |

No phase-level verify bullet is a GAP: the only PARTIAL is sequencing within P2 (T2.5 supplies what the earlier tasks shape), resolved by phase exit.

## Review gate

**D1 Primitive Sufficiency Review** asks: *Can the first product templates be built from the primitive set without hidden special cases?*

**Verdict: `approve_with_conditions`.**

The Redirect-if condition (any template needs a private renderer, private object model, or revived project/ADR component) is **not** tripped — the load-bearing finding, confirmed independently by all three lenses. The six vector primitives + tags + text-on-everything compose todo, wiki, ADR/design, and server-architecture structure with no template-specific renderer object; old Shape/project demotes to an ordinary `frame`; schema/storage evolution is additive (no `ALTER TABLE`); the `events` op-log table already exists. So the shared primitive layer **is** sufficient.

It is not a clean approve because D1's Approve-if bullet 3 ("missing needs promoted to shared primitives") is unmet and one of six binding templates is undemonstrated:

- **`image_artifact` is a vocabulary slot with no renderable substrate** — the wgpu core has only a text-glyph-atlas pipeline, no image quad/decode/upload path, and no image field on `SceneSnapshot`/`RenderCard`. Presentation, wiki evidence, and the architecture-image embed depend on real previews, so this is a *missing need that must be promoted to a shared primitive*.
- **Investigation map has no demonstrated recipe** — it is a binding Approve-if template but absent from every P2 universality table; buildability is asserted by omission, not proven.

**Critical correction the conditions carry forward:** the repeated design claim that the enum demotion is a "no-op for renderer/core (byte-identical projection)" is **false**. The Rust core *does* branch on the decision-graph vocabulary — `node_type_label()` (`webgpu.rs:4696`) renders a per-`nodeType` badge pill, and `defaultStyles`/`edgeToRenderEdge` select style by `nodeType`/`edgeType`. This does **not** revive a privileged primitive (only badge text + a string-keyed style map), so the verdict stands, but it must become an owned task.

### Conditions imposed (mostly land in P4/migration, after this phase)

| Condition | Lands in | Bearing on P2 |
| --- | --- | --- |
| **C1** — Promote the image substrate before any template depends on a preview (or degrade to a documented captioned-box placeholder). | P4 / shared primitive promotion | Image substrate is a *missing need* surfaced by P2's primitive proof; the promotion itself is post-P2. |
| **C2** — Add the investigation-map recipe to the universality proof before P4 begins. | P4 / T0.1+T2.1 universality tables | Discharges the one undemonstrated D1 template; touches P2's T2.1 universality table. |
| **C3** — Record `nodeType → styleKey` decoupling as a sequencing dependency on T4.1; keep enums redundantly populated until the template style-map exists. | T4.1 (post-D1) | Consistent with T2.1's "enums retained through P2"; visual identity of demoted ADR cards depends on T4.1. |
| **C4** — Restore semantic validation per template (namespaced `meta` Zod schemas). | T4.1 | T2.1/T2.4 explicitly defer `meta` validation to T4.1; recorded as a debt the approved model owes. |
| **C5** — Own the Rust-core badge migration (currently owned by nobody); correct the "no-op for core" framing. | New migration task | Directly contradicts a P2 design assertion; must be corrected before implementation. |
| **C6** — Gate the demotion with a real migration round-trip fixture, not the disproven "byte-identical" assertion. | Migration (fixture gates the reshape; backfill owned by T6.1) | Replaces P2's optimistic universality claim with an evidence gate. |
| **C7** — Regression-check the selection-as-ephemeral change (T2.5 §4 stops `select` bumping `sceneVersion`). | Migration | Directly tests a P2/T2.5 behavioral change against storage + `Scene.selection` back-compat. |
| **C8** — Resolve the implicit-root-frame question before templates assume free-floating objects. | Before P4 templates (deferred to T2.5/T6.1) | Surfaced by T2.4 §10 / T2.5; required-`groupId` means no canonical bare-canvas object yet. |

## Stop-conditions encountered

No task tripped a stop-or-ask condition; all five report **CLEAR (not triggered)**.

| Task | Stop-conditions watched | Outcome |
| --- | --- | --- |
| T2.1 | Existing canonical schema must be broken instead of evolved. | **CLEAR** — strictly additive (`meta` default `{}`, enums retained, `styleKey` already free, `SceneSelection` unchanged, no `ALTER TABLE`); enum-relax deferred to T6.1. |
| T2.2 | Tool list drifts into full design-tool parity. | **CLEAR** — rotate omitted; no path/boolean/auto-layout/component/snap-guide/constraint/property-panel; vocabulary grows by the six-op floor only. |
| T2.3 | Label behavior requires a separate text engine per primitive type. | **CLEAR** — all five sites route through the one `text.rs` engine; T2.3 adds editing plumbing + one generalized op, zero shaping code. |
| T2.4 | (1) grouping needs a private project/ADR model; (2) nested grouping enforces a fixed hierarchy; (3) freeform hull needs boolean/vector editing; (4) labels become a separate taxonomy. | **CLEAR** (all four) — free-form `SceneGroup` with no type field; arbitrary `parentGroupId` nesting; hull is a derived render-only polygon; labels reuse the single `Tag` registry. |
| T2.5 | (1) operations become a raw scene blob without semantic targets; (2) path requires CRDT/transport/auth/cursor now. | **CLEAR** (both) — `targetIds` is a required typed field per `kind`; design stops at the local append-only `events` log with `baseRevision` as an unused future hook. |

**Deliberate decisions flagged (not stop-conditions, but recorded so they are not silent defaults):** multi-select echo option A vs B (T2.2/T2.4 recommend ephemeral-core B); `batch` wrapper vs per-verb plural ops (T2.2, deferred to T2.5); box-less `text` as degenerate `shape` vs its own kind (T2.1/T2.3, deferred); `SceneSelection` variant naming (deferred to T2.4/T6.1); required-`groupId` / implicit-root-frame (T2.4 §10, deferred to T2.5/T6.1 — also D1 C8); `Scene.selection` retained-but-demoted (T2.5, clean removal deferred to T6.1); `events` retention/compaction policy (T2.5, deferred); comment-`body` edit path divergence and `edit-card-text` alias lifetime (deferred to T2.5/T6.1).
