# Canvas Cockpit Redesign + Full Rust Migration & Realtime-Ready Sync — Task Breakdown

> Source: UX 방향 + storage/콜라보/서버 토폴로지/전송·운영/canvas-단위·분산 토론 (2026-06-06~07). 별도였던 storage-adapter 문서를 편입(원본 삭제). storage 원안: [idea1.md](./idea1.md). 선행 맥락: [AI Companion Canvas Task Breakdown](./ai-companion-canvas-task-breakdown.md).
> 관련 코드(현행, status-reconciled): `src/renderer/core/src/{webgpu.rs,model.rs}`(Rust 렌더 코어 — **렌더-패치 적용·입력·히트테스트만**), `src/shared/{renderPatch.ts,operation.ts,schema.ts,templates/}`(**도메인 op-apply·baseRevision·템플릿이 현재 TS에 존재** — scene-core로 *포트* 대상), `src/storage/core/*`(Rust storage core — **Memory/File 어댑터·번들 export/import·테스트 기구현, 소비자만 0개; sqlite/postgres/s3/remote는 stub**), `src/server/{index.ts,storage.ts(49KB, sql.js),mcp.ts,mcpClients.ts,local.ts}`(현행 Node/Fastify — **제거 대상**), `src/client/...`(셸·UI; `App.svelte` `multiSelectIds` T2.2 기구현).

이 문서는 두 트랙을 한 마이그레이션으로 묶는다.

1. **Canvas Cockpit Redesign:** 하단 리모콘, move/hand 툴 + 드래그 마퀴, 사용자 구성·등록형 템플릿 라이브러리(빌트인 포함 삭제), 상황별 우클릭 메뉴, 설정(readonly 단축키) + 단축키 중앙화.
2. **Full Rust Migration & Realtime-Ready Sync:** 백엔드 전면 Rust(axum/tokio, **canvas 단위 문서당 액터**), **storage core(sqlite) 도입 — 서버 native + 클라 wasm 공통**, 전송 WebSocket(프로토콜 교체 가능), Figma식 server-authoritative per-property LWW, **대용량 canvas 부분 로딩(server·client 메모리-바운드)**, stateless 백엔드.

**전제(사용자 확정):** **개발 단계 하위호환성 전부 불요.** 기존 코드·데이터 다 버리고 신스택으로 클린 컷오버, 필요 데이터는 새로 생성. 듀얼런/레거시/마이그레이션 없음.

## Objective

백엔드·코어를 단일 Rust 워크스페이스로 수렴시켜 (a) 편집 표면을 Figma식 직접조작에 가깝게, (b) op-apply·동기화·저장 권한을 서버 Rust로, (c) 클라(WASM)·서버(native)가 **동일 `scene-core`+`storage-core`** 로 "편집의 의미"와 저장을 공유, (d) **canvas 단위**로 미래 멀티테넌트/실시간 공유가 가능하고, **각 canvas가 메모리를 초과해도 양측이 부분 로딩으로 처리**하는 stateless·확장형 토대를 짓는다.

## Guiding Architecture (Rust-first, server 포함) — Locked

- **Svelte 셸 = 웹 배포용 thin 표현 레이어.** DOM/팝오버/드로어, 키 캡처, 전송 클라이언트(WS) 호출, core 카탈로그/목록 렌더링만. 도메인 로직·기하·모델변경·히트테스트·op-apply·저장을 셸로 올리지 않는다.
- **Rust `scene-core`(공유, wasm32+native) = 순수 로직.** 모델, op enum, op-apply, per-property LWW, 검증, fractional index, 템플릿(recipe/apply/seed/from-selection), wire serde, command 카탈로그. 플랫폼-순수(시간·난수·seq·id 주입).
- **Rust `renderer-web`(클라, WASM) = optimistic 복제 + 렌더.** wgpu/WebGPU, 텍스트, LOD, 입력. `scene-core` 의존.
- **Rust `storage-core`(서버 native + **클라 wasm**) = canonical/local durable.** Record{id,kind,version,payload} + **sqlite 어댑터**. 서버=native sqlite, **클라=wasm sqlite(OPFS VFS)** 로 동일 코드. 저널/체크포인트. 도메인 중립.
- **Rust `coordination`(서버) = ephemeral 조정.** canvas 소유권 리스, 인스턴스 디렉터리, presence, pub/sub. InMemory/File(현재)→Redis(추후).
- **Rust `server`(axum/tokio) = 전송·권한·팬아웃·정적호스팅·MCP.** WS 종단, **canvas 단위 문서당 액터**, scene-core+storage-core+coordination 링크. MCP=[modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk).
- **셸↔core seam은 선언적**(input event / 선언적 op / command).

