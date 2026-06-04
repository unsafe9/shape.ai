# Infinite Canvas Engine Strategy

> 목표: shape.ai의 그래프 편집 경험을 React Flow 기반 노드 에디터에서 Illustrator/Figma에 가까운 연속적인 무한 캔버스로 발전시킨다.

이 문서는 무한 캔버스 구현 방향에 대한 브레인스토밍, 결정사항, 조사 결과, 참고 오픈소스, 기술적 리스크, 다음 검증 과제를 한 곳에 모은 전략 문서다.

## 현재 맥락

shape.ai는 현재 React, Vite, `@xyflow/react`를 기반으로 그래프 캔버스를 구성하고 있다. React Flow는 제품 초기 검증에는 좋다. 노드 편집, 엣지 연결, 선택 상태, 레이아웃 저장 같은 기능을 빠르게 만들 수 있기 때문이다.

하지만 우리가 원하는 최종 상태는 React Flow의 확장판이 아니다.

원하는 방향은 다음에 가깝다.

- 모든 그래프가 하나의 큰 월드 좌표계 안에 존재한다.
- 사용자는 계속 확대/축소/이동해도 같은 대상을 보고 있다고 느낀다.
- 멀리 있다고 점이나 클러스터로 갑자기 바뀌는 느낌은 피한다.
- 확대해도 깨지지 않고, 축소해도 캔버스 이동이 부드러워야 한다.
- DOM 노드 수, React commit, 레이아웃 재계산이 프레임 성능을 지배하면 안 된다.

즉, 목표는 "DOM 기반 그래프 에디터를 더 최적화"하는 것이 아니라 "캔버스 자체가 제품의 중심인 그래픽 엔진"에 가깝다.

## 핵심 결정

- 최종 캔버스는 Illustrator/Figma처럼 연속적인 벡터 캔버스를 지향한다.
- React Flow는 현재 제품 스캐폴드이자 비교 기준으로 남길 수 있지만, 최종 렌더러로 보지 않는다.
- HTML-in-Canvas는 당장 기반 기술로 쓰지 않는다. 현재 Chrome Canary/origin trial 조건이 강하고, 표준/브라우저 지원이 안정적이지 않다.
- DOM/Web Components는 전체 캔버스 객체 표현이 아니라 활성 편집, 플로팅 UI, 패널, 메뉴에 사용한다.
- Rust는 비즈니스 로직을 옮기기 위한 언어가 아니라 캔버스/그래픽 코어를 만들기 위한 후보로 본다.
- 웹 앱은 Rust 코어를 WebAssembly로 받아 JavaScript API를 통해 사용한다.
- 제품 로직, AI/MCP 워크플로우, 저장, 댓글, export 정책은 TypeScript 앱 레이어에 남긴다.

## 명시적 비목표

- 전체 CSS layout engine을 새로 만들지 않는다.
- 모든 노드와 엣지를 live DOM element로 유지하지 않는다.
- Rust 안에 제품 비즈니스 로직을 깊게 결합하지 않는다.
- Figma나 Illustrator 전체를 재구현하지 않는다.
- 확대/축소에 따라 카드가 점, 클러스터, 다른 카드 디자인으로 갑자기 바뀌는 semantic LOD를 기본 UX로 삼지 않는다.
- 브라우저별 실험 API에 최종 구조를 의존시키지 않는다.

## 사용자가 원하는 감각

가까이/멀리에 따라 표현이 바뀌는 방식은 기술적으로는 효율적이지만, 사용자 경험상 어색할 수 있다. 이번 목표는 "멀면 점, 가까우면 카드"가 아니다.

원하는 감각은 다음이다.

```text
객체는 항상 같은 객체다.
확대하면 더 잘 보인다.
축소하면 작아져서 읽기 어려울 수는 있다.
하지만 다른 표현으로 바뀌거나 사라지는 느낌은 최소화한다.
```

이것은 Illustrator나 Figma에서 벡터 객체를 다루는 감각에 가깝다. 객체가 멀리 있어도 존재 방식은 그대로이고, 카메라만 움직인다.

