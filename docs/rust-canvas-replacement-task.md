# Rust Canvas Replacement Task

이 파일은 장기 실행 작업의 단일 `goal` 파라미터로 넘길 수 있는 self-contained task brief다.

## Objective

shape.ai의 기존 React DOM/SVG 기반 scene canvas를 완전히 대체한다. 최종 캔버스는 웹 DOM/CSS 렌더링에 기대지 않고 Rust/WASM/WebGPU 단에서 scene, camera, hit testing, layout, text shaping, graphics effects, selection geometry, GPU buffer/cache를 소유해야 한다. 웹 기술은 제품 shell, floating UI, API/MCP/business state, 개발용 debug drawer, active native text input overlay처럼 필요한 최소 영역에만 남긴다.

완료 후에는 검증된 renderer 구현을 production 구조로 승격하고, 기존 DOM/SVG scene canvas path를 제거한다. 기존 path를 장기 fallback으로 남기지 않는다.

## Locked Decisions

- Canvas object의 정상 렌더링은 live DOM element가 아니라 Rust renderer scene object다.
- 전체 CSS layout engine을 새로 만들지는 않는다. 대신 shape.ai 카드/그룹/엣지에 필요한 graphics library 계층을 Rust에 만든다.
- HarfBuzz급 shaping을 도입한다. 현재 bitmap fallback text는 production 품질로 보지 않는다.
- Korean text, mixed Latin/CJK text, punctuation, no-space long text가 깨지지 않아야 한다.
- Active text editing은 native DOM textarea/input overlay를 사용할 수 있다. IME, selection, copy/paste를 Rust에서 직접 재구현하지 않는다.
- Active text input overlay도 현재처럼 못생긴 임시 textarea가 아니라, Rust가 계산한 text metrics/style token을 받아 카드 내부 입력처럼 보여야 한다.
- Comments, artifacts, exports, MCP/API, product business semantics는 TypeScript app/server layer에 남긴다.
- 디버그 패널은 삭제하지 않는다. 개발 단계에서는 버튼 안에 숨겨진 diagnostics drawer로 유지한다.
- Earlier risky experiments lived in an isolated POC package. The stable renderer implementation is now promoted into production-owned paths.
- 최종 단계에서는 기존 구현을 완전히 제거하고 renderer engine path가 production canvas를 대체한다.

## Current Final State

Repo root: `/Users/wshan/workspace/shape.ai`

Current production renderer:

- `src/client/App.tsx`: product shell, panels, commands, persistence wiring, selection state, floating UI, and `RendererCanvasHost` mount.
- `src/client/components/RendererCanvasHost.tsx`: production renderer host, WebGPU canvas lifecycle, Rust input bridge, native text overlay, and renderer availability handling.
- `src/client/components/RendererDiagnosticsDrawer.tsx`: hidden development diagnostics drawer.
- `src/client/components/SelectedNodeInspector.tsx`: selected-node shell UI outside the canvas renderer.
- `src/client/renderer`: production TS facade for engine lifecycle, input/patch/frame/debug batching, scene contract, fixtures, benchmark support, and WASM loading.
- `src/renderer/core`: Rust/WASM scene core and visible WebGPU renderer.
- `src/renderer/core/src/model.rs`: render scene, patch contract, input contract, overlay request contract, and debug snapshot model.
- `src/renderer/core/src/webgpu.rs`: WebGPU probe, retained renderer, culling, hit testing, dirty buffer updates, shaped text, glyph atlas, camera/focus handling, and frame stats.
- `src/shared/renderScene.ts`: production app `Scene` to renderer `SceneSnapshot` adapter.
- `src/shared/renderPatch.ts`: renderer patch to app `ScenePatch` translation.
- `src/shared/schema.ts`: canonical app `Scene`, `Group`, `Node`, `Edge`, `Tag`, `Comment`, `Artifact`, `selection`.
- `src/shared/graph.ts`: graph helper semantics for exports, labels, bounds, and product shell operations.
- `src/server/index.ts`: `/api/scene`, `/api/scene/render-snapshot`, comments, tags, exports, MCP routes.
- `tests/infinite-canvas-renderer.test.ts`: render contract, fixture, adapter, app patch translation coverage.
- `docs/renderer/evidence`: historical renderer prototype evidence preserved after production promotion.

