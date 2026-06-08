# Object Redesign — Wave 2: 통합 포인터 · 선택/변환 · 드래그생성+스냅 · 드로우모드 · 인라인텍스트 · 툴바재편 · 코어 리팩토링 — Task Breakdown

> Source: object-primitive-redesign 라이브 cutover(OB-4 / FC-01~16) 이후 **최초 실브라우저 구동에서 드러난 런타임 UX 결함 + 다음 웨이브 리디자인** 토론 (2026-06-08, `/design`). 선행 맥락: [Object Primitive Redesign](./object-primitive-redesign.md), [Canvas Cockpit Redesign](./canvas-cockpit-redesign-task-breakdown.md). 실행 규칙: `feature/object-primitive-redesign` 브랜치.
> 관련 코드(현행, 4-에이전트 탐색으로 확인): 렌더 코어 `src/renderer/core/src/{webgpu.rs(6861줄),object_pipeline.rs,render_object.rs,hit_test_object.rs,stats.rs,outline.rs}` + 셰이더 `shaders/{object_fill,object_stroke}.wgsl`; 셸 `src/client/svelte/{App.svelte,Toolbar.svelte,ContextMenu.svelte}`, `src/client/renderer/engine.ts`, `src/client/lib/{canvasHost.ts,objectPrimitives.ts,toolbar.ts,shortcuts.ts}`, `src/client/scene/sceneCoreWasm.ts`, `src/shared/object.ts`, `src/client/styles.css`.

이 문서는 object 단일 substrate 위의 **편집 인터랙션 레이어**(포인터·선택·변환·도형생성·드로우·텍스트·툴바)를 Figma류 수준으로 끌어올리고, 그 과정에서 비대해진 렌더 코어를 정리한다. 7개 요청 → 8개 서브시스템(S1~S8) → 13개 태스크(W2-01~W2-13).

## Objective (요청 원문 → 매핑)

1. Move/Hand 분리 커서를 **단일 스마트 포인터**로 통합(캔버스 팬 + 오브젝트 이동/변환 + 멀티셀렉트, 가능한 한 마우스만, 키보드 1개까지 허용) → **S2**.
2. 마우스 입력 결과가 **우상단으로 어긋남**(캔버스 크기/위치 불일치) 수정 → **S1**.
3. 하단 툴바 draw/shapes 나열 → **기본 모드 vs 드로우 모드 분리**(브러시·지우개·컬러 팔레트) → **S5**.
4. 도형이 사전구성 geometry 즉시삽입 → **버튼 후 드래그 생성**, 기존 오브젝트 **아웃라인 실시간 추종 점 + 앵커 스냅**(여유있게), **모디파이어로 스냅 무력화** → **S4**.
5. 오브젝트 **선택 부재** → 좌클릭 시 아웃라인 하이라이트, 모서리에 **리사이즈/회전 핸들** → **S3**.
6. 드로잉/도형 **생성 후 선택**; 기본도형 = **네모/동그라미/선/텍스트 한 섹션**(Frame 분리); 텍스트도형 = 보더 없는 네모(모든 object가 텍스트 보유); 선택 후 **Enter로 텍스트 입력**, 텍스트도형 생성 즉시 **입력모드** → **S6**.
7. 툴바 **More 그룹 동작**: Templates를 도형 섹션으로 이동 + **스크롤 팝업**으로 선택삽입, Debug = **패널 토글**, Export = **무수정**(추후 정리) → **S6/요청7**.
8. (추가) 드래그/프리드로잉 시 **매 프레임 전체 재테셀레이션(P4 zero-rebake 위배)** 제거 → **S7**. (추가) 비대한 **Rust 코어 모놀리스 분리 + `webgpu.rs` 네이밍 정리** → **S8**.

## Locked Principles (상위 문서 계승 + 본 웨이브 적용)

