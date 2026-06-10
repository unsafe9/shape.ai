# Object Primitive Redesign — Open/Closed 클래스, 앵커 의미론, 프리드로잉 인식 (설계 v3)

> Source: 앵커-팔로우 버그 추적(2026-06-10)에서 드러난 의미론 구멍 → v1(선 전용
> 프로파일, 일관성 반론으로 폐기) → v2(균일 두 표면 + 팔로워-핀 DU2) → **v3(현재)**.
> v2의 핀 방식은 translate엔 깨끗하지만 resize/rotate가 팔로워에 들어오면
> "한 노드는 핀, 나머지는 변형"이라는 의도 불명의 결과를 낳는다는 사용자 반론
> ("본질적으로 transform이 들어갈 때의 문제가 해결이 안 된다")로 대체됐다.
> 상위 결정 원장: [object-primitive-redesign.md](./object-primitive-redesign.md) —
> 특히 P2(단일 substrate), D5(anchor), D7(transform=3×3, 조작 0-rebake),
> D11/D13(프리드로잉 파이프라인).
> 관련 코드(현행): `crates/scene-core/src/object/{anchor_follow.rs, move_together.rs,
> cascade.rs, drawing.rs, model.rs}`, `src/renderer/core/src/{object_pipeline.rs,
> webgpu/scene_build.rs, webgpu/scene_feed.rs}`, `src/client/renderer/engine.ts`,
> `src/client/svelte/App.svelte`.

Status: **v3 승인 — 구현 진행 (2026-06-10 사용자 결정).** 잠긴 결정:
chord-similarity 변형 / 앵커 1개 몸통 드래그 = 자유 끝 추종 / 전체 한 사이클
(표면+slave+변형+인식+프리드로잉 앵커링).

## 1. 모델: 모든 오브젝트는 closed(도형) 아니면 open(선)이다

분기 기준은 UI 분류기가 아니라 **데이터 레벨 속성**이다:

```
open-class   ⇔  subpath가 정확히 1개이고 그 subpath가 closed:false
closed-class ⇔  그 외 전부 (닫힌 패스, 멀티 subpath, 텍스트, 프레임, 레거시 잉크)
```

| | closed-class (도형) | open-class (선/곡선) |
|---|---|---|
| transform 표면 | bbox 8핸들 + 회전 (D7, 0-rebake) | **없음** — 끝점 핸들 2개만 |
| fill | 가능 | **불가** (스타일 패널에서 숨김, 빌드에서 무시 — 현행 G7 skip_fill과 합치) |
| 앵커 역할 | **target만** | target 가능 + **follower 가능 (유일한 팔로워 계급)** |
| 포즈의 진실 | transform 행렬 T | **끝점 쌍** — 나머지는 끝점의 함수 |

v1 기각 사유("선만 특수 처리 = 일관성 붕괴")가 재발하지 않는 이유: 기준이
"선 vs 나머지"(자의적)가 아니라 `SubPath.closed`(전수적 이분, `drawing.rs`)이고,
§4의 인식이 **생성 시점에 모든 오브젝트를 이 이분법으로 정규화**하기 때문이다.
모호한 중간물(정리 안 된 잉크)이 캔버스에 존재하지 않는다.

## 2. open-class 의미론: 포즈 ≡ 끝점 쌍, chord-similarity 변형

### 2a. 핵심 수학 (잠김)

끝점 2개 = 자유도 4 = similarity 변환(회전+균일스케일+평행이동 2)의 자유도.
끝점이 (s,e)→(s′,e′)로 움직이면 옛 chord를 새 chord로 보내는 유일한 similarity
S를 **모든 노드(베지어 in/out 핸들 포함)에** 적용한다:

```
S = T(s′) · R(Δθ) · σI · T(−s),   σ = |e′−s′| / |e−s|,  Δθ = angle(e′−s′) − angle(e−s)
```