Verification status:

- Rust core tests and generated WASM builds are available through normal repo scripts:

```bash
npm run renderer:rust:test
npm run renderer:wasm:build
```

The scripts prefer repo-local `.renderer-toolchains` when present and fall back to system `cargo`/`wasm-pack` otherwise.

## Done Criteria

The task is complete only when all are true:

- Production canvas renders through the Rust/WASM/WebGPU engine by default.
- Existing DOM/SVG scene canvas implementation is removed, not kept as long-lived fallback.
- Rust owns retained scene rendering, camera, hit testing, text layout/shaping, selection geometry, culling, GPU buffers, style/effect rendering, and frame/debug stats.
- Web shell owns only product panels, commands, API/MCP/business state, floating UI, diagnostics drawer, and active input overlay.
- Group frames, node cards, text snippets/detail, edge curves/labels, selected/focus state, ports/handles, shadows, gradients, rounded corners, strokes, badges, and polished text input visual states are rendered or coordinated by the new engine path.
- Text rendering uses HarfBuzz-grade shaping/font fallback/glyph atlas logic, with Korean and mixed text quality verified.
- Active text input overlay aligns to Rust text metrics across pan/zoom and passes Korean IME, selection, copy/paste, commit, cancel, blur behavior checks.
- Current workflows still work: group create, tag attach/filter, node create/edit/delete/duplicate/copy/paste, z-order, edge create/select/delete, pan/zoom/fit/fullscreen, comments, export, selection persistence, MCP/API compatibility.
- Debug panel is hidden behind a development button/drawer and still exposes renderer health, frame time, visible counts, glyph/cache stats, dirty writes, buffer growth/compaction, hit result, selected object, and backend availability.
- Automated verification and browser/visual QA pass.

## Verification Commands

Run these at appropriate gates:

```bash
npm run typecheck
npm run test:unit
npm run build
npm run renderer:test
npm run renderer:rust:test
npm run renderer:wasm:build
```

Browser verification must include:

- Production renderer opening successfully in a WebGPU-capable browser.
- Canvas nonblank pixel checks.
- Desktop and mobile viewport screenshots.
- Pan/zoom/fit benchmark on 1k+ cards/edges.
- Real backend scene load/save through `/api/scene`.
- Korean IME text edit and overlay alignment across zoom levels.

Future browser/e2e verification should use the available Browser agentic workflow. Playwright tests, config, and direct project dependency are intentionally removed.

## Task Graph

### P0. Replacement Contract Refresh

Outcome: Update the replacement plan around the strengthened requirement: Rust owns the canvas completely, web only handles minimal shell/overlay responsibilities.

Actions:

- Reconcile `docs/infinite-canvas-engine-task-breakdown.md` and renderer evidence docs against this file.
- Mark earlier "production replacement not ready" conclusions as historical evidence, not current objective.
- Add explicit acceptance criteria for HarfBuzz text, Rust graphics effects, polished input overlay, hidden diagnostics drawer, and legacy removal.

Verify:

- A reviewer can tell exactly what must be true before deleting the old DOM/SVG path.
- No task silently shrinks the requirement to "prototype mount" or "better bitmap text".

Depends on: none

Stop or ask if:

- The product direction changes back to keeping DOM/CSS-rendered canvas objects.

### P1. Toolchain And Build Reproducibility

Outcome: Rust/WASM build is reproducible from normal repo scripts.

Actions:

- Fix `renderer:wasm:build` or add a small script that uses `.renderer-toolchains` when system `cargo`/`wasm-pack` are unavailable.
- Add a Rust core test command to package scripts or docs.
- Keep generated WASM glue gitignored unless intentionally promoted.

