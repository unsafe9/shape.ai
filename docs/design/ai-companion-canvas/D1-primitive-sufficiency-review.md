# Decision D1 — Primitive Sufficiency Review

Synthesis of three adversarial lens reports (`primitive-sufficiency`,
`migration-of-old-model`, `first-template-buildability`) against D1 in
`docs/ai-companion-canvas-task-breakdown.md:451`.

D1 question: Can the first product templates be built from the primitive set
without hidden special cases?

- **Approve if:** todo, wiki note, ADR/design, server architecture diagram,
  presentation outline, and investigation map all compose from shared
  primitives; ADR/design and old Shape/project content no longer require
  canonical project/ADR-specific components; missing needs are promoted to
  shared primitives or shared components.
- **Redirect if:** any template needs a private renderer, a private object
  model, or a revived project/ADR component before the shared primitive layer
  is usable.

## Verdict

**approve_with_conditions.**

The Redirect-if condition is **not** tripped, and that is the load-bearing
finding. All three lenses independently confirm that:

1. **No private object model is required.** The six vector primitives
   (`shape` / `text` / `edge` / `frame` / `comment_marker`, plus `tags` and
   text-on-everything) compose todo, wiki, ADR/design, and server-architecture
   structure with no template-specific renderer object. `RenderObjectKind` is
   already `group|card|edge|port|text` and each recipe uses only those kinds +
   `meta` + tags (primitive-sufficiency, finding 1).
2. **No revived project/ADR component is required.** Old `Shape/project`
   demotes to an ordinary `frame` — `migrateLegacyDesign` (`storage.ts:475`)
   already maps a legacy design into a `SceneGroup` + nodes/edges, and
   `SceneGroup` carries no privileged `type`. ADR content demotes to
   `frame + shape + edge + comment_marker` with MADR/YADR/mermaid as export
   presets that *read* primitives (`makeMermaid` at `graph.ts:109` reads a
   `DecisionGraph` derived from the scene). The structured-ADR fields
   (`nodeType` / `edgeType` / `nodeStatus` / `confidence` / `evidenceRefs` /
   `childDecisionIds` / `rationale`) are already listed by
   `excludedBusinessFields()` (`renderScene.ts:242`) and demote into a `meta`
   bag (migration-of-old-model, findings 1/6/7).
3. **Schema/storage evolution is additive, not breaking.** `meta` adds as
   optional `default {}`, geometry is denormalized into JSON columns, and the
   structured fields already live inside `node_json` / `edge_json` blob columns
   (`storage.ts:387`), so the demotion is a JSON reshape with **no ALTER TABLE**.
   The `events(id,type,payload_json,created_at)` table already exists
   (`storage.ts:433`) and is unused, so the collaboration-ready op log lands
   without schema change.

So the shared primitive layer **is** sufficient: nothing in the first templates
forces a private renderer, a private object model, or a revived project/ADR
component. That clears D1's Approve-if bullets 1 (partially — see below) and 2,
and avoids every Redirect-if trigger.

**Why not a clean approve.** D1's Approve-if has three bullets and the third
("missing needs are promoted to shared primitives") is not yet satisfied, and
one of the six binding templates is not demonstrated:

- **`image_artifact` is a vocabulary slot with no renderable substrate.** All
  three lenses converge here. The wgpu core has exactly one pipeline whose only
  texture is the text glyph atlas (`webgpu.rs:658`; `write_texture` at 2767
  uploads glyphs only). `build_card_vertices` (2995) emits shadow/glow/
  focus-ring/fill/stroke + shaped text — no image quad, no sampled-image draw,
  no decode/upload path. `SceneSnapshot` (`renderScene.ts:183`) has no image
  array and `RenderCard` has no image field. No export produces bitmap pixels
  (`architecture_image` returns only a text `imagePrompt`, `local.ts:175`). The
  "rides the shape/RenderCard path" claim renders a captioned **empty box**, not
  a preview. Presentation (T4.5), wiki evidence, and server-architecture
  `architecture_image` embed depend on real image previews, so this is
  load-bearing — it is a *missing need that must be promoted to a shared
  primitive*, which is exactly D1's Approve-if bullet 3.