근거 Locked Decisions(ai-companion): scene mutation/hit test/selection geometry/input batching은 Rust core; 성능 책임을 Svelte로 안 올림; active tool은 ephemeral; canonical write path=granular op+actor+base revision; web/native 셸이 같은 core 공유.

## Persistence & Collaboration Direction (Decided)

- **PC1.** 서버 권한 영속화(순수 클라 로컬 저장은 cross-client 수렴 불가).
- **PC2.** canonical durable 엔진=storage core(sqlite). Record version=LWW 토큰, kind=canvas/card/edge/group/tag/comment/artifact/template.
- **PC3.** Figma식 server-authoritative per-property LWW. 타이브레이크=서버 monotonic seq(arrival). baseRevision=메타데이터.
- **PC4.** client/server가 `scene-core`로 op-apply·LWW·wire 공유 + `storage-core`로 저장 공유 — 단일 진실원.
- **PC5.** realtime-ready를 처음부터(granular op+opId+baseRevision+actor(userId)+서버 seq+저널). 멀티유저는 컷오버 후 최종 tail이나 엔진은 그 전에 콜라보형.
- **PC6.** storage-core 소비자=scene+템플릿(둘 다 1급 기능, 파일럿 아님). 합류는 의존성 순서.
- **PC7.** stateless(교체·복구 가능) 백엔드. durable=storage-core, ephemeral=coordination. canvas당 single-writer 리스.
- **PC8.** 전송=WebSocket(현재), 프로토콜 교체 가능. Transport 논리 2채널(reliable_ordered, ephemeral_besteffort)+wire는 scene-core. 미래 WebTransport·gRPC stream 교체 가능.
- **PC9. canvas = 문서/sync/액터/리스/라우팅 단위.** 무한 Scene이지만 사용자가 **여러 canvas**를 가질 수 있고, 미래 멀티테넌트/실시간 공유를 canvas 단위로. 현재는 서버↔클라 단일 사용자.
- **PC10. 대용량 canvas = 양측 메모리-바운드 부분 로딩.** 각 canvas가 메모리를 초과할 수 있으므로 **서버 액터는 bounded working set(viewport/관심도 구동 hot region) + storage-core region 질의로 backing**, **클라는 windowed replica(뷰포트 구독 + 렌더 culling)**. 어느 쪽도 full-canvas in-memory를 가정하지 않는다. 이는 렌더러 소유 culling과 별개인 **데이터 레이어 windowing**이다.

## Template Model (최종 스펙)

- 템플릿 = primitive 구성의 저장 단위. 빌트인 = 사전 구성 컴포넌트 모음(seeded, 커스텀과 동일 kind=template, 삭제 가능). 템플릿화 단위 3종: 단일 객체 / 멀티선택 여러 객체 / 단일 그룹. `recipe_from_selection`이 3종 처리. 파일럿 아닌 1급 기능.

## Sync Technical Design (모두 도입)

- 문서 모델 `Map<ObjectID, Map<Property, Value>>`; optimistic 적용(렉 0); **durable outbox(storage-core wasm+sqlite)** append→전송→ack→제거, opId=(clientId,localSeq) 멱등; unacked-property discard=transient ownership; coalescing(~33ms); 재접속=snapshot+reapply; 서버 저널+체크포인트(seq); z-order=fractional index; 2채널(op/presence) 분리; 텍스트 동시편집은 미래 그 필드만 CRDT.

## Operations & Scale-out Design (stateless 계층)

> 이 문서에서 **"문서(document)" = canvas**(sync/액터/리스/라우팅 단위, PC9).
- **canvas 배치 = 리스/디렉터리 동적 배치(정적 해싱 아님).** 라우팅=coordination에서 canvasId owner 룩업→없으면 claim. scale-out 시 기존 canvas는 owner drain까지 그대로(대량 리샤딩 없음). 해시는 placement hint, 리스가 source of truth.
- **룩업 per-connection + 인메모리 캐시 → 핫패스 홉 0.** 라우팅은 연결/attach 시 1회(캐시). 핫패스는 owner 메모리 local. **단순 TCP LB** + 앱단 canvasId 룩업/캐시로 각 **서버 pod(당장은 프로세스)** 가 효율 처리.
- single-writer per canvas(owner만 seq·직렬화·저널). 리스 TTL+갱신, 만료 시 인수.
- 클라 재연결(백오프+jitter), 미연결 중 outbox 버퍼링+로컬 적용+UI offline, 재연결 replay.
- graceful shutdown(신규 거부→flush+체크포인트→draining→리스 해제→drain) / 세션 handoff(owner 사망→타 인스턴스 인수, outbox가 미승인 op 보장→무손실).
- coordination 인터페이스: `acquire_lease/find_owner/renew/release`, `publish/subscribe(canvasId)`, `presence_put/get(ttl)`. InMemory/File→Redis.

