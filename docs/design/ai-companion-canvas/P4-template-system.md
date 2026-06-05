# Phase P4: Template System On Primitives

> Reviewer-facing phase rollup for P4 of the AI Companion Canvas plan
> (`docs/ai-companion-canvas-task-breakdown.md:562`). Design-only: no production
> code is written, moved, or scaffolded in this phase.
> Tasks: T4.1, T4.2, T4.3, T4.4, T4.5. Full detail lives in `tasks/<ID>.md`.

## Goal / Why now

Build the common AI-agent work surfaces — todo board, wiki/idea board,
ADR/architecture, presentation — as **templates over the shared primitives**,
proving the universal canvas model without turning shape.ai into many separate
apps. A template is a *recipe over the existing primitives and ops*, consumed at
apply time; it owns no new primitive, op, schema field, renderer object, persisted
template record, or editing mode. P4 runs after enough primitive coverage from P2
and after D1 cleared the primitive-sufficiency review
(`docs/ai-companion-canvas-task-breakdown.md:104`).

## Decisions at a glance

The template contract and the five concrete templates, deduped, with pointers to
the owning task for full detail.

| Decision | Summary | Detail |
| --- | --- | --- |
| **Template = recipe over primitives** | A `TemplateContract` is `{ metadata, recipe, layout, exports, tags, promptHints? }`. It lowers entirely to existing `create-group`/`create-card`/`create-edge`/`create-tag`/`set-object-tags` ops run through `applyRenderPatchToShapeScene`→`commitAppPatch`, wrapped in one T2.5 `OperationEnvelope`. After apply the scene holds only `SceneGroup`/`SceneNode`/`SceneEdge`/`Tag` rows — byte-identical to hand-drawn objects, distinguishable only by an ordinary `meta.templateKind` string. No template runtime, mode, or object survives application. | `tasks/T4.1.md` §2, §7 |
| **No new op / kind / schema field** | Every recipe element maps to an op that already exists or is owned by T2.4/T2.5. `meta.templateKind`/`semanticType`/`status`/`slideIndex` ride the already-additive `meta` bag (no `ALTER TABLE`). The de-facto `seedGroupScene` ADR graph becomes the first declared `TemplateContract`; `createGroup`/`create_group` default `templateId` to it for back-compat. | `tasks/T4.1.md` §0, §8, §9 |
| **`styleKey` from template, not `nodeType`** (D1 C3) | Each recipe element sets `styleKey` explicitly (e.g. ADR "decision" → `"decision_point"`, todo card → `"task"`). `SceneStyleToken.id`/`RenderCard.styleKey` already accept any free string, so demoted objects keep visual identity with no enum coupling and no schema change. The core `node_type_label()` badge decoupling is the separately-owned D1 C5 migration task, not P4. | `tasks/T4.1.md` §2 |
| **Todo / task board** (`templateKind:"todo"`) | Root frame + one child frame per status column; `shape` cards as tasks; status/owner/priority as `meta` + status-color `Tag` chips; dependency `edge`s; `comment_marker`s. Ordering is canvas state (frame membership + `position`/`zIndex`), **no list database**. Human and MCP edits funnel through the same op model. | `tasks/T4.2.md` §2–§7 |
| **Wiki note + idea board** (`"wiki-note"`, `"idea-board"`) | Two `TemplateContract`s: titled frames, prose note/idea cards (text in `title`/`summary`/`detail`), nested section/theme sub-frames, source/evidence cards (citation in canonical `evidenceRefs[]`), reference edges. Spatial reorg preserves all data because every asset keys on stable object **id**, not position. Long-form stays an *export*, not an in-canvas word processor. | `tasks/T4.3.md` §2–§5, §7 |
| **ADR / decision map / server-arch / dependency / investigation map** | Five engineering `TemplateContract`s over only `frame`/`shape`/`edge`/`comment_marker` + tags. Engineering semantics live in `meta.semanticType` (demoted enum strings), labels, comments, and export rules — no structured-ADR core component. T4.4 owns the previously-undemonstrated investigation-map recipe (closes D1 C2). | `tasks/T4.4.md` §1–§6 |
| **Export presets read primitive content** | Exports stay keyed off `exportType` and read a `DecisionGraph` *derived from primitive `SceneNode`/`SceneEdge` rows* via `sceneGraphForGroup` — not a privileged ADR component. The only adjustment is a `meta.semanticType ?? node.type` read shim (with fallback) so presets read demoted metadata. ADR advertises `madr`/`yadr`/`design_doc_md`/`confluence_html`/`mermaid`; todo `ai_plan_md`; presentation a slide outline. | `tasks/T4.4.md` §7; `tasks/T4.2.md` §8 |
| **Presentation** (`"presentation"`) | Deck = top-level frame of child slide `frame`s ordered by `meta.slideIndex`; title/body `text`, `image_artifact` previews, connector `edge`s, speaker-note `shape`/`comment_marker`. The one new code is an additive `design_doc_md` slide-**outline reader** over primitives (no new `exportType`) + five additive style tokens. Minimum composition, not a PowerPoint clone. | `tasks/T4.5.md` §1–§8, §11 |
| **Image substrate is a hard dependency** (D1 C1) | Real image *pixels* (presentation slides, wiki evidence, architecture-image embed) depend on the image/texture-quad primitive D1 promotes, which does not yet exist in `webgpu.rs`. Until it lands, an image recipe element applies as a documented captioned-box placeholder carrying the artifact ref; pixels render once the substrate ships with **no contract change**. Export side is unaffected (emits a text `imagePrompt` today). | `tasks/T4.1.md` §2; `tasks/T4.5.md` §4 |

