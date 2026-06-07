# Object Primitive Redesign — Single-Substrate Canvas Model, Zero-Lag Vector Renderer, Pure-Rust KV Storage — Task Breakdown

> Source: 단일 object primitive / geometry substrate / 렌더·제로랙 / anchor·outline·projective transform / 프리드로잉 / storage(KV) / Rust-first 토론 (2026-06-07~08). 선행/병렬 맥락: [Canvas Cockpit Redesign + Full Rust Migration](./canvas-cockpit-redesign-task-breakdown.md) (이하 "마이그레이션 문서").
> 관련 코드(현행): `src/renderer/core/src/{model.rs,webgpu.rs}`(렌더 코어 — 현재 `RenderGroup/RenderCard/RenderEdge` 모델, **순수 2D** 파이프라인: ViewUniform `[f32;4]`·depth 없음), `crates/scene-core`(op-apply/LWW/wire/templates), `src/storage/core`(StorageAdapter·region index·Memory/File/SQLite), `src/client/lib/{sceneClient,outbox,syncEngine}.ts`(클라 데이터계층 seam — `OutboxStore` 인터페이스, 현재 InMemory 백엔드).

이 문서는 캔버스의 **primitive를 `object` 하나로 통일**하고, 그것을 **고속(zero-lag) 벡터 렌더링**·**pure-Rust KV 영속**으로 받치는 후속 작업을 분해한다. 기존 도형/노드/엣지/그룹 개념을 전부 없앤다.

## 마이그레이션 문서와의 관계 (중요)

이 재설계는 마이그레이션의 일부 결정을 **대체(supersede)** 한다:
- scene-core **모델**: `Group/Card/Edge` → **`object` 단일 종**. (마이그레이션 MG0.2a/g가 만드는 모델을 교체.)
- **storage**: sqlite(native) + wasm-sqlite-OPFS → **redb(pure-Rust KV)**. (MG1.2/MG1.2w 대체.)
- **템플릿**: 마이그레이션의 "template = primitive 구성"과 정합 — 여기선 "스타일 입힌 object(들)".
- **Rust-first 정리**: TS op-apply 제거 = 마이그레이션 MG7.3과 동일.

> **시퀀싱(locked — 사용자 결정):** 후속으로 진행한다. 마이그레이션은 그대로 두고 이 재설계를 그 위에 얹는다. 단 마이그레이션의 sqlite/`Group-Card-Edge` 작업이 *미완*인 부분은 그 작업 뒤가 아니라 **대신** 이 재설계로 수행해 버릴 코드를 줄인다(같은 결정의 자연 귀결, 새 결정 아님). OB0.1이 진행도를 확인해 폴드인 범위를 산출한다.

## Locked Principles

- **P1. Rust-always-first.** 모든 캔버스/도메인 로직(op-apply·LWW·검증·geometry·tessellation·hit-test·템플릿)과 영속의 **유일 진실원은 Rust(scene-core/storage-core)**. TS 셸은 도메인 로직 0(DOM/UI/전송 배선/키 캡처/presence만). TS 도메인 미러가 생기면 그것은 **제거 태스크를 단 임시 tooling 우회**일 뿐 승인된 이중 경로가 아니다. (CLAUDE.md의 "deliberate TS op-apply" 문구는 오기 — 정정 대상, OB4.4.)
- **P2. 단일 substrate.** 캔버스 위 모든 것은 `object`. 도형 종류는 *타입(union)이 아니라* 단일 geometry 값(노드 리스트)의 특수 케이스.
- **P3. 진실=vector, 화면=파생 mesh.** 저장/편집은 벡터(path), 렌더는 그로부터 파생·캐시된 GPU 버퍼. 둘은 선택이 아니라 원본↔메모이제이션.
- **P4. Zero-lag.** 최초 stutter·조작 지연 불용. 조작=행렬만(재tessellation 0), re-bake는 geometry 편집 시 그 object만.
- **P5. storage = 도메인 중립 KV.** Record=`id→JSON` 블롭. 관계/그래프/검증은 scene-core가 소유, storage는 멍청한 영속. 인덱스(region/secondary)는 storage-core가 직접 소유.
- **P6. 의미 통합 금지.** shape는 통합하되 관계는 명시 데이터로 — 연결은 `anchor`로 질의 가능. AI 가독용 재표현은 **MCP 레이어 책임**(캔버스와 분리).
- **P7. 3D 비범위.** 현재 도입 안 함(미래 별도 3D 캔버스 여지). transform은 2D 전용.