Verify:

- Clean shell can run the Rust tests and WASM build command without manual env discovery.
- `npm run renderer:test` still passes.

Depends on: P0

Stop or ask if:

- The desired policy is to require system Rust instead of repo-local `.renderer-toolchains`.

### P2. Rust Canvas Ownership Boundary

Outcome: The TS `ShapeCanvasEngine` facade stops owning canvas behavior that should be Rust-owned.

Actions:

- Move camera/input interpretation, hit target classification, overlay request generation, selection geometry, and patch batching into Rust-facing APIs.
- Narrow the web facade to canvas lifecycle, API persistence, shell event routing, DOM overlay mount, diagnostics drawer, and renderer availability messages.
- Replace many small JS/WASM calls with batched input, patch, frame, and debug snapshot calls.

Verify:

- Public engine boundary is small and stable: load scene, resize, apply patch batch, input batch, render frame, overlay request, debug snapshot.
- Pan/zoom/select/drag/edge creation still work in the renderer path.
- Boundary call stats remain visible.

Depends on: P1

Stop or ask if:

- Moving a responsibility into Rust would pull product business rules into the renderer.

### P3. Rust Graphics Library Layer

Outcome: Replace ad hoc rectangles with a reusable graphics layer for shape.ai's visual design.

Actions:

- Define Rust render primitives for filled/stroked rounded rects, gradients, shadows, inner strokes, badges, separators, labels, focus rings, edge labels, and ports.
- Define style tokens richer than current fill/stroke/text/accent: radius, shadow, blur/glow, stroke widths, typography, spacing, state variants.
- Port the current CSS visual language from `src/client/styles.css` into renderer-owned style tokens and draw commands.
- Keep the scope shape.ai-specific. Do not build a generic browser CSS engine.

Verify:

- Rust-rendered group/card/edge visuals visually match or improve the current DOM/CSS design.
- Screenshots cover default, selected, hover/focus, edge selected, group selected, input active, low/high zoom.

Depends on: P2

Stop or ask if:

- A requested visual effect requires a full CSS layout engine or browser-only rendering.

### P4. HarfBuzz-Grade Text Rendering

Outcome: Text becomes production-quality Rust-rendered text, not bitmap fallback.

Actions:

- Choose and integrate a HarfBuzz-grade shaping path suitable for WASM/WebGPU.
- Implement font loading, font fallback, shaping, glyph rasterization/vector upload, glyph atlas, line wrap, clipping, ellipsis where needed, and cache invalidation.
- Support Korean and mixed Latin/CJK text as first-class cases.
- Expose glyph/fallback/cache metrics in diagnostics.

Verify:

- Tests cover Korean, Latin, punctuation, mixed text, no-space long text, multiline summaries, edge labels.
- Browser screenshots prove text quality at representative zoom levels.
- Bitmap fallback path is removed or demoted to explicit unavailable/debug behavior, not normal production rendering.

Depends on: P3

Stop or ask if:

- The selected text stack makes WASM size/build/performance unacceptable and a different shaping backend must be chosen.

### P5. Product-Quality Text Input Overlay

Outcome: Active text editing keeps native browser input behavior while looking and aligning like part of the Rust-rendered card.

Actions:

- Have Rust return overlay geometry, typography, padding, line metrics, field style, and state tokens.
- Style the DOM input/textarea to visually match the rendered card field.
- Keep IME, selection, clipboard, focus, blur, commit, cancel, and keyboard behavior native.
- Ensure overlay tracks camera and card movement exactly.

Verify:

- Korean IME manual/browser test passes.
- Browser-agentic verification checks focus, type, commit, cancel, persisted text, and overlay disappearance when e2e coverage is needed.
- Screenshots show input overlay is not visually jarring.

Depends on: P4

Stop or ask if:

- The task starts requiring custom rich-text editing beyond title/summary/detail fields.

### P6. Product Shell Integration

Outcome: Production app uses the Rust renderer path while preserving app semantics.