## Large-canvas / Partial Loading Design (PC10)

- **저장:** storage-core가 객체를 **공간 인덱스(bbox/region) + canvasId** 로 질의 가능하게(region 질의, windowed read, bounded ingest — 기존 records stream/bounded 토대 활용).
- **서버 액터:** canvas 전체가 아니라 **활성 region(구독된 뷰포트 합집합)의 working set**만 메모리에. cold region은 LRU eviction(저널/체크포인트가 durable backing). 액터는 storage-core region 질의로 필요 시 로드.
- **클라:** **windowed replica** — 뷰포트(+여유 margin) region을 서버에 구독, 그 안 Record만 로컬 보유. 카메라 이동 시 region 구독 갱신(들어온 것 로드, 멀어진 것 evict). 렌더 culling(렌더러 소유)과 **별개 데이터 레이어**.
- **sync:** region-scoped subscription — `subscribe{canvasId, region}` → 서버가 그 region Record 스냅샷 + 이후 op 스트림. region 밖 op는 안 보냄(대역폭·메모리 바운드).
- **편집 정합:** outbox/optimistic은 보유한 region 내 객체에. region 밖을 건드리는 연산(예: 멀리 붙여넣기)은 명시적 로드 후 처리.

## Target Architecture (end state)

```
Cargo workspace
├─ crates/scene-core    (shared, wasm32+native) : model(canvas/object), op, apply, LWW, validate,
│                                                  fractional-index, templates, wire serde, command catalog
├─ crates/renderer-web  (client, wasm32)        : wgpu render, text, LOD, input, bindgen  → scene-core
├─ crates/storage-core  (native + wasm32)       : Record, sqlite 어댑터(native sqlite / wasm sqlite+OPFS),
│                                                  region/spatial 질의, 번들 export/import, journal/checkpoint
├─ crates/coordination  (server native)         : 리스/디렉터리/presence/pubsub — InMemory|File|Redis
└─ crates/server        (server native)         : axum/tokio, WS(논리 2채널), canvas 단위 액터(bounded working set),
                                                   MCP(rust-sdk), static hosting  → scene-core+storage-core+coordination
Svelte 셸 (web)          : DOM/UI + WS 전송 클라이언트 + scene-core(wasm) + storage-core(wasm sqlite) outbox/replica
```

**전송:** WebSocket(논리 2채널). 미래 WebTransport·gRPC stream 교체 가능(O14). WebRTC P2P 미사용.

**제거 대상(컷오버 후):** `src/server/{index,storage,mcp,mcpClients,local}.ts`, `src/client/lib/api.ts`(HTTP), `src/shared/renderPatch.ts` TS op-apply(및 scene-core 대체 `src/shared/*`), package.json 의존성(fastify/@fastify/static/sql.js/@types/sql.js/@modelcontextprotocol/sdk/tsx/concurrently), 스크립트(dev:server/mcp/start tsx), tsconfig.server.json·dist-server. **기존 `.local` 데이터도 폐기(이관 없음).**

## Storage Core — Decided (편입: D1–D8)

- **D1.** 교환 번들 vs at-rest 분리. **D2.** 도메인 중립. **D3.** Record 1개=파일 1개(평문). **D4.** 가독성=인코딩 함수. **D5.** 어댑터=가치 seam. **D6.** 평문=canonicalize+lint. **D7.** lint 2티어. **D8.** Record 단위≈사람 단위.

## Confirmed Decisions