- **P0. 퍼포먼스 최우선(CLAUDE.md):** 모든 구현에서 성능이 1순위 타겟. "느릴 게 뻔한데 일단 동작만" 구현은 금지 — 바를 못 맞추면 설계를 고쳐서 맞춘다(조작=행렬만, 매 프레임 재tessellation 0, 핫패스 불필요 할당 0). 특히 W2-04(핸들 렌더)·W2-06(곡선 스냅 쿼리)·W2-07(드래그 생성 프리뷰)·W2-11(zero-rebake)에 직접 적용.
- **경계(CLAUDE.md):** 캔버스 로직은 Rust 코어, 플랫폼 레이어는 얇게. → 선택 핸들·변환 수학·아웃라인 스냅·hit-test = **코어**. 입력/IME(인라인 텍스트 편집·키 캡처)·UI 크롬(툴바/팝업) = **셸**.
- **P1. Rust-first.** 도메인/geometry 진실원 = Rust. TS는 배선·입력·전송만.
- **P4. Zero-lag.** 조작=행렬만(재tessellation 0). → **S7**이 현행 위반(드래그 시 feedScene 전체 씬 재구성)을 교정.
- **포터블 코어.** 포인터-폭 무관, time/random/thread/IO 없음. → S8 분리 시 web 전용(web_sys) 서피스와 포터블 wgpu 렌더러를 명시 분리.

## Decisions Ledger (본 세션 잠금)

- **D1. 선택 핸들·리사이즈/회전 수학 = Rust 코어 GPU.** SVG 오버레이 아님. 근거: 선택 외곽선(FocusRing)이 이미 코어 GPU(`render_object.rs:424`, `webgpu.rs:3913`)에 있고, 경계 규칙 준수 + 네이티브(macOS/iOS) 포팅 시 그대로 동작. 비용: 코어 작업량 증가 감수.
- **D2. 통합 포인터 팬 = Space-홀드 / 휠·트랙패드 스크롤 / 중간버튼 드래그.** 빈 곳 좌드래그 = 마퀴 선택(Figma 표준). 키보드는 Space 하나만 요구(요청1의 "하나 허용" 범위). 마우스 단독도 휠·중간버튼으로 가능.
- **D3. 아웃라인 앵커 스냅 = 곡선까지 풀 구현.** 직선 세그먼트 + 타원 + 프리핸드 폴리라인 최근접점 실시간 추종. (단계화 대신 풀 구현 선택 — 정밀도/성능 리스크 일괄 감수.)
- **D4. 지우개(현행 미존재 확인됨) = 통째 삭제 기본 + 모디파이어 부분.** 기본 = 닿은 펜 스트로크 통째 delete-object, 단축키 홀드 시 부분 지우기(단순 subpath 분할, scene-core 신설).

**확정 — 요청2 근원(draw 우상단 드리프트):** 오브젝트 카메라 유니폼에 **물리 픽셀**이 주입됨. `webgpu.rs:1344-1345`(`update_camera` 매프레임)·`1501-1502`(`ObjectRenderer::new`)가 `self.config.width/height`(=논리×DPR)를 넘김. 셰이더 `object_fill.wgsl:64`/`object_stroke.wgsl:70` `world_to_clip`이 **논리 좌표(camera.xy + zoom)** 를 **물리 뷰포트**로 나눠 1/DPR만큼 축소→레티나(2×)에서 우상단 드리프트. 포인터(`engine.ts:638`)·레거시 유니폼(`webgpu.rs:2183`)은 논리 픽셀로 올바름. 수정 = 두 호출이 `self.width/height`(논리)를 넘기도록.

## 현행 인터랙션 자산 (탐색 결과 — 무엇이 있고 무엇이 없나)