## Per-task deliverable summaries

### T4.1 Template Contract — AUTHORED (`tasks/T4.1.md`)

Defines the `TemplateContract` — template metadata + a primitive **recipe**
(objects/edges/groups/tags expressed as the existing create ops), a default
layout, an allowed-export set, suggested tags, and optional AI prompt hints — that
lowers entirely to existing `RenderScenePatch` create/group/tag ops run through
the existing `applyRenderPatchToShapeScene`/`commitAppPatch` funnel. The seam
already exists in code: `seedGroupScene` is the one hard-coded, unnamed template
(a fixed 10-node/9-edge ADR graph emitted as a `GroupSeedOutput`), and T4.1
generalizes it into declared recipe data routed through the logged-op funnel. Six
contract parts (metadata, recipe, layout, exports, tags, prompt hints) plus the
application pipeline that proves "results in normal canvas objects": apply
allocates ids, resolves suggested tags, builds the op list in dependency order,
places into free space via `nextGroupOffset`/`boundsForNodes`, and runs everything
through the funnel inside one `OperationEnvelope`. Net delta: one new shared file
`src/shared/template.ts` (contract + a `lowerTemplateToOps` helper); **zero** new
op kinds, schema fields, tables, or renderer objects.

### T4.2 Todo And Task Board Template — AUTHORED (`tasks/T4.2.md`)

Authors the todo/task board as a single declared `TemplateContract`: a root frame
+ one child frame per status column (To Do / In Progress / Done as plain frame
labels, not an enum), `shape` cards as tasks, status/owner/priority as `meta` +
status-color `Tag` chips, dependency `edge`s, and `comment_marker`s. The Verify
proof maps every editable aspect (create/rename/move/reorder/change-column/status/
owner/priority/dependency/comment/delete/apply-template) to one shared op, and
shows both the human canvas path and the MCP `patch_scene`/`add_comment`/template-
tool path funnel through the same `applyRenderPatchToShapeScene`→`commitAppPatch`.
Ordering is pure canvas state (frame membership + `position`/`zIndex`/`parentGroupId`),
so the "separate list database" stop-condition is cleared. `ai_plan_md` export
reuses `taskPlanSections`, which already reads `type:"task"` cards. Net delta: one
registered `TemplateContract` value; no schema change, no new op, no new exporter,
no list DB.

### T4.3 Wiki Note And Idea Board Templates — AUTHORED (`tasks/T4.3.md`)

Authors two `TemplateContract`s — wiki note clusters and idea boards — plus the
shared source/evidence-card and reference-edge recipes. Notes carry text in
`title`/`summary`/`detail`; sources are an `evidence`-styled card whose canonical
`evidenceRefs[]` (business layer, already in `excludedBusinessFields()`) holds the
citation; references are ordinary `SceneEdge`s. The Verify proof is field-by-field:
spatial reorg (`move-card`/`move-group`/regroup/`set-card-z-index`) mutates only
geometry/membership and never touches text/tags/comments/source-refs, because every
data asset keys on stable object **id**, not position. Long-form note bodies are
the existing multi-paragraph `detail` field edited through the same T2.3 overlay —
explicitly *not* a separate document editor, so the "long-form document editor
becomes primary surface" stop-condition is cleared (long documents remain exports).
Net delta: two `TemplateContract` records; zero new primitives, ops, schema fields,
style tokens, or export presets.

