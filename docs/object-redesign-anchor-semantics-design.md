# Object Primitive Redesign — Anchor Semantics & Two-Surface Manipulation (설계)

> Source: 앵커-팔로우 버그 추적 세션(2026-06-10)에서 드러난 의미론 구멍 + 방향 토론.
> v1은 "선 전용 조작 프로파일"(segment vs bbox 분류기)을 제안했으나 **일관성
> 반론으로 폐기** — 분류기는 P2(단일 substrate)를 UI 레이어에서 다시 깨고,
> 경계가 자의적이다(3노드 폴리라인? 핸들 달린 2노드 곡선? 프리드로잉?).
> v2(현재)는 분류기를 제거하고 균일 모델로 재설계.
> 상위 결정 원장: [object-primitive-redesign.md](./object-primitive-redesign.md) —
> 특히 P2(단일 substrate), D5(anchor), D7(transform = 3×3 행렬, 조작 0-rebake).
> 관련 코드(현행): `crates/scene-core/src/object/{anchor_follow.rs, move_together.rs,
> cascade.rs}`, `src/renderer/core/src/{object_pipeline.rs, webgpu/scene_feed.rs}`,
> `src/client/renderer/engine.ts`, `src/client/svelte/App.svelte`.

Status: **draft v2 — 구현 착수 전 리뷰용.** 승인되면 결정(DU*)을 원장에 폴드인한다.

## 1. 문제 정의

앵커의 약속은 불변식 하나다:

```
world(follower, node) == T_target · at        (at = 타깃-로컬 양자화 점, D5)
```