- **Investigation map has no demonstrated recipe.** It is an explicit
  Approve-if template (`ai-companion-canvas-task-breakdown.md:457`) and a P4
  deliverable, but it is absent from every P2 universality table (T0.1 §6 and
  T2.1 §6 enumerate only five templates). It would plausibly compose from the
  same nodes/edges/evidence/frames primitives as ADR, but the fragments never
  show it, so its buildability is asserted-by-omission, not proven.

These are gaps inside the primitive/template proof, not Redirect triggers — the
fix is to *promote one primitive* and *demonstrate one recipe*, not to revive a
private component. Hence **approve_with_conditions**, not redirect and not a
clean approve.

**Critical correction the conditions must carry forward.** The
migration-of-old-model lens surfaced a factually wrong premise that the other
two lenses missed: the design repeatedly asserts the demotion is a "no-op for
renderer/core (byte-identical projection)." This is **false**. The Rust core
**does** branch on the decision-graph vocabulary:
`node_type_label()` (`webgpu.rs:4696`) hardcodes the full ADR enum
(`decision_point→"Decision"`, `tradeoff→"Tradeoff"`, `blocker→"Blocker"`, …)
and its output is drawn as a **badge pill on every card** via
`build_card_vertices` (`webgpu.rs:3094-3096`). Style selection is likewise
coupled: `defaultStyles` (`renderScene.ts:206-219`) ships 12 tokens whose ids
*are* the `nodeType` enum values, `nodeToCard` sets `styleKey=node.type`
(`renderScene.ts:284`), and `edgeToRenderEdge` hardcodes
`blocks||trades_off_with → 'risk'` (`renderScene.ts:298`). After demotion, every
migrated card's badge text and style silently degrades unless a template-driven
source replaces them. No P2 task owns this work because it is asserted not to
exist. This does **not** revive a privileged primitive (the core still has no
ADR *object* — only badge text and a style map keyed on a string), so it does
not change the verdict, but it must become an owned task before implementation.

## Per-template buildability table

| Template | Verdict | Composes from | Blocking gap |
|---|---|---|---|
| **Todo / task board** | Buildable | `frame`-per-column + `shape` cards + `tagIds` (status/owner/priority) + `edge` deps + `comment_marker`; identity in `meta.templateKind` | None |
| **Wiki note / idea board** | Mostly buildable | note/idea `shape` + reference `edge` + tags + `comment_marker`; text+shape+edge+tag subset fully real | Evidence-as-image-preview routes to `image_artifact` (no substrate); box-less `text` hit/selection unproven |
| **ADR / design** | Buildable (strongest demotion) | `frame`-per-decision + `shape` cards + `edge` w/ `meta.semanticType` + `comment_marker`; MADR/YADR/mermaid as export presets reading primitives | None for authoring; `meta` loses Zod enum validation (deferred to T4.1); core badge/style coupling needs the migration task below |
| **Server architecture diagram** | Buildable | `shape` boxes + `edge` connectors + `frame` zones/tiers + editable `edge` labels (single `text.rs` engine) | None for authoring; `architecture_image` *embed* inherits `image_artifact` substrate gap |
| **Presentation outline** | Partially buildable | `frame`-per-slide + title/body `text` + `edge` connectors + speaker-note `shape`/`comment` | Slide **images** center the recipe and ride unbacked `image_artifact` (no image path in `SceneSnapshot`/`RenderCard`/`webgpu.rs`) |
| **Investigation map** | Not demonstrated | Plausibly nodes/edges/evidence/frames like ADR, but **no fragment shows the recipe** | Absent from every P2 universality table despite being a binding Approve-if template (`:457`) |