Actions:

- Replace `src/client/App.tsx` canvas rendering with a renderer host component.
- Keep sidebar, tags, comments, export drawer, context menu, and API persistence in the web shell.
- Use `src/shared/renderScene.ts` or its successor as the app-to-render scene adapter.
- Wire renderer patches back to app `ScenePatch` persistence.
- Preserve selection/export semantics from app `Scene`, not renderer-only snapshots.

Verify:

- Existing e2e workflow passes on the new canvas: create group, tag/filter, edit node, add/resolve comment, linked node, z-order, copy/paste, export.
- `/api/scene/render-snapshot` remains useful or is replaced with a better internal comparison/debug route.

Depends on: P5

Stop or ask if:

- A product workflow has no clear mapping between renderer patch and app `ScenePatch`.

### P7. Hidden Diagnostics Drawer

Outcome: Current debug panel remains useful but is hidden behind a development control.

Actions:

- Move renderer stats/readouts into a diagnostics drawer/button.
- Keep renderer health, frame stats, visible counts, glyph stats, cache stats, dirty writes, buffer slots, growth/compaction, hit result, selected target, backend availability, and last error.
- Make the default app surface production-like, without the large prototype sidebar.

Verify:

- Default screen has no exposed debug sidebar.
- Opening diagnostics shows all needed renderer stats.
- Diagnostics state does not interfere with canvas input.

Depends on: P6

Stop or ask if:

- The app needs a user-facing performance panel rather than development-only diagnostics.

### P8. Browser, Visual, And Real-Scene Verification

Outcome: Replacement is proven in a browser against real app data.

Actions:

- Run backend and renderer against real `.local` or temporary test scene.
- Verify real scene load/save, drag, text edit, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge create/delete, tag attach/filter, comments, exports.
- Capture desktop/mobile screenshots and canvas pixel checks.
- Run 1k+ fixture benchmark and record frame/glyph/cache/buffer stats.

Verify:

- Browser-agentic verification covers the real workflow when e2e coverage is needed.
- Visual screenshots show nonblank, correctly framed, polished canvas and input states.
- Real backend persistence survives reload.

Depends on: P7

Stop or ask if:

- Browser WebGPU availability blocks the current target environment.

### P9. Renderer Promotion And Legacy Removal

Outcome: Renderer implementation becomes production engine structure and old DOM/SVG implementation is removed.

Actions:

- Move stable Rust core and web facade into production-owned locations.
- Remove or archive prototype-only harness code that is no longer needed.
- Delete legacy DOM/SVG scene canvas rendering, legacy LOD card rendering, old edge SVG layer, and CSS only used by the old canvas.
- Keep production shell CSS for non-canvas UI.
- Update README/scripts/tests/docs to describe the new renderer path.

Verify:

- `git status` shows only intentional source/docs changes.
- `npm run typecheck`
- `npm run test:unit`
- `npm run build`
- Rust tests and WASM build pass.
- No production code path imports or mounts the old DOM/SVG canvas.

Depends on: P8

Stop or ask if:

- Legacy removal would delete still-required product shell behavior, app semantics, or tests without replacement.

## Execution Rules

- Do not implement substantial work on `main` without explicit approval. Create a branch/worktree first if execution begins.
- Do not delete the legacy production canvas until P8 proves parity and visual quality.
- Do not move app business logic into Rust just to make rendering easier.
- Do not treat generated evidence docs as proof; verify through code, tests, browser, and screenshots.
- Keep changes surgical per phase. Avoid unrelated refactors.
- If a phase exposes a better architecture, stop and update downstream tasks before continuing.
- Use one writer at a time for the same codebase. Parallel readers/reviewers are fine.

## First Unblocked Step

Start with P0 and P1:

1. Refresh the durable task graph/docs around this stricter Rust-owned canvas objective.
2. Make Rust/WASM build reproducible through normal repo commands.
3. Re-run `npm run renderer:test`, `npm run typecheck`, Rust core tests, and WASM build.