- 실루엣 보존: 그린 모양 그대로 회전·신축 — "밧줄/고무줄" 은유의 답.
- v2 DU3(handle-follow)를 흡수: 핸들도 같은 S를 타므로 별도 메커니즘 불요.
- **퇴화 가드**: |e−s| < ε(예: 4q)이면 σ 폭주 — σ를 [1/σ_max, σ_max]로 클램프하고
  클램프가 발동하면 회전 없이 평행이동 폴백. 나선처럼 chord ≪ 호 길이인 입력 보호.
- 구현 위치: scene-core 순수 함수 `deform_open_path(d, new_start_px, new_end_px)
  -> Option<String>` — **커밋(EditGeometry)과 라이브(렌더러 reexpand+patch)가
  같은 함수를 쓴다** (G12-B path-dep rlib 경유, per-frame FFI 없음).

기각한 대안(기록): (i) T-누적 방식(S를 transform에만 합성, 0-rebake) — 스트로크
**폭이 chord 길이를 따라 변하는** 부작용으로 기각; 폭 보존이 자연스러운
geometry-rewrite + G14 1-오브젝트 패치 경로 채택. (ii) arc-length falloff —
실루엣 비례가 깨짐. (iii) 직선화 보간 — 그린 곡선 모양 소실.

### 2b. 조작 표면

- 선택 시 bbox 8핸들/회전존 대신 **끝점 핸들 2개**(노드 0과 마지막 노드)를
  렌더(`scene_build.rs` 핸들 레이아웃 분기). 끝점 핸들 드래그 = 그 끝점 이동 →
  chord 변형. 릴리즈는 생성과 동일한 스냅 머신을 타서 앵커 rebind/unbind
  (v2 DU5의 끝점 케이스가 편집 모드 없이 앞당겨진 것).
- **몸통 드래그 정책 (잠김)**: 앵커 0개 → 평행이동(양 끝 같은 delta — SetTransform
  translate 그대로, 인스턴스 행렬 라이브, 0-rebake 유지). 앵커 1개 → 앵커 끝은
  접착점에 핀, **자유 끝만 delta를 받아** 고무줄처럼 변형. 앵커 2개 → no-op
  (양 끝 핀), 이동하려면 Alt-드래그 detach(DU4 유지: 앵커 삭제 + 통째 이동,
  `gestures.rs` 카탈로그 등록).
- **그룹/멀티 transform 라우팅**: open-class에 resize/rotate를 금지하되 그룹
  조작은 깨지 않는다 — 그룹 변형은 open-class 멤버의 **끝점을 통해서만 도달**한다.
  멤버의 양 끝점을 그룹 delta로 월드 매핑 → 새 끝점 쌍 → chord 변형. 회전하는
  그룹 안의 선은 시각적으로 통째 회전과 동일(S가 곧 그 회전), 타깃 하나만 그룹에
  든 커넥터는 한 끝만 따라가고 chord가 수습. 순수 translate 그룹이면 양 끝 delta가
  같으므로 SetTransform 패스트패스.
- 중간 노드 직접 편집은 편집 모드(후속 wave, v2 DU5)의 몫 — chord-space 실루엣
  변경 = EditGeometry, 본 설계와 직교.

### 2c. slave 규칙 (제안 1)

앵커가 붙은 open-class 오브젝트는 **자체 transform 표면이 없다** — v2 DU2의
"팔로워 이동 시 핀" 대신 표면 제거로 방향성 구멍을 원천 차단한다. §1에 의해
팔로워는 open-class뿐이므로(DU7=(b) 잠김) 이 규칙이 rect 등에 적용될 일이 없어
일관성 비용이 0이다. 집행 위치는 **커맨드 합성 + UI 표면**이다: 쉘/엔진이
핸들을 제공하지 않고, `move_ops`/cascade 합성이 open-class에 SetTransform
resize/rotate를 저작하지 않는다(끝점 EditGeometry로 라우팅). **op-apply
오라클(apply.rs/op.rs/undo.rs)은 건드리지 않는다** — 와이어로 온 op는 여전히
적용된다(서버/동기화 경로 보호).

