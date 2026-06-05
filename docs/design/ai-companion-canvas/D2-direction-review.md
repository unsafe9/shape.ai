# Decision D2 — AI Companion Canvas Direction Review

Synthesis of three adversarial lens reports (`architecture-portability`,
`product-coverage-and-lod`, `mcp-and-migration-boundary`) against D2 in
`docs/ai-companion-canvas-task-breakdown.md:906`.

D2 question: Is the next implementation wave still the universal AI companion
canvas, rather than a narrow engineering-only diagram editor or a generic
design-tool clone?

- **Approve if:** approved package/module boundaries still support web, macOS,
  and future iOS; primitive/template separation still covers the first templates
  without special-case renderers; good LOD keeps object identity stable while
  making large scenes performant; the MCP companion tracker still provides useful
  agent observability instead of only decorative animation; Svelte owns the web
  shell and Rust/core owns canvas performance-sensitive behavior.
- **Redirect if:** the product should narrow back to engineering architecture
  work only; it should prioritize full Figma-like vector editing over AI-agent
  work memory; the selected layout creates migration cost without improving
  portability; React migration or Rust boundary work should be split into a
  separate migration program before implementation.

## Verdict

**approve_with_conditions.**

**No Redirect-if trigger is tripped, and that is the load-bearing finding.** All
three lenses independently confirm it, against real code:

1. **The product is not narrowing to an engineering-only diagram editor.** The
   entire decision-graph identity (`nodeType` / `edgeType` / `nodeStatus` /
   `confidence` / `evidenceRefs` / `childDecisionIds` / `rationale`) demotes into
   `meta` + `styleKey`; the old `Shape/project` container and the structured-ADR
   component are removed from the primitive layer (project → ordinary
   `frame.title`, ADR structure → template recipe + export preset). ADR and
   server-arch are five of six templates, not the base model. No privileged
   engineering container survives the migration write-path (T2.1, T6.1).
2. **The product is not sliding toward a Figma vector clone.** Explicit
   Non-Goals — no path editing, no boolean geometry, no auto-layout, no component
   system — are honored throughout. Freeform hulls are a *derived*
   `pad(union(member_bounds))` render-only polygon with no user-editable
   vertices or boolean ops (T2.4 §3); templates carry no editing mode (T4.1); the
   presentation floor holds (no play mode, theme engine, slide object type,
   reflow, WYSIWYG, PPTX — T4.5 §11).
3. **The selected layout does not create migration cost without portability
   gain.** The core/web-adapter/Metal-adapter/Svelte-shell/server split genuinely
   supports web, macOS, and future iOS without forking the semantic scene model:
   `SceneSnapshot` / `RenderScenePatch` / `CanvasInputEvent` / `CoreHitResult` /
   `CoreOverlayRequest` are plain serde structs the core ingests/emits; the
   platform-specific surface (GPU device/surface, input source, text-overlay
   widget, display scale, boundary serialization) sits entirely in the adapter;
   `wgpu` already vendors a Metal backend, so the heaviest native lift is a
   backend swap, not a renderer rewrite (T1.1, T1.3). Schema evolution is
   evolve-in-place and additive — demoted fields already live inside
   `node_json` / `edge_json` blob columns, so demotion is a JSON reshape with no
   `ALTER TABLE` (T2.1, T6.1).
4. **React/Rust boundary work does not need a separate migration program.**
   T6.4 sequences a single feature branch S0–S4 with two ordered mandatory gates
   (GATE-R Rust boundary before GATE-X React removal), build/test/typecheck green
   at every commit. The Rust boundary is certified on the pre-Svelte shell, and
   React removal lands on a locked boundary. This *is* the correct disposition —
   gated inside one implementation branch — not a deferral.

So the binding directional question is answered: the next wave is still the
universal AI-companion canvas. Each Approve-if criterion is satisfiable as a
design, which clears the Approve-if bar in principle.

**Why not a clean approve.** Three Approve-if criteria are satisfied as *design*
but rest on artifacts that do not yet exist as built or authored units, and two
design claims are stated more strongly than the code currently supports. These
are gaps between an architecturally-correct direction and code/docs that
currently only realize the web/WASM/WebGPU slice — they add owned tasks, not a
redirect. Hence **approve_with_conditions**.