- **C1.** move/hand + 마퀴 기하·히트테스트=Rust. 마퀴는 core가 교차 id 반환→**기존 `multiSelectIds`(T2.2) set에 병합**(단일 앵커 불변식).
- **C2.** 템플릿=사용자 구성·등록형(단일/멀티/그룹), 빌트인=사전 구성 모음(동일 kind, 삭제 가능), storage core 영속.
- **C3.** 하단 리모콘=툴+도형+템플릿+줌. 진단/트레이스만 우상단.
- **C4.** Rust-first(server 포함, 웹은 thin 배포층).
- **C5.** 백엔드 전면 Rust(axum/tokio + scene-core 공유). Go 비채택. 동시성 tokio.
- **C6.** 전송=WebSocket(현재), 프로토콜 교체 가능(PC8).
- **C7.** 하위호환·데이터 이관 없음, 클린 컷오버 + 기존코드·데이터 제거.
- **C8.** stateless 백엔드 + 외부화 조정(durable=storage-core, ephemeral=coordination), canvas당 single-writer 리스 + 동적 배치.
- **C9. canvas = 문서/sync/액터/리스 단위.** 다중 canvas, 미래 멀티테넌트/공유 대비.
- **C10. 대용량 canvas 양측 메모리-바운드 부분 로딩**(서버 bounded working set + 클라 windowed replica, 데이터 레이어 windowing).
- **C11. storage = sqlite, 서버 native + 클라 wasm(OPFS) 공통.** 클라 outbox/replica도 storage-core(wasm sqlite). (O11 해소)
- **C12. MCP = [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk).** 하위호환 없음, 서버 Rust 스펙. (O13 해소)
- **C13. 사용자 = userId만 구분, 인증 없음.** op `actor`=userId, 단일 개인 사용자 핵심 기능 우선. 미래 인증은 **코드 적절 위치에 TODO 한 줄**. (O12 해소)

## Open Decisions

**Wave 0 게이트(착수 전 잠금):**
- **O2.** storage at-rest = **sqlite** (서버 native + 클라 wasm 공통). → 확정.
- **O3.** payload 인코딩 = **JSON 텍스트**. → 확정.

**기타(권고 확정 / 나중):**
- **O5.** 빌트인 삭제 = tombstone+seed-guard. → 권장 확정.
- **O7.** 설정 표면 = 모달(Cmd+,). → 권장 확정.
- **O14.** 전송 스왑(미래): WebTransport(QUIC LB) 또는 gRPC stream. abstraction 뒤. → deferred.
- **O16.** coordination 백엔드: InMemory/File(현재)→Redis(추후 pod 확장). 인터페이스 동일.
- *(해소: O1 storage native+wasm, O6 템플릿=scene-core, O8 서버=Rust, O9 전송=WS, O10 abstraction, O11=C11(클라 wasm sqlite), O12=C13(userId), O13=C12(rust-sdk), O15 라우팅=리스/디렉터리+per-conn 캐시, O17 LB=단순 TCP+앱 룩업.)*

## Constraints / Non-goals

- 단일 앵커 persisted-selection 불변식(multi=셸 ephemeral, T2.2).
- gesture gating 보존(coalescing 확장).
- TS에서 캔버스 객체·op-apply·저장 만들지 않음(scene-core/storage-core 이관).
- 단축키 편집 UI 비범위(설정 readonly; binding/command 분리).
- WebRTC P2P 비채택(미래 미디어 한정).
- 멀티유저 협업 UI는 컷오버 후 최종 tail.
- 하위호환·데이터 이관·듀얼런 없음.
- full-canvas in-memory 가정 금지(PC10): 서버·클라 모두 windowed.
- durable↔ephemeral 분리.
- Surgical.

## Architecture Notes (구현 지침)

**셸↔core seam:** `SetTool{tool}`/`Pick{screen}`/`pointer-*`; tool-aware pointer-down(hand=Pan, move=빈히트 Marquee); `InputDragState::Marquee`+`InputBatchResult.marquee{rect,ids}`→기존 `multiSelectIds` 병합; op `insert-primitive`/`apply-template`; `command_catalog()->JSON`.

**전송/sync seam(wire=scene-core serde):** Transport trait=`reliable_ordered`+`ephemeral_besteffort`(WS 한 소켓). **// 미래 WebTransport·gRPC stream 등 교체 가능 구조 유지.** `hello{canvasId,region,lastAckSeq}`→`welcome{snapshot(region)|deltaSince}`→live; 업스트림 `ops[{opId,objectId,kind,propDelta,baseRevision,actor=userId,ts}]`→`ack{opIds,seq,revision}`; 다운 `patch{ops,seq}`; presence=ephemeral; region 변경 시 재구독; 재접속=snapshot+reapply. 서버: canvas 액터(tokio task)가 op mpsc 직렬→seq→LWW(scene-core)→저널(storage-core)→broadcast; bounded working set + region 질의; 대량 재적용 `spawn_blocking`.

**운영 seam:** coordination(리스/디렉터리/presence/pubsub) InMemory|File|Redis. 라우팅=canvasId owner 룩업(per-conn, 인메모리 캐시→핫패스 홉 0)→없으면 claim; 단순 TCP LB. shutdown=flush+체크포인트+리스 해제. 재연결=백오프+outbox replay.

---

## Task Breakdown

**MG**(Migration·Sync·Ops·Scale) 스파인 + **CC**(Cockpit) 병렬. `scene-core` 포트(MG-0) 키스톤. `wave`=병렬 묶음.

