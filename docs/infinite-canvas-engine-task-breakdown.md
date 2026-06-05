# Infinite Canvas Engine Task Breakdown

> Source: [Infinite Canvas Engine Strategy](./infinite-canvas-engine-strategy.md)

이 문서는 무한 캔버스 전략을 실행 가능한 phase/task graph로 분해했던 historical task graph다. 현재 구현 지시는 `docs/rust-canvas-replacement-task.md`를 우선한다.

## Current Contract Refresh

2026-06-05 update: `docs/rust-canvas-replacement-task.md` is now the stronger replacement contract. The earlier POC task graph remains useful as historical evidence, but its "production replacement not ready" conclusion is a historical gate result from the POC cycle, not the current objective.

The current objective is full production replacement: Rust/WASM/WebGPU owns the canvas, and the web layer keeps only product shell, floating UI, API/MCP/business state, a hidden diagnostics drawer, and the native active text input overlay. No follow-up task should shrink the target to "POC mount", "better bitmap text", or a long-lived DOM/SVG fallback.

P9 update: the isolated `poc/` directory has been removed. Stable renderer code now lives under `src/client/renderer` and `src/renderer/core`; historical POC evidence has moved to `docs/renderer/evidence/`.

## Objective

shape.ai의 현재 DOM/SVG 기반 scene canvas 구현을 최종적으로 custom Rust/WASM/WebGPU 기반 performance-aware infinite canvas engine으로 전부 대체한다. Earlier evidence cycles used an isolated POC workspace; current promoted implementation lives in `src/client/renderer` and `src/renderer/core`, with preserved evidence under `docs/renderer/evidence`.

## Historical POC Evidence Done Criteria

이 task graph의 POC/evidence cycle이 완료됐다고 볼 수 있는 상태:

- isolated Rust/WASM canvas core가 browser canvas에 mount된다.
- 하나의 world scene 안에서 group frame, node card, text snippet, edge를 연속적으로 pan/zoom 렌더링한다.
- 1,000개 이상 card/edge fixture에서 frame time, memory, interaction latency를 측정한다.
- DOM overlay 기반 inline text editing이 실제 카드 내부 편집처럼 동작한다.
- TypeScript app layer와 Rust render scene의 책임 경계가 명확하다.
- 현재 shape graph 하나를 새 canvas scene으로 변환해 renderer harness에서 사용할 수 있다.
- 현재 DOM/SVG scene canvas baseline과 새 canvas path의 성능/UX/복잡도 비교가 문서화되어 있다.
- 현재 구현된 graph editor 기능인 group/tag filtering, node create/edit/delete/duplicate/copy, linked node/edge creation, edge select/delete, pan/zoom/fit, selection persistence, comments, z-order, export compatibility를 새 engine 위에 올릴 수 있음이 증명된다.
- HTML-in-Canvas 없이도 실행 가능한 경로가 있다.

## Current Replacement Acceptance Criteria

기존 DOM/SVG scene canvas path를 삭제하기 전에 다음 조건이 모두 참이어야 한다.

- Production canvas가 기본적으로 Rust/WASM/WebGPU engine을 통해 렌더링되고, 기존 DOM/SVG canvas는 장기 fallback이나 alternate rendering mode로 남지 않는다.
- Rust owns retained scene rendering, camera, hit testing, layout-relevant selection geometry, culling, GPU buffers/caches, style/effect rendering, text layout/shaping, and frame/debug stats.
- Web owns only product panels, commands, API/MCP/business state, floating UI, hidden diagnostics drawer, and active native input overlay.
- Text uses HarfBuzz-grade shaping/font fallback/glyph atlas logic. Korean, mixed Latin/CJK, punctuation, multiline, and no-space long text pass tests and browser visual QA. Bitmap text is not the production path.
- Group frames, node cards, text snippets/detail, edge curves/labels, selected/focus states, ports/handles, shadows, gradients, rounded corners, strokes, badges, and polish-level card/edge effects are Rust graphics primitives/style tokens, not CSS-only DOM rendering.
- Active text input uses native DOM input/textarea only while editing, but the overlay receives Rust-computed geometry, typography, padding, line metrics, and state style tokens so it visually sits inside the Rust-rendered card across pan/zoom.
- Korean IME, selection, copy/paste, commit, cancel, blur, and persisted text behavior pass for the overlay.
- Diagnostics are hidden behind a development button/drawer and still expose renderer health, frame time, visible counts, glyph/cache stats, dirty writes, buffer growth/compaction, hit result, selected object, and backend availability.
- Existing workflows still pass on the Rust path: group create, tag attach/filter, node create/edit/delete/duplicate/copy/paste, z-order, edge create/select/delete, pan/zoom/fit/fullscreen, comments, export, selection persistence, and MCP/API compatibility.
- Automated checks, browser interaction checks, desktop/mobile visual captures, and production DOM/SVG path deletion verification pass in the same replacement cycle.

## Execution Snapshot

2026-06-05 진행 상태:

- Historical isolated Rust/WASM scene core scaffold와 Vite web harness를 추가했다.
- POC harness는 group frame, node card, text snippet, edge를 하나의 retained scene fixture로 렌더링하고 pan/zoom/fit, selection, group tag attach/filtering, comments, product export preview, group drag, card drag, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge creation/deletion, DOM text edit overlay, scripted benchmark를 제공한다.
- `createBenchmarkFixture()`는 1,000개 이상 card/edge fixture를 deterministic하게 생성한다.
- `src/shared/renderScene.ts`의 `shapeSceneToRenderSnapshot()`은 current `Scene`에서 renderer scene으로 변환하는 production-side adapter module을 제공하고 comments/artifacts/export/MCP/business fields를 제외한다.
- `GET /api/scene/render-snapshot`은 production `Scene` data를 renderer snapshot으로 읽는 internal comparison route를 제공하며, 새 renderer를 production canvas에 mount하지 않는다.
- `createShapeSceneFixture()`와 POC harness의 `Shape scene` loader는 app-level `Scene`을 renderer snapshot으로 투영하고, group tag attach/filtering, comment creation, product export preview, group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge/select interaction을 in-memory app semantics로 되돌린다.
- POC harness의 `Real scene` loader는 backend가 실행 중일 때 `/api/scene`을 proxy로 읽고 group tag filtering을 적용하며 selected-group tag attach를 `/api/groups/:id/tags`, comments를 `/api/comments`, exports를 `/api/groups/:id/export`로 보내고 group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge app patches를 real `PATCH /api/scene` route로 보낼 수 있다.
- Temporary backend verification으로 POC proxy를 통한 group 생성, scene load, node position patch, text patch, group create/delete patch, edge create/delete patch persistence round trip을 확인했다.
- POC test는 `sceneGraphForGroup()`, `selectedSubgraph()`, `generateLocalExport()`가 renderer snapshot이 아니라 app `Scene`에서 deterministic export/subgraph semantics를 유지함을 검증한다.
- P0~P7 evidence는 `docs/renderer/evidence/` 아래에 baseline, benchmark criteria, source deep dive, renderer/editing confirmations, scene contract/parity, scale hardening/replacement plan, replacement decision summary로 보존했다.
- repo-local `.renderer-toolchains/` 아래에 Rust stable toolchain과 `wasm-pack`을 설치해 `npm run renderer:wasm:build`를 검증했다. Generated WASM glue는 gitignore된 `src/client/renderer/wasm/`에 생성된다.
- Web harness는 POC scene rendering을 Rust/WASM WebGPU 단일 경로로 좁혔다. Generated WASM 또는 visible WebGPU renderer가 없으면 TS Canvas2D fallback으로 그리지 않고 renderer unavailable 상태를 표시한다.
- Rust core는 `wgpu` 29 기반 WebGPU readiness probe를 제공해 detached canvas에서 browser WebGPU support, canvas surface, adapter/device request, default surface config, clear render pass submit/present를 검증한다.
- Rust core는 visible WebGPU canvas를 소유하는 `ShapeWebGpuRenderer`를 제공해 group/card primitive와 cubic edge/arrow/label vertex upload, selected group/card/edge outline styling, shared style token color ingestion, Rust-owned padded viewport culling과 merged draw-range rendering, rustybuzz/fontdue 기반 shaped title/summary/edge-label rendering, explicit bundled Latin/Korean Noto Sans KR renderer font subsets, font fallback, glyph atlas upload, text line/wrap layout cache, CJK/fallback/missing glyph/raster-cache metric reporting, compact render patch ingestion, fixed-slot dirty group/card/edge buffer writes for group translate/card move/edit/select patches, z-order patch의 order-preserving buffer rebuild, spare group slot dirty writes for group create/delete patches, group spare slot 고갈 시 edge/card segment 앞에 추가 group slots를 삽입하는 GPU buffer growth, group deletion 후 과도한 free slots를 줄이는 GPU segment-copy compaction, spare card slot dirty writes for card create/delete patches, card spare slot 고갈 시 buffer suffix에 추가 card slots를 append하는 GPU buffer growth, card deletion 후 과도한 free slots를 줄이는 GPU segment-copy compaction, spare edge slot dirty writes for edge create/delete patches, edge spare slot 고갈 시 card draw segment 앞에 추가 edge slots를 삽입하는 GPU buffer growth, edge deletion 후 과도한 free slots를 줄이는 GPU segment-copy compaction, Rust-owned card/text/port/cubic-edge/group hit testing, render pass submit/present, visible object/drawn vertex/draw range/GPU vertex/shaped-glyph/fallback-run/missing-glyph/CJK-glyph/glyph-atlas/raster-cache/text-cache/style-token/patch/dirty-write/rebuild/group-slot/group-grow/group-compact/card-slot/card-grow/card-compact/edge-slot/edge-grow/edge-compact count reporting을 수행한다.
- TS harness는 WebGPU mode에서도 per-frame spatial visible 계산을 하지 않고 Rust frame stats를 HUD/benchmark에 사용한다. Camera/input interpretation, hit/selection classification, overlay geometry request, patch batching, and debug snapshots now flow through Rust-facing batch/debug APIs instead of single-purpose JS/WASM calls.
- P7 decision summary는 이전 POC cycle의 gate result로서, 그 branch에서 production DOM/SVG scene canvas를 제거하지 말고 rendered browser interaction verification과 product-quality Rust/wgpu hardening을 먼저 진행하라고 권고했다.
- 이 historical "not ready" 판단은 현재 objective를 축소하지 않는다. 현재 replacement gate는 HarfBuzz-grade shaping/font fallback/Korean glyph quality, Rust-owned graphics effects, polished DOM input overlay, hidden diagnostics drawer, real backend browser interaction verification, visual QA, and legacy DOM/SVG removal까지 요구한다. Current replacement cycle treats that gate as passed before P9 legacy removal; future e2e/browser verification should use the Browser agentic workflow, not a committed Playwright harness.

## Locked Inputs

- 최종 목표는 semantic LOD 중심 UX가 아니라 continuous vector canvas다.
- HTML-in-Canvas는 지금 foundation이 아니다.
- Rust는 business logic 이전용이 아니라 canvas/graphics core 구현 언어로 사용한다.
- TypeScript app layer는 shape business model, persistence, AI/MCP workflow, comments/export/proposals를 계속 책임진다.
- DOM은 app chrome, floating UI, active editing overlay에 사용한다.
- Debug panel은 개발용 button/drawer 뒤에 숨기되 renderer health/debug stats를 잃지 않는다.
- HarfBuzz-grade shaping/font fallback/glyph atlas와 product-quality Korean/mixed text rendering은 production replacement gate다. P4에서 bitmap text는 정상 rendering path에서 제거되고 rustybuzz/fontdue 기반 renderer text path가 들어왔다. Browser/visual approval belongs to P8 and is not represented by committed Playwright tests in this branch.
- 평상시 canvas object는 live DOM element가 아니라 renderer scene object다.
- renderer path는 custom Rust + wgpu/WebGPU로 잠근다. Vello, CanvasKit/Skia, Graphite, ThorVG, Pathfinder, Lyon, Kurbo, Peniko, Swash, Cosmic Text는 직접 구현 범위를 줄이고 위험을 검증하기 위한 참고 자료다.
- historical POC evidence is preserved under `docs/renderer/evidence`; current fixture, benchmark, adapter, and renderer implementation live in production paths.
- 최종 migration 목표는 현재 DOM/SVG scene canvas path를 유지보수용 fallback으로 남기는 것이 아니라 새 engine으로 대체하고 기존 path를 제거하는 것이다.

## Must-Haves

- Continuous zoom: 줌 중 객체 표현이 dot/cluster/다른 카드로 갑자기 바뀌지 않는다.
- Retained scene: 모든 객체는 하나의 world scene과 stable object id를 가진다.
- Batched boundary: JS/WASM 호출은 scene patch, input batch, frame render 중심으로 묶는다.
- Active edit bridge: 텍스트 편집은 DOM overlay로 처리하되 scene과 좌표 동기화가 정확해야 한다.
- Product-quality text: Rust renderer가 HarfBuzz-grade shaping/font fallback/glyph atlas, Korean/mixed text quality, line wrap, clipping, ellipsis, and cache invalidation을 소유한다.
- Rust graphics effects: card/group/edge visual polish, shadows, gradients, strokes, rounded corners, badges, ports, focus/selected states, and edge labels are renderer-owned primitives/style tokens.
- Hidden diagnostics: visible-by-default debug panels become a development drawer without losing renderer health/frame/cache/buffer/hit/backend stats.
- Performance-aware renderer: spatial index, culling, geometry/text/edge cache, batched GPU updates, JS/WASM boundary budget을 foundation 요구사항으로 둔다.
- Current editor parity: 현재 Scene/Group/Node/Edge/Tag/Comment/Artifact workflow를 새 canvas 위에 다시 올릴 수 있어야 한다.
- Migration safety: earlier isolated evidence was used to compare the replacement path before removing the DOM/SVG scene canvas path.