## Criterion-by-criterion (Approve-if)

| # | Approve-if criterion | Status | Basis |
|---|---|---|---|
| 1 | Approved package/module boundaries support web, macOS, future iOS | **AT-RISK** | Boundary *design* is sound and code-grounded (plain serde payloads, adapter-isolated platform surface, vendored Metal backend), but (a) the platform-neutral core does not exist as a buildable unit — `ShapeWebGpuRenderer` + the op/stats types are entirely `#[cfg(feature="wgpu-probe")]`-gated, `default == wgpu-probe`, and `web-sys`/`wasm-bindgen` are non-optional deps, so today there is no DOM-free non-WASM compilation path of the core; and (b) T0.2 (package boundaries) and T0.3 (target layout placement + migration map) are unauthored, so Redirect-if-3 cannot be judged against a costed migration plan, only against the P0 rollup's reconstructed Option B. |
| 2 | Primitive/template separation covers first templates without special-case renderers | **AT-RISK** | All six binding templates (todo, wiki/idea, ADR/design, server-arch, presentation, investigation-map — the sixth now demonstrated in T4.4 §6) lower entirely to existing create-group/create-card/create-edge/create-tag ops with zero new renderer object, op kind, or schema field; identity is an ordinary `meta.templateKind` string. The one real gap is image-preview coverage: the `image_artifact` rendering substrate (D1 C1) is confirmed still absent (single glyph-atlas texture, one pipeline), so presentation slide images, wiki evidence-as-image, and server-arch `architecture_image` embed degrade to captioned-box placeholders. This is a missing need promoted to ONE shared primitive — it does **not** trip the special-case-renderer Redirect trigger — but coverage is only fully true for non-image surfaces until it lands. |
| 3 | Good LOD keeps object identity stable while making large scenes performant | **MET** (perf evidence pending) | T3.1 is a fixed paint-only tier ladder (Full/Compact/ShapeOnly/Density/Minimap) selected per-object-per-frame by `apparent_px`, with id/position/bounds/selection/hit-test identity as BINDING invariants tied to real symbols (`vertex_ranges`, `selection_world_rect`, `CoreHitResult`); dot↔card swaps and N→1 clustering forbidden; a Minimap-tier card is still hoverable/selectable/MCP-targetable. Group containment identity (`groupId`/`parentGroupId`) is never read or altered by LOD. Real streaming/residency behind it (T3.2: uniform-grid spatial index, viewport+zoom SQL query, Hot/Warm/Cool/Cold residency, LRU eviction, prefetch ring) over named existing symbols; heterogeneous fixtures (T3.3) exercise the actual template mix. Policy is fully MET; the LOD threshold constants and memory budgets are provisional seeds pending T3.4 latency measurement, so "performant" is a credible plan with the measurement still outstanding. |
| 4 | MCP companion tracker provides useful agent observability, not decoration | **AT-RISK** | All four observability pillars are designed against real SDK + codebase surfaces: IDENTITY from the actual MCP `Implementation` payload with deterministic per-clientId color and lifecycle-driven dock states; TARGET mapping every one of the 10 registered tools to a concrete `CanvasTarget` reusing existing Rust `selection_world_rect`/`world_rect_to_screen_rect`; FOLLOW as a pure ephemeral controller over the existing `focusBounds`/`setCamera` handle with a `gesture.active` interlock; TRACE derived from the T2.5 events log + read ring, with pulses, an activity-trail drawer, and a ghost write-preview. The risk: HTTP identity depends on an unbuilt, human-decision-gated transport change — `src/server/index.ts:75-76` runs `sessionIdGenerator: undefined` (stateless), so two concurrent identical HTTP clients collapse to one dock entry until `/mcp` is made stateful. The whole envelope foundation (clientId/actorId/targetIds/events INSERT) is also design-only today (grep-clean in `src/`). |
| 5 | Svelte owns the web shell; Rust/core owns canvas performance-sensitive behavior | **MET** | The shell↔canvas seam is already the narrow imperative `RendererCanvasHostHandle` + `EngineEvent` pair, so Svelte wraps the same adapter rather than rewriting the engine. T6.3 audits all 12 canvas responsibilities to real owners — 9 already Rust/core (`screen_to_world`, `hit_scene_at_screen`, `selection_world_rect`, `build_draw_list`, `text.rs`), 2 a deletable JS parity residue (`scene.ts`, scheduled for deletion not promotion), 1 an `App.tsx` clipboard leak relocated to the shell/template layer. The real anti-pattern (`RendererCanvasHost.tsx:180` `useEffect([camera])→setCamera` write-back) is correctly removed, making camera one-way/core-authoritative. Boundary contract tests + a structural import-graph test (production `src/client` imports only TYPES from `scene.ts`) back it. The MCP companion arrives as ordinary ephemeral render primitives, not core logic and not Svelte canvas-state. |