## Decisions Ledger (locked)

**모델**
- **D1. object 스키마:** `{ id, parent, order(fractional), transform, geometry, fill?, stroke?, text?, anchors?, layout?, clip?, comments?, tags? }`. 도형 다양성으로 필드가 늘지 않음(능력 추가 시만). comments·tags 상세=D20.
- **D2. geometry = 단일 path substrate:** open/closed + 노드 리스트, 노드=`{x, y, inHandle?, outHandle?, width?}`. 핸들 없으면 직선(폴리라인/낙서), 있으면 곡선(원=베지어). **multi-subpath** 한 문자열(구멍=even-odd, 한 번에 그린 스케치). 인코딩 = **SVG-subset path-string**(AI 호환; 자체 압축 문법은 deferred 레버). **SVG는 *교환·geometry 어휘*일 뿐 — 런타임(파싱된 노드)·저장(KV)·렌더(tessellation)는 우리 모델이지 SVG-as-model 아님**(anchor/auto-layout/identity/LWW/local 좌표를 SVG는 못 줌). import/export만 SVG 1급. 좌표 = object-로컬 양자화 정수(단위·비트폭은 agent-discretion 기본 = 1/8px·i32, 프로파일 시 조정).
- **D3. identity 규칙(3단):** subpath(identity 없음, object의 style/transform/text 공유) / object(identity 1) / children=group(독립 identity + 옵션 auto-layout). `split`/`merge`로 상호 변환.
- **D4. style:** `fill`(solid/gradient/image; **파생 region**에 적용, **stroke보다 아래**), `stroke`(테두리/그린 선; +dash, +per-node width), `text`(**runs 배열**; region 기준 위치; markdown thin은 미래).
- **D5. anchor:** 노드별 옵션 `{node, target, at:localPoint}`. **엣지 흡수**(open contour + 끝점 anchor). 끝점 위치는 *파생*; target geometry 편집 시 outline에 re-project. 두 anchor의 `target`으로 **연결 그래프 질의**.
- **D6. 파생 outline/region("모양"):** open geometry=**alpha shape/concave hull**, closed=내부. fill 영역·text 레이아웃·hit-test·selection·**anchor 보더**를 모두 정의. geometry 편집 시 1회 계산+캐시.
- **D7. transform = 3×3 projective 행렬:** TRS + shear + perspective를 한 표현으로. 조작=행렬만 **0-rebake**. 비선형(bend/envelope/text-on-path)은 **FFD 격자 warp**(스키마 슬롯 예약, 구현 deferred). 파이프라인 `screen = camera · M(3×3) · warp(local)`.
- **D8. hit-test:** 포인터를 inverse-transform → object-로컬에서 region point-in-test. 회전/스케일/perspective 자동 처리.
- **D18. clip(optional, Figma식):** object에 `clip?` 플래그. 참이면 children 렌더를 **자기 region(D6)/bounds로 클립**(Figma Frame "clip content"). 기본 off(투명·무한). GPU scissor/stencil로 구현. SVG `clipPath`→여기 매핑(부분), `mask`(알파 마스크)는 deferred(효과 레이어).
- **D20. comments·tags = object 필드:** `comments?`=`{id, author, body, at?:(node|localPoint)}` 배열(문서 상태·LWW, anchor 가능). `tags?`=string id 배열(이름/색 레지스트리는 canvas 메타). **P6 구분:** tags=단순 라벨(in-model), AI용 의미 재표현은 여전히 MCP 레이어.
- **D21. undo/redo = 역op, 동일 파이프라인:** 모든 편집은 op(단일 op-apply). undo=역op를 *동일 op-apply·LWW·wire*로 author(상태 롤백 아님 → 동시편집과 합성). **per-actor 스택**(내 op만 undo, peer 것 아님). 코얼레싱/제스처 경계=1 undo 단위(드래그=1스텝). **document state만**(ephemeral=selection/camera 제외, operation.ts 경계). undo 스택=클라 scene-core 로컬 세션 상태(미동기); 역op 자체는 정상 op라 동기·영속.