### Track MG

#### Phase MG-0 — Workspace & `scene-core` 포트 (키스톤)
> 도메인 op-apply는 현재 TS에. "추출" 아닌 **TS→Rust 포트 + 동치성 검증**.
```
MG0.1  Cargo workspace 구성            → renderer/core·storage/core 멤버화 + crates/ + wasm/native 빌드.  wave 0  rust
MG0.2a op enum + apply 포트            → TS renderPatch.ts apply를 scene-core로, golden-vector 동치성.  wave 0  rust+test
MG0.2b LWW + 검증 포트                 → per-property LWW + group cycle/target/bounds.  wave 1  rust+test
MG0.2c fractional index               → z-order 생성/비교(정수 zIndex 대체 준비).  wave 1  rust+test
MG0.2d wire serde(O3)                  → hello/ops/ack/patch/resume + region/presence.  wave 1  rust
MG0.2e command 카탈로그               → command_catalog() 데이터.  wave 1  rust
MG0.2f 템플릿 도메인 포트              → recipe+apply_template+builtin seed → scene-core.  wave 1  rust
MG0.2g canvas 모델(PC9)               → scene-core 모델에 canvas(=문서) 개념 + 객체의 canvasId.  wave 0  rust
MG0.3  renderer-web → scene-core 의존  → op-apply는 scene-core 호출.  wave 1  rust
MG0.4  TS apply 대체 경로 명시         → renderPatch.ts apply/schema 대체 경로(컷오버 MG-7).  wave 1  rust+bridge
```
- **Verify:** scene-core wasm32+native 컴파일(no time/rng/thread) + TS 대비 golden-vector 동치. cargo test.

#### Phase MG-1 — Storage core 정비 + sqlite + wasm (status-reconciled)
> Memory/File·번들·테스트 기구현. 신규=sqlite + wasm 타깃 + region 질의.
```
MG1.1  workspace 편입 + 시그니처 확인  → 멤버화·빌드 연결. Record CRUD/stream 확인(대부분 done).  wave 1  rust
MG1.2  sqlite 어댑터(O2)               → per-record I/O+커서+bounded ingest+ACID. stub 대체 + integrity suite.  wave 2  rust+test
MG1.2w storage-core wasm 빌드(C11)     → wasm32 타깃 + **wasm sqlite(OPFS VFS)** 어댑터. native sqlite와 동일 API.  wave 3  rust+test
MG1.6  region/spatial 질의(PC10)       → canvasId+bbox region 질의 + windowed read.  wave 4  rust+test
MG1.3  포맷 분리(D1/D5) 문서화         → 교환 번들 vs at-rest.  wave 2  rust(doc)
MG1.4  번들 원자성 검증/보강           → stage-and-swap 충족 검증 + 중간 실패 intact 회귀.  wave 2  rust+test
MG1.5  평문 어댑터 검증/보강(D3/D6/D7)  → FileAdapter canonicalize/lint·envelope 검증 + 거부 보강.  wave 3  rust+test
```
- **Verify:** {memory,sqlite,plaintext} 크로스 round-trip; **클라 wasm sqlite(OPFS) round-trip**; region 질의 정확성.

#### Phase MG-2 — Rust server (axum/tokio) + canvas 액터
```
MG2.1  server 골격                    → axum+tokio, 정적 호스팅, 헬스/레디니스.  wave 2  rust
MG2.2  canvas 단위 액터               → canvas당 tokio task + mpsc·broadcast + **bounded working set**(PC10). scene-core 링크. (verify: 생성/idle/evict/종료 라이프사이클)  wave 3  rust
MG2.3  MCP on Rust(C12)               → [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)로 tools/clients/trace 구현. 작업자가 레포 보고 구현.  wave 4  rust
MG2.4  storage-core 서버 임베드        → 액터가 storage-core(native sqlite)로 Record CRUD+저널+region 질의.  wave 4  rust
```

#### Phase MG-3 — WebSocket 전송 (논리 2채널)
```
MG3.1  WS 서버 종단                   → axum WS, reliable_ordered+ephemeral_besteffort 한 소켓.  wave 5  rust
MG3.2  전송 클라이언트(셸)            → WS 연결/논리 채널, abstraction 뒤. api.ts(HTTP) 대체 시작.  wave 5  svelte
MG3.3  wire 배선                      → MG0.2d 타입으로 hello/welcome/ops/ack/patch/resume+region+presence.  wave 5  rust
```