Net: 3 of 6 fully buildable from proven primitives (todo, ADR/design,
server-arch authoring). Wiki and presentation are buildable *except* their
image-preview surfaces. Investigation map is undemonstrated.

## Primitives to promote

1. **Image/texture quad primitive (shared, not per-template).** A renderable
   sampled-image path so `image_artifact` shows real pixels (slide images,
   exported-diagram thumbnails, evidence screenshots). Concretely requires, in
   the wgpu core: a second bind group + image-sampling pipeline, an image
   decode + upload path, a per-image-or-atlas texture strategy, and an
   image-quad emit in the card builder; and in the TS contract: an image array
   on `SceneSnapshot` or an image-kind `RenderCard` variant. This is shared
   across presentation, wiki evidence, and architecture-image embed — promote it
   once, do not special-case it per template.

2. **Investigation-map recipe (demonstration, not new geometry).** Add the
   explicit primitive composition for investigation map to the T0.1/T2.1
   universality tables so the sixth binding D1 template is shown, not assumed.
   Expected to reuse existing `frame`/`shape`/`edge`/`comment_marker`/`tags`
   primitives — if it does, no new primitive is needed, only the proof. If the
   recipe surfaces a genuinely new need, promote that need here before P4.

3. **Settled box-less `text` primitive rule.** A decided hit/selection/resize
   rule for free-standing `text` (or explicit promotion of `text` to its own
   kind), so wiki/presentation text notes rest on a proven primitive rather than
   the currently-deferred open question (T2.1 §1, T2.3 open questions). Lower
   stakes than the image substrate; can be a binding rule rather than new
   geometry.

## Conditions on P4 (templates) and migration

**On P4 / templates:**

- **C1 — Promote the image substrate before any template depends on a preview.**
  Presentation (T4.5), wiki evidence, and the architecture-image embed must not
  be authored against `image_artifact` until the image/texture quad primitive
  (promotion 1) exists, or the templates must explicitly degrade to a documented
  captioned-box placeholder and not claim preview rendering.
- **C2 — Add the investigation-map recipe to the universality proof before P4
  template work begins.** D1 cannot be considered fully discharged while one of
  its six binding templates has no demonstrated primitive composition.
- **C3 — Record the `nodeType → styleKey` decoupling as a sequencing
  dependency on T4.1 (post-D1).** `defaultStyles` ids *are* the `nodeType` enum
  and `edgeToRenderEdge` hardcodes `blocks/trades_off_with → risk`. Until the
  template style-map (T4.1) exists, keep the enums redundantly populated so
  demoted objects retain visual identity. Visual-identity preservation of ADR
  content depends on T4.1, which sits after D1.
- **C4 — Restore semantic validation per template.** The `meta`-bag demotion
  drops Zod enum validation for `nodeType` / `edgeType` / `status` / `confidence`.
  The per-template namespaced validation (deferred to T4.1) must land so the
  approved model's semantic integrity is specified, not just its shape.

**On migration:**

- **C5 — Own the Rust-core badge migration (currently owned by nobody).** Add an
  explicit task to replace `node_type_label()`'s hardcoded decision-graph badge
  (`webgpu.rs:4696`, rendered via `build_card_vertices:3094`) with
  template/`meta`-driven badge text — or document the graceful `'Node'` fallback
  as accepted behavior. The "demotion is a no-op for core / byte-identical
  projection" framing is disproven and must be corrected in the design.
- **C6 — Gate the demotion with a real migration round-trip fixture, not the
  disproven "byte-identical" assertion.** Pin a test proving an existing
  decision-graph Scene (real `decision_point`/`tradeoff` cards,
  `depends_on`/`blocks` edges) round-trips through the demoted `meta` shape **and**
  still renders the correct badge/style and exports correct MADR/mermaid. Legacy
  `meta` backfill is owned by T6.1; this fixture should gate the reshape.