Net: 2 of 5 MET, 3 AT-RISK. None is a redirect; the AT-RISK criteria are
design-correct but gated on unbuilt artifacts (buildable portable core, image
substrate, MCP stateful transport) or unauthored docs (T0.2/T0.3), captured as
conditions below.

## Conditions / required follow-ups

These conditions carry forward, and explicitly inherit the D1 conditions (the
image substrate and the core badge migration are the same load-bearing items in
both decisions).

**On portability and package layout:**

- **C1 — Assign ONE owned task to produce a buildable platform-neutral core.**
  Make `web-sys`/`wgpu` optional, ungate the op/stats types and a portable
  `SceneRenderer` from `wgpu-probe`, split the WASM binding from the portable
  engine, inject the GPU surface, and prove the core compiles without `web-sys`.
  Reconcile the contradictory claims: T1.1 marks "core has no browser DOM
  assumption" MET while T1.3 honestly marks the same bullet PARTIAL, and T6.3 §2.1
  frames the fix as "a pure `#[cfg]` removal, no logic" — that framing is
  understated. Until this task exists and passes, iOS-readiness is design-only,
  not demonstrated.
- **C2 — Author T0.2 and T0.3 (or fold them into one layout-decision task) with
  a real path-by-path migration map and import/build/test impact.** Cost the
  `src/`→boundary-name rename's churn-vs-legibility tradeoff and the two known
  build-coupling re-points (`renderer:wasm:build` out-dir into
  `client/renderer/wasm`; `tsconfig.server.json` includes), so Redirect-if-3 is
  judged against an actual plan rather than the P0 rollup's reconstructed Option
  B. The recommended in-place Option B looks low-risk but is currently uncosted.

**On product coverage and the demoted model:**

- **C3 — Make the image/texture-quad primitive a scheduled, owned task (D1 C1),
  not just a D1 condition.** It is the single load-bearing missing primitive on
  the product-coverage lens. Concretely: a second bind group + image-sampling
  pipeline, a decode/upload path, a per-image-or-atlas strategy, an image-quad
  emit, and a `SceneSnapshot` image array / `RenderCard` image variant. Promote it
  once as a shared primitive — do not special-case per template — before
  presentation, wiki-evidence, or server-arch-image templates claim image-preview
  coverage.
- **C4 — Own the Rust-core badge/style decoupling (D1 C5, currently owned by
  nobody).** Replace `node_type_label()`'s hardcoded decision-graph badge
  (`webgpu.rs:4696`, rendered via `build_card_vertices`) and the
  `defaultStyles`/`edgeToRenderEdge` enum coupling with template/`meta`-driven
  text, or document the graceful `'Node'` fallback as accepted. This is unowned
  engineering/ADR vocabulary inside the supposedly business-free portable core;
  it must become an owned task so engineering semantics do not re-accrete in the
  core along the macOS/iOS reuse path and so demoted ADR/decision/investigation
  cards keep visual identity. The "byte-identical no-op for core" framing is
  disproven and must be corrected in the design.