## Current Implementation Features To Preserve

현재 구현에서 새 engine 위에 다시 올려야 하는 기능:

- Scene model: SQLite-backed `Scene` 안의 `Group`, `Node`, `Edge`, `Tag`, `Comment`, `Artifact`, `selection`.
- Canvas navigation: smooth pan/zoom, pinch/wheel zoom, fit scene/group/node, fullscreen, viewport query.
- Canvas rendering: group frame, node preview/detail card, selected state, edge curve/label, z-index ordering, performance HUD.
- Graph editing: node select, inline title/summary/detail/status/type edit, linked node creation, node delete/duplicate/copy/paste, z-order move.
- Edge editing: edge selection, source/target validation, linked node/edge creation, edge deletion, inspector/export selection compatibility.
- Product shell: group creation, group tag attach/filter, comments, export drawer, deterministic group/node/edge/selection exports, MCP/API semantics.

## Deferred Or Explicitly Out Of Scope

- 전체 CSS layout engine 구현.
- 모든 node/card를 live DOM/Web Component로 유지하는 구조.
- Rust로 AI/MCP/product business logic 이전.
- Figma/Illustrator 수준의 범용 디자인 툴 기능.
- HTML-in-Canvas 기반 production path.
- 임의 path 편집, 복잡한 boolean geometry, full SVG editor 기능.
- 완전한 접근성 구현. 다만 접근성 데이터 모델을 나중에 붙일 수 있게 막지는 않는다.

## Phase Graph

```text
P0 Evidence Baseline
  -> P1 Prototype Scaffold
  -> P2 Custom Renderer Foundation
  -> D1 Renderer Architecture Confirmation
  -> P3 Interaction Vertical Slice
  -> D2 Editing Boundary Confirmation
  -> P4 Shape Scene Contract
  -> P5 Current App Parity Slice
  -> D3 Replacement Readiness Confirmation
  -> P6 Scale And Hardening
  -> P7 Final Review And Replacement Plan
```

Open-source reference deep dive는 P1/P2와 일부 병렬 가능하다. 현재 app source는 POC adapter와 parity fixture를 만들 때 참고하되, replacement phase 전까지 production path를 직접 바꾸지 않는다.

## Phase P0: Evidence Baseline

Goal: 현재 DOM/SVG scene canvas path와 원하는 custom Rust canvas engine path 사이의 비교 기준을 만든다.

Why now: 기준 없이 Rust/WebGPU 작업을 시작하면 "빠른가"와 "충분한가"를 판단할 수 없다.

Tasks: T0.1, T0.2, T0.3

Verify or evaluate:

- baseline report가 문서화되어 있다.
- 현재 앱의 핵심 canvas workflow가 목록화되어 있다.
- benchmark 목표가 숫자 또는 관찰 기준으로 정해져 있다.

Review gate:

- `human-decision`: benchmark 목표와 UX 판단 기준이 제품 목표를 제대로 반영하는지 승인한다.

### T0.1 Current Canvas Workflow Baseline

Outcome: 현재 DOM/SVG 기반 workflow와 성능/UX 한계를 비교 기준으로 기록한다.

Source refs:

- README의 Web UI graph editing 설명.
- `docs/infinite-canvas-engine-strategy.md`의 현재 맥락과 핵심 결정.
- 현재 구현의 `Scene`, `Group`, `Node`, `Edge`, `Tag`, `Comment`, `Artifact` 모델.

Read first:

- `README.md`
- `src/shared/schema.ts`
- `src/shared/graph.ts`
- 현재 canvas entrypoint와 node/editor components
- 관련 frontend tests 또는 e2e tests

Deliverables:

- 현재 가능한 workflow 목록.
- 현재 DOM/SVG scene canvas path에서 유지해야 할 UX 목록.
- 새 canvas가 대체해야 하는 interaction checklist.
- 현재 baseline 측정 방법.

Verify:

- `npm run typecheck`
- 현재 앱을 실행해 pan/zoom/edit/select/connect/export 흐름이 어디에서 일어나는지 확인한다.

Acceptance:

- 새 엔진이 반드시 보존해야 할 workflow와 버려도 되는 current-renderer-specific behavior가 분리되어 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- 현재 product workflow 자체가 바뀌어야 하는지 결정이 필요해진다.

### T0.2 Success Metrics And Benchmark Fixture

Outcome: custom renderer prototype을 평가할 수 있는 fixture와 성공 기준을 정의한다.

Source refs:

- 전략 문서의 성능 모델.
- 전략 문서의 custom Rust/WebGPU renderer 검증 과제.

Deliverables:

- card/edge/text fixture 규모 정의.
- frame time, memory, interaction latency, text quality 평가 기준.
- zoom/pan 시각 검토 checklist.
- 현재 DOM/SVG scene canvas baseline과 비교할 최소 fixture.

Verify:

- fixture 기준이 "1,000개 card + edge + text snippet" 이상을 포함한다.
- 숫자로 측정 가능한 항목과 사람이 봐야 하는 항목이 분리되어 있다.

Acceptance:

- custom renderer 품질을 감으로 판단하지 않고 같은 fixture로 검증할 수 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- 목표 성능 기준이 제품 기대와 맞지 않는다고 판단된다.

### T0.3 Source Architecture Deep Dive

Outcome: Graphite/Figma/Vello/wgpu/CanvasKit 참고가 custom implementation task에 쓸 수 있는 수준으로 정리된다.

Source refs:

- 전략 문서의 Figma 조사 메모.
- 전략 문서의 오픈소스 참고.

Deliverables:

- Graphite의 Rust backend/web frontend boundary 요약.
- Figma의 custom renderer ownership 방향에서 가져올 점과 가져오지 않을 점.
- Vello/wgpu에서 참고할 scene building, GPU lifecycle, glyph/cache caveat.
- CanvasKit/Skia에서 참고할 path/text quality baseline과 피해야 할 dependency ownership.
- Lyon/Kurbo/Peniko/Swash/Cosmic Text로 직접 구현할 때 필요한 building block 목록.
- 가져오면 안 되는 Graphite/Figma급 과복잡도 목록.

Verify:

- 각 결론이 링크 또는 코드 위치에 연결되어 있다.
- downstream task가 사용할 "적용할 패턴"과 "피할 패턴"이 구분되어 있다.

Acceptance:

- P1/P2 작업자가 renderer/API 구조를 잡을 때 참고할 수 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- custom Rust/wgpu path가 현재 browser target에서 명백히 부적합하다는 증거가 나온다.

## Phase P1: Prototype Scaffold