- **C7 — Regression-check the selection-as-ephemeral change.** T2.5 §4 stops
  `select`/multi-select from bumping `sceneVersion` and writes only the ephemeral
  `selection_json` key. This is the correct document/ephemeral classification but
  it changes how the existing select op and MCP `set_selection` behave against
  storage; add an explicit check that renderer/back-compat reads of
  `Scene.selection` are not broken.
- **C8 — Resolve the implicit-root-frame question before templates assume
  free-floating objects.** Required `SceneNode.groupId` (`schema.ts:113`) means
  there is no canonical "object on the bare canvas" state. T2.4 §10 flags this
  and proposes an implicit root frame, deferring to T2.5/T6.1. Several
  template/UX assumptions (free-floating notes, drag-out-to-canvas) ride on it;
  decide it before P4 templates depend on it.

## Lens evidence summary

**primitive-sufficiency → approve_with_conditions.** Confirmed the six vector
primitives compose todo/wiki/ADR/server-arch/presentation-outline/
investigation-map structure with no template-specific renderer object; decision-
graph identity is demotable without breaking geometry (`nodeToCard`
`renderScene.ts:284`; `excludedBusinessFields()` `renderScene.ts:242`); schema
evolution is additive (`schema.ts:153`); typed op vocabulary + collaboration
envelope are a real discriminated union, not a blob (`renderPatch.ts`; events
table at `storage.ts:433`); `actor_marker` correctly scoped ephemeral. **The one
failing finding:** `image_artifact` has no rendering primitive — single
text-atlas pipeline (`webgpu.rs:658`), no image quad in `build_card_vertices`
(2995), exports never emit bitmaps (`local.ts:175`). Promotes the image/texture
quad as a shared primitive.

**migration-of-old-model → approve_with_conditions.** Confirmed demotion
targets are real and precisely identified (`schema.ts:62-82`); additive
evolution holds at schema + persistence (blob columns at `storage.ts:387`, no
ALTER TABLE); the events table genuinely exists and is unused; old `Shape/project`
fully demotes with no surviving privileged container (`migrateLegacyDesign`
`storage.ts:475`); ADR export presets read primitives, not a revived component
(`makeMermaid` `graph.ts:109`); client-UI enum coupling is real but correctly
out of P2 scope (Svelte replacement in P6). **The two failing findings (the
key correction):** the Rust core **does** branch on the decision-graph
vocabulary — `node_type_label()` (`webgpu.rs:4696`) renders a per-`nodeType`
badge, and `defaultStyles`/`edgeToRenderEdge` select style by `nodeType`/
`edgeType` (`renderScene.ts:206-219`, `:298`). "Byte-identical no-op for core"
is false; the badge migration is owned by nobody and the style decoupling depends
on T4.1. Crucially, this does **not** revive a privileged primitive — the core
holds badge text + a string-keyed style map, not an ADR object model — so the
primitive layer remains sufficient.

**first-template-buildability → approve_with_conditions.** Todo and
server-architecture authoring fully buildable; ADR/design is the strongest
demotion story (`model.rs:59-61` opaque strings). **The three failing/partial
templates:** wiki (evidence-as-image leans on hollow `image_artifact`;
box-less `text` unproven), presentation outline (most exposed — slide images
center the recipe on unbacked `image_artifact`), and investigation map (not
addressed in any P2 universality table despite being a binding Approve-if
template, `:457`). Also flagged: `text`/`port` hit kinds exist only in the TS
contract, unverified on the core side that does hit testing. Promotes the image
substrate, an explicit investigation-map recipe, and a settled box-less text rule.

**Convergence.** All three lenses independently reach approve_with_conditions
and independently identify the `image_artifact` rendering substrate as the single
load-bearing missing primitive. The migration lens uniquely (and correctly)
identifies the false "no-op for core" premise; that correction is folded into
conditions C5/C6 and does not change the verdict because it adds a *task*, not a
*revived component*.