**있음(재사용):**
- 도구 상태 `ActiveTool="select"|"hand"|"draw"`(`engine.ts:24`), 배선 Toolbar→App→host→engine→Rust(`ActiveTool::Select|Hand`만). 커서=CSS `data-tool`(`styles.css:189`).
- 단일-오브젝트 hit-test(top-most, 무상태)(`hit_test_object.rs:137`, `webgpu.rs:6486`). **호버 상태 없음.**
- 선택 모델: 셸 `ObjectSelection={canvas|object|multi}`(`object.ts:188`, `App.svelte` selection state), 코어 `selection:Option<String>`+`multi_select:Vec`(`render_object.rs:189`).
- 드래그 이동: `InputDragState::Object`→누적 `ObjectTransformDelta{id,dx,dy}`(`webgpu.rs:6121`); 비파괴 프리뷰 `feedScene`/`sceneWithObjectShifted`(`App.svelte:1016/988`); 1-op `set-transform` 커밋(`App.svelte:139`, pendingCommit 스냅백 방지).
- 마퀴: `InputDragState::Marquee`→`object_marquee_ids`(`webgpu.rs:1726`, `stats.rs:161`).
- 선택 외곽선: 코어 GPU FocusRing 4px stroke(`render_object.rs:424`, `webgpu.rs:3913`).
- 펜/드로우 컨트롤러: engine이 draw 입력 가로채 `{type:"draw",phase,world}` 방출→App `handleDraw`(`App.svelte:474`) 누적→`sceneCore.freehandToObject` 커밋(`App.svelte:62-67,474-494`), sticky.
- 템플릿 lowering: `buildObjectTemplate` wasm(`sceneCoreWasm.ts:240`)+`templateApply` feature(`App.svelte:682`), id=todo_board/decision_map/presentation. 현행 UI=인라인 하드코딩 div(`App.svelte:1160`).
- 디버그 패널: `diagnosticsOpen` 토글 + 인라인 패널(`App.svelte:73,1168`). 재사용 팝업 패턴=`ContextMenu.svelte`.
- 변환 표현: `Transform3x3` row-major(`object.ts:40`), 현재 평행이동만 사용.

**없음(신설):**
- 호버 상태 / affordance 기반 동적 커서.
- 8핸들·회전영역 렌더, 핸들 hit-test, 리사이즈/회전 행렬 수학(델타는 `{dx,dy}` 이동 전용).
- 도형 드래그 생성(현재 뷰포트 중앙 고정크기 즉시삽입 `App.svelte:461`), 러버밴드 프리뷰.
- 아웃라인 최근접점 스냅 쿼리 + 인디케이터.
- 드로우 서브툴바(브러시/컬러/지우개), 지우개 일체, 부분 subpath 분할.
- 인라인 캔버스 텍스트 편집기(현재 `window.prompt`/속성패널 rename만; `engine.ts:312`에 휴면 텍스트 오버레이 잔재).
- 드래그 중 인스턴스-행렬 GPU 경로(현재 매 프레임 전체 씬 재구성=재tessellation).

## Subsystem Map

| Sx | 이름 | 요청 | 핵심 신규 | 위치 |
|----|------|------|-----------|------|
| S1 | 좌표 픽셀 수정 | 2 | 논리 px 유니폼 + DPR 회귀 | 코어 |
| S2 | 통합 스마트 포인터 | 1 | 호버 커서 + Space/휠/중간 팬 + Shift 다중 | 코어+셸 |
| S3 | 선택 핸들 + 리사이즈/회전 | 5 | 8핸들 렌더·hit·풀 행렬 변환 | 코어+셸 |
| S4 | 드래그 생성 + 아웃라인 스냅 | 4 | 러버밴드 생성 + 곡선 최근접점 스냅 | 코어+셸 |
| S5 | 드로우 모드 분리 | 3 | 서브툴바 + 지우개(통째/부분) | 셸+소코어 |
| S6 | 툴바 재편 + 인라인 텍스트 | 6,7 | 섹션 재편·템플릿 팝업·디버그·인라인 편집 | 셸 |
| S7 | 드래그 zero-rebake | 8(추가) | 인스턴스 행렬 GPU 경로 | 코어+셸 |
| S8 | 코어 리팩토링 | 8(추가) | webgpu.rs 6861줄 분리 + 네이밍 | 코어 |

## Task Breakdown

표기: `ID  subject → 의도/산출. [files] · verify · depends · lane`. lane = 파일 커플링 직렬화 차선(CORE=webgpu.rs/object_pipeline 직렬, SHELL=App.svelte 직렬, TOOLBAR=Toolbar.svelte 병렬).

### Phase P0 — 토대 (최우선)