물론 내부적으로는 culling, caching, batching, spatial index, text cache 같은 최적화가 필요하다. 다만 그 최적화가 사용자에게 "표현이 바뀐다"는 느낌으로 드러나면 안 된다.

## 배제하거나 약화한 접근

### 1. Full DOM / Web Components Canvas

장점:

- CSS 디자인이 쉽다.
- inline text edit가 자연스럽다.
- 접근성, IME, copy/paste, selection을 브라우저에 맡길 수 있다.
- 기존 React/웹 컴포넌트 생태계를 활용하기 쉽다.

문제:

- 캔버스 객체 수가 많아지면 DOM 수와 레이아웃 비용이 커진다.
- pan/zoom이 React/DOM/SVG 갱신과 묶이기 쉽다.
- 대규모 월드에서 viewport virtualization이 필요해지고, pop-in 느낌이 생긴다.
- Illustrator/Figma식 연속 벡터 캔버스 감각과는 거리가 있다.

결론:

- 전체 캔버스 표현에는 부적합하다.
- 활성 텍스트 편집, floating panel, context menu, toolbar에는 계속 적합하다.

### 2. HTML-in-Canvas

Chrome의 HTML-in-Canvas origin trial은 DOM 콘텐츠를 canvas/WebGL/WebGPU 쪽으로 그리는 방향이라 매우 흥미롭다. 이론상 CSS 기반 카드 디자인과 GPU canvas 렌더링을 이어줄 수 있다.

하지만 현재 판단은 보류다.

이유:

- Chrome Canary/origin trial 중심이다.
- API와 동작이 아직 안정된 표준으로 보기 어렵다.
- 개인용 초기 실험에는 가능하지만, 제품의 최종 구조를 여기에 고정하기에는 위험하다.
- 브라우저 지원이 넓어지기 전까지는 migration/fallback 부담이 크다.

결론:

- 미래 backend 후보로 추적한다.
- 지금의 최종 아키텍처 기반으로 삼지는 않는다.

### 3. Semantic LOD 중심 UX

semantic LOD는 멀리서 dot/cluster, 중간에서 compact card, 가까이서 full card를 보여주는 방식이다.

장점:

- 성능 최적화가 쉽다.
- 대규모 그래프 요약에는 유리하다.
- 지도나 데이터 시각화에서는 자연스럽다.

문제:

- 이번 제품이 원하는 Illustrator-like 감각과 다르다.
- 줌 과정에서 객체가 다른 것으로 바뀌는 느낌이 난다.
- 사용자가 "내가 보고 있던 카드"와 "렌더러가 보여주는 대체 표현" 사이의 연결을 의식하게 된다.

결론:

- 기본 UX로 두지 않는다.
- 추후 아주 먼 줌에서 overview helper로 제한적으로 쓸 수는 있다.
- 핵심은 continuous vector rendering이다.

## 권장 최종 아키텍처

```text
shape-canvas-core/        Rust
  렌더 가능한 scene/document primitive
  월드 좌표계와 camera
  spatial index와 hit testing
  card, edge, text, selection geometry
  render cache key
  renderer abstraction
  wgpu/WebGPU backend
  WASM bindings

shape-canvas-web/         TypeScript
  WASM loader
  JavaScript API facade
  canvas element lifecycle
  pointer/keyboard event bridge
  DOM overlay positioning
  browser integration/debug hooks

shape.ai app/             TypeScript
  제품 도메인 모델
  persistence/API
  AI/MCP workflow
  comments/export/proposals
  floating panels/commands
```

이 구조의 핵심은 Rust가 "앱 전체"가 아니라 "캔버스 엔진"이라는 점이다.

## 책임 경계

비즈니스 문서와 렌더링 scene을 분리해야 한다.

```text
Business document:
  node title
  summary/detail
  comments
  status/confidence
  evidence refs
  export/proposal state
  AI/MCP workflow state

Canvas scene:
  object id
  bounds/transform/z-order
  style key
  text runs
  ports
  edge route
  hit regions
  selection geometry
  render cache keys
```

