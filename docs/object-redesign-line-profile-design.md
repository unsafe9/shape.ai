# Object Primitive Redesign — Line Profile & Anchor Semantics (설계)

> Source: 앵커-팔로우 버그 추적 세션(2026-06-10)에서 드러난 의미론 구멍 + 방향 결정
> 토론("선은 bbox transform이 아니라 양끝점 조작이 맞다. 정량 resize 없음. 단,
> 데이터 타입은 바꾸지 않는다"). 상위 결정 원장:
> [object-primitive-redesign.md](./object-primitive-redesign.md) — 특히 P2(단일
> substrate), D5(anchor), D7(transform = 3×3 행렬, 조작 0-rebake).
> 관련 코드(현행): `crates/scene-core/src/object/{anchor_follow.rs, move_together.rs,
> cascade.rs}`, `src/renderer/core/src/{object_pipeline.rs, webgpu/scene_feed.rs}`,
> `src/client/renderer/engine.ts`, `src/client/svelte/App.svelte`.

Status: **draft — 구현 착수 전 리뷰용.** 승인되면 결정(DL*)을 원장에 폴드인한다.

## 1. 문제 정의

앵커의 약속은 불변식 하나다:

```
world(follower, node) == T_target · at        (at = 타깃-로컬 양자화 점, D5)
```

현행 구현은 이 불변식을 **타깃이 이동할 때만** 집행한다. 그 결과 세 가지 증상이
브라우저에서 관측됐다 ("anchored object를 움직이거나 transform할 때 끝점이
부적절한 위치로 움직인다"):

1. **방향성 구멍.** 팔로워(앵커를 가진 선) 자신을 이동/리사이즈/회전하면 —
   라이브 프리뷰는 인스턴스 행렬로 통째 이동(`engine.ts` `setObjectPreviewTransform`),
   커밋의 `anchor_follow_ops`(`anchor_follow.rs:274`)는 "움직인 오브젝트의
   팔로워"만 찾으므로 자기 자신의 앵커는 아무것도 안 한다. 끝점이 접착점을
   이탈한 채 앵커 레코드만 살아남고, **다음 타깃 이동 때 옛 접착점으로 스냅백
   점프**한다.
2. **멀티앵커 커밋 버그 2건.** START 코너 앵커 픽스 이후 앵커 2개 팔로워가
   기본 케이스가 됐는데:
   - 커밋은 `find()`로 타깃당 **첫 앵커만** 재투영(`anchor_follow.rs:295`).
     렌더러 프리뷰는 전체 앵커 루프(`scene_feed.rs:231`) — release 순간 발산.
   - 두 타깃이 한 배치에서 움직이면(멀티셀렉) 팔로워의 EditGeometry를 각각
     **씬 원본 geometry 기준으로** 계산해 두 번 emit → 뒤 op가 앞 op의 노드
     재작성을 덮어씀.
3. **근원: 조작 공간 불일치.** 선의 본질은 node-공간(양끝점)인데 닫힌 도형과
   동일하게 transform-공간(bbox resize/rotate)에 노출돼 있다. 선에 정량적
   resize는 의미가 없고, transform-공간 조작이 앵커(node-공간 제약)와
   충돌한다.

## 2. 결정 (DL — proposed)

### DL1. 조작 프로파일은 파생 속성이다 (데이터 타입 불변)

object 스키마(D1)는 그대로 둔다. 프로파일은 기존 데이터에서 유도한다:

| 프로파일 | 조건 | 조작 |
|---|---|---|
| **segment** | 열린 패스 + 노드 2개 | 끝점 핸들 2개 + 몸통 translate |
| **bbox** | 그 외 전부 | 기존 translate/resize/rotate |

유도 함수는 scene-core 소유(P1), 커맨드/제스처 카탈로그처럼 단일 소스로
export — 셸/렌더러는 읽기만 한다. 노드 2개짜리 펜 스트로크가 segment로
분류되는 것은 **수용 한계**(실용상 무해; 거슬리면 미래에 생성 도구가 힌트를
남기는 식으로 좁힘 — 그때 가서 데이터 추가).

### DL2. segment 어포던스: bbox 핸들 제거, 끝점 핸들 도입

segment 단독 선택 시 리사이즈/회전 핸들을 노출하지 않는다(정량 resize 없음).
대신 양 끝점에 노드 핸들 2개. 몸통 드래그(translate)는 허용하되 의미는 DL4.

### DL3. 끝점 드래그 = 노드 편집 + create-스냅 재사용 = 앵커 rebind/unbind

끝점 핸들 드래그는 EditGeometry(해당 노드 이동)이고, **생성과 동일한 스냅
머신**(`nearestOutlinePoint` + 빨간 링, 8px 톨러런스)을 태운다. release 시:

- 타깃 아웃라인에 스냅 → 그 노드의 앵커를 **재작성**(rebind: `target`/`at` 재계산)
- 빈 공간 → 그 노드의 앵커 **삭제**(unbind)

생성 스냅과 편집 스냅이 한 경로가 되어 앵커의 저작/갱신/해제가 전부 같은
코드를 지난다. **별도 분리(detach) 제스처가 불필요해진다** — 끝점을 끌어내면
자연히 풀린다. (Alt = 스냅 무효화는 생성과 동일하게 적용.)

### DL4. 몸통 이동 의미론: 앵커된 노드는 핀

segment의 translate는 "모든 노드를 delta만큼 이동"으로 해석한다. 앵커된
노드는 불변식이 잡고 있으므로 제자리(핀), 자유 노드만 이동한다. 양끝이 모두
앵커면 몸통 드래그는 시각적 no-op — 커넥터로서 올바른 동작이며, 이동하려면
먼저 끝점을 떼야 한다(DL3). FigJam/tldraw 커넥터와 동일한 의미론.

이는 일반 불변식 집행의 환원이다. 일반형:

```
node_local = inv(T_follower_new) · (T_target_new · at)
T_new = delta·T  (움직인 쪽만),  T  (안 움직인 쪽)
```

양쪽이 같이 움직이면(멀티셀렉 union, 부모-자식 cascade) delta가 소거된다:
`inv(δF)·(δT·at) = inv(F)·inv(δ)·δ·T·at = inv(F)·T·at` — **대수적 no-op**.
현행 "moved set에 든 팔로워는 skip"(`anchor_follow.rs`,
`move_together.rs:136`)은 이 일반식의 최적화 특수해로 그대로 유지한다.

### DL5. 앵커 저작은 열린 패스(segment)에만

`synthesize_create_anchors`를 segment 생성에만 적용한다. 닫힌 도형(rect 등)의
생성 스냅은 **위치 가이드만**(코너를 아웃라인에 끌어붙임) — 앵커는 저작하지
않는다. 현재 START 픽스로 닫힌 도형 코너에도 앵커가 붙는데, 이대로면 rect
이동 시 "코너 하나만 핀되어 찌그러지는" 케이스가 생긴다. glue는 커넥터 전용
(FigJam/Excalidraw와 동일)으로 제한해 닫힌 도형의 follower-이동 문제를
**원천 소멸**시킨다.

이 제한으로 DL4의 핀 의미론이 적용되는 대상은 segment뿐이 되고, 불변식
집행에서 남는 작업은 두 개의 작은 규칙뿐이다: ① 타깃 이동 → 팔로워 노드
재투영(기존 동작), ② segment 몸통 이동 → 자유 노드만 이동.

### DL6. 멀티앵커 커밋 = 팔로워당 단일 EditGeometry 누적 재작성

`anchor_follow_ops`를 프리뷰 루프(`scene_feed.rs:231`)와 동등하게 고친다:
한 팔로워의 **모든 해당 앵커**를 하나의 path-string에 누적 재작성해 **단일
EditGeometry**로 emit. closure의 reproject dedup도 follower-id 단위
(`move_together.rs:139`)에서 **(follower, target) 쌍 단위**로 바꿔, 두 타깃에
양끝이 앵커된 선이 멀티셀렉 이동에서 양끝 모두 따라오게 한다.
커밋·프리뷰 바이트 동등은 크로스코어 핀 테스트로 고정한다.

### DL7. 그룹/멀티셀렉 경로는 행렬 조작 허용

bbox 게이트(DL2)는 **segment 단독 조작에만** 적용한다. 멀티셀렉/그룹 resize에
segment가 포함되면 행렬이 그대로 적용된다(tldraw 방식, D7 0-rebake 유지).
이때 앵커-타깃이 같은 셀렉션에 있으면 DL4의 delta 소거로 자동 정합.

## 3. 알려진 갭 (이번 범위 밖, 문서화만)

- **타깃 geometry 편집 시 재투영 (D5 후반부).** `at`은 타깃-로컬 점이므로
  타깃의 **transform** 변화는 재투영이 따라가지만, 타깃의 **geometry**가
  편집되면(예: 타깃이 segment이고 그 끝점을 드래그) 아웃라인이 움직여도
  팔로워는 모른다. D5는 "target geometry 편집 시 outline에 re-project"를
  명시하므로, EditGeometry도 팔로워 재투영을 트리거하고 `at`을 새 아웃라인의
  최근접점으로 클램프해야 한다. **DL3 구현 후 후속 phase** — segment→segment
  체인이 생기는 순간 필요해진다.
- **멀티노드 폴리라인의 중간 노드 핸들.** segment(2노드)만 우선. 펜 스트로크
  노드 편집은 비범위.

## 4. 구현 스케치 / 영향 지도

| 영역 | 변경 |
|---|---|
| scene-core | 프로파일 유도 fn + export; `anchor_follow_ops` 누적 단일 EditGeometry(DL6); `synthesize_create_anchors` segment 제한(DL5); segment 몸통 이동 규칙(DL4 — `moveOps` cascade에서 segment+anchored 분기); closure 쌍-dedup(DL6) |
| renderer core | 끝점 핸들 hit-test + 라이브 프리뷰 — **기존 앵커-팔로우 경로 재사용**: `reexpand_single_object` + `patch_follower_geometry`(단일 오브젝트 노드 재작성, 재테셀레이션 0, P4 충족). bbox 핸들을 프로파일로 게이트 |
| shell | 끝점 핸들 이벤트 배선(스냅 질의는 기존 `querySnap` 재사용), App.svelte 생성 경로에서 닫힌 도형 앵커 저작 제거 |
| tests | 크로스코어 핀 확장: 몸통-이동 핀 고정 / both-moved no-op / 멀티앵커 단일 EditGeometry / rebind·unbind 왕복 / 닫힌 도형 앵커 미저작 |

성능 노트(P4): 끝점 드래그·핀 재투영 모두 per-frame wasm 호출 없이 기존
preview-write + follower-patch 경로를 타고, 재테셀레이션은 편집된 그 오브젝트
1개로 한정 — 새 핫패스 없음.

## 5. 즉시 수정 가능한 독립 버그 (설계 승인과 무관)

DL6의 두 커밋 버그(첫-앵커-만, EditGeometry 덮어쓰기)는 의미론 결정과 무관한
정합성 버그라 먼저 패치 가능하다. DL4/DL5가 승인되면 그 위에 얹는다.
