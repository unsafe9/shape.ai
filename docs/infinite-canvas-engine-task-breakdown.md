# Infinite Canvas Engine Task Breakdown

> Source: [Infinite Canvas Engine Strategy](./infinite-canvas-engine-strategy.md)

이 문서는 무한 캔버스 전략을 실행 가능한 phase/task graph로 분해한 것이다. 구현 지시는 아니며, 사람 또는 에이전트가 순차 실행하고 검증할 수 있는 작업 단위와 decision gate를 정의한다.

## Objective

shape.ai의 현재 React Flow 기반 그래프 캔버스를 최종적으로 Illustrator/Figma-like continuous vector canvas로 대체할 수 있는 Rust/WASM/WebGPU 기반 canvas engine 경로를 검증하고, 충분한 증거가 쌓이면 현재 앱에 통합한다.

## Done Criteria

이 task graph가 완료됐다고 볼 수 있는 상태:

- Rust/WASM canvas core가 browser canvas에 mount된다.
- 하나의 world scene 안에서 card, text snippet, edge를 연속적으로 pan/zoom 렌더링한다.
- 1,000개 이상 card/edge fixture에서 frame time, memory, interaction latency를 측정한다.
- DOM overlay 기반 inline text editing이 실제 카드 내부 편집처럼 동작한다.
- TypeScript app layer와 Rust render scene의 책임 경계가 명확하다.
- 현재 shape graph 하나를 새 canvas scene으로 변환해 앱에서 사용할 수 있다.
- React Flow baseline과 새 canvas path의 성능/UX/복잡도 비교가 문서화되어 있다.
- HTML-in-Canvas 없이도 실행 가능한 경로가 있다.

## Locked Inputs

- 최종 목표는 semantic LOD 중심 UX가 아니라 continuous vector canvas다.
- HTML-in-Canvas는 지금 foundation이 아니다.
- Rust는 business logic 이전용이 아니라 canvas/graphics core 후보로만 사용한다.
- TypeScript app layer는 shape business model, persistence, AI/MCP workflow, comments/export/proposals를 계속 책임진다.
- DOM은 app chrome, floating UI, active editing overlay에 사용한다.
- 평상시 canvas object는 live DOM element가 아니라 renderer scene object다.
- 렌더러 선택은 prototype evidence 이후 결정한다. 기본 후보는 Rust + wgpu + Vello다.

## Must-Haves

- Continuous zoom: 줌 중 객체 표현이 dot/cluster/다른 카드로 갑자기 바뀌지 않는다.
- Retained scene: 모든 객체는 하나의 world scene과 stable object id를 가진다.
- Batched boundary: JS/WASM 호출은 scene patch, input batch, frame render 중심으로 묶는다.
- Active edit bridge: 텍스트 편집은 DOM overlay로 처리하되 scene과 좌표 동기화가 정확해야 한다.
- Measured decision: Vello/wgpu, CanvasKit, custom wgpu 중 무엇을 선택할지 benchmark와 visual evidence로 판단한다.
- Migration safety: 현재 React Flow path를 즉시 삭제하지 않고 비교/대체 가능한 migration path를 둔다.

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
  -> P2 Renderer Evidence
  -> D1 Renderer Decision
  -> P3 Interaction Vertical Slice
  -> D2 Editing Boundary Decision
  -> P4 Shape Scene Contract
  -> P5 App Integration Slice
  -> D3 Migration Decision
  -> P6 Scale And Hardening
  -> P7 Final Review