Goal: 현재 앱을 건드리지 않고 Rust/WASM canvas prototype을 실행할 수 있는 격리된 기반을 만든다.

Why now: renderer 실험은 production DOM/SVG scene canvas path와 분리되어야 한다. 그래야 실패해도 앱을 망가뜨리지 않고 비교할 수 있다.

Tasks: T1.1, T1.2, T1.3, T1.4

Verify or evaluate:

- prototype page가 canvas를 mount한다.
- Rust/WASM package가 Vite dev flow에서 로드된다.
- scene snapshot을 넣고 빈 frame 또는 debug primitive를 그릴 수 있다.

Review gate:

- `human-verify`: prototype scaffold가 production app path를 오염시키지 않는지 확인한다.

### T1.1 Isolated Prototype Workspace

Outcome: Rust/WASM canvas experiment가 현재 app과 분리된 위치에서 빌드된다.

Source refs:

- 전략 문서의 Phase 1: Evidence Prototype.
- README의 local development/verification commands.

Files/ownership:

- `poc/` 아래의 prototype package, Rust crate, web harness, fixture directory.
- production DOM/SVG scene canvas path는 수정하지 않는다.

Deliverables:

- `poc/` 아래 Rust crate 또는 package scaffold.
- web prototype entrypoint.
- build/run instructions.
- minimum CI/local verification command.

Verify:

- prototype build command가 성공한다.
- 기존 `npm run typecheck`가 깨지지 않는다.

Acceptance:

- production app을 실행하지 않고도 renderer prototype을 따로 검증할 수 있다.

Depends on: T0.1, T0.2

Parallel wave: B

Stop or ask if:

- `poc/` 밖에 Rust toolchain이나 generated artifact를 둬야 할 것처럼 보인다.

### T1.2 WASM Loader And Canvas Mount

Outcome: TypeScript에서 Rust/WASM canvas core를 load하고 HTML canvas에 mount한다.

Source refs:

- 전략 문서의 JavaScript API 형태.
- 전략 문서의 JS/WASM Boundary 리스크.

Deliverables:

- `mount`, `resize`, `renderFrame` 최소 API.
- devicePixelRatio 처리.
- browser resize 처리.
- panic/error reporting path.

Verify:

- prototype page에서 canvas가 resize와 DPR을 반영한다.
- blank/debug frame이 안정적으로 render된다.

Acceptance:

- 이후 renderer task가 같은 mount lifecycle을 재사용할 수 있다.

Depends on: T1.1

Parallel wave: serial

Stop or ask if:

- WASM bundling이 Vite와 충돌해서 별도 toolchain 결정이 필요하다.

### T1.3 Minimal Scene Snapshot Contract

Outcome: TypeScript와 Rust가 공유할 최소 scene snapshot 구조를 정의한다.

Source refs:

- 전략 문서의 책임 경계.
- 전략 문서의 Canvas scene 모델.

Deliverables:

- `SceneSnapshot`, `ScenePatch`, `CameraState`, `WorldRect` draft.
- stable object id rule.
- business document와 canvas scene field 분리 규칙.
- serialization/deserialization path.

Verify:

- TypeScript fixture가 Rust에서 읽힌다.
- Rust가 알 필요 없는 business field가 scene contract에 들어가지 않는다.

Acceptance:

- 이후 renderer, hit test, app parity adapter가 같은 contract를 기준으로 작업할 수 있다.

Depends on: T1.1

Parallel wave: B

Stop or ask if:

- product data model 자체가 renderer scene에 필요한 값을 제공하지 못한다.

### T1.4 Benchmark Harness Skeleton

Outcome: prototype에서 custom renderer의 성능/품질을 같은 fixture로 반복 평가할 수 있는 harness를 만든다.

Source refs:

- T0.2 benchmark fixture.
- 전략 문서의 renderer 검증 과제.

Deliverables:

- deterministic fixture generator.
- frame stats collection.
- pan/zoom scripted path.
- screenshot or visual capture path.

Verify:

- 같은 seed로 같은 scene이 생성된다.
- frame stats가 기록된다.

Acceptance:

- P2 custom renderer 작업이 같은 기준으로 측정된다.

Depends on: T0.2, T1.2, T1.3

Parallel wave: serial

Stop or ask if:

- benchmark 결과를 저장할 위치나 형식이 repo policy와 충돌한다.

## Phase P2: Custom Renderer Foundation

Goal: custom Rust/wgpu renderer가 shape.ai의 group/node/edge scene을 직접 그릴 수 있는 최소 foundation을 만든다.

Why now: renderer path는 custom implementation으로 결정됐다. interaction과 app parity를 시작하기 전에 직접 구현할 primitive pipeline, text path, cache boundary가 실제로 작동해야 한다.

Tasks: T2.1, T2.2, T2.3, T2.4

Verify or evaluate:

- 같은 fixture에서 custom renderer의 frame stats와 visual notes가 있다.
- 오픈소스 reference에서 가져온 패턴과 피해야 할 복잡도가 구현 결정에 반영되어 있다.
- card/text/edge primitive pipeline과 cache boundary가 다음 phase에서 재사용 가능하다.

Review gate:

- `human-verify`: D1 Renderer Architecture Confirmation.

### T2.1 Custom wgpu Card Graph Renderer Foundation

Outcome: custom Rust/wgpu path로 group frame, card, text snippet, edge를 렌더링한다.

Source refs:

- 전략 문서의 Track C.
- 전략 문서의 Product-Specific Implications.
- T0.3의 오픈소스 reference findings.

Deliverables:

- group frame rendering.
- card rectangle/border/background rendering.
- title/summary text snippet rendering.
- edge line/arrowhead rendering.
- camera pan/zoom.
- frame stats report.

Verify:

- 1,000개 이상 card fixture에서 pan/zoom이 측정된다.
- zoom 중 object가 표현 전환 없이 연속적으로 보인다.
- artifact와 text 품질 문제가 기록된다.

Acceptance:

- custom Rust/wgpu renderer가 shape.ai canvas의 primary path로 계속 갈 수 있는지 판단할 수 있다.

Depends on: T1.2, T1.3, T1.4

Parallel wave: C

Stop or ask if:

- wgpu/WebGPU browser support가 핵심 요구를 막는다.

### T2.2 Open-Source Reference Pattern Check

Outcome: Graphite/Vello/CanvasKit/ThorVG 등에서 참고할 패턴을 custom renderer 구현에 반영한다.

Source refs:

- 전략 문서의 오픈소스 참고.
- T0.3 Source Architecture Deep Dive.

Deliverables:

- Graphite wrapper/message boundary에서 가져올 API pattern.
- Vello/wgpu scene/device/cache caveat checklist.
- CanvasKit/Skia text/path quality baseline note.
- ThorVG/Pathfinder/Lyon/Kurbo/Peniko/Swash/Cosmic Text에서 가져올 primitive/text building block note.
- custom renderer에 가져오지 않을 editor-wide complexity list.