- **C5 — Land per-template `meta` validation and a settled box-less `text` rule
  (D1 C4 + D1 promotion 3).** The `meta`-bag demotion drops Zod enum integrity;
  per-template namespaced schemas must specify the demoted model's semantic
  integrity, not just its shape. Settle the free-standing `text`
  hit/selection/resize rule (deferred to T2.2/T2.3) before wiki/presentation text
  notes rest on it.

**On MCP observability and migration:**

- **C6 — Make the MCP-HTTP stateful-transport decision explicit and binding
  before P5 build.** Turn `/mcp` stateful via `sessionIdGenerator` + a
  `Map<sessionId, McpClientIdentity>`, keeping the registry ephemeral and out of
  Scene/SQLite. Without it, identity for concurrent HTTP clients is not
  deliverable — the weaker stateless fallback collapses two identical clients to
  one dock entry. Carry this as an explicit human decision, not an assumption that
  identity is solved.
- **C7 — Settle HTTP disconnect detection and an events-log retention policy.**
  Naive "stream close = disconnected" flaps every client between tool calls; key
  presence off session expiry/heartbeat for the "disconnected" dock state. The
  `events` log becomes both the primary edit unit and the trace read source (with
  an unindexed `json_extract` clientId filter) — define retention/compaction
  before large-scene history or long-lived workspaces degrade trace queries.
- **C8 — Honor the GATE-R-before-GATE-X ordering and the vitest react() catch.**
  Keep React removal gated on a certified Rust boundary in one branch (no dual
  shell, no compat layer). T6.4 independently caught that `vitest.config.ts` also
  loads `react()` (under-counted by T6.2) — without removing it,
  `@vitejs/plugin-react` cannot be fully uninstalled; this is folded into GATE-X
  and must stay there.

**On reviewer-facing documentation:**

- **C9 — Author P1/P3/P5/P6 phase rollups (or accept their absence explicitly).**
  All three lenses flag that only P0 and D1 rollups exist on disk; phase detail
  lives entirely in `tasks/T*.md`. A reviewer approving D2 must reconstruct the
  MCP-observability and migration-boundary story from 9 task files. Either
  consolidate P5/P6 (and P1/P3) rollups or record that D2's evidence base is the
  task fragments, not phase-level reviewed rollups.

## Recommended next implementation wave

Sequence the binding-condition work first so the AT-RISK criteria become MET
before the dependent template and MCP build waves run. This refines the
breakdown's Wave A–L ordering, not replaces it.

1. **Layout decision (C2).** Author T0.2 + T0.3 with the costed migration map and
   build-coupling re-points. This is Wave A and the current "Next Unblocked"
   front; it gates any physical file move and makes Redirect-if-3 judgable. Do not
   move files until the map exists.
2. **Buildable portable core (C1) + core badge/style decoupling (C4).** One owned
   refactor task to ungate the `SceneRenderer`/op/stats types, make
   `web-sys`/`wgpu` optional, split the WASM binding, and prove a non-`web-sys`
   compile — folding in the `node_type_label()`/`defaultStyles` decoupling so the
   portable core is literally business-free. This converts criterion 1 from
   AT-RISK to MET and removes the unowned engineering residue. It can run
   alongside Wave B (T1.x boundary contracts) since both define the core/adapter
   seam.
3. **Image/texture-quad primitive (C3).** Promote the shared image substrate
   before any image-dependent template (presentation, wiki-evidence,
   server-arch-image) is authored — i.e. before Wave H. This is the single
   load-bearing missing primitive and converts criterion 2's image surfaces from
   placeholder to real coverage.
4. **MCP stateful-transport decision (C6) + envelope foundation.** Make the
   binding `/mcp` stateful decision, then land T2.5 (op envelope) → T5.1 (stable
   clientId) → T5.2 (target mapping) before the P5 companion build (Wave G/H), so
   identity/target observability is deliverable, not asserted. Settle disconnect
   detection and events-log retention (C7) alongside.
5. **LOD calibration (criterion 3 evidence).** Run T3.4 latency measurement
   against T3.3 heterogeneous fixtures to replace the provisional LOD thresholds
   and memory budgets with measured values, turning "performant" from credible
   plan into demonstrated evidence (Wave F).