```
W2-01 (S1) 좌표 논리픽셀 수정 + DPR 회귀
  → update_camera/ObjectRenderer::new이 논리 px(self.width/height) 사용. draw 우상단 드리프트 해소.
  [webgpu.rs:1344-1345,1501-1502; shaders/object_{fill,stroke}.wgsl:64/70 확인]
  · verify: DPR=2 screen→world→NDC 왕복 일치 회귀 + renderer:rust:test
  · depends: —  · lane: CORE#1

W2-13 (S8) 렌더러 코어 모놀리스 분리 + webgpu 네이밍 정리 (행동보존)
  → webgpu.rs 6861줄을 책임별 분리: (a)wgpu 디바이스/서피스/config 라이프사이클[web_sys 종속=웹 전용]
    (b)프레임 렌더/present (c)입력 상태머신+hit-test 라우팅(InputDragState/apply_input_event)
    (d)오브젝트 씬 로드/피드 (e)stats/debug 스냅샷. webgpu/ 모듈 디렉토리화(lib.rs:27 갱신).
    네이밍: 포터블 wgpu 렌더러 vs 웹 전용 서피스(target_arch=wasm32 게이트) 분리로 'webgpu' 오해 해소.
    부가: 1000줄 초과(object_pipeline 1207/text_layout 1071/tessellate 1025/outline 1017) 책임 점검.
  [src/renderer/core/src/*]
  · verify: 전체 게이트 green + 동작/API 시그니처 불변(renderer:rust:test 163 통과 = 회귀 오라클)
  · depends: W2-01  · lane: CORE#2  · ⚠ 피처(02/04/06/11)보다 먼저 — 모놀리스 추가 적재 차단
```

### Phase P1 — 포인터 키스톤

```
W2-02 (S2 core) 호버 affordance + 입력결과 계약
  → pointermove(버튼 없음) 호버 hit-test로 affordance(empty/body/resize-{corner}/rotate/handle) 산출,
    CoreInputBatchResult에 필드 추가. 셸 커서 전환 입력.
  [hit_test_object.rs:137 재사용, stats.rs:159 인근, webgpu.rs 입력경로]
  · verify: 본체/빈곳/핸들 위 호버 힌트 단위테스트 · depends: W2-13 · lane: CORE

W2-03 (S2 shell) 통합 Move 포인터 + Space/휠/중간버튼 팬 + 다중선택
  → Hand 버튼 제거, 단일 Move. 빈곳클릭=해제·오브젝트클릭=선택·오브젝트드래그=이동·빈곳드래그=마퀴·
    Shift+클릭=다중토글. 팬=Space홀드/휠·트랙패드/중간버튼. W2-02 affordance로 동적 커서.
  [engine.ts:638 eventPoint/커서, App.svelte selection 집합/키핸들, styles.css:189]
  · verify: typecheck + 선택토글/팬의도 분류 단위테스트 + test:unit · depends: W2-02 · lane: SHELL#1
```

### Phase P2 — 병렬 코어/셸

