# Object Primitive Redesign — Wave 3 UX Task Breakdown

Branch: `feature/object-primitive-redesign`. Follow-up to Wave 2 (landed on `main`).
21 user requests surfaced during in-browser use, decomposed into 23 tasks. Reviewed
once via the `design` task-graph-review pass; review edits folded in (paths corrected,
RA2 split, App.svelte chain de-serialized under worktree isolation).

Execution rules: see memory `object-redesign-autonomous-mandate` (fully autonomous,
chain workflows, no checkpoints). Constraint this wave: **build-level only, no
browser/GPU pixel verification** (user choice) — so every "diagnose from code" task
must name a falsifiable code-level assertion, not just "compiles". Gates between
tasks: `scripts/renderer-toolchain.sh cargo test --workspace`,
`npm run renderer:rust:test`, `npm run renderer:wasm:build`, `npm run scene:wasm:build`,
`npm run typecheck`, `npm run test:unit`, `npm run build`.

## Execution model — same file ≠ serial

Writers are isolated per git worktree (`sdd`) and merged back. **Same-file is NOT a
serial dependency**; only a true data/contract dependency or an *overlapping edit
region* forces order. Tasks editing disjoint regions of the same file run in the same
wave and merge cleanly. The ownership rules that keep `App.svelte` merges clean are
stated under Critical Path. The only edits that MUST stay serial are flagged
explicitly (e.g. RB2/RB4 both touch `build_scene_geometry`).

## File paths (verified on disk)

- Shell Svelte: `src/client/svelte/{App.svelte, Toolbar.svelte, SettingsModal.svelte}`
- Shell CSS: `src/client/styles.css`
- Shell TS: `src/client/renderer/{engine.ts, scene.ts, objectPrimitives.ts}` —
  note `objectPrimitives.ts` is under `src/client/lib/objectPrimitives.ts`;
  `src/client/lib/{canvasHost.ts, syncEngine.ts, toolbar.ts, shortcuts.ts}`;
  `src/client/renderer/engine.ts`, `src/client/renderer/scene.ts`,
  `src/client/renderer/wasmLoader.ts`.
- Renderer core: `src/renderer/core/src/` — `object_pipeline.rs`, `hit_test_object.rs`,
  `render_object.rs`, `text_layout.rs`, `curve_lod.rs`, `webgpu/{device,frame,input,scene_build}.rs`,
  `shaders/{object_fill,object_stroke}.wgsl`.
- scene-core: `crates/scene-core/src/object/` — `model.rs`, `op.rs`, `commands.rs`,
  `anchors.rs`, `region.rs`, `templates.rs`; `crates/scene-core/src/wasm_api.rs`.

## Locked Decisions