**렌더/제로랙**
- **D9. 3레이어:** at-rest(압축 path-string + ref) / scene-core 런타임(파싱된 Path 단일 구조) / renderer(캐시 mesh + GPU-expand stroke + instancing).
- **D10. 렌더 전략:** 닫힌 fill→lyon triangulate+캐시(geometry 편집 시만 rebake); stroke→**GPU 버텍스셰이더 expansion**(폭/dash 셰이더, 복잡 join/cap은 CPU lyon 폴백); 조작/projective→행렬 0-rebake; 곡선 LOD→zoom-bucket re-flatten; **instancing**(같은 geometry/템플릿 공유); 유니크 작은 메시 다수(낙서)는 **megabuffer 배칭**(정적 geometry 병합+범위 드로).
- **D19. anti-aliasing:** 텍스트=**MSDF**(무한줌 크리스프, 공짜). fill/stroke 가장자리 AA = **MSAA(간단) vs 셰이더 analytic AA(거리 기반, 더 곱고 일거리)** 중 택1 — 결정은 OB3.R(렌더) 단계에서 벤치 후. tessellation 방식의 알려진 비용이며 모델 결함 아님.
- **D11. size:** 객체 수↓(세션-object)·노드 수↓(RDP+베지어fit)·로컬 양자화 정수·**raw 점/mesh 미저장**·**path-string + zstd**(읽으면서 바이너리급). 바이너리 blob/primitive shorthand/SDF fast-path=deferred 레버.

**프리드로잉**
- **D12. 캡처 평평·분류 on-demand:** 드로잉 세션(펜 잡고 commit 전까지) = **object 1개**(multi-subpath 누적). 그릴 때 granularity 추측 0. 이후 `split`. 점선=**dash 스타일**(객체 아님).
- **D13. 파이프라인:** 점 캡처(transient·미저장)→실시간 GPU-expand→pen-up RDP 단순화+베지어fit→압축 path-string→object. 노드 `width` 슬롯(미래 필압/브러시).

**storage**
- **D14. redb(pure-Rust KV)**, sqlite+JS shim 폐기. native+wasm 공유, `StorageAdapter` 트레잇 뒤.
- **D15. 서버=redb-on-file(현재)→ 멀티-pod 플릿/handoff 시 외부 DB**(FoundationDB/Postgres). 클라=**redb-OPFS(Web Worker, sync/async 브리지)** — 기존 `OutboxStore` seam + 미래 replica store 뒤.
- **D16. SQL 없음:** region windowing=**Morton/Z-order 키 range scan**, secondary 인덱스(kind/parent/tag)=storage-core가 keyspace로 소유.
- **D17. 트레잇 = async + backend-중립:** redb-ism(동기·단일writer·txn) 호출부 미노출 → 외부 DB 스왑 저비용. 영구 공유=scene-core + 트레잇 + Record/region/번들 포맷; redb 구현체=갈아끼우는 리프.

## Deferred / Accepted Ceilings (현재 비범위 — 출처 명시)

- **3D**(P7) / **FFD warp 구현**(D7 슬롯만) / **SDF fast-path·primitive shorthand·바이너리 geometry blob**(D11 레버) / **components(componentOf)** / **gradient-mesh·multi-fill**(분리=split로만) / **geometry boolean(union/subtract/intersect)·partial erase**(0-rebake 위배·boolean robustness·노드 폭증 — 무겁고 얻는 것 적어 **제외**; eraser=contour 단위 제거면 충분=OB3.D4) / **silhouette text flow·vertex 편집·markdown 텍스트**(thin 미래) / **external file 타입**(content/embed 채널 슬롯 예약, 타입별 추후) / **sub-shape 동시편집·노드 애니메이션**(geometry 원자성 — 미래 sync에서) / **server 외부 DB**(D15, 멀티-pod 시) / **semantic role/type 메타(AI 재표현)**(P6 — MCP 레이어; 단순 tags는 in-model=D20) / **SVG import/export**(일단 비범위 — 추후 SVG↔object ops 얇은 레이어).
- **Hedge 슬롯(지금 스키마에 박되 구현 0):** `text=runs[]`, `transform=3×3`, 노드 index-addressable, `content/embed` 채널, 노드 `width`, `anchors`, `componentOf`(미봉쇄).

---

## Objective

캔버스 primitive를 `object` 하나로 수렴시켜 (a) 모든 도형/낙서/엣지/그룹/텍스트를 단일 geometry substrate로 표현하고, (b) 그 substrate를 **조작·드로잉에서 0랙**으로 렌더하며, (c) 영속을 **pure-Rust KV(redb, native+wasm 공유)** 로 두고, (d) **Rust가 도메인·영속의 유일 진실원**인 상태를 만든다.

## Done Criteria