## 3. 앵커 의미론 (v2에서 단순화 상속)

불변식은 그대로: `world(follower, node) == T_target · at` (at = 타깃-로컬 양자화
점, D5). v3에서의 집행:

| 케이스 | 결과 |
|---|---|
| 타깃 이동/변형 | 앵커된 끝점 재투영(기존 `reproject_node_local_quantized`) → **나머지 노드는 chord 변형** (기존: 끝점만 재작성 → 중간이 안 따라오는 스파이크) |
| 팔로워 단독 transform | **존재하지 않음** (표면 제거, §2c) — v2의 핀 케이스 소멸 |
| 양쪽 같은 배치 이동 | delta 소거 no-op — 기존 moved-set skip(`move_together.rs:136`) 유지 |
| 자유 끝점 드래그 릴리즈 | 스냅 → rebind, 빈 공간 → unbind (생성 스냅 머신 재사용) |
| Alt-드래그 | detach: 앵커 삭제 + 통째 이동 (DU4) |

- DU7 저작 정책 **(b)로 잠김**: 앵커는 open-class의 끝점에만 저작된다(생성·
  프리드로잉·끝점 드래그 릴리즈). closed-class 생성 시 스냅은 위치 가이드로만.
- DU6(팔로워당 누적 단일 EditGeometry + (follower,target) 쌍 dedup)은 G14에
  랜딩 완료 — 유지. 커밋·프리뷰 바이트 동등은 크로스코어 핀으로 계속 고정.
- open-class도 타깃이 될 수 있다(선→선 앵커). 재투영된 팔로워를 moved로 보고
  closure를 재귀시키는 **체인 전파는 후속 wave** (§3 타깃-geometry-편집 재투영과
  같은 wave).

## 4. 프리드로잉 → 도형 인식 (제안 3, D13 확장)

프리드로잉은 잉크가 아니라 **도형 입력기**다: pen-up 순간 스트로크를 가장
유사한 정규형으로 변환한다. 글씨/그림 용도가 아니라 데이터 표현 용도 — raw
잉크는 오브젝트로 살아남지 않는다(토글 없음, 사용자 결정).

```
점 캡처(기존, transient) → pen-up:
  1. 닫힘 판정: dist(start,end) < k · bbox 대각 (+ 자기교차 휴리스틱)
  2. 정준 fit 시도(신뢰도 임계 통과 시 채택):
     열린: 직선(최대 편차), 부드러운 곡선(기존 fit_beziers)
     닫힌: 원/타원(최소제곱), rect(회전 min-area + 각 스냅), 삼각/다각형(코너 검출)
  3. 폴백(임계 미달): 실루엣 보존 노멀라이즈 — RDP(거친 ε) + 코너 검출 +
     스무딩 fit. 꽃을 그리면 정돈된 꽃 윤곽이 남는다. 닫힘 판정 결과로 close.
```

- 위치: **scene-core 신규 `recognize.rs`** (순수 기하, no time/rng/IO) +
  `wasm_api` 브리지. 쉘은 점 배열을 넘기고 geometry/closed를 받는 글루만.
- **per-stroke 오브젝트**: 스트로크 하나 = 인식된 오브젝트 하나. D13의
  "세션=오브젝트 1개(multi-subpath)" 정책을 대체한다(인식된 rect와 인식된 선이
  한 오브젝트일 이유가 없다). 기존 멀티 subpath 오브젝트는 레거시로 남고
  closed-class 취급(§1) — 마이그레이션 불요.
- **프리드로잉 앵커링**: 스트로크 시작/끝이 타깃 아웃라인 근처면 기존 호버
  스냅 링 표시, 릴리즈 결과가 open이면 기존 생성 경로
  (`resolveCreateRelease` + `synthesizeCreateAnchors`, 시작/끝 양 코너)로 앵커
  저작 — 박스 두 개 사이에 선을 찍 그으면 앵커된 커넥터가 된다.
- 인식 후보 집합·임계값은 agent discretion + 사용자 브라우저 검증 루프로 튜닝.