Rust core는 렌더링과 상호작용에 필요한 최소한의 scene 의미만 알아야 한다. AI 워크플로우, 저장 정책, 댓글의 비즈니스 규칙, export 상태 같은 제품 로직까지 알 필요는 없다.

## Canvas 객체는 무엇으로 그릴 것인가

기본 상태에서는 DOM이 아니라 GPU-rendered scene object로 그린다.

```text
Normal state:
  Rust/WebGPU가 card, border, text preview, edge, handle, highlight를 렌더링한다.

Active editing:
  선택된 텍스트/카드 영역 위에 DOM textarea/input/contenteditable overlay를 띄운다.
  IME, selection, copy/paste, native input은 브라우저에 맡긴다.
  편집 완료 시 Rust scene patch로 반영한다.
```

이 방식은 "노드 내부를 바로 눌러 편집"하는 현재 제품 감각을 유지하면서도, 모든 노드를 DOM으로 유지하지 않게 해준다.

즉, 노드는 평소에는 GPU 객체이고, 편집 중인 한 순간에만 DOM 편집기가 올라온다.

## JavaScript API 형태

웹 앱은 Rust/WASM 코어를 canvas engine처럼 사용한다.

```ts
type ShapeCanvas = {
  mount(canvas: HTMLCanvasElement): Promise<void>;
  resize(width: number, height: number, devicePixelRatio: number): void;
  loadScene(scene: SceneSnapshot): void;
  applyPatch(patch: ScenePatch): void;
  setCamera(camera: CameraState): void;
  fitToBounds(bounds: WorldRect): void;
  pointerDown(event: PointerInput): CanvasEvent[];
  pointerMove(event: PointerInput): CanvasEvent[];
  pointerUp(event: PointerInput): CanvasEvent[];
  keyDown(event: KeyInput): CanvasEvent[];
  renderFrame(now: number): FrameStats;
  hitTest(point: ScreenPoint): HitResult | null;
  beginTextEdit(target: TextEditTarget): DomOverlayRequest;
  commitTextEdit(target: TextEditTarget, value: string): ScenePatch;
};
```

중요한 원칙:

- 매 프레임 수많은 작은 JS/WASM 호출을 만들지 않는다.
- scene patch와 input event는 batch로 넘긴다.
- Rust는 compact event list와 frame stats를 돌려준다.
- DOM overlay 요청은 Rust가 좌표/대상을 계산하고 TypeScript가 실제 DOM을 띄우는 식으로 분리한다.

## Rust 도입으로 얻는 것과 잃는 것

### 얻는 것

- WebGPU/wgpu 기반 렌더링 코어를 제품 구조의 중심으로 둘 수 있다.
- scene graph, hit test, culling, spatial index, render cache를 JS UI와 분리할 수 있다.
- 대량 객체 처리에서 React commit과 DOM layout 비용을 피할 수 있다.
- 웹뿐 아니라 추후 native shell 가능성도 열린다.
- Figma/Graphite류의 "웹 위에 올라간 자체 그래픽 엔진" 방향과 더 가까워진다.

### 잃는 것

- 개발 복잡도가 크게 올라간다.
- Rust/WASM 빌드, 디버깅, panic, sourcemap, profiling, memory management를 다뤄야 한다.
- 디자이너/프론트엔드 친화적인 CSS 기반 표현을 그대로 쓸 수 없다.
- 텍스트 렌더링, hit testing, selection, snapping, edge routing이 엔진 책임이 된다.
- JS/WASM boundary 설계를 잘못하면 Rust 성능 이점이 사라질 수 있다.
- renderer dependency가 alpha이거나 무거울 수 있다.

### 중요한 판단

Rust를 도입한다고 무한 캔버스가 자동으로 해결되지는 않는다. 하지만 "무한 캔버스를 제품의 핵심 그래픽 엔진으로 직접 소유하겠다"는 결정에는 Rust가 의미가 있다.