Verify:

- 각 reference conclusion이 링크나 코드 위치에 연결되어 있다.
- custom renderer task에서 바로 쓸 "adopt", "avoid", "benchmark only" 항목이 분리되어 있다.

Acceptance:

- 오픈소스가 primary implementation을 대체하지 않고 custom implementation을 좁히는 자료로 정리된다.

Depends on: T1.4

Parallel wave: C

Stop or ask if:

- reference가 custom implementation이 아니라 dependency adoption으로 방향을 바꾸게 만든다.

### T2.3 Performance-Aware Primitive Pipeline

Outcome: custom Rust tessellation + wgpu pipeline에 필요한 primitive, cache, batching 구조를 구현한다.

Source refs:

- 전략 문서의 Track C.
- 전략 문서의 Lyon/Kurbo/Peniko/Swash/Cosmic Text 참고.

Deliverables:

- card/edge/text primitive data model.
- geometry tessellation/cache path.
- text shaping/layout/cache path.
- edge route/arrowhead cache path.
- frame-level batch/update budget.

Verify:

- pan/zoom 중 layout recomputation 없이 render되는지 측정한다.
- cache hit/miss와 JS/WASM call count가 기록된다.

Acceptance:

- custom renderer의 risky subsystem과 다음 phase에서 보강해야 할 부분이 명확하다.

Depends on: T0.3

Parallel wave: C

Stop or ask if:

- custom primitive/text/cache 구현이 POC 범위를 넘어 product work를 장기간 막을 정도로 커진다.

### T2.4 Renderer Architecture Confirmation Report

Outcome: custom renderer foundation의 결과를 D1 확인 자료로 묶는다.

Source refs:

- T2.1, T2.2, T2.3 outputs.

Deliverables:

- custom renderer architecture recap.
- adopted open-source reference patterns.
- rejected dependency adoption and reasons.
- risks to carry forward.
- next phase changes if benchmark or visual evidence exposes constraints.

Verify:

- confirmation이 benchmark, visual evidence, integration complexity에 근거한다.

Acceptance:

- D1에서 custom renderer foundation을 승인하거나 제한된 추가 probe만 남긴다.

Depends on: T2.1, T2.2, T2.3

Parallel wave: serial

Stop or ask if:

- custom renderer가 핵심 기준을 통과하지 못한다.

## Decision D1: Renderer Architecture Confirmation

Resolved decision: primary renderer는 custom Rust + wgpu/WebGPU implementation이다.

Confirmed defaults:

- Vello는 renderer dependency가 아니라 scene/GPU architecture reference로 사용한다.
- CanvasKit/Skia는 path/text quality benchmark reference로만 사용한다.
- Graphite/Figma는 custom engine ownership, wrapper/API boundary, 피해야 할 editor-wide complexity를 판단하는 reference로 사용한다.
- P3/P4/P5는 custom renderer를 기준으로 진행한다.

Required evidence:

- frame stats.
- visual quality screenshots.
- text rendering notes.
- browser integration notes.
- implementation complexity estimate.

Downstream impact:

- P3/P4/P5는 custom Rust/wgpu renderer를 기준으로 진행한다.
- renderer dependency 전환은 기본 계획이 아니라 stop condition을 만족할 때만 별도 의사결정으로 연다.

## Phase P3: Interaction Vertical Slice

Goal: canvas가 단순히 그리는 것이 아니라 직접 조작 가능한 editor가 될 수 있음을 검증한다.

Why now: pan/zoom rendering이 좋아도 selection, drag, hit test, text edit가 어색하면 제품으로 쓸 수 없다.

Tasks: T3.1, T3.2, T3.3, T3.4

Verify or evaluate:

- 작은 scene에서 group/node/edge select/drag/edit/connect 흐름이 가능하다.
- DOM overlay가 canvas object와 정확히 연결된다.

Review gate:

- `human-verify`: inline editing이 현재 제품의 "노드 안에서 바로 편집" 감각을 유지하는지 확인한다.

### T3.1 Hit Testing And Selection

Outcome: rendered object를 클릭했을 때 stable object id와 selection geometry가 반환된다.

Source refs:

- 전략 문서의 Canvas scene 모델.
- 전략 문서의 JavaScript API 형태.

Deliverables:

- Hit result through the Rust input/debug boundary.
- group/card/edge/text/port hit region.
- selected object highlight.
- selection event output.

Verify:

- 여러 zoom level에서 같은 object id가 선택된다.
- selection highlight가 object geometry와 맞다.

Acceptance:

- UI layer가 Rust core의 hit result만으로 selection state를 표시할 수 있다.

Depends on: D1, T1.3, T2.1

Parallel wave: D

Stop or ask if:

- hit region과 visual geometry가 renderer 구조상 일치하기 어렵다.

### T3.2 Drag And Camera Interaction

Outcome: card drag와 canvas pan/zoom이 같은 input bridge 위에서 안정적으로 동작한다.

Source refs:

- 전략 문서의 JavaScript API 형태.
- 전략 문서의 성능 모델.

Deliverables:

- pointer input batching.
- camera update path.
- card transform patch.
- drag frame stats.

Verify:

- pan/zoom 중 layout recomputation이 발생하지 않는다.
- drag 후 object position이 scene patch로 안정적으로 반영된다.

Acceptance:

- 직접 조작이 현재 DOM/SVG scene canvas path와 비교 가능한 수준으로 동작한다.

Depends on: T3.1

Parallel wave: serial

Stop or ask if:

- app-level layout persistence와 engine-level transform ownership이 충돌한다.

### T3.3 DOM Text Edit Overlay

Outcome: GPU-rendered text region 위에 DOM editor overlay를 띄워 inline editing을 구현한다.

Source refs:

- 전략 문서의 Canvas 객체는 무엇으로 그릴 것인가.
- 전략 문서의 Text Editing 리스크.

Deliverables:

- `beginTextEdit` -> `DomOverlayRequest` flow.
- overlay positioning and transform sync.
- IME/copy/paste/selection behavior.
- `commitTextEdit` -> scene patch flow.

Verify:

- 여러 zoom level에서 overlay 위치 오차가 눈에 띄지 않는다.
- 한국어 IME 입력이 동작한다.
- 편집 완료 후 rendered text가 갱신된다.

Acceptance:

- 모든 노드를 DOM으로 만들지 않고도 현재 inline edit 감각을 유지할 수 있다.

Depends on: T3.1, T3.2

Parallel wave: serial

Stop or ask if:

- overlay alignment가 product quality를 만족하지 못한다.

### T3.4 Edge Creation And Port Interaction

Outcome: card port에서 edge를 만들고 rendered edge를 선택할 수 있다.

Source refs:

- README의 edge creation/editing workflow.
- 전략 문서의 Product-Specific Implications.

Deliverables:

- port hit regions.
- edge preview while dragging.
- edge creation event.
- edge selection.

Verify:

- source/target object id가 정확하다.
- edge preview와 final edge route가 어색하게 튀지 않는다.

Acceptance:

- graph editor로서 최소 연결 workflow가 가능하다.

Depends on: T3.1, T3.2

Parallel wave: D-late

Stop or ask if:

- edge routing 정책이 renderer core와 app layer 중 어디에 있어야 할지 불명확해진다.

## Decision D2: Editing Boundary Confirmation

Resolved decision:

- Normal canvas objects는 Rust-rendered scene object로 유지한다.
- Active text/card editing만 DOM overlay로 올린다.
- Selected card의 toolbar, context menu, inspector 같은 affordance는 TypeScript DOM shell이 담당한다.
- full DOM card rendering 또는 live Web Component canvas는 production replacement path가 아니다.

Confirmation criteria:

- DOM overlay가 Korean IME, selection, copy/paste, zoomed alignment를 만족한다.
- overlay positioning은 Rust scene geometry와 TypeScript DOM shell이 같은 camera transform을 공유해 계산한다.
- commit 결과는 scene patch로 돌아오고, business validation은 TypeScript app layer가 수행한다.

Required evidence:

- Korean IME result.
- zoomed edit alignment.
- selection/copy/paste behavior.
- commit/undo implications.

Downstream impact:

- P4 scene contract에 text edit target, overlay geometry, patch shape가 확정된다.
- overlay 품질 문제가 나오면 DOM full-card 전환이 아니라 overlay geometry, focus, IME, patch flow를 보강한다.

## Phase P4: Shape Scene Contract

Goal: current `Scene` data를 renderer scene으로 변환하는 안정적인 contract를 만든다.

Why now: renderer와 interaction slice가 증명된 뒤에야 current app data와 결합할 가치가 있다.

Tasks: T4.1, T4.2, T4.3, T4.4

Verify or evaluate:

- TypeScript business scene과 Rust canvas scene이 분리된다.
- scene patch가 app state update로 되돌아오는 흐름이 정의된다.

Review gate:

- `human-decision`: Rust가 가져가는 scene 의미가 과하거나 부족하지 않은지 승인한다.

### T4.1 Business Scene To Render Scene Adapter

Outcome: stored shape `Scene`을 renderable scene snapshot으로 변환한다.

Source refs:

- README의 `Scene`, `Group`, `Node`, `Edge`, `Tag` 설명.
- `src/shared/schema.ts`
- 전략 문서의 책임 경계.

Deliverables:

- app-level adapter contract.
- group -> frame mapping.
- node -> card mapping.
- edge -> route mapping.
- tag/comment/export/artifact field exclusion rule.

Verify:

- sample scene이 deterministic scene snapshot으로 변환된다.
- business-only fields가 Rust scene에 들어가지 않는다.

Acceptance:

- 현재 shape scene data를 canvas engine이 렌더링할 수 있는 input으로 만들 수 있다.

Depends on: D2, T1.3

Parallel wave: E

Stop or ask if:

- current data model에서 visual bounds/layout source가 불명확하다.

### T4.2 Scene Patch To App Update Contract

Outcome: engine interaction 결과가 app state와 persistence로 돌아가는 patch contract를 정의한다.

Source refs:

- 전략 문서의 JavaScript API 형태.
- README의 layout persistence, inline field edits, edge creation, selection 설명.

Deliverables:

- drag patch.
- text edit patch.
- edge creation/deletion patch.
- selection patch.
- group/tag/filter-visible scene input boundary.
- validation rules for incoming engine events.

Verify:

- patch가 business mutation과 visual-only mutation을 구분한다.
- invalid patch를 app layer에서 거부할 수 있다.

Acceptance:

- engine이 product rules를 몰라도 app이 안전하게 변경을 적용할 수 있다.

Depends on: T4.1, T3.2, T3.3, T3.4

Parallel wave: serial

Stop or ask if:

- proposal/approval workflow와 direct UI mutation 경계가 충돌한다.

### T4.3 Style Token And Text Model

Outcome: card 디자인과 text rendering에 필요한 style/text 정보를 scene contract로 고정한다.

Source refs:

- 전략 문서의 Product-Specific Implications.
- 전략 문서의 Text Editing 리스크.

Deliverables:

- style key/token map.
- text run model.
- truncation/wrapping rule.
- selected/editing visual states.

Verify:

- renderer scene이 CSS 전체가 아니라 제한된 style model만 받는다.
- 디자인 변경이 app token에서 scene style로 변환될 수 있다.

Acceptance:

- CSS를 그대로 렌더링하지 않아도 제품 카드 디자인을 충분히 표현할 수 있다.

Depends on: T4.1, D2

Parallel wave: E

Stop or ask if:

- 카드 디자인이 renderer가 감당할 수 없는 CSS features에 강하게 의존한다.

### T4.4 Accessibility Data Hook

Outcome: GPU canvas content에 대한 최소 접근성 데이터 hook을 둔다.

Source refs:

- 전략 문서의 Accessibility 리스크.

Deliverables:

- selected/visible object accessibility summary.
- keyboard focus target model.
- future screen reader integration note.

Verify:

- renderer scene에서 label/role/selection state를 app이 얻을 수 있다.

Acceptance:

- 접근성 구현을 나중에 붙일 수 없는 구조로 막지 않는다.

Depends on: T4.1

Parallel wave: E

Stop or ask if:

- 접근성 요구 수준이 product release gate로 올라간다.

## Phase P5: Current App Parity Slice

Goal: `poc/` harness에서 현재 shape.ai app의 핵심 graph editor workflow를 새 canvas로 렌더링하고 편집한다.

Why now: prototype이 현재 제품 데이터와 workflow parity를 증명해야 실제 replacement migration을 판단할 수 있다. 이 phase는 production source를 직접 바꾸지 않고 `poc/` 안에서 대체 가능성을 검증한다.

Tasks: T5.1, T5.2, T5.3, T5.4

Verify or evaluate:

- `poc/` harness에서 current app fixture와 같은 scene을 렌더링한다.
- 현재 DOM/SVG scene canvas baseline과 같은 shape를 비교할 수 있다.
- node/edge/group editing workflow가 current app semantics와 맞는다.

Review gate:

- `human-verify`: current app parity가 replacement migration을 시작할 만큼 충분한지 확인한다.

### T5.1 POC App Parity Harness

Outcome: `poc/` 안에서 current app의 scene load/render shell을 재현한다.

Source refs:

- 전략 문서의 Phase 4: Current App Integration.
- README의 local development commands.
- 현재 `src/client/App.tsx` canvas workflow.

Files/ownership:

- `poc/` app canvas shell.
- fixture/API-compatible scene loader.
- production DOM/SVG scene canvas path는 수정하지 않는다.

Deliverables:

- `poc/` route or standalone page.
- canvas mount lifecycle.
- shape selection/load integration.