## 5. v2 → v3 대체 맵

| v2 | v3 |
|---|---|
| DU1 균일 두 표면 (선도 bbox resize) | **대체** — §1 open/closed 이분 표면. 편집 모드(노드 공간)는 후속 wave에서 양 클래스 균일 유지 |
| DU2 팔로워 이동 시 핀 | **대체** — §2c 표면 제거. 불변식·both-moved no-op은 §3로 상속 |
| DU3 handle-follow | **흡수** — §2a chord-similarity가 핸들 포함 균일 적용 |
| DU4 Alt-드래그 detach | 유지 (§3) |
| DU5 편집 모드 rebind/unbind | 유지, 후속 wave — 끝점 케이스만 §2b로 앞당김 |
| DU6 누적 단일 EditGeometry + 쌍 dedup | 유지 (G14 랜딩 완료) |
| DU7 (a)/(b) 열린 결정 | **(b)로 잠김** (§3) — 제안 1(slave)이 강제 |
| §3 타깃 geometry 편집 재투영 | 유지, 후속 wave (+ 선→선 체인 전파 추가) |

## 6. 구현 지도

| 영역 | 변경 |
|---|---|
| scene-core 신규 | `deform_open_path`(chord-similarity, 퇴화 클램프) · `recognize.rs`(인식) · open-class 판별 헬퍼 |
| scene-core `cascade.rs`/`move_ops` | open-class moved 멤버: 순수 translate → SetTransform 유지, 그 외 → 끝점 매핑 + `deform_open_path` EditGeometry 라우팅 (오라클 무접촉) |
| scene-core `anchor_follow.rs` | 끝점 재투영 후 전체를 `deform_open_path`로 — 커밋·라이브 단일 소스 |
| scene-core `gestures.rs`/`commands.rs` | `detach-alt` 등록 |
| renderer `scene_build.rs` | open-class 선택 = 끝점 핸들 2개(8핸들/회전존 미생성), 끝점 드래그 제스처 → endpoint delta 신호 |
| renderer `scene_feed.rs` | 라이브: open-class의 비-translate 프리뷰/팔로워 재투영이 `deform_open_path` 결과로 G14 `reexpand_single_object`+`patch_follower_geometry` 경로를 탐 (컴파일타임 가드 승계) |
| scene-core `drawing.rs`+`wasm_api` | pen-up per-stroke 인식 커밋 |
| shell (`App.svelte`/engine) | 끝점 핸들 이벤트 배선, fill UI 숨김(open), 프리드로잉 시작/끝 스냅 + 릴리즈 앵커 저작, Alt-detach 커밋(SetAnchor 삭제 동반) |
| tests | chord 수학(항등/평행이동/회전신축/퇴화 클램프/핸들 추종), 인식 골든(합성 스트로크), 크로스코어 핀(커밋 EditGeometry == 라이브 패치), 몸통 드래그 0/1/2 앵커, 그룹 회전 라우팅 |

성능(P4): 순수 translate는 기존 인스턴스 행렬 0-rebake 그대로; 끝점 변형
라이브는 G14에 이미 입증된 1-오브젝트 reexpand+patch(재테셀 1개 한정, megabuffer
in-place); 인식은 pen-up 1회. per-frame wasm/FFI 추가 없음(렌더러는 scene-core를
rlib로 인프로세스 호출). 데이터타입 변경 0, 마이그레이션 0.

## 7. 알려진 갭 / 후속 wave

- 편집 모드(중간 노드 편집 + 전 노드 rebind/unbind) — v2 DU5.
- 타깃 geometry 편집 시 팔로워 재투영(`at` 아웃라인 클램프) + 선→선 체인 전파.
- 인식 품질 튜닝(임계값/후보 추가: 화살표 등) — 브라우저 검증 루프.
- 기존 캔버스의 레거시 멀티 subpath 잉크는 closed-class로 남는다 — 인식을
  소급 적용하지 않는다(파괴적). 필요해지면 명시적 "정규화" 커맨드로.