반대로 비즈니스 로직까지 Rust로 옮기는 것은 현재로서는 얻는 것보다 잃는 것이 크다.

## Renderer 선택지

### Track A: Rust + wgpu + Vello 탐색

Vello는 Rust의 GPU compute-centric 2D renderer이고 `wgpu`를 사용한다. 큰 2D scene을 interactive하게 렌더링하는 방향과 잘 맞는다.

장점:

- Rust/WebGPU 생태계와 잘 맞는다.
- Web/native 양쪽 가능성이 있다.
- modern GPU renderer 구조를 직접 이해하고 가져갈 수 있다.
- Graphite도 wgpu/Vello 계열을 참고할 만한 구현으로 사용한다.

단점:

- Vello는 아직 alpha 성격이 있다.
- blur/filter artifact, GPU memory allocation, glyph caching 같은 caveat가 공개적으로 언급되어 있다.
- 전체 editor interaction은 직접 만들어야 한다.

추천:

- 첫 프로토타입의 기본 후보.
- 단, renderer abstraction을 둬서 Vello 내부 의존을 바꿀 수 있게 한다.

### Track B: CanvasKit / Skia

CanvasKit은 Skia를 WebAssembly로 컴파일해 웹에서 사용할 수 있게 한 도구다. WebGL-backed vector drawing과 mature graphics API가 장점이다.

장점:

- Skia 기반이라 렌더링 성숙도가 높다.
- path/text/shader 기능이 강하다.
- custom vector renderer를 직접 만드는 부담을 줄인다.

단점:

- Rust-first 구조와 맞지 않는다.
- C++/Skia API와 dependency ownership이 커진다.
- 제품 엔진이 Skia wrapper처럼 굳어질 수 있다.

추천:

- 첫 선택보다는 fallback/benchmark 비교 대상으로 둔다.

### Track C: Custom Rust Tessellation + wgpu

Lyon, Kurbo, Peniko, Swash/Cosmic Text, wgpu 같은 building block으로 직접 renderer를 만든다.

장점:

- 최대한 제품에 맞는 엔진을 만들 수 있다.
- card/graph workspace에 필요한 기능만 최적화할 수 있다.
- dependency churn을 줄일 수 있다.

단점:

- 구현 비용이 가장 크다.
- antialiasing, text rendering, cache, shadow, gradient, path quality가 모두 직접 책임이 된다.
- 엔진 개발이 제품 개발을 압도할 위험이 있다.

추천:

- Vello/CanvasKit prototype이 요구 성능이나 시각 품질을 못 맞출 때만 선택한다.

## Figma 조사 메모

Figma는 직접적인 구현 템플릿이라기보다는 방향성의 증거다.

공개 글 기준으로 Figma는 초기부터 HTML/SVG/2D canvas만으로는 전문 디자인 툴의 성능과 줌 경험을 만들기 어렵다고 판단했다. C++로 작성한 코어를 asm.js/WebAssembly로 올렸고, custom WebGL renderer를 사용했다. 이후 WebAssembly 전환으로 load time을 크게 줄였고, 최근에는 WebGPU renderer도 도입했다.

중요한 점:

- Figma의 canvas object는 일반 CSS/DOM 카드가 아니다.
- 주변 UI는 웹 기술을 쓰지만, 캔버스 내부는 자체 그래픽 엔진에 가깝다.
- WebAssembly 성능 이득은 단순히 "WASM이 JS보다 빠르다"가 아니라 binary format, parsing/compilation/cache, 기존 C++ 코드, renderer architecture가 함께 만든 결과다.
- Figma가 WebGPU로 간 것도 renderer abstraction을 유지한 상태에서 backend를 진화시킨 흐름으로 봐야 한다.

shape.ai에 적용할 교훈:

```text
캔버스가 제품의 핵심이라면 renderer를 소유해야 한다.
하지만 Figma 전체를 복제할 필요는 없다.
shape.ai는 카드/그래프 workspace에 맞는 더 좁은 엔진이면 된다.
```

## 오픈소스 참고

### Graphite