#### Phase MG-4 — Server-authoritative sync 엔진 (콜라보 가능형)
```
MG4.1  서버 LWW+seq+저널/체크포인트    → 수신순 seq, per-property LWW, 저널+체크포인트, 재시작 복구.  wave 6  rust
MG4.2  granular op+opId+dedup          → op 모델(propDelta/actor=userId/ts) + idempotent dedup.  wave 6  rust
MG4.3  클라 durable outbox(C11)        → **storage-core(wasm sqlite)** 에 append→전송→ack→제거, 종료 후 replay.  wave 6  svelte+rust
MG4.4  optimistic+unacked discard      → 로컬 즉시 적용, 미승인 property 원격 무시.  wave 7  svelte+rust
MG4.5  coalescing+재접속 reconcile     → ~33ms throttle; 재접속 snapshot+reapply+outbox replay.  wave 7  svelte
MG4.6  fractional index 적용           → MG0.2c를 zIndex 대체로 배선, 동시삽입 서버 유일위치.  wave 7  rust
```
- **Verify:** 단일 사용자 렉 0, 강제 종료 후 재접속 미승인 편집 유실 0, 중복 replay 무영향.

#### Phase MG-5 — Canonical scene store 운용 (이관 없음)
```
MG5.1  scene→Record 매핑(O2/O3)        → canvas/group/node/edge/tag/comment/artifact → kind별 Record, version=LWW.  wave 7  rust
MG5.2a kind별 apply 동치성             → kind별 매핑·apply가 현 storage.ts 의미와 동치임을 테스트 게이트.  wave 8  rust+test
MG5.2b 서버 scene 권한 경로           → 전 편집이 WS→canvas 액터→scene-core→storage-core(region) 경유.  wave 8  rust
MG5.3  클라 전송 클라이언트 완성       → scene 로드/저장/선택을 전송 클라이언트로 일원화(api.ts 제거 대상화). **데이터 이관 없음 — 빈 상태에서 새로 생성.**  wave 8  svelte
```

#### Phase MG-6 — Realtime server→client (멀티유저, 컷오버 후 최종 tail)
```
MG6.1  op broadcast+원격 LWW          → 액터가 적용 op를 seq 달아 (region 구독자에) broadcast, 클라 LWW 적용.  wave 12 rust+svelte
MG6.2  presence/커서(ephemeral 채널)   → 비영속 latest-wins, MCP companion/follow 통합.  wave 12 svelte+rust
MG6.3  권한 강화(C13 위)              → userId 기반 읽기/쓰기 게이트(인증은 여전히 TODO).  wave 13 rust
MG6.4  멀티유저 회귀                  → 동시 편집 LWW 수렴, 드래그 중 원격 무시, region 구독 정합.  wave 13 test
```

#### Phase MG-7 — Decommission & 클린 컷오버 (MG-6보다 먼저)
```
MG7.1  컷오버 검증(회귀 스위트)        → 측정 가능 전기능: canvas CRUD/전환, scene CRUD, 템플릿, MCP trace, export, companion, 단축키, 툴/마퀴, 대용량 windowing. 신스택 단독 통과.  wave 9  rust+svelte+test
MG7.2  Node 서버 제거                 → src/server/{index,storage,mcp,mcpClients,local}.ts 삭제.  wave 10 cleanup
MG7.3  TS 전송·op-apply 제거          → api.ts, renderPatch.ts apply, scene-core 대체 src/shared/* 정리.  wave 10 cleanup
MG7.4  의존성/스크립트/설정 + 참조0    → package.json(fastify/@fastify/static/sql.js/@types/sql.js/@modelcontextprotocol/sdk/tsx/concurrently)+dev:server/mcp/start+tsconfig.server.json+dist-server. **grep 참조 0 증명.** 기존 `.local` 폐기.  wave 10 cleanup+test
MG7.5  제거 후 전체 회귀              → cargo test(workspace)+wasm build+vitest(잔존)+e2e+README/CLAUDE 갱신.  wave 11 test
```

#### Phase MG-8 — Operations & Scale-out (stateless)
```
MG8.1  coordination 인터페이스         → trait(리스/디렉터리/presence/pubsub)+InMemory(dev).  wave 6  rust
MG8.2a canvas 리스                    → acquire/renew/release+TTL, single-writer.  wave 7  rust
MG8.2b 동적 배치 라우팅(O15)          → canvasId owner 룩업(per-conn, 인메모리 캐시→핫패스 홉 0) + 단순 TCP LB.  wave 7  rust
MG8.3  graceful shutdown/drain         → 신규 거부→flush+체크포인트→draining→리스 해제→drain.  wave 8  rust
MG8.4  클라 재연결+미연결 대기         → 백오프+jitter, outbox 버퍼링+로컬 적용+UI offline, 재연결 replay.  wave 8  svelte
MG8.5  세션 handoff 회귀              → owner kill→타 인스턴스(프로세스) 인수 무손실.  wave 9  test
MG8.6  File coordination               → 파일 락(단일 호스트 멀티프로세스 dev).  wave 10 rust
MG8.7  (deferred) Redis coordination   → pod 확장.  deferred  rust
MG8.8  (deferred) WebTransport/gRPC(O14)→ 어댑터+채널 매핑+QUIC LB.  deferred  rust+svelte
```