현행 구현은 이 불변식을 **타깃이 이동할 때만** 집행한다. 그 결과 브라우저에서
관측된 증상 ("anchored object를 움직이거나 transform할 때 끝점이 부적절한
위치로 움직인다"):

1. **방향성 구멍.** 팔로워(앵커를 가진 오브젝트) 자신을 이동/리사이즈/회전하면 —
   라이브 프리뷰는 인스턴스 행렬로 통째 이동(`engine.ts`
   `setObjectPreviewTransform`), 커밋의 `anchor_follow_ops`
   (`anchor_follow.rs:274`)는 "움직인 오브젝트의 팔로워"만 찾으므로 자기 자신의
   앵커는 아무것도 안 한다. 끝점이 접착점을 이탈한 채 앵커 레코드만 살아남고,
   **다음 타깃 이동 때 옛 접착점으로 스냅백 점프**한다.
2. **멀티앵커 커밋 버그 2건.** START 코너 앵커 픽스 이후 앵커 2개 팔로워가
   기본 케이스가 됐는데:
   - 커밋은 `find()`로 타깃당 **첫 앵커만** 재투영(`anchor_follow.rs:295`).
     렌더러 프리뷰는 전체 앵커 루프(`scene_feed.rs:231`) — release 순간 발산.
   - 두 타깃이 한 배치에서 움직이면(멀티셀렉) 팔로워의 EditGeometry를 각각
     **씬 원본 geometry 기준으로** 계산해 두 번 emit → 뒤 op가 앞 op의 노드
     재작성을 덮어씀.
   - closure의 reproject dedup이 follower-id 단위(`move_together.rs:139`)라
     양끝이 서로 다른 두 이동 타깃에 앵커된 팔로워는 프리뷰에서 한쪽만 따라옴.

## 2. 모델: 두 조작 표면 + 단일 불변식 (분류기 없음)

### DU1. 모든 object는 동일한 두 조작 표면을 가진다

| 표면 | 진입 | 조작 | 대상 |
|---|---|---|---|
| **transform 공간** | 선택 | translate / resize / rotate (bbox 핸들, D7 행렬, 0-rebake) | 모든 object — 선 포함 |
| **node 공간** | 편집 모드 (더블클릭, 후속 wave) | 개별 노드 드래그 (EditGeometry) | 모든 패스 — 선 끝점, 프리드로잉 중간 노드, rect 코너 모두 같은 메커니즘 |

"선의 끝점 핸들"은 별도 기능이 아니라 **2노드 패스의 편집 모드가 보여주는
노드가 마침 2개**인 것. 분류기(노드 수, open/closed, 도구 종류)가 모델 어디에도
없다. 선의 bbox resize는 금지하지 않는다 — "늘리기"라는 멀쩡한 의미가 있고
(Figma 동일), 금지가 만드는 일관성 비용이 더 크다. 선 사용자는 자연히 편집
모드를, 프리드로잉 사용자는 자연히 transform을 주로 쓰게 된다 — **기능 차이가
아니라 사용 빈도 차이**로 UX가 분화한다.

### DU2. 앵커 불변식은 모든 변형 경로에서 균일하게 집행된다

```
node_local = inv(T_follower_new) · (T_target_new · at)
T_new = delta·T  (moved set에 든 쪽),  T  (안 든 쪽)
```

| 케이스 | 결과 |
|---|---|
| 타깃 이동/변형 | 팔로워 노드 재투영 — **기존 동작** |
| 팔로워 이동/리사이즈/회전 | 앵커 노드는 접착점에 **핀**, 나머지 geometry가 변형 — **신규** |
| 양쪽이 같은 배치에서 이동 (멀티셀렉 union, 부모-자식 cascade) | delta 소거로 **자동 no-op**: `inv(δF)·(δT·at) = inv(F)·T·at` |

이 규칙은 도형 종류를 모른다 — 선/rect/프리드로잉/곡선 전부 "앵커된 노드는
항상 접착점에 있다" 하나로 동작한다. 현행 "moved set에 든 팔로워는 skip"
(`anchor_follow.rs`, `move_together.rs:136`)은 세 번째 행의 최적화 특수해로
유지한다. 두 번째 행(팔로워 단독 이동)이 §1-1 방향성 구멍의 수정이다.

귀결: 양끝이 모두 앵커된 선의 몸통 드래그는 시각적 no-op(양 끝 핀). 이동하려면
앵커를 떼야 한다(DU4/DU5) — FigJam/tldraw 커넥터와 동일한 의미론.

### DU3. 곡선 노드의 핀: handle-follow

베지어 노드(D2 `inHandle`/`outHandle`)가 재투영될 때 인접 핸들도 같은 delta로
평행이동한다(노드만 움직이면 곡률이 비의도적으로 변형). 현행
`set_path_node`는 좌표쌍 하나만 재작성하므로 C-세그먼트 컨트롤 포인트 추적이
추가로 필요 — 프리드로잉(베지어 fit, D13) 끝점 앵커가 자연스러워지는 조건.

### DU4. 분리(detach) 제스처: Alt-드래그 (편집 모드 도입 전 interim)

앵커된 오브젝트를 Alt 누르고 드래그하면 불변식을 무시하고 통째 이동 +
해당 앵커 삭제. 생성 시 Alt = 스냅 무효(기존 C2 `no-snap-alt`)와 일관 —
"Alt = 제약 없이"의 균일 확장. 제스처 카탈로그(scene-core `gestures.rs`)에
등록해 단일 소스 유지.

### DU5. 편집 모드의 노드 스냅 = 앵커 rebind/unbind (후속 wave)

편집 모드에서 노드 드래그는 생성과 **동일한 스냅 머신**(`nearestOutlinePoint`
+ 링)을 태우고, release 시: 타깃 아웃라인에 스냅 → 그 노드의 앵커
재작성(rebind), 빈 공간 → 앵커 삭제(unbind). 생성/편집 스냅이 한 경로.
모든 패스의 모든 노드에 균일 적용(선 끝점만이 아니라). 라이브 프리뷰는 기존
앵커-팔로우 경로(`reexpand_single_object` + `patch_follower_geometry`,
재테셀레이션 0) 재사용 — 새 핫패스 없음(P4).

### DU6. 멀티앵커 커밋 = 팔로워당 단일 EditGeometry 누적 재작성

`anchor_follow_ops`를 프리뷰 루프(`scene_feed.rs:231`)와 동등하게: 한 팔로워의
**모든 해당 앵커**(자신이 움직인 경우 포함)를 하나의 path-string에 누적
재작성해 **단일 EditGeometry**로 emit. closure의 reproject dedup은
**(follower, target) 쌍 단위**로 수정. 커밋·프리뷰 바이트 동등은 크로스코어
핀 테스트로 고정.

### DU7. 앵커 저작 정책 (열린 결정 — 엣지 정책이라 어느 쪽이든 모델 일관성 무관)

"스냅된 생성이 앵커를 저작하는가"는 모델이 아니라 제스처 해석이다. 나중에
뒤집어도 모델 수술이 없다:

- **(a) 균일 저작**: 스냅 링이 보인 모든 생성이 앵커 저작 — rect 코너 포함.
  모델상 최순수. 단, 코너 앵커된 rect를 옮기면 그 코너가 핀되어 변형되는데
  (DU2의 균일 귀결), 이게 사용자 의도였는지 의문.
- **(b) 열린 패스 끝점만 저작**: glue는 커넥터 전용 — FigJam/Excalidraw 표준.
  엣지에서의 정책 분기 하나로, 닫힌 도형은 스냅을 위치 가이드로만 쓴다.

권고: **(b)로 시작** (업계 표준 + 변형 서프라이즈 회피), 편집 모드(DU5)가
생기면 rect 코너도 명시적으로 붙일 수 있게 되므로 그때 (a)와의 간극이
자연 소멸한다.

## 3. 알려진 갭 (이번 범위 밖, 문서화만)

- **타깃 geometry 편집 시 재투영 (D5 후반부).** `at`은 타깃-로컬 점이므로
  타깃의 transform 변화는 따라가지만 geometry 편집(예: 편집 모드로 타깃
  노드를 드래그)은 아웃라인이 움직여도 팔로워가 모른다. D5는 "target geometry
  편집 시 outline에 re-project"를 명시 — EditGeometry도 팔로워 재투영을
  트리거하고 `at`을 새 아웃라인 최근접점으로 클램프해야 한다. **DU5 구현과
  같은 wave** — 편집 모드가 생기는 순간 필요해진다.

## 4. 구현 스케치 / 영향 지도

즉시(이번 라운드) — DU2 + DU4 + DU6:

| 영역 | 변경 |
|---|---|
| scene-core `move_together.rs` | closure가 "moved set에 든 오브젝트 자신이 미이동 타깃에 앵커된 경우"의 (follower, target) 쌍도 방출 + 쌍에 어느 쪽이 움직였는지 표시; dedup을 쌍 단위로 |
| scene-core `anchor_follow.rs` | 무버 자신의 앵커 재투영(팔로워 NEW transform × 타깃 씬 transform); 팔로워당 모든 앵커 누적 단일 EditGeometry |
| scene-core `gestures.rs` | `detach-alt` 등록 (DU4) |
| renderer `scene_feed.rs` | 프리뷰 reproject에 팔로워-쪽 delta 케이스 추가 (`inv(delta·F_base)`); 기존 `reexpand_single_object`/`patch_follower_geometry` 재사용 |
| shell | Alt-드래그 커밋이 앵커 삭제 op 동반 (SetAnchor 기존 op) |
| tests | 크로스코어 핀 확장: 팔로워-이동 핀 / both-moved no-op / 멀티앵커 단일 EditGeometry / 쌍-dedup / Alt-detach |

후속 wave — DU5 + DU3 + §3 (편집 모드 + handle-follow + 타깃 geometry 재투영).

성능 노트(P4): 모든 라이브 경로가 기존 preview-write + follower-patch를 타고,
재테셀레이션은 편집된 오브젝트 1개로 한정. per-frame wasm/FFI 추가 없음.

## 5. 즉시 수정 가능한 독립 버그 (설계 승인과 무관)

DU6의 두 커밋 버그(첫-앵커-만, EditGeometry 덮어쓰기)와 쌍-dedup은 의미론
결정과 무관한 정합성 버그라 먼저 패치 가능하다. DU2/DU4가 승인되면 그 위에
얹는다.