Graphite는 가장 가까운 app-level 참고 사례다. Rust backend와 web frontend를 가진 오픈소스 vector/raster graphics editor다.

볼 점:

- Rust editor backend와 web UI shell 분리.
- wasm-bindgen `EditorWrapper` 패턴.
- Rust backend와 frontend manager 사이의 message routing.
- `wgpu`와 Vello를 렌더링 실행 경로에서 사용하는 방식.
- 캔버스가 제품 중심일 때 얼마나 많은 editor complexity가 Rust 쪽으로 이동하는지.

### Vello

Rust GPU compute-centric 2D renderer다. `wgpu`를 사용하고 large 2D scene에 초점을 둔다.

볼 점:

- scene building.
- render-to-texture flow.
- GPU compute 기반 vector rendering.
- glyph cache와 GPU memory tradeoff.

### wgpu

Rust에서 WebGPU 및 native graphics backend를 다루는 핵심 후보다.

볼 점:

- device/queue/surface lifecycle.
- browser WebGPU limits.
- WASM publishing constraints.
- native와 web을 동시에 고려하는 renderer abstraction.

### CanvasKit / Skia

성숙한 vector rendering baseline으로 비교할 가치가 있다.

볼 점:

- path/text rendering 품질.
- mature graphics API shape.
- WebAssembly graphics package 운영 비용.

### ThorVG

C++ vector graphics engine이다. CPU/SIMD, OpenGL/ES, WebGL, WebGPU backend와 smart partial rendering을 언급한다.

볼 점:

- partial rendering architecture.
- backend abstraction.
- vector scene 최적화 아이디어.

### Pathfinder

Rust GPU rasterizer for fonts/vector graphics다. incomplete/heavy development 성격이 있지만 GPU vector rasterization 참고로 볼 수 있다.

### Lyon, Kurbo, Peniko, Swash, Cosmic Text

custom Rust renderer를 만들 때의 building block 후보들이다.

- Lyon: GPU rendering을 위한 path tessellation.
- Kurbo: curve/path/2D geometry.
- Peniko: brush/color/gradient/drawing data type.
- Swash: font introspection, shaping, glyph rendering. 단 full text layout은 아니다.
- Cosmic Text: text shaping, fallback, layout, optional rasterization.

### Penpot, Inkscape, GodSVG

renderer stack보다는 제품/UX/object model/export 관점의 참고다.

- Penpot: open standard, SVG/CSS/HTML/JSON 중심의 web design platform.
- Inkscape: mature SVG vector editor.
- GodSVG: structured SVG editor와 SVG-code 중심 경험.

## 제품별 구현 함의

shape.ai는 Figma처럼 모든 그래픽 자산을 다루는 디자인 전문 툴이 아니다. 그래서 엔진의 범위를 더 좁힐 수 있다.

우선 필요한 객체:

- Card: rounded rectangle, border, background, shadow, badge, port.
- Text: title, summary, detail snippet.
- Edge: Bezier/routed line, arrowhead, label.
- Interaction: select, drag, resize, connect, text edit, comment anchor.
- View: pan, zoom, fit, minimap/overview candidate.
- Export: graph format, SVG/image snapshot candidate.

이 정도 범위는 Figma보다 훨씬 작지만, DOM-only React Flow보다는 그래픽 엔진에 더 가깝다.

## 성능 모델

무한 캔버스에서 중요한 것은 "무한히 많은 것을 매 프레임 다 그리는 것"이 아니다.

중요한 모델:

```text
하나의 world scene을 유지한다.
camera viewport 기준으로 필요한 object를 찾는다.
object geometry와 text layout은 cache한다.
GPU buffer update는 최소화한다.
pan/zoom 중에는 layout을 다시 계산하지 않는다.
render pass는 batch한다.
hit testing은 spatial index로 처리한다.
```

사용자에게는 하나의 연속적인 캔버스로 보이지만, 내부적으로는 viewport culling, cache invalidation, dirty region, partial update가 작동해야 한다.

## 주요 리스크

### Text Editing

canvas 안에서 native text editing을 직접 구현하는 것은 비용이 크다.