#### Phase MG-9 — Multi-canvas & Large-canvas windowing (PC9/PC10)
```
MG9.1  canvas CRUD + 액터/리스 키       → canvas 생성/삭제/목록, 액터·리스·라우팅 키=canvasId. (MG0.2g 모델 의존)  wave 4  rust
MG9.2  canvas 전환 UI(셸)             → canvas 목록/선택/생성. 전환 시 region 구독 재설정.  wave 6  svelte
MG9.3  서버 bounded working set         → 액터가 활성 region working set만 메모리, cold LRU eviction(storage backing).  wave 7  rust
MG9.4  클라 windowed replica           → 뷰포트(+margin) region 구독, 들어옴 로드/멀어짐 evict. 렌더 culling과 별개 데이터 레이어.  wave 7  svelte+rust
MG9.5  region-scoped subscription       → subscribe{canvasId,region} → region 스냅샷+이후 op만. 카메라 이동 시 재구독.  wave 7  rust+svelte
MG9.6  대용량 windowing 회귀           → 메모리 상한 내 거대 canvas 편집·팬/줌·region 경계 정합.  wave 8  test
```
- **Verify:** canvas 다중 운용, 메모리 상한 내 대용량 canvas 동작(서버·클라), region 전환 시 데이터 정합·유실 0.

### Track CC — Canvas Cockpit (병렬; 신코어 타깃)
```
CC0.1 active-tool(scene-core) wave1 rust | CC0.2 command 카탈로그 배선 wave1 rust | CC0.5 marquee/pick 브리지 wave1 bridge
CC0.4 셸 단축키 dispatcher wave2 svelte | CC0.3 insert-primitive op(scene-core) wave2 rust
CC1.1 하단 리모콘 스캐폴드 wave3 svelte | CC1.2 줌/핏/풀스크린 이관 wave3 svelte
CC1.3 ShapePalette 제거+라우팅 wave4 svelte | CC1.4 스타일+툴 커서 wave4 svelte
CC2.1 tool-aware pointer-down(core) wave4 rust | CC2.2 Marquee state+기하(node+group) wave4 rust
CC2.3 marquee→기존 multiSelectIds 병합 wave5 bridge+svelte | CC2.4 마퀴 오버레이+Space-hold wave5 svelte | CC2.5 툴/마퀴 테스트 wave5 test
CC3.1 recipe_from_selection(단일/멀티/그룹) wave5 rust | CC3.2 템플릿 영속(Record kind=template, tombstone+seed-guard, depends MG2.4) wave6 rust
CC3.3 라이브러리 팝오버+등록 wave6 svelte | CC3.4 템플릿 테스트(3종 round-trip/삭제 영속/seed 멱등) wave7 test
CC4.1 우클릭 pick(core) wave5 rust | CC4.2 컨텍스트 메뉴 컴포넌트 wave6 svelte | CC4.3 컨텍스트별 액션 배선 wave6 svelte
CC5.1 설정 오버레이(O7, 모달) wave6 svelte | CC5.2 향후 편집 대비 구조(binding/command 분리) wave6 rust(doc)
CC6.1 단축키 광범위 부착(체크리스트 verify) wave7 rust+svelte | CC6.2 orphan 정리 wave7 test
```

---

## Dependency / Wave Summary

```
wave 0 : MG0.1 MG0.2a MG0.2g | 게이트 O2 O3
wave 1 : MG0.2b MG0.2c MG0.2d MG0.2e MG0.2f MG0.3 MG0.4 MG1.1 | CC0.1 CC0.2 CC0.5
wave 2 : MG1.2 MG1.3 MG1.4 MG2.1 | CC0.3 CC0.4
wave 3 : MG1.2w MG1.5 MG2.2 | CC1.1 CC1.2
wave 4 : MG1.6 MG2.3 MG2.4 MG9.1 | CC1.3 CC1.4 CC2.1 CC2.2
wave 5 : MG3.1 MG3.2 MG3.3 | CC2.3 CC2.4 CC2.5 CC3.1 CC4.1
wave 6 : MG4.1 MG4.2 MG4.3 MG8.1 MG9.2 | CC3.2 CC3.3 CC4.2 CC4.3 CC5.1 CC5.2
wave 7 : MG4.4 MG4.5 MG4.6 MG5.1 MG8.2a MG8.2b MG9.3 MG9.4 MG9.5 | CC3.4 CC6.1 CC6.2
wave 8 : MG5.2a MG5.2b MG5.3 MG8.3 MG8.4 MG9.6
wave 9 : MG7.1 MG8.5
wave 10: MG7.2 MG7.3 MG7.4 MG8.6
wave 11: MG7.5
wave 12: MG6.1 MG6.2
wave 13: MG6.3 MG6.4
deferred: MG8.7(Redis) MG8.8(WebTransport/gRPC)
```