### T4.4 ADR And Architecture Diagram Templates — AUTHORED (`tasks/T4.4.md`)

Authors five engineering `TemplateContract`s (ADR, decision map, server
architecture, dependency diagram, investigation map) as pure primitive recipes
over `frame`/`shape`/`edge`/`comment_marker` + tags, differing only in
`meta.semanticType` vocabularies, default `styleKey`s, tags, layout, and advertised
exports. The ADR template is the mechanical transcription of `seedGroupScene`'s 10
nodes / 9 edges into recipe data (becomes the first authored contract, T4.1 §9).
The investigation-map recipe discharges D1 C2 (previously undemonstrated sixth
template) using only existing primitives. The **export-preset plan** shows ADR
formats already read primitives: `exportGroupContent`→`sceneGraphForGroup` reads
primitive `SceneNode`/`SceneEdge` rows, not a privileged ADR component; the single
adjustment is a `meta.semanticType ?? node.type` / `meta.status ?? node.status`
read shim (with fallback) across `madrSections`/`yadrSections`/`taskPlanSections`/
`makeMermaid`/`graphTextDigest`. The migration path for existing structured-ADR
scenes is a `node_json`/`edge_json` JSON reshape (no `ALTER TABLE`) to `meta.*` +
`styleKey` + `meta.templateKind:"adr"`, owned in code by T6.1 and gated by a
round-trip fixture (D1 C6). Net delta: five `TemplateContract` records + one export
read shim; no new primitive, op, exportType, or table.

### T4.5 Presentation Template — AUTHORED (`tasks/T4.5.md`)

Authors the presentation `TemplateContract`: a deck as a top-level `frame` of child
slide `frame`s ordered by `meta.slideIndex`, with title/body `text` (box-less
`shape`s), `image_artifact` previews, optional connector `edge`s, and speaker-note
`shape`/`comment_marker` cards — the recipe spine already pinned in T0.1 §6 / T2.1
§6. Title/body/notes land in the same `SceneNode.title`/`summary`/`detail` fields
cards use, edited by the same `edit-card-text` overlay through the single `text.rs`
shaper, so "same editing tools used for shapes and notes" holds structurally. The
one genuinely new reader is an additive slide-**outline preset**: route
`design_doc_md` for a presentation-tagged group through a new
`presentationOutline(scene, groupId)` branch in `generateLocalExport` (selected on
`meta.templateKind === "presentation"`, MADR fallback preserved for other groups) —
a reader over the produced primitives, no new `exportType`. A scope guard (§11)
holds the floor against PowerPoint drift: no play/transition/animation runtime, no
master-slide/theme engine, no slide object type, no auto-layout, no WYSIWYG editor.
Net delta: one `TemplateContract`, five additive `SceneStyleToken`s, one additive
outline reader; zero new primitive/op/schema/table/renderer object/editing mode.

## Phase verify-or-evaluate audit

Each P4 phase "Verify or evaluate" bullet, restated, with MET / PARTIAL / GAP. The
load-bearing property: templates yield **normal editable primitive objects, not
opaque widgets**.

| Phase verify bullet | Status | Basis |
| --- | --- | --- |
| Templates create editable primitive groups, not opaque widgets. | **MET** | T4.1 §7 proves every `TemplateContract` lowers to existing `create-group`/`create-card`/`create-edge`/`create-tag`/`set-object-tags` ops and leaves only `SceneGroup`/`SceneNode`/`SceneEdge`/`Tag` rows, byte-identical to hand-drawn objects and distinguishable solely by an ordinary `meta.templateKind` string — no template runtime, mode, renderer object, or persisted template record. T4.2/T4.3/T4.4/T4.5 each restate this for their concrete recipes (no widget, no private object family). There is no template runtime the objects depend on after application; the de-facto `seedGroupScene` template already demonstrates this in current code. |
| Users can break apart, modify, connect, and export template content. | **MET (with one owned dependency)** | After apply, objects are selected/moved/resized/regrouped/ungrouped/relabeled/text-edited/deleted/commented/exported by the *same* ops as any primitive (T4.2 §7, T4.3 §7 facet table, T4.5 §9). **Break apart / regroup**: existing group/ungroup + `parentGroupId`. **Modify**: T2.3 `edit-text` on `title`/`summary`/`detail`/`label`. **Connect**: `create-edge` reference/dependency/connector edges. **Export**: existing presets reading primitive content (`ai_plan_md`, `madr`/`yadr`/`design_doc_md`/`confluence_html`/`mermaid`, the new presentation outline). The one dependency on full parity is the **image substrate** (D1 C1): image previews degrade to documented captioned-box placeholders until the image/texture-quad primitive ships in `webgpu.rs` — export is unaffected (emits a text prompt). This is an owned promotion outside P4, not a template gap. |