- **모델:** `Group/Card/Edge` 제거, `object` 단독. 도형/낙서/엣지/그룹/텍스트가 object으로 표현·왕복(round-trip). anchor로 엣지 재라우팅 + 연결 그래프 질의.
- **렌더/제로랙:** 1만 object 드래그·회전·스케일에서 재tessellation 0; 프리드로잉 실시간 스트로크 무지연; 곡선 zoom LOD; 파생 region 기반 fill/hit-test.
- **storage:** redb 단독(native+wasm), sqlite/sql.js/JS shim 제거; Morton region windowing; `StorageAdapter` async·backend-중립; 클라 store가 `OutboxStore` seam 뒤.
- **Rust-first:** scene-core-wasm이 node/vitest에서 실 op-apply로 테스트, `renderPatch.ts` TS op-apply 제거, 셸 도메인 로직 0, CLAUDE.md 정정.
- **셸/UI:** 유일 영속 플로팅 UI=하단 중앙 툴바(Toolbar). Sidebar·CanvasEditingToolbar·SelectedNodeInspector·ExportDrawer·RendererDiagnosticsDrawer·CanvasSwitcher 흡수/제거, CompanionDock·Trace는 기능째 제거; 우클릭·단축키·도구가 object 모델·scene-core 카탈로그에 정합; select 모드 화면 pan.
- **코어 통신 단일화:** 셸의 bespoke REST(/api/*) 제거, 전 feature 통신이 scene-core wire(OB1.2) 단일 채널 경유; TS는 raw 배선만(도메인 API 0).
- **undo/redo:** cmd+z/cmd+shift+z=역op 기반 예측가능 되돌리기/다시실행; per-actor·드래그=1스텝·동시편집 안전(내 op만){D21}.

---

## Task Breakdown

ID 체계: `OB<phase>.<n>`. `wave`=병렬 묶음(독립 실행 가능). 같은 wave가 같은 크레이트를 동시 편집할 수 있고, 그 겹침은 worktree 머지에서 해소(완전 disjoint 강제 안 함 — 병렬 폭 우선).

### Phase OB-0 — Discovery & enabling (짧은 serial prefix)
> Goal: 착수 전 리스크 제거 + Rust-first 테스트 seam 확보. Why now: storage 선택과 단일 op-apply가 하위 전체를 좌우.
```
OB0.1  redb/OPFS de-risk spike      → 결정(D14/D15 redb-OPFS)을 검증: redb-opfs/Manifold 성숙도 + Safari OPFS-worker 동작 + Morton region range-scan 성능 벤치. 산출=증거 + 진행 확정(또는 evidence가 막으면 fallback 발동).  wave 0  spike
OB0.2  Rust-first 테스트 하네스      → scene-core-wasm을 node/vitest에서 로드(init에 .wasm 바이트 주입 또는 --target nodejs). 산출=실 Rust op-apply 통과하는 node 테스트.  wave 0  rust+test
```
- **Verify:** OB0.1 벤치 수치가 redb-OPFS 생존성 충족(미충족 시에만 fallback 검토). OB0.2 vitest가 TS op-apply 없이 통과.
- **Contingency gate(human-verify):** OB0.1 증거가 redb-OPFS 비생존을 가리킬 때만 fallback(IndexedDB) 승인.

### Phase OB-1 — Object contract (키스톤 foundation)
> Goal: object 스키마·op·파생 region·storage 트레잇이라는 계약을 공표해 하위를 넓게 병렬화. Why now: 모든 스트림이 이 계약에 빌드.
```
OB1.1  object 모델·스키마(키스톤)    → scene-core에 Object{D1} 정의(transform 3×3·geometry path-string·fill/stroke/text(runs)/children/anchors/layout/comments/tags, hedge 슬롯 포함). RenderGroup/Card/Edge 대체 설계.  wave 0  rust(contract)
OB1.2  op 세트 + wire serde         → insert-object/edit-geometry/set-transform/set-style/set-text/set-anchor/set-layout/add-comment/set-tags/reparent/reorder/delete + hello/ops/ack/patch + feature 채널(canvas 전환·comment·template apply·export 요청/응답 — bespoke REST 대체). (geometry 인코딩=D2 SVG-subset) 각 op=역op 도출 가능(undo·D21).  wave 1  rust(contract) (OB1.1)
OB1.3  파생 region 인터페이스        → geometry→outline(open=alpha shape, closed=내부){D6} 계약 + 참조 stub. fill/text/hit/anchor 소비자 공용.  wave 1  rust(contract) (OB1.1)
OB1.4  StorageAdapter 트레잇 정련     → async·backend-중립·KV Record·Morton region query 시그니처{D16/D17}. (Object 스키마 독립)  wave 0  rust(contract)
```
- **Verify:** scene-core wasm32+native 컴파일; 계약 타입에 대한 stub 테스트.
- **Decision checkpoint:** 스키마가 더 나은 구조를 드러내면 OB1.1 조정 후 하위 진행.

### Phase OB-2 — Vertical slice (foundation 저비용 증명)
> Goal: 한 경로로 전 레이어를 관통해 계약이 맞물리는지 증명.
```
OB2.1  rect 수직슬라이스            → 닫힌-rect object insert op → scene-core apply → redb 저장 → 렌더(fill+stroke) → 3×3 transform 이동/스케일/회전(0-rebake). 각 레이어 thin 구현.  wave 2  rust+renderer+storage
```
- **Verify:** rect 표시·드래그 시 재tessellation 0·리로드 후 영속.
- **Review gate:** 슬라이스가 드러낸 구조 문제로 OB-1 계약 재작업 여부.

### Phase OB-3 — Wide parallel implementation (계약 위 fan-out)
> Goal: 렌더·scene-core·storage·드로잉·레이아웃을 독립 seam으로 동시 구현.

**Renderer (src/renderer/core)**
```
OB3.R1 tessellation+캐시           → 닫힌 fill lyon, geometry 편집 시만 rebake; 유니크 작은 메시 megabuffer 배칭(D10).  wave 3  renderer
OB3.R2 GPU stroke expansion        → open/outline 버텍스셰이더 expand + dash + per-node width.  wave 3  renderer
OB3.R3 instancing + 3×3 조작        → projective 행렬 셰이더(perspective divide), 조작 0-rebake.  wave 3  renderer
OB3.R4 파생 outline 구현            → alpha shape/내부 캐시; fill-below-stroke·selection·text ref 공급.  wave 3  renderer (OB1.3)
OB3.R5 hit-test                    → inverse-transform→로컬 region point-in.  wave 4  renderer (OB3.R3,R4)
OB3.R6 zoom-bucket LOD             → 곡선 re-flatten 캐시.  wave 4  renderer (OB3.R1)
OB3.R7 clip(D18)                   → clip=true object가 children을 region/bounds로 GPU scissor/stencil 클립.  wave 4  renderer (OB3.R3,R4)
OB3.R8 anti-aliasing(D19)          → 텍스트 MSDF + fill/stroke AA(MSAA vs analytic 벤치 후 택1).  wave 4  renderer (OB3.R1,R2)
OB3.R9 텍스트 렌더/레이아웃(D4)      → runs를 region(D6) 기준 레이아웃 + MSDF 글리프 렌더(기존 text.rs 확장).  wave 4  renderer (OB3.R4)
OB3.R10 스타일 기본값/selection 시각 소유 → 현행 `src/shared/renderScene.ts`(`shapeSceneToRenderSnapshot`/`defaultStyles`/`shapeStyleToken`) 분해. 구조적 기본값(object가 fill/stroke 누락 시 적용)·selection/hover/focus/state 시각 처리를 renderer-core가 소유; styleKey→palette 해석 폐기(object 인라인 style D4). 의미 프리셋(decision/risk/option 등 12종)은 renderer 아님 → 템플릿(OB3.S5) 인라인 스타일로 이관. 셸 잔존 0. (AA는 OB3.R8, radius→geometry/typography→text runs/shadow→deferred 효과층이라 R10 범위 외.)  wave 3  renderer (OB1.1)
```
**scene-core (crates/scene-core)**
```
OB3.S1 op-apply + per-property LWW  → object op 적용·LWW + 역op 캡처(undo 엔트리, D21).  wave 3  rust (OB1.2)
OB3.S2 검증                        → degenerate geometry/children cycle/anchor target 존재/bounds.  wave 3  rust (OB1.1)
OB3.S3 identity/subpath/split·merge → 3단 규칙{D3} + split/merge op.  wave 4  rust (OB3.S1)
OB3.S4 anchors                     → 끝점 파생 해석 + target 편집 re-project + 그래프 질의{D5}.  wave 4  rust (OB3.S1,OB1.3)
OB3.S5 템플릿=object                 → recipe/apply(스타일 입힌 object들), 기존 템플릿 도메인 교체 + renderScene.ts 의미 프리셋(decision/risk/option…) 흡수(OB3.R10).  wave 4  rust (OB3.S1,OB3.R10)
OB3.S6 MCP 툴셋 object화             → SceneMcp 툴(query/list/get/create/patch/tag/selection/comment/export)을 Group/Card/Edge → object op(insert-object·edit-geometry·set-style·set-text·reparent·delete 등)으로 재작성; 에이전트가 object 모델로 캔버스 확장. (mcp.rs는 OB3.U6의 trace 절단과 같은 파일; U6 시그니처 변경 선행 — 머지 조율.)  wave 4b  rust (OB1.2,OB3.S1,OB3.U6)
OB3.S7 서버 wire feature 핸들러        → OB1.2 feature 채널(canvas 전환·export·comment/tag ops)을 서버 ws에서 처리(ws.rs/app.rs); 클라 REST 대체분의 서버측 구현. (app.rs는 OB3.U6의 mcp 라우트 제거와 같은 파일; U6 라우트 제거 선행 — 머지 조율.)  wave 4b  rust (OB1.2,OB3.S1,OB3.U6)
OB3.S8 undo/redo 엔진               → per-actor undo/redo 스택 + undo=역op를 동일 op-apply·LWW·wire로 author(롤백 아님); 제스처/코얼레싱 경계=1스텝; document state만. 스택=클라 scene-core 로컬(미동기){D21}.  wave 4  rust (OB1.2,OB3.S1)
OB3.S9 object command catalog       → 사용자 명령 목록(라벨·기본 단축키·op 매핑): copy/paste·duplicate·group/ungroup·select-all·nudge·undo/redo 등. U3 메뉴·U4 단축키 공용(현 command_catalog object화). copy/paste 클립보드 I/O=셸(P1).  wave 3  rust (OB1.2)
```
**storage (src/storage/core)**
```
OB3.T1 redb 어댑터(native)          → StorageAdapter 뒤 redb-on-file, KV Record CRUD.  wave 3  rust (OB1.4)
OB3.T2 Morton region 인덱스         → Z-order 키 + windowed read + region query.  wave 4  rust (OB3.T1)
OB3.T3 redb-OPFS 어댑터(wasm)        → Web Worker, sync/async 브리지.  wave 4  rust+worker (OB0.1,OB3.T1)
OB3.T4 zstd at-rest                 → path-string payload 압축.  wave 4  rust (OB3.T1)
```
**drawing (셸/코어 경계)**
```
OB3.D1 프리드로잉 캡처→object         → 라이브 GPU-expand → pen-up RDP+베지어fit → 세션=object 1개(multi-subpath){D12/D13}.  wave 4  rust+svelte (OB3.R2,OB3.S1)
OB3.D2 dash 스타일 + width 캡처      → 점선=stroke 스타일, 필압-ready width 슬롯.  wave 4  rust+renderer (OB3.R2)
OB3.D3 펜 도구 UX                  → draw 모드 상태(진입/이탈) + 브러시 파라미터 모델(색·두께·dash) + 세션 commit 트리거(도구전환/Esc). **Toolbar의 펜 도구 버튼·브러시 컨트롤 UI는 U1(도구셋) 소유** — D3는 동작·모델만.  wave 4b  rust+svelte (OB3.D1,OB3.U1)
OB3.D4 eraser                     → 그레인=subpath(contour) 단위·전 object 동일 규칙(타입 분기 0): multi-subpath(낙서·구멍 도형)=터치한 contour만 제거(split 경유), 단일-subpath 도형(rect 등)=contour=object라 삭제됨(단일 substrate의 정직한 귀결). whole-object 삭제=Delete 키. 부분 지우기(stroke 중간 trim·shape notch=geometry boolean)=**제외**(무겁고 얻는 것 적음 — Deferred 참조). eraser는 contour 단위 제거가 전부.  wave 4b  rust+renderer (OB3.S1,OB3.S3)
```
**auto-layout**
```
OB3.A1 thin auto-layout(children)   → dir/gap/padding/align/sizing; 출력은 파생(미저장·미동기).  wave 4b  rust (OB3.S1,OB3.S3)
```
**shell/UI (src/client/svelte) — 인터랙션 표면 (object 계약 기준 재작성; 컷오버 통합은 OB4.3)**
> 불변식: 유일한 영속 플로팅 UI = 하단 중앙 툴바(Toolbar). 그 외 패널/드로어/툴바는 흡수 또는 제거. (CompanionDock/Trace=AI 프레즌스는 thin → OB3.U6에서 기능째 제거.)
> **실행:** U-태스크는 App.svelte를 공유 → **단일 오너·스트림 내 직렬**(U1=Toolbar foundation → U2~U6). 병렬성은 다른 스트림(R/S/T/D/A) 사이에서 확보.
```
OB3.U1 하단 툴바 단일화          → Toolbar(현 CockpitRemote.svelte 개명·재작성; cockpit* 네이밍 일괄 제거 — cockpitCommands·cockpit.test 등)를 유일 영속 플로팅 UI로: object 도구셋 + ExportDrawer·RendererDiagnosticsDrawer·CanvasSwitcher·선택 시 object 속성편집(구 SelectedNodeInspector) 흡수.  wave 3  svelte (OB1.2)
OB3.U2 잔존 패널/툴바 제거           → Sidebar(groups/tags)·CanvasEditingToolbar(정렬/편집)·SelectedNodeInspector 제거(기능은 U1로 흡수), 구 SceneGroup/Node 의존 소거.  wave 4  svelte (OB3.U1)
OB3.U3 우클릭 = 단일 object 메뉴      → ContextMenu/NodeContextMenu 통합 → split/merge·reparent·set-anchor·set-style·reorder·delete; SceneNode 의존 제거. 액션 목록=OB3.S9 카탈로그 소비.  wave 4b  svelte (OB1.2,OB3.S3,OB3.S4,OB3.S9)
OB3.U4 단축키 = scene-core 카탈로그   → shortcuts.ts가 command_catalog 직소비(TS 미러 제거, P1) + 신 command set 바인딩 + undo(cmd+z)·redo(cmd+shift+z)→코어 커맨드(OB3.S8).  wave 4b  svelte+rust (OB1.2,OB3.U1,OB3.S8,OB3.S9)
OB3.U5 select+pan 동시 인터랙션      → select 모드에서도 화면 pan(space-drag/중버튼/트랙패드); 모드↔nav=코어(tool.rs/카메라), 제스처=셸.  wave 4  rust+svelte (OB1.2,OB3.U1)
OB3.U6 companion 기능 제거            → 클라: CompanionDock·CompanionTrace·followController·mcpDock 제거. 서버: /api/mcp/clients·/api/mcp/trace 라우트 + mcp_clients.rs 삭제 + SceneMcp↔ClientRegistry 결합 절단(trace() helper·self.trace 호출 13곳·get_client_trace 툴·clients 필드/ctor 인자 제거) + build_router_with_mcp/SceneMcp::new 시그니처에서 clients 제거 → 호출 테스트(mcp/scene_api/templates/ws) 정리. op-apply 경로(ws/sync/actor)는 registry-free라 무영향, mcp.rs 툴은 유지.  wave 4  rust+svelte (OB3.U1)
```
- **Verify:** 스트림별 cargo test + 렌더 골든. region 질의 정확성. 낙서 round-trip. 셸: 영속 플로팅 UI=Toolbar 1개(Sidebar/툴바/인스펙터/드로어 제거 확인), 단축키·우클릭이 scene-core 카탈로그 기반.

### Phase OB-4 — Integration & clean cutover (C7: 하위호환 없음)
> Goal: 신모델·신스토리지로 전면 교체, 구개념·TS 도메인 제거. (셸 인터랙션 표면은 OB-3 shell/UI 스트림에서 object 계약 기준으로 재작성되고 OB4.3 배선으로 통합.)
```
OB4.1  모델 컷오버                  → RenderGroup/Card/Edge + SceneSnapshot + SceneSelection을 Object으로 교체, 구종 제거.  wave 5  rust+renderer (OB-3)
OB4.2  storage 컷오버               → Record kind=canvas/object/template, redb를 durable store로(sqlite 대체).  wave 5  rust (OB3.T1~T4)
OB4.3  클라 데이터계층 배선          → store(D15 redb-OPFS)를 OutboxStore + replica store 뒤, 클라는 scene-core-wasm op-apply + 셸 UI(OB3.U*) 컴포넌트를 신 object 데이터에 배선.  wave 5  svelte+rust (OB3.T3,OB4.1,OB3.U1)
OB4.4  Rust-first 정리              → renderPatch.ts TS op-apply + renderScene.ts(shapeSceneToRenderSnapshot/defaultStyles) 삭제, CLAUDE.md 정정(P1), 셸 도메인 0 증명.  wave 6  cleanup+test (OB0.2,OB4.1,OB3.R10)
OB4.5  코어 wire 단일 채널화          → TS API 표면 제거: src/client/lib/{sceneServerApi,sceneClient,templatesApi}.ts의 bespoke REST(/api/canvases·comments·groups·templates) → scene-core wire(OB1.2) 단일 채널로. TS는 raw socket/fetch 배선만(프레임 운반), 도메인 API 구성 0(P1). (mcp 채널은 OB3.U6에서 선제거; 서버측 핸들러=OB3.S7.)  wave 6  cleanup (OB1.2,OB4.1,OB4.3,OB3.S7)
```
- **Verify:** grep로 구종/TS op-apply/renderScene.ts 참조 0. 전 편집이 Rust op-apply 경유.

### Phase OB-5 — Hardening & verification
> Goal: 제로랙·size·정합 게이트 통과.
```
OB5.1  제로랙 perf 게이트           → 1만 object 드래그=재tessellation 0, 라이브 스트로크, region windowing, 대용량 canvas frame budget.  wave 6  test
OB5.2  round-trip + 골든 벡터        → path-string↔render↔storage 왕복; op-apply 골든(Rust 단일원).  wave 6  test (OB4.4)
OB5.3  회귀                        → anchor 재라우팅·split/merge·auto-layout·dash·identity·회전/perspective hit-test·undo/redo(역op 왕복·per-actor·코얼레싱 1스텝).  wave 6  test
OB5.4  data-size 게이트             → path-string+zstd 목표치, raw점/mesh 미저장, instancing dedup 확인.  wave 7  test
```

---

## Dependency / Wave Summary
```
wave 0 : OB0.1 OB0.2 OB1.1 OB1.4
wave 1 : OB1.2 OB1.3
wave 2 : OB2.1
wave 3 : OB3.R1 OB3.R2 OB3.R3 OB3.R4 OB3.R10 OB3.S1 OB3.S2 OB3.S9 OB3.T1 OB3.U1
wave 4 (4a): OB3.R5 OB3.R6 OB3.R7 OB3.R8 OB3.R9 OB3.S3 OB3.S4 OB3.S5 OB3.S8 OB3.T2 OB3.T3 OB3.T4 OB3.D1 OB3.D2 OB3.U2 OB3.U5 OB3.U6
wave 4b    : OB3.S6 OB3.S7 OB3.D3 OB3.D4 OB3.A1 OB3.U3 OB3.U4
wave 5 : OB4.1 OB4.2 OB4.3
wave 6 : OB4.4 OB4.5 OB5.1 OB5.2 OB5.3
wave 7 : OB5.4
```
> **wave 캐비엇:** wave=페이즈 게이트(OB2.1 통과→OB-3 fan-out→OB-4 컷오버) + 위상레벨. OB-3 fan-out은 **4a(계약만 의존)→4b(4a 산출 소비)** 2층으로 분리해 'wave 4' 내부 producer→consumer를 해소. 잔여 같은-레벨 간선은 OB4.3→OB4.1(wave 5)·OB5.2→OB4.4(wave 6)뿐(통합/하드닝, dep로 직렬화). 실제 순서는 항상 dep 리스트가 지배.

핵심: **OB1.1(object 스키마)** 키스톤. OB0.1(storage 결정)·OB0.2(Rust-first seam)는 병렬 spike. OB2.1 수직슬라이스로 계약 검증 후 wave 3~4 광폭 fan-out. 컷오버(OB-4)에서 구모델·sqlite·TS op-apply 동시 제거.

## Verification Strategy
- **Rust:** crate별 cargo test(scene-core wasm32+native), 렌더 골든, region 질의, storage native+wasm(OPFS) round-trip.
- **제로랙:** frame budget 벤치(조작 재tessellation 0, 라이브 드로잉, windowing).
- **size:** path-string+zstd 목표치, raw점/mesh 미저장, instancing dedup.
- **Rust-first:** vitest가 실 scene-core-wasm로 통과, TS op-apply 참조 0.
- **불변식:** 단일 substrate(union 0), identity 3단, anchor 그래프 질의.

## Next step
OB0.1(redb/OPFS·성능 spike)·OB0.2(Rust-first 하네스)·OB1.1(object 스키마) 키스톤부터. 실제 착수는 별도 브랜치 분리 후.