핵심: **MG0.2a(op-apply 포트)·MG0.2g(canvas 모델)** 키스톤. 게이트 O2/O3 wave 0. storage **wasm sqlite(MG1.2w)** 로 클라 outbox/replica 통일. **canvas 단위 액터·리스(MG9.1)·windowing(MG9.3~5)** 로 대용량 분산. 템플릿(CC3.2)은 MG2.4 의존(wave 6, 1급 기능). **컷오버·제거(MG-7, wave 9~11)가 멀티유저(MG-6, 12~13)보다 먼저.**

## Verification Strategy

- **Rust:** crate별 cargo test(workspace). scene-core wasm32+native + TS golden-vector 동치. storage-core **native+wasm sqlite** round-trip. region 질의.
- **전송/sync:** WS e2e(op→ack/스냅샷/재접속/region 재구독), durable outbox(종료→유실 0→멱등), LWW 수렴, coalescing.
- **운영:** kill/drain handoff 무손실, 미연결→재연결 유실 0, scale-out 시 기존 canvas 재배치 없음.
- **대용량(PC10):** 메모리 상한 내 거대 canvas 서버·클라 동작, region 경계 정합.
- **컷오버:** MG7.1 전기능 회귀 + MG7.4 참조 0 → MG7.5 전체 회귀.
- **불변식:** 단일 앵커 selection, gesture gating.

## Done Criteria

- **Cockpit:** 하단 리모콘 / move·hand+마퀴(node+group) / 템플릿(단일/멀티/그룹 등록+빌트인 삭제 영속) / 상황별 우클릭 / 설정 readonly 단축키 + 대부분 기능 단축키.
- **Migration:** 백엔드 Rust 단독, scene/템플릿 storage core(sqlite) 권한 저장, 전송 WS(교체 가능), client/server scene-core+storage-core 공유, **Node/Fastify/sql.js/HTTP api/TS op-apply + 기존 데이터 제거**.
- **Canvas/Scale:** 다중 canvas, **대용량 canvas가 서버·클라 메모리 상한 내 동작(windowing)**; stateless 인스턴스 + canvas 리스/동적 배치 + graceful shutdown + 무손실 handoff; userId 구분(인증 TODO); (MG-6, 컷오버 후) 멀티유저 LWW 수렴.

## Prior Art / References

- Figma — [multiplayer](https://www.figma.com/blog/how-figmas-multiplayer-technology-works/), [ordered sequences](https://www.figma.com/blog/realtime-editing-of-ordered-sequences/)(fractional), [reliable](https://www.figma.com/blog/making-multiplayer-more-reliable/)(저널/체크포인트/lock UUID), [Rust](https://www.figma.com/blog/rust-in-production-at-figma/), [LiveGraph](https://www.figma.com/blog/livegraph-real-time-data-fetching-at-figma/).
- 충돌 — Shapiro [CRDTs](https://inria.hal.science/inria-00609399v2/document), Kleppmann [Hard Parts](https://martin.kleppmann.com/2020/07/06/crdt-hard-parts-hydra.html), [Fluid TOB](https://fluidframework.com/docs/concepts/tob).
- Sync — [Replicache](https://doc.replicache.dev/concepts/how-it-works), PowerSync, InstantDB, tldraw sync-core, [Liveblocks](https://liveblocks.io/blog/understanding-sync-engines-how-figma-linear-and-google-docs-work).
- 동시성/배치 — Pingora, Discord Go→Rust, Tokio; virtual-actor — Orleans/Akka/Dapr.
- MCP — [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk).

## Next step

게이트 O2(sqlite)·O3(JSON) 잠금 완료 → **MG0.1/MG0.2a(op-apply 포트)·MG0.2g(canvas 모델)** 키스톤부터. 병렬로 CC0·MG-1(정비)·MG8.1·MG9.1. 실제 착수는 `sdd` 스킬로 브랜치 분리 후.