권장 타협:

- static text는 Rust/WebGPU가 렌더링한다.
- 활성 편집 중인 텍스트만 DOM overlay로 띄운다.
- 편집 완료 후 scene patch로 반영한다.

이렇게 하면 IME, selection, copy/paste, browser native input을 유지할 수 있다.

### Accessibility

GPU-rendered canvas content는 자동으로 접근 가능하지 않다.

필요한 대응:

- 선택된 객체 또는 visible object에 대한 parallel accessibility model.
- keyboard navigation.
- screen reader용 focused DOM affordance.

초기부터 완벽히 구현하지 않더라도 엔진 구조에서 접근성 데이터를 꺼낼 수 있어야 한다.

### JS/WASM Boundary

Rust를 써도 JS/WASM 호출이 너무 잘게 나뉘면 성능 이점이 사라진다.

원칙:

- patch batch.
- input batch.
- frame당 호출 수 제한.
- binary/typed-array 기반 데이터 전달 검토.
- scene snapshot과 incremental patch 분리.

### Renderer Maturity

Vello는 유망하지만 alpha다. CanvasKit은 성숙하지만 Rust-native가 아니다. custom renderer는 ownership이 크지만 비용도 크다.

따라서 결정은 논리만으로 끝내면 안 되고 prototype benchmark로 해야 한다.

### Business Logic Coupling

비즈니스 로직을 Rust에 넣으면 제품 개발 속도가 느려질 수 있다.

Rust는 renderable scene, interaction geometry, hit testing, renderer state를 책임진다. 제품 규칙은 TypeScript app이 책임진다.

## 열려 있는 검증 과제

### 1. Graphite architecture deep dive

목표:

- Graphite의 Rust backend/web frontend 분리 방식을 분석한다.
- `frontend/wrapper`, `editor`, `node-graph`, `wgpu-executor` 흐름을 본다.
- shape.ai에 맞는 wrapper API 패턴을 추출한다.

산출물:

- Graphite 구조 요약.
- shape.ai에 적용 가능한 API sketch.
- 가져오면 안 되는 과도한 복잡도 목록.

### 2. Vello/wgpu prototype

목표:

- 1,000개 카드, 텍스트 snippet, 엣지를 렌더링한다.
- pan/zoom을 연속적으로 실행한다.
- frame time, memory, text cache behavior를 측정한다.

판단 기준:

- 60fps 근처 pan/zoom 가능 여부.
- 텍스트 품질.
- zoom 중 artifact 여부.
- WASM/browser integration 난이도.

### 3. DOM edit overlay prototype

목표:

- GPU-rendered card 내부 텍스트를 클릭한다.
- 해당 위치에 DOM textarea/input overlay를 정확히 띄운다.
- 편집 후 scene patch로 commit한다.

판단 기준:

- 위치 오차가 눈에 띄지 않는가.
- zoom 중 편집 상태가 어색하지 않은가.
- IME/copy/paste/selection이 자연스러운가.

### 4. Renderer 비교

비교 후보:

- Vello/wgpu.
- CanvasKit/Skia.
- custom Rust tessellation + wgpu.

비교 항목:

- visual quality.
- frame time.
- memory.
- text rendering 품질.
- implementation complexity.
- dependency maturity.
- API ownership.

### 5. JS API publishing

목표:

- 작은 Rust/WASM package를 만든다.
- `loadScene`, `applyPatch`, `setCamera`, `hitTest`, `renderFrame`만 노출한다.
- Vite 앱에서 import해 canvas에 붙인다.

판단 기준:

- 번들링이 안정적인가.
- dev server/HMR과 충돌이 없는가.
- sourcemap/debugging이 가능한가.
- panic/error handling이 다룰 만한가.

## 구현 단계 제안

### Phase 1: Evidence Prototype

현재 React Flow UI와 별도로 최소 canvas engine prototype을 만든다.

범위:

- Rust/WASM package.
- web page with canvas.
- pan/zoom camera.
- cards + edges rendering.
- basic hit test.
- frame stats.