```
[CORE lane]
W2-04 (S3 core) 선택 핸들 렌더 + 리사이즈/회전 변환 상태·수학
  → 8핸들+회전영역 스크린픽셀 고정 quad 렌더(zoom 불변). 핸들 hit-test→InputDragState::Resize{corner}/Rotate.
    ObjectTransformDelta{dx,dy}→풀 Transform3x3 델타(리사이즈=반대 앵커 scale, 회전=bbox 중심). 단일 우선.
  [object_pipeline.rs:764 build_scene_geometry, render_object.rs:424 FocusRing 형제, webgpu.rs:500 InputDragState, stats.rs]
  · verify: 핸들 zoom 불변 + NE드래그→예상 행렬 + 회전 θ 단위테스트 · depends: W2-13,W2-02 · lane: CORE

W2-06 (S4 core) 아웃라인 최근접점 쿼리 (곡선 풀 구현)
  → 월드점+허용오차 → 모든 오브젝트 아웃라인 최근접점: 직선 투영·타원 최근접·폴리라인 투영. 스냅 점+대상 id.
    허용오차=zoom 보정 스크린픽셀(여유있게).
  [hit_test_object.rs / outline.rs:derive_region 재사용]
  · verify: 사각엣지/타원/폴리라인 최근접점 + 허용오차 경계 단위테스트 · depends: W2-13,W2-01 · lane: CORE
  · ⚠ 최대 리스크: 타원/곡선 최근접점 정밀도·성능

[SHELL lane — App.svelte 직렬]
W2-05 (S3 shell) 리사이즈/회전 비파괴 프리뷰 + 커밋
  → feedScene/sceneWithObjectShifted를 풀 행렬 적용으로 일반화. 비파괴 프리뷰→pointerup 1-op set-transform
    (pendingCommit 스냅백 방지 유지). 핸들 커서(nwse/nesw/회전).
  [App.svelte:1016/988/139] · verify: typecheck + 프리뷰→1op 커밋 단위테스트 + test:unit
  · depends: W2-04(델타 계약) · lane: SHELL#2

W2-07 (S4 shell) 드래그 생성 도형 + 실시간 스냅 + 생성후 선택
  → 도형 도구→드래그 생성 서브모드(펜 컨트롤러 패턴). 러버밴드 프리뷰(drag bbox)→buildPrimitiveObject를
    bbox 크기로 커밋→선택. W2-06 스냅 점 실시간 추종, 모디파이어 홀드 시 무력화.
  [App.svelte:461/474 패턴, objectPrimitives.ts 고정크기 제거, engine.ts 도형-draw 라우팅]
  · verify: typecheck + bbox 크기/생성후 선택/스냅 토글 단위테스트 · depends: W2-06,W2-03 · lane: SHELL#3

W2-08 (S5) 드로우 모드 서브툴바 + 지우개(통째/부분)
  → 펜 활성 시 컨텍스추얼 툴바(브러시 크기·컬러 팔레트·지우개). 하드코딩 PEN(App.svelte:63)→반응형 상태.
    지우개: 기본 닿은 스트로크 통째 delete-object, 모디파이어 홀드 시 부분 subpath 분할(scene-core 신설).
    펜 커밋 후 선택.
  [App.svelte/Toolbar.svelte(or DrawToolbar.svelte), engine.ts erase 분기, scene-core subpath split]
  · verify: typecheck + 지우개 통째/부분 단위테스트 + test:unit · depends: W2-03 · lane: SHELL#4 + 소코어

W2-10 (S6 text) 인라인 텍스트 편집 + 텍스트 도형(보더 없는 네모)
  → 텍스트 프리미티브=보더/스타일 없는 네모(objectPrimitives.ts 스타일 제거, "Note" 기본값 제거).
    선택 후 Enter 또는 텍스트 생성 직후 즉시 입력: contenteditable 오버레이를 bbox worldToScreen 배치→
    blur/Enter set-text 커밋, Esc 취소(IME=플랫폼). engine.ts:312 휴면 오버레이 재활용 검토.
  [App.svelte 오버레이, objectPrimitives.ts, shortcuts.ts Enter→edit, engine.ts worldToScreen]
  · verify: typecheck + 무보더/즉시편집/set-text 커밋 단위테스트 · depends: W2-03,W2-01 · lane: SHELL#5

[TOOLBAR lane — 병렬]
W2-09 (S6/요청7) 툴바 재편 + 템플릿 스크롤 팝업 + 디버그 토글
  → 기본도형 한 섹션=네모/동그라미/선/텍스트, Frame 분리. Templates를 도형 섹션으로 이동→클릭 시 위로
    스크롤 팝업 리스트(TemplatePopup.svelte 신설, ContextMenu 패턴/.floating 스타일 재사용)→항목 삽입
    (applyTemplate/buildObjectTemplate 재사용). Debug=패널 토글 실동작 보장(diagnosticsOpen/App.svelte:1168
    가시성·위치 점검). Export=무수정.
  [Toolbar.svelte:205-272, TemplatePopup.svelte(신설), App.svelte:1160-1176, styles.css]
  · verify: typecheck/build + 팝업 리스트·삽입·디버그 토글 동작 · depends: W2-03 · lane: TOOLBAR(병렬)
```

### Phase P3 — 퍼포먼스 + 통합

```
W2-11 (S7) 드래그 zero-rebake 인스턴스 행렬 GPU 경로
  → 드래그/프리드로잉 중 전체 재테셀레이션 제거. 드래그 대상 오브젝트의 인스턴스 모델행렬만 GPU 갱신
    (object_pipeline.rs 인스턴스 변환버퍼/per-object model matrix). 셸 드래그 프리뷰(feedScene 전체 씬 재구성)→
    인스턴스 행렬 푸시로 전환(canvasHost/engine 경유).
  [object_pipeline.rs, webgpu.rs, App.svelte:1016, canvasHost.ts]
  · verify: 기존 10k-object zero-retess 게이트를 드래그까지 확장 + renderer:rust:test
  · depends: W2-04,W2-05 · lane: CORE+SHELL · ⚠ GPU 픽셀 미검증(빌드레벨 범위 밖, 플래그)

W2-12 통합 + 전체 게이트 재실행
  → e2e(object-live-path.test.ts 확장): 통합 포인터 선택/이동/마퀴/팬, 핸들 리사이즈/회전, 드래그생성+곡선 스냅,
    드로우+지우개(통째/부분), 인라인 텍스트, 템플릿 팝업, 생성후 선택. 전체 게이트 green.
  · verify: cargo workspace + renderer:rust:test + scene/renderer wasm build + typecheck + test:unit + npm build
  · depends: W2-01..11 · lane: —
```