```

Renderer 비교와 Graphite deep dive는 P1/P2와 일부 병렬 가능하다. 현재 앱 통합은 renderer와 editing boundary가 증거로 통과하기 전까지 시작하지 않는다.

## Phase P0: Evidence Baseline

Goal: 현재 React Flow path와 원하는 canvas engine path 사이의 비교 기준을 만든다.

Why now: 기준 없이 Rust/WebGPU 작업을 시작하면 "빠른가"와 "충분한가"를 판단할 수 없다.

Tasks: T0.1, T0.2, T0.3

Verify or evaluate:

- baseline report가 문서화되어 있다.
- 현재 앱의 핵심 canvas workflow가 목록화되어 있다.
- benchmark 목표가 숫자 또는 관찰 기준으로 정해져 있다.

Review gate:

- `human-decision`: benchmark 목표와 UX 판단 기준이 제품 목표를 제대로 반영하는지 승인한다.

### T0.1 Current Canvas Workflow Baseline

Outcome: 현재 React Flow 기반 workflow와 성능/UX 한계를 비교 기준으로 기록한다.

Source refs:

- README의 Web UI graph editing 설명.
- `docs/infinite-canvas-engine-strategy.md`의 현재 맥락과 핵심 결정.

Read first:

- `README.md`
- 현재 React Flow canvas entrypoint
- 관련 frontend tests 또는 e2e tests

Deliverables:

- 현재 가능한 workflow 목록.
- React Flow path에서 유지해야 할 UX 목록.
- 새 canvas가 대체해야 하는 interaction checklist.
- 현재 baseline 측정 방법.

Verify:

- `npm run typecheck`
- 현재 앱을 실행해 pan/zoom/edit/select/connect/export 흐름이 어디에서 일어나는지 확인한다.

Acceptance:

- 새 엔진이 반드시 보존해야 할 workflow와 버려도 되는 React Flow-specific behavior가 분리되어 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- 현재 product workflow 자체가 바뀌어야 하는지 결정이 필요해진다.

### T0.2 Success Metrics And Benchmark Fixture

Outcome: renderer prototype을 평가할 수 있는 fixture와 성공 기준을 정의한다.

Source refs:

- 전략 문서의 성능 모델.
- 전략 문서의 Vello/wgpu prototype 검증 과제.

Deliverables:

- card/edge/text fixture 규모 정의.
- frame time, memory, interaction latency, text quality 평가 기준.
- zoom/pan 시각 검토 checklist.
- React Flow baseline과 비교할 최소 fixture.

Verify:

- fixture 기준이 "1,000개 card + edge + text snippet" 이상을 포함한다.
- 숫자로 측정 가능한 항목과 사람이 봐야 하는 항목이 분리되어 있다.

Acceptance:

- renderer 선택을 감으로 하지 않고 같은 fixture로 비교할 수 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- 목표 성능 기준이 제품 기대와 맞지 않는다고 판단된다.

### T0.3 Source Architecture Deep Dive

Outcome: Graphite/Figma/Vello/wgpu 참고가 실제 구현 task에 쓸 수 있는 수준으로 정리된다.

Source refs:

- 전략 문서의 Figma 조사 메모.
- 전략 문서의 오픈소스 참고.

Deliverables:

- Graphite의 Rust backend/web frontend boundary 요약.
- Vello caveat 목록과 shape.ai에 미치는 영향.
- wgpu browser publishing 제약 정리.
- 가져오면 안 되는 Graphite/Figma급 과복잡도 목록.

Verify:

- 각 결론이 링크 또는 코드 위치에 연결되어 있다.
- downstream task가 사용할 "적용할 패턴"과 "피할 패턴"이 구분되어 있다.

Acceptance:

- P1/P2 작업자가 renderer/API 구조를 잡을 때 참고할 수 있다.

Depends on: none

Parallel wave: A

Stop or ask if:

- Vello/wgpu가 현재 browser target에서 명백히 부적합하다는 증거가 나온다.

## Phase P1: Prototype Scaffold

Goal: 현재 앱을 건드리지 않고 Rust/WASM canvas prototype을 실행할 수 있는 격리된 기반을 만든다.

Why now: renderer 실험은 production React Flow path와 분리되어야 한다. 그래야 실패해도 앱을 망가뜨리지 않고 비교할 수 있다.

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

- 새 prototype package 또는 experiment directory.
- production React Flow canvas path는 수정하지 않는다.

Deliverables:

- Rust crate 또는 package scaffold.
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

- repo 구조상 Rust toolchain을 어디에 둘지 product-level 결정이 필요하다.

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

- 이후 renderer, hit test, app integration이 같은 contract를 기준으로 작업할 수 있다.

Depends on: T1.1

Parallel wave: B

Stop or ask if:

- product data model 자체가 renderer scene에 필요한 값을 제공하지 못한다.

### T1.4 Benchmark Harness Skeleton

Outcome: prototype에서 같은 fixture로 renderer 후보를 비교할 수 있는 harness를 만든다.

Source refs:

- T0.2 benchmark fixture.
- 전략 문서의 Renderer 비교 검증 과제.

Deliverables:

- deterministic fixture generator.
- frame stats collection.
- pan/zoom scripted path.
- screenshot or visual capture path.

Verify:

- 같은 seed로 같은 scene이 생성된다.
- frame stats가 기록된다.

Acceptance:

- P2 renderer 후보가 같은 기준으로 비교된다.

Depends on: T0.2, T1.2, T1.3

Parallel wave: serial

Stop or ask if:

- benchmark 결과를 저장할 위치나 형식이 repo policy와 충돌한다.

## Phase P2: Renderer Evidence

Goal: renderer 후보를 감이 아니라 evidence로 비교한다.

Why now: renderer 선택은 downstream architecture를 크게 바꾼다. app integration 전에 결정해야 한다.

Tasks: T2.1, T2.2, T2.3, T2.4

Verify or evaluate:

- 같은 fixture에서 renderer 후보별 frame stats와 visual notes가 있다.
- Vello/wgpu를 계속 쓸지, CanvasKit으로 전환할지, custom path를 열지 결정할 수 있다.

Review gate:

- `human-decision`: D1 Renderer Decision.

### T2.1 Vello/wgpu Card Graph Renderer Spike

Outcome: Vello/wgpu 기반으로 card, text snippet, edge를 렌더링한다.

Source refs:

- 전략 문서의 Track A.
- 전략 문서의 Product-Specific Implications.

Deliverables:

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

- Vello/wgpu가 shape.ai canvas의 1차 후보로 유지 가능한지 판단할 수 있다.

Depends on: T1.2, T1.3, T1.4

Parallel wave: C

Stop or ask if:

- Vello/wgpu API 또는 browser support가 핵심 요구를 막는다.

### T2.2 CanvasKit Comparison Spike

Outcome: CanvasKit/Skia가 Vello/wgpu 대비 더 적합한 fallback인지 비교한다.

Source refs:

- 전략 문서의 Track B.

Deliverables:

- 같은 fixture의 CanvasKit render result.
- text/path quality comparison.
- package size/build complexity note.
- app integration cost note.

Verify:

- T1.4 harness와 동일하거나 동등한 fixture로 비교한다.
- Vello/wgpu와 비교 가능한 report를 만든다.

Acceptance:

- CanvasKit을 primary 또는 fallback으로 둘 가치가 있는지 판단할 수 있다.

Depends on: T1.4

Parallel wave: C

Stop or ask if:

- CanvasKit adoption이 Rust-first core 결정을 근본적으로 바꾸어야 한다.

### T2.3 Custom wgpu Primitive Feasibility Note

Outcome: custom Rust tessellation + wgpu가 실제로 열어둘 가치가 있는지 판단한다.

Source refs:

- 전략 문서의 Track C.
- 전략 문서의 Lyon/Kurbo/Peniko/Swash/Cosmic Text 참고.

Deliverables:

- card/edge/text를 custom primitive로 만들 때 필요한 building block 목록.
- text rendering 난이도 판단.
- Vello/CanvasKit 대비 ownership/cost 비교.

Verify:

- 직접 구현해야 하는 risky subsystem이 명확히 드러난다.

Acceptance:

- custom renderer를 지금 선택할지, 미래 fallback으로 둘지 결정할 수 있다.

Depends on: T0.3

Parallel wave: C

Stop or ask if:

- custom path가 product work를 장기간 막을 정도로 커진다.

### T2.4 Renderer Decision Report

Outcome: renderer 후보 비교를 하나의 결론으로 묶는다.

Source refs:

- T2.1, T2.2, T2.3 outputs.

Deliverables:

- renderer recommendation.
- rejected alternatives and reasons.
- risks to carry forward.
- next phase changes if recommendation differs from default.

Verify:

- recommendation이 benchmark, visual evidence, integration complexity에 근거한다.

Acceptance:

- D1에서 한 후보를 선택하거나, 제한된 추가 probe만 남긴다.

Depends on: T2.1, T2.2, T2.3

Parallel wave: serial

Stop or ask if:

- 후보들이 모두 핵심 기준을 통과하지 못한다.

## Decision D1: Renderer Decision

Default recommendation before evidence: Rust + wgpu + Vello-inspired renderer.

Decision options:

- Continue with Vello/wgpu.
- Switch prototype focus to CanvasKit/Skia.
- Open a custom Rust/wgpu renderer path.
- Stop engine path and revisit product expectations.

Required evidence:

- frame stats.
- visual quality screenshots.
- text rendering notes.
- browser integration notes.
- implementation complexity estimate.

Downstream impact:

- P3/P4/P5는 선택된 renderer를 기준으로 진행한다.
- 선택되지 않은 후보는 fallback note로 남긴다.

## Phase P3: Interaction Vertical Slice

Goal: canvas가 단순히 그리는 것이 아니라 직접 조작 가능한 editor가 될 수 있음을 검증한다.

Why now: pan/zoom rendering이 좋아도 selection, drag, hit test, text edit가 어색하면 제품으로 쓸 수 없다.

Tasks: T3.1, T3.2, T3.3, T3.4

Verify or evaluate:

- 작은 scene에서 select/drag/edit 흐름이 가능하다.
- DOM overlay가 canvas object와 정확히 연결된다.

Review gate:

- `human-verify`: inline editing이 현재 제품의 "노드 안에서 바로 편집" 감각을 유지하는지 확인한다.

### T3.1 Hit Testing And Selection

Outcome: rendered object를 클릭했을 때 stable object id와 selection geometry가 반환된다.

Source refs:

- 전략 문서의 Canvas scene 모델.
- 전략 문서의 JavaScript API 형태.

Deliverables:

- `hitTest` API.
- card/edge/text hit region.
- selected object highlight.
- selection event output.

Verify:

- 여러 zoom level에서 같은 object id가 선택된다.
- selection highlight가 object geometry와 맞다.

Acceptance:

- UI layer가 Rust core의 hit result만으로 selection state를 표시할 수 있다.

Depends on: D1, T1.3, T2.1 or selected renderer task

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

- 직접 조작이 React Flow path와 비교 가능한 수준으로 동작한다.

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

## Decision D2: Editing Boundary Decision

Decision question:

- DOM overlay editing이 제품 품질을 만족하는가?

Possible outcomes:

- Continue with DOM overlay for active edit.
- Add more DOM affordances for selected card only.
- Reconsider full DOM card rendering for near-edit state.
- Stop and redesign text editing model.

Required evidence:

- Korean IME result.
- zoomed edit alignment.
- selection/copy/paste behavior.
- commit/undo implications.

Downstream impact:

- P4 scene contract에 text edit target, overlay geometry, patch shape가 확정된다.

## Phase P4: Shape Scene Contract

Goal: current shape graph data를 renderer scene으로 변환하는 안정적인 contract를 만든다.

Why now: renderer와 interaction slice가 증명된 뒤에야 current app data와 결합할 가치가 있다.

Tasks: T4.1, T4.2, T4.3, T4.4

Verify or evaluate:

- TypeScript business graph와 Rust canvas scene이 분리된다.
- scene patch가 app state update로 되돌아오는 흐름이 정의된다.

Review gate:

- `human-decision`: Rust가 가져가는 scene 의미가 과하거나 부족하지 않은지 승인한다.

### T4.1 Business Graph To Scene Adapter

Outcome: stored shape graph를 renderable scene snapshot으로 변환한다.

Source refs:

- README의 typed shape graph 설명.
- 전략 문서의 책임 경계.

Deliverables:

- app-level adapter contract.
- node -> card mapping.
- edge -> route mapping.
- comment/export/proposal field exclusion rule.

Verify:

- sample shape가 deterministic scene snapshot으로 변환된다.
- business-only fields가 Rust scene에 들어가지 않는다.

Acceptance:

- 현재 shape data를 canvas engine이 렌더링할 수 있는 input으로 만들 수 있다.

Depends on: D2, T1.3

Parallel wave: E

Stop or ask if:

- current data model에서 visual bounds/layout source가 불명확하다.

### T4.2 Scene Patch To App Update Contract

Outcome: engine interaction 결과가 app state와 persistence로 돌아가는 patch contract를 정의한다.

Source refs:

- 전략 문서의 JavaScript API 형태.
- README의 layout persistence, inline field edits, edge creation 설명.

Deliverables:

- drag patch.
- text edit patch.
- edge creation/deletion patch.
- selection patch.
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

## Phase P5: App Integration Slice

Goal: 현재 shape.ai app에서 하나의 graph path를 새 canvas로 렌더링하고 편집한다.

Why now: prototype이 제품 데이터와 만나야 실제 migration 가능성을 판단할 수 있다.

Tasks: T5.1, T5.2, T5.3, T5.4

Verify or evaluate:

- 기존 app에서 새 canvas path를 켜고 끌 수 있다.
- React Flow baseline과 같은 shape를 비교할 수 있다.

Review gate:

- `human-decision`: React Flow fallback 유지 여부와 migration 범위를 결정한다.

### T5.1 Feature-Gated Canvas Integration

Outcome: current app에 새 canvas path를 feature gate로 연결한다.

Source refs:

- 전략 문서의 Phase 4: Current App Integration.
- README의 local development commands.

Files/ownership:

- app canvas shell.
- feature gate/config.
- React Flow path는 fallback으로 유지한다.

Deliverables:

- feature-gated route or mode.
- canvas mount lifecycle.
- shape selection/load integration.

Verify:

- feature off: 기존 React Flow path가 그대로 동작한다.
- feature on: 새 canvas가 같은 shape scene을 렌더링한다.

Acceptance:

- integration 실패가 기존 app workflow를 막지 않는다.

Depends on: T4.1, T4.2, T4.3

Parallel wave: F

Stop or ask if:

- fallback 유지가 routing/state 구조상 과도한 복잡도를 만든다.

### T5.2 Selection And Editing Integration

Outcome: 새 canvas에서 selection, inline edit, drag가 app state/persistence와 연결된다.

Source refs:

- README의 graph editing workflow.
- T4.2 patch contract.

Deliverables:

- selected node/edge state sync.
- text edit save path.
- layout persistence path.
- error handling for rejected patches.

Verify:

- card drag 후 reload해도 위치가 유지된다.
- inline edit 후 app/backend state가 갱신된다.
- edge/node selection detail panel이 동작한다.

Acceptance:

- 최소 편집 workflow가 React Flow path와 비교 가능하다.

Depends on: T5.1, T4.2, T3.3

Parallel wave: serial

Stop or ask if:

- direct UI edit와 proposal workflow의 정책 경계가 바뀌어야 한다.

### T5.3 Edge Workflow Integration

Outcome: 새 canvas에서 edge creation/selection/deletion이 current graph workflow와 연결된다.

Source refs:

- README의 edge creation and deletion workflow.
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

Outcome: 새 canvas path가 기존 export 흐름을 깨지 않는다.

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

- canvas renderer 교체가 graph export semantics를 바꾸지 않는다.

Depends on: T5.2, T5.3

Parallel wave: serial

Stop or ask if:

- image export를 renderer output 기반으로 바꾸는 scope creep가 생긴다.

## Decision D3: Migration Decision

Decision question:

- 새 canvas path를 React Flow replacement로 계속 진행할 만큼 증거가 충분한가?

Possible outcomes:

- Continue migration and keep React Flow as temporary fallback.
- Continue prototype only; production integration deferred.
- Keep React Flow and use engine only for large-canvas mode.
- Stop Rust/WASM path and revisit DOM/CanvasKit/other approach.

Required evidence:

- feature-gated app integration result.
- React Flow baseline comparison.
- user-facing editing workflow result.
- performance report.
- complexity/risk estimate.

Downstream impact:

- P6의 hardening scope와 fallback lifetime이 결정된다.

## Phase P6: Scale And Hardening

Goal: 대규모 그래프에서 continuous canvas가 안정적으로 유지되도록 최적화하고 검증한다.

Why now: vertical slice와 app integration이 통과된 뒤에야 scale optimization이 정확한 대상을 가진다.

Tasks: T6.1, T6.2, T6.3, T6.4

Verify or evaluate:

- large fixture에서 pan/zoom/edit/select가 측정된다.
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

## Phase P7: Final Review And Migration Plan

Goal: prototype과 integration 결과를 바탕으로 다음 실제 migration scope를 결정한다.

Why now: 이 단계 전에는 engine path가 증거 기반인지, 연구 과제가 제품 개발을 과하게 잡아먹는지 알 수 없다.

Tasks: T7.1, T7.2, T7.3

Verify or evaluate:

- migration recommendation이 evidence에 근거한다.
- 남은 리스크와 fallback 전략이 명확하다.

Review gate:

- `human-decision`: 새 canvas engine을 production replacement로 밀지, 제한된 mode로 둘지, 중단할지 결정한다.

### T7.1 Evidence Summary

Outcome: 모든 prototype/integration/benchmark 결과를 하나의 decision summary로 묶는다.

Deliverables:

- renderer decision recap.
- editing boundary recap.
- app integration recap.
- performance comparison.
- unresolved risks.

Verify:

- 각 결론이 task output 또는 benchmark artifact에 연결된다.

Acceptance:

- 최종 migration decision에 필요한 증거가 한 문서에서 추적된다.

Depends on: P6 complete

Parallel wave: H

Stop or ask if:

- 핵심 benchmark가 누락되어 결론을 낼 수 없다.

### T7.2 Production Migration Scope

Outcome: React Flow replacement의 실제 범위와 순서를 정한다.

Deliverables:

- migration phases.
- fallback lifetime.
- compatibility requirements.
- deleted/deprecated React Flow behavior list.

Verify:

- scope가 evidence summary와 모순되지 않는다.
- current production workflow가 무리 없이 이어진다.

Acceptance:

- 다음 구현 cycle이 research가 아니라 migration execution으로 시작할 수 있다.

Depends on: T7.1

Parallel wave: serial

Stop or ask if:

- product owner가 fallback 정책 또는 release scope를 결정해야 한다.

### T7.3 Stop/Continue Recommendation

Outcome: engine path를 계속 갈지, 축소할지, 중단할지 명확히 권고한다.

Deliverables:

- continue/limit/stop recommendation.
- reasoned tradeoff.
- cost to next milestone.
- recommended next task.

Verify:

- 권고가 선호가 아니라 증거와 비용에 기반한다.

Acceptance:

- 다음 사람이 "무엇을 지금 해야 하는지" 알 수 있다.

Depends on: T7.1, T7.2

Parallel wave: serial

Stop or ask if:

- 결과가 애매해서 추가 benchmark 없이는 결론을 낼 수 없다.

## Parallelization Map

Safe parallel groups:

- Wave A: T0.1, T0.2, T0.3. 모두 read-heavy/discovery 중심이다.
- Wave B: T1.1 and T1.3 can start after baseline if ownership is separated. T1.2 waits for scaffold.
- Wave C: T2.1, T2.2, T2.3 can run as separate renderer spikes after harness exists, but each must write to isolated spike areas.
- Wave E: T4.1, T4.3, T4.4 can be drafted in parallel after D2, then T4.2 wires them.
- Wave G: T6.1, T6.3, T6.4 can begin after D3 if they do not edit the same renderer internals. T6.2 depends on T6.1.

Serial work:

- T1.2 before renderer spikes.
- T1.4 before evidence comparison.
- T2.4 after renderer spikes.
- T3.2 after T3.1.
- T3.3 after hit testing and camera/drag basics.
- T4.2 after adapter and interaction patch evidence.
- T5 integration tasks should be mostly serial because they touch shared app state.
- Final migration recommendation should be serial.

## Next Unblocked Tasks

1. T0.1 Current Canvas Workflow Baseline
2. T0.2 Success Metrics And Benchmark Fixture
3. T0.3 Source Architecture Deep Dive

These three can run before any implementation. They create the evidence boundary that prevents the Rust/WebGPU work from becoming open-ended engine research.

## Open Decisions

- Renderer choice after D1: Vello/wgpu, CanvasKit, custom wgpu, or stop.
- DOM overlay quality after D2: acceptable as final editing model or needs redesign.
- React Flow fallback after D3: temporary migration aid, long-lived alternative mode, or removal path.
- Accessibility release bar: future-compatible hook only, or early keyboard/screen-reader support.
- Native/non-web target: keep optional, or explicitly design for web-only first.

## Stop Conditions

Stop and ask for human direction if:

- Renderer prototype cannot meet continuous canvas UX without visible representation swaps.
- DOM overlay editing cannot preserve inline edit quality, especially Korean IME and zoom alignment.
- Rust scene contract starts absorbing AI/MCP/business logic.
- JS/WASM boundary overhead erases the expected performance benefit.
- The work requires browser-only experimental APIs such as HTML-in-Canvas as a foundation.
- Current app integration would require deleting React Flow fallback before evidence is sufficient.
- Scope expands toward a general-purpose Figma/Illustrator clone.

## Verification Commands

Use existing project commands where they apply:

```bash
npm run typecheck
npm run test:unit
npm run build
npm run test:e2e
```

Prototype-specific commands should be added by T1.1. Until then, renderer work must report the exact build/run commands it introduces.