gate:

- Vello/wgpu가 충분하지 않으면 CanvasKit 비교로 넘어간다.

### Phase 2: Shape Scene Contract

렌더 가능한 scene snapshot과 patch format을 정의한다.

범위:

- TypeScript scene snapshot type.
- Rust deserialization.
- stable object IDs.
- camera/bounds model.
- patch application.

gate:

- business field가 Rust에 불필요하게 새지 않는지 확인한다.

### Phase 3: Interaction Slice

작은 그래프에서 직접 조작을 구현한다.

범위:

- select card.
- drag card.
- select edge.
- begin text edit with DOM overlay.
- commit text edit.

gate:

- inline edit 경험이 현재 제품 감각을 유지하는지 확인한다.

### Phase 4: Current App Integration

현재 shape graph 경로 하나를 새 canvas로 대체한다.

범위:

- existing shape graph -> scene snapshot 변환.
- layout persistence.
- selection persistence.
- node edit save path.
- export path 유지.

gate:

- React Flow fallback을 유지할지 제거할지 결정한다.

### Phase 5: Scale And Polish

대규모 그래프와 연속 조작 성능을 최적화한다.

범위:

- spatial index.
- viewport culling.
- render cache.
- text cache.
- edge route cache.
- benchmark fixtures.
- visual regression screenshots.

gate:

- 기존 React Flow baseline과 비교한다.

## 최종 방향

현재 가장 타당한 방향은 다음이다.

```text
Rust canvas core
+ wgpu/Vello 기반 prototype
+ WASM publishing
+ TypeScript app shell/product logic
+ DOM overlay for active editing/floating UI
```

이 방향은 다음을 만족한다.

- Illustrator-like continuous canvas 목표에 맞다.
- DOM/React가 대규모 렌더링 병목이 되는 구조를 피한다.
- 비즈니스 로직을 Rust에 과하게 묶지 않는다.
- HTML-in-Canvas 같은 실험 API에 의존하지 않는다.
- Vello, CanvasKit, custom wgpu renderer를 prototype evidence로 비교할 수 있다.

## 참고 링크

- Graphite: https://github.com/GraphiteEditor/Graphite
- Graphite frontend README: https://github.com/GraphiteEditor/Graphite/blob/master/frontend/src/README.md
- Graphite wrapper README: https://github.com/GraphiteEditor/Graphite/blob/master/frontend/wrapper/README.md
- Graphite wgpu executor: https://github.com/GraphiteEditor/Graphite/blob/master/node-graph/libraries/wgpu-executor/src/lib.rs
- Vello: https://github.com/linebender/vello
- wgpu: https://wgpu.rs/index.html
- CanvasKit: https://docs.skia.org/docs/user/modules/canvaskit/
- ThorVG: https://github.com/thorvg/thorvg
- Pathfinder: https://github.com/servo/pathfinder
- Lyon: https://github.com/nical/lyon
- Kurbo: https://github.com/linebender/kurbo
- Peniko: https://github.com/linebender/peniko
- Swash: https://github.com/dfrg/swash
- Cosmic Text: https://pop-os.github.io/cosmic-text/cosmic_text/
- Penpot: https://github.com/penpot/penpot
- Inkscape: https://github.com/inkscape/inkscape
- GodSVG: https://github.com/MewPurPur/GodSVG
- Figma, Building a professional design tool on the web: https://www.figma.com/blog/building-a-professional-design-tool-on-the-web/
- Figma, WebAssembly load time: https://www.figma.com/blog/webassembly-cut-figmas-load-time-by-3x/
- Figma, rendering powered by WebGPU: https://www.figma.com/blog/figma-rendering-powered-by-webgpu/
- Figma, speeding up file load times: https://www.figma.com/blog/speeding-up-file-load-times-one-page-at-a-time/
- Chrome HTML-in-Canvas origin trial: https://developer.chrome.com/blog/html-in-canvas-origin-trial
- WICG HTML-in-Canvas proposal: https://github.com/WICG/html-in-canvas