## Review gate

**None.** P4 has no `human-decision` gate — it instantiates the contract D1 already
cleared (`approve_with_conditions`), and every template lowers to ops already owned
by P2. The binding constraint that must hold instead of a gate: **template output
must be normal canvas objects.** Applying any template produces only ordinary
`SceneGroup`/`SceneNode`/`SceneEdge`/`Tag` rows — no opaque widget, no template-
private renderer or object family, no persisted template record, no private editing
mode — distinguishable from hand-drawn objects only by an ordinary `meta.templateKind`
value that gates nothing in the renderer or core (T4.1 §7; restated in every T4.x
Stop-or-ask). Any template found to need a private editing mode, a private renderer,
or a revived structured component to be usable would break this constraint and
require escalation; none in the todo/wiki/idea/ADR/architecture/presentation set
does.

The D1 conditions P4 must carry forward (owned, sequenced — not gates here): **C1**
image substrate before any template depends on a real preview (else degrade to
captioned box); **C2** investigation-map recipe demonstrated (done in T4.4 §6);
**C3** `styleKey` decoupled from `nodeType` in every recipe (done, T4.1 §2); **C4**
per-template `meta` Zod validation authored with each contract (recommended,
deferred into the T4.x instances); **C5** the Rust-core `node_type_label()` badge
migration is a separate owned task (keep `node.type` redundantly populated until it
lands).

## Stop-conditions encountered

- **T4.1:** "A template requires a private editing mode to be useful." —
  **CLEAR (not triggered).** The contract is consumed entirely at apply time and
  lowers to standard create ops; afterwards objects are edited by the same
  select/move/resize/group/label/text/delete ops as any primitive. No template
  carries an editor, mode flag, or behavior — only declarative recipe + advisory
  `promptHints` (`tasks/T4.1.md` Stop-or-ask).
- **T4.2:** "Todo ordering requires a separate list database instead of canvas
  operations." — **CLEAR (not triggered).** A task's column is its frame membership
  (`SceneNode.groupId`), within-column order is `position`/`zIndex`, columns are
  nested frames — every ordering change is an op through `commitAppPatch`, persisted
  in the existing `Scene` rows. No list table, no parallel order index
  (`tasks/T4.2.md` Stop-or-ask).
- **T4.3:** "Long-form document editing becomes the primary surface instead of
  canvas-backed note/detail editing." — **CLEAR (not triggered).** Note bodies are
  the existing `SceneNode.detail` field edited through the same T2.3 overlay every
  card uses; long documents remain *exports* (`design_doc_md`/`confluence_html`),
  not an in-canvas word processor. True long-form rich-text as the primary surface
  would trip this and require a decision; not present (`tasks/T4.3.md` Stop-or-ask).
- **T4.4:** "Architecture or ADR objects need behavior that should be promoted to a
  generic primitive." — **CLEAR (not triggered).** All five recipes compose from the
  existing `frame`/`shape`/`edge`/`comment_marker` + tag primitives; the only surface
  wanting a capability the primitive layer lacks is the architecture-image **embed**,
  which is the `image_artifact` substrate already owned by D1 C1 — not a new ADR/
  architecture-specific primitive (`tasks/T4.4.md` Stop-or-ask).
- **T4.5:** "The feature drifts into a full PowerPoint clone instead of minimum
  presentation composition." — **CLEAR (not triggered).** No play/transition/
  animation runtime, no master-slide/theme engine, no slide object type or per-slide
  schema, no auto-layout/reflow, no rich-text WYSIWYG editor, no PPTX renderer; the
  only new code is five style tokens + one additive outline reader, both additive and
  primitive-reading (`tasks/T4.5.md` §11, Stop-or-ask).
- **Phase-level / cross-cutting:** The **image substrate (D1 C1)** is the single
  load-bearing dependency that crosses three templates (presentation, wiki evidence,
  architecture-image embed). It is owned outside P4; until it ships in `webgpu.rs`,
  image recipe elements degrade to documented captioned-box placeholders carrying the
  artifact ref, and the contracts are shaped to render pixels with no change once it
  lands. Not a P4 blocker, but the dependency must stay owned.