Verify:

- existing app: production path가 변경되지 않는다.
- POC: 새 canvas가 같은 shape scene fixture를 렌더링한다.

Acceptance:

- POC 실패가 기존 app workflow를 막지 않는다.

Depends on: T4.1, T4.2, T4.3

Parallel wave: F

Stop or ask if:

- parity를 증명하려면 `poc/` 밖 production source를 먼저 수정해야 할 것처럼 보인다.

### T5.2 Selection And Editing Integration

Outcome: 새 canvas에서 selection, inline edit, drag가 app state/persistence semantics와 연결된다.

Source refs:

- README의 graph editing workflow.
- T4.2 patch contract.
- current implementation의 node edit, copy/paste, z-order action.

Deliverables:

- selected node/edge state sync.
- text edit save path.
- layout persistence path.
- node create/delete/duplicate/copy/paste parity note.
- z-order move parity note.
- error handling for rejected patches.

Verify:

- card drag 후 reload해도 위치가 유지된다.
- inline edit 후 POC app state와 persistence patch가 갱신된다.
- edge/node selection detail panel이 동작한다.

Acceptance:

- 최소 편집 workflow가 현재 DOM/SVG scene canvas path와 비교 가능하다.

Depends on: T5.1, T4.2, T3.3

Parallel wave: serial

Stop or ask if:

- direct UI edit와 proposal workflow의 정책 경계가 바뀌어야 한다.

### T5.3 Edge Workflow Integration

Outcome: 새 canvas에서 edge creation/selection/deletion이 current graph workflow semantics와 연결된다.

Source refs:

- README의 graph editing workflow.
- T3.4 edge interaction.

Deliverables:

- edge create event -> app update.
- edge select -> inspector.
- edge delete -> app update.
- invalid connection rejection.

Verify:

- edge 생성/삭제 후 stored graph가 일관된다.
- invalid connection이 UI와 app layer에서 모두 안전하게 처리된다.

Acceptance:

- 새 canvas가 graph editor의 핵심 연결 기능을 수행한다.

Depends on: T5.2, T3.4

Parallel wave: serial

Stop or ask if:

- edge validation rules가 renderer core에 들어가야 할 것처럼 보인다.

### T5.4 Export And Snapshot Compatibility

Outcome: 새 canvas path가 기존 export semantics를 깨지 않는다.

Source refs:

- README의 exports 설명.
- 전략 문서의 Product-Specific Implications.

Deliverables:

- graph export compatibility check.
- selected subgraph export behavior check.
- image/screenshot export candidate note.

Verify:

- 기존 deterministic export가 canvas path와 무관하게 유지된다.
- selected node/edge subgraph export가 selection state와 충돌하지 않는다.

Acceptance:

- canvas renderer 교체가 graph export semantics를 바꾸지 않음을 POC에서 증명한다.

Depends on: T5.2, T5.3

Parallel wave: serial

Stop or ask if:

- image export를 renderer output 기반으로 바꾸는 scope creep가 생긴다.

## Decision D3: Replacement Readiness Confirmation

Resolved decision:

- 최종 목표는 current DOM/SVG scene canvas path 전체 replacement다.
- current path는 migration 동안 임시 비교/fallback으로만 유지한다.
- 새 engine은 large-canvas special mode가 아니라 기본 graph editor surface가 되어야 한다.
- production replacement는 POC parity와 scale evidence가 통과된 뒤 별도 migration execution cycle에서 시작한다.

Confirmation criteria:

- P5 parity harness가 current app의 core workflow를 재현한다.
- D1/D2 evidence가 custom renderer와 DOM overlay boundary를 지지한다.
- performance report가 current DOM/SVG baseline 대비 replacement 근거를 제공한다.
- complexity/risk estimate가 production migration을 task로 나눌 수 있을 만큼 구체적이다.

Required evidence:

- POC app parity result.
- current DOM/SVG scene canvas baseline comparison.
- user-facing editing workflow result.
- performance report.
- complexity/risk estimate.

Downstream impact:

- P6의 hardening scope와 P7의 replacement/removal plan이 결정된다.

## Phase P6: Scale And Hardening

Goal: 대규모 그래프에서 continuous canvas가 안정적으로 유지되도록 최적화하고 검증한다.

Why now: vertical slice와 POC parity가 통과된 뒤에야 scale optimization이 정확한 대상을 가진다.

Tasks: T6.1, T6.2, T6.3, T6.4

Verify or evaluate:

- large fixture에서 pan/zoom/edit/select/connect가 측정된다.
- performance bottleneck이 React/DOM이 아니라 renderer pipeline 안에서 추적된다.

Review gate:

- `human-verify`: large canvas UX가 Illustrator-like expectation에 가까운지 확인한다.

### T6.1 Spatial Index And Culling

Outcome: world scene에서 viewport 기준 visible/hittable object를 빠르게 찾는다.

Source refs:

- 전략 문서의 성능 모델.

Deliverables:

- spatial index.
- viewport culling.
- hit-test acceleration.
- culling debug view or stats.

Verify:

- visible object count와 total object count가 분리되어 측정된다.
- pan/zoom 중 object 표현 전환 없이 culling이 작동한다.

Acceptance:

- 큰 scene에서도 hit test와 render candidate lookup이 안정적이다.

Depends on: D3

Parallel wave: G

Stop or ask if:

- culling이 사용자에게 pop-in으로 느껴진다.

### T6.2 Geometry, Text, And Edge Caches

Outcome: 반복 layout/geometry/text 작업을 cache해 frame stability를 높인다.

Source refs:

- 전략 문서의 성능 모델.
- 전략 문서의 JS/WASM Boundary 리스크.

Deliverables:

- card geometry cache.
- text layout/cache.
- edge route cache.
- cache invalidation rules.

Verify:

- drag/pan/zoom 중 cache hit/miss가 기록된다.
- text 변경 시 필요한 cache만 invalidation된다.

Acceptance:

- frame drop 원인이 불필요한 recomputation으로 남지 않는다.

Depends on: D3, T6.1

Parallel wave: serial

Stop or ask if:

- cache invalidation이 scene patch semantics보다 복잡해진다.

### T6.3 Boundary And Memory Profiling

Outcome: JS/WASM boundary와 memory usage가 scale bottleneck이 아닌지 확인한다.

Source refs:

- 전략 문서의 JS/WASM Boundary 리스크.

Deliverables:

- frame당 JS/WASM call count.
- data transfer size.
- memory usage profile.
- panic/error diagnostics.

Verify:

- scene patch/input batching이 작동한다.
- boundary overhead가 주요 bottleneck이면 mitigation plan이 있다.

Acceptance:

- Rust 도입의 성능 이점이 boundary overhead로 사라지지 않는다.

Depends on: D3

Parallel wave: G

Stop or ask if:

- boundary overhead가 renderer 이득보다 커진다.