## Dependency / Lane Summary

```
CORE lane (webgpu.rs/object_pipeline 직렬):   01 → 13 → 02 → 04 → 06 → 11
SHELL lane (App.svelte 직렬):                  03 → 05 → 07 → 08 → 10
TOOLBAR lane (Toolbar.svelte, 병렬):           09
교차 의존: 02→03, 04→05, 06→07, (04,05)→11, all→12
```

- **키스톤:** W2-13(코어 분리)이 이후 모든 코어 편집의 토대 — P0에서 W2-01 직후 선행.
- **병렬 가능:** P2에서 CORE(04→06)와 SHELL(05→07→08→10), TOOLBAR(09)가 동시 진행. SDD worktree 격리 + lane별 순차 머지 적합.
- **단일→다중:** 리사이즈/회전은 단일 오브젝트 우선. 다중 선택은 이번 범위에서 *이동/마퀴*까지, 다중 bbox 변환은 후속 확장.

## Risks / Open

- **W2-06 곡선 최근접점**(타원·프리핸드) 정밀도/성능 — D3 풀 구현의 대가, 본 웨이브 최대 리스크.
- **W2-13** 6861줄 행동보존 분리 — 회귀 위험, 테스트 게이트(163)에 의존. API 시그니처 보존이 안전판.
- **W2-08 부분 지우기** — scene-core path subpath 분할 신규(단순 절단 수준으로 한정).
- **GPU 픽셀 미검증** — standing 제약(브라우저/GPU 검증 제외)상 실제 렌더·포인터 픽셀 동작은 빌드+단위까지만. **W2-11** 특히 시각 동작 미확인.
- **다중선택 변환** 범위 한정(위 참조) — 사용자 기대와 어긋나면 후속 확장 필요.

## Deferred (플래그만, 본 웨이브 범위 밖)

OB4.5 canvas CRUD/`/api/templates` REST→단일 wire 채널 · redb-OPFS 피처조합 빌드/워커 브리지 · 렌더러 Tess/Flatten·derived region/text 캐시의 라이브경로 편입(latent fill-AA/stroke-expansion) · 텍스트 run size 미양자화(렌더 피드) · 실제 브라우저 GPU 라스터화·포인터 검증(사용자 제외).

## Verification Strategy

태스크별 단위/회귀 게이트(위 각 카드) + lane 머지 후 전체 게이트: `scripts/renderer-toolchain.sh` 경유 `cargo test --workspace`, `npm run renderer:rust:test`, `npm run scene:wasm:build`/`renderer:wasm:build`, `npm run typecheck`/`test:unit`/`build`. 코어 시그니처 변경 시 wasm 재빌드 필수. **빌드+단위까지가 본 웨이브 검증 범위**(GPU 픽셀 제외).

## Done Criteria

7개 요청 전부 구현 + 13 태스크 verify 통과 + 전체 게이트 all_passed. 통합 포인터로 선택/이동/마퀴/팬, 모서리 핸들 리사이즈/회전, 도형 드래그 생성 + 아웃라인 곡선 스냅(+모디파이어 무력화), 드로우 모드 브러시/컬러/지우개(통째+부분), 생성 후 선택, 텍스트도형 즉시 인라인 편집 + Enter 편집, 툴바 기본도형 섹션 재편 + 템플릿 스크롤 팝업 + 디버그 토글, 드래그 zero-rebake, 코어 모놀리스 분리 + 네이밍 정리가 모두 코드에 존재하고 게이트로 검증됨.

## Next step

설계·결정·분해 확정. 실행은 SDD(태스크당 fresh worker + worktree 격리 + lane별 순차 머지) 또는 페이즈별 워크플로우로 P0(W2-01→W2-13)부터 착수. 호스트 태스크 W2-01~W2-13이 본 문서를 미러(세션 TaskList).