6. **React→Svelte migration under the gates (C8).** Execute T6.4's single-branch
   S0–S4 with GATE-R before GATE-X, carrying the camera-write-back fix and
   JS-parity demotion into `lib/canvasHost.ts` once (T6.3 before T6.2), and
   removing the vitest `react()` plugin as part of GATE-X (Wave I–L).

Per-template validation (C5) and the phase rollups (C9) ride alongside their
owning waves (C5 with Wave G/H template authoring, C9 before the next review
gate).

## Lens evidence summary

**architecture-portability → approve_with_conditions (do not redirect).** The
core/web-adapter/Metal-adapter/Svelte-shell/server split genuinely supports web,
macOS, and iOS without forking the semantic scene model; product/business logic
is kept out of renderer adapters (the one real leak — `App.tsx` clipboard's
`formatNodeMarkdown` — is evicted to shell/template); schema migration is
additive in-place; no premature native scaffold is treated as architecture. **The
failing findings:** the platform-neutral core does not exist as a buildable unit
(everything is `#[cfg(feature="wgpu-probe")]`-gated, `web-sys`/`wasm-bindgen`
non-optional), the "core has no DOM assumption" claim is overstated against the
code (T1.1 MET vs T1.3 PARTIAL for the same bullet), T0.2/T0.3 are unauthored so
the layout migration cost is uncosted, and `node_type_label()`/`defaultStyles`
embed ADR vocabulary in the core. These add owned tasks (C1, C2, C4), not a
redirect.

**product-coverage-and-lod → approve_with_conditions.** Primitive/template
separation covers all six first templates — including the now-demonstrated
investigation map (T4.4 §6) — with no special-case renderer, no private object
model, no revived project/ADR component; the good-LOD policy is genuine visual
degradation with binding id/position/bounds/selection/hit-test invariants, group
containment identity preserved, and real streaming/residency/eviction (T3.2) over
heterogeneous fixtures (T3.3); both engineering-only-narrowing and
Figma-vector-clone regressions are well-defended. **The one load-bearing gap:**
the `image_artifact` rendering substrate is confirmed absent (single glyph-atlas
texture, one pipeline), so image-preview coverage degrades to captioned boxes —
promoted once as a shared primitive (C3), it does not trip the special-case
trigger. Also carried: the nodeType→badge/style coupling (C4).

**mcp-and-migration-boundary → approve_with_conditions.** The MCP companion
tracker provides real agent observability (identity from the actual
`Implementation` payload, per-tool `CanvasTarget` mapping reusing existing Rust
geometry, ephemeral follow over the existing `focusBounds`/`setCamera` handle, a
derived trace + ghost preview off the unused events log) rather than decoration;
Svelte cleanly owns the web shell while Rust/core owns performance-sensitive
behavior; React is fully removed with no dual shell or compat layer; the
migration is gated inside one branch (GATE-R before GATE-X), not split into a
separate program. Independently verified: exactly 10 `.tsx` files import React,
both vite AND vitest load `react()` (T6.4 caught T6.2's under-count), the events
table is genuinely unused, the `[camera]→setCamera` write-back exists at
`RendererCanvasHost.tsx:180` and is slated for removal, and the HTTP MCP transport
is stateless. **The failing findings:** HTTP identity depends on an unbuilt
stateful-transport decision (C6), the whole observability/envelope foundation is
design-only in `src/`, and disconnect detection + events-log retention are
unresolved (C7).

**Convergence.** All three lenses independently reach approve_with_conditions and
independently confirm that **no Redirect-if trigger fires** — the next wave is
still the universal AI-companion canvas, not an engineering-only editor, not a
Figma vector clone, with the layout improving portability and the React/Rust work
correctly gated inside one branch. The shared load-bearing conditions are: the
image substrate and core badge migration (inherited from D1), the
not-yet-buildable platform-neutral core, the unauthored layout migration map, and
the MCP stateful-transport decision. Each adds an owned task or a binding human
decision, not a revived component or a redirect, which is why the verdict is
**approve_with_conditions** rather than redirect or a clean approve.