### T6.4 Visual Regression And UX Review Fixtures

Outcome: canvas 품질을 반복 검증할 수 있는 screenshot/visual fixture를 만든다.

Source refs:

- 전략 문서의 continuous canvas 목표.

Deliverables:

- desktop/mobile or viewport-size screenshots.
- zoom level capture set.
- editing overlay alignment captures.
- visual regression checklist.

Verify:

- same fixture가 deterministic하게 capture된다.
- object overlap, text clipping, blank canvas, overlay misalignment를 잡을 수 있다.

Acceptance:

- renderer 변경이 UX를 깨는지 반복적으로 확인할 수 있다.

Depends on: D3, T5.2, T5.3

Parallel wave: G

Stop or ask if:

- visual approval 기준이 사람마다 달라져 product owner 판단이 필요하다.

## Phase P7: Final Review And Replacement Plan

Goal: prototype, POC parity, hardening 결과를 바탕으로 current DOM/SVG scene canvas를 새 engine으로 대체하는 실제 migration scope를 정한다.

Why now: 이 단계 전에는 새 engine이 current app workflow를 대체할 만큼 증거 기반인지, 어떤 순서로 production source를 교체해야 하는지 알 수 없다.

Tasks: T7.1, T7.2, T7.3

Verify or evaluate:

- replacement recommendation이 evidence에 근거한다.
- 남은 리스크와 temporary fallback removal 전략이 명확하다.

Review gate:

- `human-decision`: production replacement 순서, fallback 제거 시점, release scope를 승인한다.

### T7.1 Evidence Summary

Outcome: 모든 prototype/parity/benchmark 결과를 하나의 decision summary로 묶는다.

Deliverables:

- renderer decision recap.
- editing boundary recap.
- POC app parity recap.
- performance comparison.
- unresolved risks.

Verify:

- 각 결론이 task output 또는 benchmark artifact에 연결된다.

Acceptance:

- 최종 replacement plan에 필요한 증거가 한 문서에서 추적된다.

Depends on: P6 complete

Parallel wave: H

Stop or ask if:

- 핵심 benchmark가 누락되어 결론을 낼 수 없다.

### T7.2 Production Migration Scope

Outcome: current DOM/SVG scene canvas replacement의 실제 범위와 순서를 정한다.

Deliverables:

- migration phases.
- temporary fallback lifetime.
- compatibility requirements.
- deleted/deprecated current-renderer behavior list.
- production source replacement order.

Verify:

- scope가 evidence summary와 모순되지 않는다.
- current production workflow가 무리 없이 이어진다.

Acceptance:

- 다음 구현 cycle이 research가 아니라 migration execution으로 시작할 수 있다.

Depends on: T7.1

Parallel wave: serial

Stop or ask if:

- product owner가 temporary fallback removal 정책 또는 release scope를 결정해야 한다.

### T7.3 Replacement Execution Recommendation

Outcome: 다음 cycle에서 replacement execution을 어떻게 시작할지 명확히 권고한다.

Deliverables:

- replacement readiness recommendation.
- reasoned tradeoff.
- cost to next milestone.
- recommended next task.

Verify:

- 권고가 선호가 아니라 증거와 비용에 기반한다.

Acceptance:

- 다음 사람이 production replacement를 어디서 시작해야 하는지 알 수 있다.

Depends on: T7.1, T7.2

Parallel wave: serial

Stop or ask if:

- 결과가 애매해서 추가 benchmark 없이는 결론을 낼 수 없다.

## Parallelization Map

Safe parallel groups:

- Wave A: T0.1, T0.2, T0.3. 모두 read-heavy/discovery 중심이다.
- Wave B: T1.1 and T1.3 can start after baseline if ownership is separated. T1.2 waits for scaffold.
- Wave C: T2.1, T2.2, T2.3 can run after harness exists if each writes to isolated `poc/` areas and only shares measured outputs.
- Wave E: T4.1, T4.3, T4.4 can be drafted in parallel after D2, then T4.2 wires them.
- Wave G: T6.1, T6.3, T6.4 can begin after D3 if they do not edit the same renderer internals. T6.2 depends on T6.1.

Serial work:

- T1.2 before renderer foundation work.
- T1.4 before performance comparison.
- T2.4 after custom renderer foundation tasks.
- T3.2 after T3.1.
- T3.3 after hit testing and camera/drag basics.
- T4.2 after adapter and interaction patch evidence.
- T5 parity tasks should be mostly serial because they share POC app state semantics.
- Final replacement recommendation should be serial.

## Next Unblocked Tasks

1. Browser-verify the POC `Real scene` rendered load/save path against a running backend for group tag attach/filtering, comments, product export preview, group drag, card drag, text edit, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge create, and edge delete.
2. Harden visible Rust/wgpu rendering for production real text shaping/font fallback/Korean glyph quality, product card styling, edge fidelity, broader dirty-range updates, and hit/overlay ownership.
3. Capture visual QA for desktop/mobile viewports and representative zoom/edit states on the WebGPU-only POC path.
4. Browser-verify the render snapshot comparison route and POC `Real scene` mode against the same real backend scene.

These tasks are the remaining evidence gate before production source replacement can start. They do not redefine the final target: the replacement cycle must still delete the DOM/SVG scene canvas path after the current replacement acceptance criteria pass.

## Resolved Decisions

- Renderer path: custom Rust + wgpu/WebGPU renderer. Vello/CanvasKit/Graphite/Figma and related projects are references, not primary implementation choices.
- Editing boundary: normal objects are Rust-rendered scene objects; active text/card editing uses DOM overlay; selected-card affordances stay in the TypeScript DOM shell.
- POC boundary: all implementation, fixtures, adapters, benchmarks, and prototype UI live under `poc/` until a replacement migration cycle starts.
- Replacement target: current DOM/SVG scene canvas path is temporary; final goal is to replace and remove it, not keep it as a long-lived alternative mode.
- Accessibility bar: POC must expose future-compatible accessibility data hooks, but full keyboard/screen-reader support is not an early release gate.
- Native/non-web target: web-first for this task graph; native is not designed in unless future evidence justifies a separate plan.

## Stop Conditions

Stop and ask for human direction if:

- Custom renderer prototype cannot meet continuous canvas UX without visible representation swaps.
- DOM overlay editing cannot preserve inline edit quality, especially Korean IME and zoom alignment.
- Rust scene contract starts absorbing AI/MCP/business logic.
- JS/WASM boundary overhead erases the expected performance benefit.
- The work requires browser-only experimental APIs such as HTML-in-Canvas as a foundation.
- POC parity would require editing production source outside `poc/` before evidence is sufficient.
- Scope expands toward a general-purpose Figma/Illustrator clone.

## Verification Commands

Use existing project commands where they apply:

```bash
npm run typecheck
npm run test:unit
npm run build
npm run renderer:test
npm run renderer:rust:test
```