- **D1 Theme (#12)** — Full semantic color token system. scene-core `Paint` gains a
  `Token { name }` variant (the wire contract). Explicit user-picked colors stay raw
  hex (`Paint::Solid`); base styles / shadow / chrome use tokens. Theme toggle = a
  renderer uniform flip (zero rebake, P4); renderer holds light/dark RGBA tables and
  resolves `Token` in `paint_color()`.
- **D2 Shadow (#11)** — Renderer-default drop shadow under every object (not a
  per-object serialized field this wave). Color from a theme token (dark in light
  mode, light in dark mode).
- **D3 Verification (#4/#7/#10)** — Build-level only; treat reported-broken items as
  fix tasks diagnosed from code (no browser). Each must ship a unit/golden assertion
  (see the task cards) that proves the runtime defect is gone without a browser.
- **D4 Anti-aliasing (#16)** — Complete shader analytic AA (populate fill `edge`
  silhouette distance; shaders already do `fwidth` coverage, stroke already AA'd)
  rather than adding MSAA targets. Less invasive, no pipeline/target rewrite.
- **D5 Coarse rotate (#3)** — Shift = 15° snap (90° = 6 ticks), registered as a
  gesture in the C2 catalog.
- **D6 Group (#9/#18)** — Group works on 1+ objects, creates a neutral parent
  container (group already builds a rect parent via `Reparent`; the local var is named
  `frame` — no clipping required). Figma-style hierarchy: double-click an object *with
  children* drills in; double-click a *leaf* enters text edit. `ungroup` only on
  right-click of an object with children; right-click a child shows "pop out one level"
  = `Reparent` to parent's parent (canvas root if none).
- **D7 Text tool removal (#6/#17)** — Remove both Frame and Text shape tools from the
  toolbar AND the `insert-text` context-menu entry. Text is authored by creating a
  rect and double-clicking to type (the object model already allows a `text` field on
  any object).
- **D8 Toast (#19)** — Transient action notices auto-dismiss (~2.5s) with a fade-out;
  persistent hints ("Drag to create…") stay until the tool changes. Timer is
  shell-side (scene-core is pure, no time).

## Coverage Notes / Key Findings (anchors)

- **#8 text never renders**: `build_scene_geometry()` processes fill+stroke only;
  `text_layout.rs` (`layout_runs()`) is never called on the live frame path. The
  contenteditable overlay is why re-pressing Enter "shows" it. Anchors:
  `src/renderer/core/src/object_pipeline.rs:800`, `src/renderer/core/src/text_layout.rs`,
  `src/renderer/core/src/render_object.rs:135`.
- **#1 handle lag**: W2-11 zero-rebake updates only GPU instance matrices;
  `selection_handles()` still reads canonical `region.transform`, so handles jump only
  on commit. Anchors: `src/renderer/core/src/webgpu/scene_build.rs:1981` (`selection_handles`),
  `:2263` (`region_world_bounds`); live preview transform lives on the GPU instance
  buffer set by `src/renderer/core/src/object_pipeline.rs:635` (`set_preview_transform`),
  pushed from `src/client/renderer/engine.ts:1012`.
- **#16 AA**: render target is single-sampled everywhere (`webgpu/device.rs:68`,
  pipelines `object_pipeline.rs:282,356` use default MultisampleState=1). Shaders
  already have analytic AA: `shaders/object_fill.wgsl:84` / `shaders/object_stroke.wgsl:114`
  use `fwidth`. Stroke works; fill `edge` is hardcoded `0.0` at
  `object_pipeline.rs:536-544`. Tessellation is zoom-adaptive LOD (`curve_lod.rs:90`).
  Fix = populate fill `edge` silhouette.
- **#15 drag fails**: body hit-test requires a closed fill polygon
  (`hit_test_object.rs:393`); stroke/text/open/zero-size objects miss and fall through
  to marquee (`webgpu/scene_build.rs:2089`). Also parent drag does not cascade to
  children (`src/client/svelte/App.svelte:201` set-transform applies to one id).
- **#21 eraser**: `emitErase` re-hit-tests per move but returns only the top-most
  object at discrete samples (`src/client/renderer/engine.ts:573`, `hit_test_object.rs:1963`);
  fast drags skip between samples. No swept-segment accumulation. `handleErase`
  `src/client/svelte/App.svelte:677`.
- **#9 group/ungroup inverted**: `ungroup` menu entry lacks `disabledFor:"object"`
  (`src/client/svelte/App.svelte:1115`); menu only checks `picked.kind`, not object
  children/parent (`App.svelte:1131-1148`). `Reparent` op already exists
  (`crates/scene-core/src/object/op.rs:116`). `groupSelection` early-returns on
  `ids.length < 2` (`App.svelte:739-741`).
- **#18 hierarchy**: model has `parent: Option<ObjectId>` (`model.rs:549`), children
  derived by filter; `Reparent` op exists (`op.rs:116`). Missing: drill-in/active-
  container selection state, double-click branch, "pop out" menu entry. Selection is
  flat (`ObjectSelection` Canvas/Object/Multi, `model.rs:630`).
- **#5 color**: `SetStyle` op exists in core (`crates/scene-core/src/object/op.rs:67`)
  and is handled in shell `src/client/lib/syncEngine.ts:108`, but no shell code authors
  it. `PEN_PALETTE` is 6 fixed colors draw-only (`App.svelte:84`); shapes use hardcoded
  `DEFAULT_FILL/STROKE` (`src/client/lib/objectPrimitives.ts:30`); no `<input type=color>`
  anywhere.
- **#2 catalog**: command catalog is the single source (`crates/scene-core/src/object/commands.rs:39`,
  struct `{id,label,category,defaultShortcut?,description,op_kind?}`), rendered
  read-only by `src/client/svelte/SettingsModal.svelte`. Hold-key gestures (Space-pan
  `engine.ts:37`, Shift/Cmd additive `:567`, Alt no-snap `:50`, Alt partial-erase
  `:600`) are all hardcoded, not in the catalog.
- **#4 cursor**: affordance→CSS cursor is wired (`hit_test_object.rs:42` →
  `webgpu/input.rs:63` → `engine.ts:679` → `App.svelte:158` → `styles.css:189`). Treat
  as a code-level diagnosis of the actual gap (D3).
- **#7 marquee / #10 modifier-click**: both wired end-to-end (`scene_build.rs:2098`
  marquee, `engine.ts:554` additive flag → `App.svelte:191` toggle). Diagnose actual
  defect from code (D3).
- **#14 anchoring**: `Anchor{node_index,target,at}` model + `resolve_endpoint`/
  `reproject` exist (`crates/scene-core/src/object/anchors.rs`, `model.rs:461`); snap
  query + Alt-bypass exist (`engine.ts:44`). Missing: synthesize a persistent `Anchor`
  on snapped create and move-together. Drag-create at `App.svelte:625`.
- **#20 templates**: defined in `crates/scene-core/src/object/templates.rs` (lowered
  via `crates/scene-core/src/wasm_api.rs:124` `build_template_pure`); flat positioned
  objects, only connectors carry anchors, no groups/parents. UI list `App.svelte:897`.
- **#19 toast**: `status` string (`App.svelte:109`), rendered `App.svelte:1547`,
  styled `styles.css:101`; persistent, no timer/animation.

## Task Graph

Lane key: **RC-A** = renderer `webgpu/{scene_build,input}.rs` + `hit_test_object.rs`;
**RC-B** = renderer `object_pipeline.rs` + `shaders/*` + `curve_lod.rs`; **SC** =
scene-core; **SH-*** = shell files.

### Contracts (Wave 0)

- **C1 — Theme token contract** · [#12] · SC (`object/model.rs`,`object/op.rs`,new `object/theme.rs`)
  - `Paint::Token{name}` variant + serde; named semantic token set (canvas-bg,
    surface, default-fill, default-stroke, text, shadow, selection-ring, …) with light
    + dark RGBA tables (pure). Verify: scene-core tests + serde round-trip.
  - Gates: RB1, RB3, AP4.
- **C2 — Gesture catalog contract** · [#2,#3] · SC (`object/commands.rs`,new `object/gestures.rs`)
  - Hold-key gesture entry type + register Space/middle pan, Shift/Cmd additive-select,
    Alt no-snap, Alt partial-erase, Shift coarse-rotate(15°). JSON export. Verify:
    scene-core tests + JSON snapshot.
  - Gates: EN1, SM1, SM2, AP5, RA2c (gesture id consumed by quantize gate).

### Renderer RC-A

- **RA1 — Handles follow live transform** · [#1] · RC-A + small RC-B
  - Surface the live preview transform out of `object_pipeline.rs` (a read API next to
    `set_preview_transform:635`) and feed it into `selection_handles()` /
    `region_world_bounds()` so resize/rotate handles track the drag; keep zero-rebake.
  - **Lane straddle**: adds a small getter to `object_pipeline.rs` (RC-B file) +
    edits `scene_build.rs` (RC-A). Merge-safe with RB tasks as long as the getter is a
    new function, not an edit inside `build_scene_geometry`/pipeline setup.
  - Verify: renderer unit test — handle screen positions equal the previewed bbox
    corners mid-drag (not the canonical ones).
- **RA2a — Marquee diagnosis** · [#7-core] · `webgpu/{scene_build,input}.rs`
  - From code, find why the wired marquee doesn't surface; fix. Verify: round-trip
    test — given objects + a marquee rect, `object_regions_in_marquee` returns the
    expected ids and they reach `marquee_ids`.
- **RA2b — Double-click branch** · [#8-dblclick] · `webgpu/input.rs`
  - Double-click on an object *with children* emits a drill-in signal; on a *leaf*
    enters text edit. Verify: input unit test on both branches.
- **RA2c — Coarse-rotate quantize** · [#3-core] · `hit_test_object.rs` · needs C2
  - Quantize `rotate_delta_matrix` (`:327`) to 15° when the coarse-rotate gesture is
    active. Verify: math test — 47° raw with gesture → 45°.
- **RA3 — Hit-test hardening** · [#15-core,#21-core] · `hit_test_object.rs`,`webgpu/input.rs`
  - Body hit for stroke/text/open/zero-size objects (bbox fallback when no fill
    region); eraser swept-segment hit-test accumulating every crossed object between
    samples. Verify: hit-test unit on a zero-fill stroke object; swept-erase test that
    a fast 2-sample drag across 3 objects erases all 3.

### Renderer RC-B (serial within lane — all touch `object_pipeline.rs`)

- **RB1 — Theme resolution + uniform** · [#12b] · needs C1
  - Theme uniform + light/dark RGBA tables; `paint_color()` resolves `Token`; canvas
    clear, focus-ring, default fill/stroke sourced from tokens; toggle = uniform.
    Verify: render unit — same scene renders different clear/chrome RGBA per theme bit.
- **RB2 — Live text render** · [#8] · **[BIG, ~2–3 commits: geometry contract / draw
  pass / run-size de-quant]** · after RB1
  - Wire `text_layout` glyph/text geometry into `build_scene_geometry` + a text draw
    pass; de-quantize run size so committed text renders without re-edit. Verify:
    scene-core/renderer golden test on produced text geometry (not just "compiles").
- **RB3 — Default drop-shadow pass** · [#11] · after RB2 · needs RB1
  - Soft macOS-style drop shadow under every object; **color resolved from the theme
    shadow token** (this is the only link to C1 — must be wired, not hardcoded).
    Verify: render unit — every object emits a shadow primitive; shadow RGBA flips with
    theme bit.
- **RB4 — Fill-edge analytic AA** · [#16] · after RB2/RB3 · **MUST NOT parallelize
  with RB2** (both edit `build_scene_geometry` fill-vertex emit `:536`)
  - Populate the fill `edge` silhouette distance (post-tessellation edge detection);
    shaders already do `fwidth` coverage. Verify: test that boundary fill vertices get
    non-zero `edge`, interior stays 0.

### Shell

- **TB1 — Toolbar cleanup + color** · [#6,#17,#5-UI] · SH-Toolbar
  (`src/client/svelte/Toolbar.svelte`,`src/client/lib/toolbar.ts`)
  - Remove Frame and Text shape tools; promote the color palette to always-visible;
    add a native color-picker popup; expose selected-color state. (Text authoring =
    rect + double-click; the `insert-text` *menu* entry is removed in AP3.)
- **EN1 — Cursor + modifier routing** · [#4,#2-route,#3-shell,#21-shell] · SH-engine
  (`src/client/renderer/engine.ts`,`src/client/lib/canvasHost.ts`) · needs C2, RA2c
  - Route hardcoded Space/Shift/Alt checks through the C2 gesture constants (single
    source); Shift coarse-rotate gate calls RA2c; eraser swept emit per move; diagnose
    & fix the hover-affordance cursor gap. Verify (#4): unit test on the
    `HoverAffordance → CSS cursor` map covering resize-*/rotate/pan; and that a pointer
    over a handle yields the resize/rotate affordance.
- **AP1 — Color apply** · [#5] · SH-App · needs TB1
  - Selected color → new-shape default (`objectPrimitives.ts` styles); recolor the
    selected object by authoring `SetStyle`. Region: insertPrimitive/draw-style paths.
- **AP2 — Selection UX** · [#10,#7-shell,#15-shell] · SH-App · needs RA1,RA2a,RA3,C2
  - Additive modifier-click accumulates multi-select; apply marquee ids; cascade a
    parent drag to its children. **Owns the `hostCallbacks` object (183–239)** and
    `onTransformCommit`. Verify (#10): unit on `toggleObjectSelection` add/remove.
- **AP3 — Group / hierarchy / menus** · [#9,#13,#18] · SH-App · needs RA2b
  - Flip `groupSelection` guard `< 2` → `< 1`; group 1+ into a neutral parent;
    drill-in active-container state; `ungroup` enabled only for an object with children
    (add `disabledFor` + property check); child "pop out one level" via `Reparent` to
    parent's parent; meaningful empty-canvas menu; **remove the `insert-text`
    CANVAS_MENU entry + its `contextHandlers` mapping (`App.svelte:1125,1163`) per D7**.
    **Owns the menu block (1110–1167) + group/ungroup (739–778).**
- **AP4 — Theme shell** · [#12c] · SH-App + `src/client/styles.css` +
  `src/client/renderer/scene.ts` · needs C1,RB1
  - Dark-mode CSS variables + theme toggle (persisted) + migrate `scene.ts`
    `defaultStyles` to token refs. Region: a theme-toggle state var + CSS (separate
    file). Disjoint from AP2/AP3/AP5/AP6.
- **AP5 — Drag-create anchoring** · [#14] · SH-App + `src/client/renderer/engine.ts` · needs C2
  - On snapped create, synthesize a persistent `Anchor`; anchored objects move with the
    target; Alt bypasses (existing). **Owns `handleCreate` (608–644).**
- **AP6 — Toast** · [#19] · SH-App + `src/client/styles.css`
  - Transient action notices auto-dismiss with fade-out; persistent hints stay. Region:
    `status` state + a dismiss timer + `.canvas-status` markup/`@keyframes`. No
    dependency on any other AP task.

### Misc / Templates / Integration

- **SM1 — Settings modal gesture docs** · [#2-render] · `src/client/svelte/SettingsModal.svelte` · needs C2
- **SM2 — CLAUDE.md structure note** · [#2-doc] · `CLAUDE.md` · needs C2
  - Document the single-source shortcut/gesture catalog (register a feature once → it
    self-documents in the settings modal). **Precondition: load + apply the
    `write-instructions` skill before editing CLAUDE.md (it is agent instructions too).**
- **TP1 — Improve built-in templates** · [#20] · `crates/scene-core/src/object/templates.rs`
  (+ UI list in `App.svelte:897`) · needs AP3,AP5,C1
  - Use groups + anchors + tuned positions; replace placeholder stubs.
- **IG1 — Full gate + integration** · [all] · run every gate; integration tests for the
  live paths; update `object-redesign-progress` memory.

## Waves

| Wave | Tasks |
|---|---|
| 0 | C1, C2 |
| 1 | RA1, RA2a, RA2b, RA2c, RB1, TB1, SM1, SM2, AP6 |
| 2 | RA3, RB2, EN1, AP1 |
| 3 | RB3, AP2, AP4 |
| 4 | RB4, AP3, AP5 |
| 5 | TP1 |
| 6 | IG1 |

Wave-1 RC-A (RA1/RA2a/RA2b/RA2c) run in parallel only because they edit disjoint
*functions* of the shared RC-A files; if merges conflict, fall back to one RC-A owner
doing them as ordered commits. RB4 is intentionally *not* in the same wave as RB2.

## Critical Path

The App.svelte tasks have **no inter-AP dependencies** — every prior "AP→AP" edge was
"same file" only, which worktree isolation dissolves for these disjoint regions. They
now fan out under their renderer/contract deps. Ownership rules that keep `App.svelte`
merges clean (state in the cards): AP2 owns `hostCallbacks`; AP3 owns the menu block +
group/ungroup; AP5 owns `handleCreate`; AP6 owns the `status`/toast region; AP1 owns
the insert/draw-style paths; AP4 is mostly CSS + `scene.ts`. No two AP tasks rewrite
the shared `selection`/`status` `$state` *declarations* — they only assign within their
own functions.

Real longest poles (not App.svelte): `C1 → RB1 → RB2 → RB3 → RB4` (renderer object
pipeline, serial) and `C2 → RA2b → AP3 → TP1 → IG1`. Both ~5 levels — the renderer
lanes, not the shell, gate the schedule.

**App.svelte module split — NOT recommended.** The regions are already cleanly
separable by function, so the parallel win above needs zero refactor. A split is itself
a whole-file single-owner edit (serial bottleneck) that delays the fan-out it's meant
to enable, changes reactive `$state`/`$derived` wiring (high-risk under no-browser
verification), and is speculative scope outside the 21 requests. If ever wanted, do it
as its own deliberate refactor wave with browser verification.

## Deferred

- Per-object editable shadow field; full gradient/image paint theming → on the C1 token
  contract later.
- Browser/GPU pixel verification (marquee overlay, text raster, shadow blur, AA) →
  build + unit only this wave (no-browser constraint).

## Coverage check

All 21 requests map: #1 RA1 · #2 C2/EN1/SM1/SM2 · #3 C2/RA2c/EN1 · #4 EN1 · #5 TB1/AP1
· #6 TB1 · #7 RA2a/AP2 · #8 RB2/RA2b · #9 AP3 · #10 AP2 · #11 RB3 · #12 C1/RB1/AP4 ·
#13 AP3 · #14 AP5 · #15 RA3/AP2 · #16 RB4 · #17 TB1/AP3 · #18 AP3 · #19 AP6 · #20 TP1 ·
#21 RA3/EN1. No request dropped.
