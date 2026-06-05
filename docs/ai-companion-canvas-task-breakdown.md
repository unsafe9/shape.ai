# AI Companion Canvas Task Breakdown

> Source: product direction discussion on 2026-06-05, plus [Infinite Canvas Engine Strategy](./infinite-canvas-engine-strategy.md), [Infinite Canvas Engine Task Breakdown](./infinite-canvas-engine-task-breakdown.md), and [Rust Canvas Replacement Task](./rust-canvas-replacement-task.md).

이 문서는 shape.ai를 "AI coding agent를 위한 local-first Figma-like infinite canvas workspace"로 확장하기 위한 추가 아이디어를 실행 가능한 작업 리스트로 정리한다.

## Objective

shape.ai를 특정 엔지니어링 diagram editor가 아니라 범용 infinite canvas workspace로 확장한다. 사용자는 Figma처럼 캔버스 위에 텍스트, 도형, 간선, 프레임, note, todo, wiki, ADR, 서버 구조도, presentation outline 같은 작업 객체를 직접 배치하고 편집할 수 있어야 한다.

MCP client는 local canvas state를 원격 MCP 경로로 읽고 조작할 수 있다. MCP client마다 companion icon이 생기고, tool call이 실제로 읽거나 쓰는 canvas 위치로 이동하면서 현재 작업을 시각적으로 보여준다. 사용자는 icon을 클릭해 spectator/follow mode로 agent 작업을 관전할 수 있다.

Canvas engine은 web 전용 구현이 아니라 `core`, `web`, `metal` 같은 port boundary를 가진다. 장기적으로 web, macOS, iOS native app이 같은 scene/core model을 공유하고, 플랫폼별 shell과 renderer adapter만 갈아끼울 수 있어야 한다.

## Locked Decisions

- 제품 감각은 Figma-like local canvas다. Figma의 full vector design tool을 복제하지 않고, direct manipulation, object comments, plugin/API 조작 모델을 가져온다.
- Canvas object는 엔지니어링 전용 컴포넌트가 아니라 범용 primitive 위에 올라간다.
- 기본 primitive는 도형, 텍스트, 간선, 프레임/그룹, 이미지 또는 artifact preview, marker/comment처럼 여러 템플릿이 공유할 수 있는 단위여야 한다.
- Todo, wiki note, ADR, 서버 구조도, presentation 같은 상위 템플릿은 특수 렌더러가 아니라 primitive 조합으로 만든다.
- 기존 Shape/project 개념과 구조화된 ADR 전용 컴포넌트는 canonical canvas primitive가 아니다. 이들은 migration input이거나 template/export preset으로 흡수되어야 하며, 기본 편집 모델을 특정 엔지니어링 문서 구조에 묶지 않는다.
- Multi-select, grouping/ungrouping, labeling/tagging은 상위 템플릿 기능이 아니라 기본 canvas editing capability다.
- Group은 project/task/decision 같은 고정 계층 타입이 아니다. 사용자가 원하는 구조를 만들기 위한 자유로운 containment/labeling primitive이며, nested grouping은 특정 방법론을 강제하지 않고 임의의 의미 계층을 만들 수 있게 해야 한다.
- 템플릿에 필요한 기본 primitive가 없다면 템플릿을 special-case하지 말고 primitive나 shared component를 추가한다.
- Zoom in/out에서 객체 정체성을 바꾸는 semantic replacement를 기본 UX로 삼지 않는다.
- 좋은 LOD는 visual degradation이다. 같은 객체가 같은 위치와 stable id를 유지하되, 멀어질수록 text, shadow, detail, badge, handle, edge label 같은 표현만 점진적으로 줄인다.
- 무한한 데이터는 화면에 항상 full detail로 올라오는 것이 아니라 viewport, zoom, 관심도, cache level에 따라 stream/render된다.
- MCP companion tracker는 장식이 아니라 agent operation observability다. 어떤 client가 무엇을 읽고, 어디를 수정하고, 어떤 artifact를 만들었는지 canvas 위에 trace로 보여야 한다.
- Realtime remote collaboration은 현재 scope가 아니다. 이 계획은 local-first single-user workspace를 기준으로 한다.
- Realtime collaboration을 지금 구현하지 않더라도, scene mutation은 나중에 remote collaboration으로 확장 가능한 형태여야 한다. Canonical write path는 stable object id, granular operation, actor metadata, targetIds, timestamp, base revision을 남기고, raw scene blob overwrite를 기본 편집 모델로 삼지 않는다.
- Document state와 ephemeral activity state를 분리한다. Object geometry/text/style/comment/export/proposal은 document state이고, viewport, hover, active tool, follow mode, companion animation, transient read cursor는 ephemeral state다.
- 현재 production code를 바로 `apps/`나 `canvas/`로 이동하지 않는다. 우선 구조 placeholder와 task graph만 만든다.

## Initial Repo Scaffold

이번 planning pass에서 다음 placeholder 구조만 둔다.

```text
apps/
  web/.gitkeep
  macos/.gitkeep
canvas/
  core/.gitkeep
  web/.gitkeep
  metal/.gitkeep
```

의미:

- `apps/web`: 현재 web product shell의 미래 위치.
- `apps/macos`: macOS native shell의 미래 위치.
- `canvas/core`: platform-neutral scene model, editing operation, layout/render contract의 미래 위치.
- `canvas/web`: WASM/WebGPU web adapter의 미래 위치.
- `canvas/metal`: macOS/iOS native Metal adapter의 미래 위치.

이 구조는 implementation placeholder다. 현재 `src/`와 `poc/` 코드는 이 문서만으로 이동하지 않는다.

## Done Criteria

이 방향의 foundation이 잡혔다고 볼 수 있는 상태:

- Canvas package boundary가 web, macOS, iOS를 모두 설명할 수 있다.
- Scene/core model이 primitive와 semantic template을 분리한다.
- Web UI는 엔지니어링 전용 node editor가 아니라 범용 canvas primitive editor로 동작한다.
- 최소 Figma-like 편집 기능이 있다: select, multi-select, move, resize, text edit, shape create, edge create, group/ungroup, nested group, frame/freeform group hull, z-order, align/snap, label/tag, comment.
- 기존 Shape/project와 ADR 전용 컴포넌트는 primitive layer에서 제거되거나 migration/template/export layer로 내려간다.
- Todo, wiki note, ADR/design, server architecture diagram, presentation outline 템플릿이 primitive 조합으로 생성된다.
- Good LOD policy가 구현되어 대량 객체에서도 객체 정체성을 유지하면서 안정적인 pan/zoom 성능을 낸다.
- MCP client별 companion icon, dock, active animation, click-to-follow, spectator mode, operation trace가 작동한다.
- Human edit와 AI/MCP write는 actor, target, operation, diff/proposal, timestamp, base revision을 남긴다.
- Canonical document mutation과 viewport/follow/hover 같은 ephemeral UI state가 분리되어 있다.
- 기존 MCP/API/export/comment semantics가 새 primitive/template model 위에서도 유지된다.

## Explicit Non-Goals

- Figma 수준의 path editing, boolean geometry, auto-layout, component system 전체를 구현하지 않는다.
- 파일시스템 저장 계층을 대체하지 않는다. shape.ai는 SQLite, files, Git, docs, artifacts, MCP source 위의 spatial working interface다.
- Shape/project 또는 structured ADR을 기본 object model로 유지하지 않는다.
- 모든 데이터를 항상 메모리에 올리거나 full detail로 렌더링하지 않는다.
- Todo, wiki, slides, architecture diagram을 각각 별도 앱으로 만든다는 뜻이 아니다.
- Realtime remote collaboration, remote teammate cursor, simultaneous co-editing은 현재 계획에 포함하지 않는다.
- CRDT/Yjs, realtime transport, remote user auth, conflict resolution UI, remote cursor rendering은 현재 계획에 포함하지 않는다.
- macOS/iOS app을 지금 구현하지 않는다.
- 기존 `src/` production app을 이 planning pass에서 이동하지 않는다.

## Phase Graph

```text
P0 Product And Package Contract
  -> P1 Portable Canvas Core Boundary
  -> P2 Universal Primitive Editing Slice
  -> D1 Primitive Sufficiency Review
  -> P3 Good LOD And Infinite Object Performance
  -> P4 Template System On Primitives
  -> P5 MCP Companion Activity Tracker
  -> P6 Product Integration And Migration Plan
  -> D2 AI Companion Canvas Direction Review
```

P2, P3, and P5 can run in parallel after P1 defines stable contracts. P4 depends on enough primitive coverage from P2.

## Phase P0: Product And Package Contract

Goal: Lock the vocabulary and package direction before implementation work fans out.

Why now: Without a shared contract, canvas engine, template model, MCP activity tracker, and future native shells will pull the repo in different directions.

Tasks: T0.1, T0.2, T0.3

Verify or evaluate:

- A reviewer can explain what belongs in app shell, canvas core, platform adapter, semantic template, and MCP activity tracker.
- Placeholder directories exist but contain no implementation.
- Existing `src/` and `poc/` code remains unmoved.

Review gate:

- `human-decision`: approve the package boundary names before code moves into them.

### T0.1 Define Product Object Vocabulary

Outcome: Define the stable object vocabulary for the universal canvas.

Source refs:

- This document's Objective and Locked Decisions.
- `docs/infinite-canvas-engine-strategy.md` responsibility boundary.
- `docs/rust-canvas-replacement-task.md` Rust/Web/WebGPU ownership target.

Deliverable:

- A short design note or section update that separates primitive objects from semantic templates.
- Object vocabulary covering shape, text, edge, frame/group, image/artifact preview, comment, marker, and actor activity.

Verify:

- Todo/wiki/ADR/server-architecture/presentation can be described without inventing template-specific renderer objects.

Depends on: none

Parallel wave: A

Stop or ask if:

- A required object would force a full Figma-like vector editor or browser CSS layout engine.

### T0.2 Confirm Package And App Boundaries

Outcome: Confirm the future repo boundary before moving code.

Deliverable:

- Boundary note for `canvas/core`, `canvas/web`, `canvas/metal`, `apps/web`, and `apps/macos`.
- Explicit migration rule: current `src/` and `poc/` stay where they are until a later migration phase.

Verify:

- The boundary supports a future iOS native app without putting product business logic inside the renderer adapter.

Depends on: none

Parallel wave: A

Stop or ask if:

- The desired native target changes from shared core plus native adapter to separate native reimplementation.

### T0.3 Add Placeholder Structure

Outcome: Reserve the future package/app directories without starting implementation.

Deliverable:

- `.gitkeep` files under `apps/web`, `apps/macos`, `canvas/core`, `canvas/web`, and `canvas/metal`.

Verify:

- `git status --short` shows only placeholder files for the new directories.
- No build config imports these directories yet.

Depends on: none

Parallel wave: A

Stop or ask if:

- Existing repo tooling treats empty package directories as packages automatically.

## Phase P1: Portable Canvas Core Boundary

Goal: Make the canvas engine architecture portable across web, macOS, and future iOS.

Why now: The Rust/WebGPU replacement plan should not accidentally hard-code web-only assumptions into the core scene model.

Tasks: T1.1, T1.2, T1.3, T1.4

Verify or evaluate:

- Core has no browser DOM assumption.
- Web and Metal adapters can share scene snapshots, operations, hit testing contracts, and LOD policy.
- Product shell retains API/MCP/business state.

Review gate:

- `human-decision`: approve the core/adapter split before production migration starts.

### T1.1 Define Platform-Neutral Core Contract

Outcome: Specify what `canvas/core` owns.

Deliverable:

- Core contract covering scene object identity, geometry, bounds, camera, hit testing, operation application, selection geometry, render primitive stream, and debug stats.

Verify:

- The contract can be consumed by both WASM/WebGPU and Metal adapters.

Depends on: P0 review gate

Parallel wave: B

Stop or ask if:

- The contract starts absorbing MCP/API/export/business semantics.

### T1.2 Define Web Adapter Contract

Outcome: Specify what `canvas/web` owns.

Deliverable:

- Web adapter contract for WASM loading, WebGPU surface, pointer/keyboard bridge, DOM text overlay, browser clipboard/IME, diagnostics drawer bridge, and shell integration.

Verify:

- The adapter does not define product templates or canonical persistence rules.

Depends on: P0 review gate

Parallel wave: B

Stop or ask if:

- Web-only behavior becomes required by the core scene model.

### T1.3 Define Metal Adapter Contract

Outcome: Specify what `canvas/metal` owns for macOS/iOS.

Deliverable:

- Metal adapter contract for native surface lifecycle, GPU pipeline, native text input bridge, pointer/touch/keyboard bridge, display scale, and platform diagnostics.

Verify:

- The same core scene and operation stream can drive Metal rendering.

Depends on: P0 review gate

Parallel wave: B

Stop or ask if:

- A native feature would require forking the semantic scene model.

### T1.4 Define App Shell Boundaries

Outcome: Separate product shell responsibility from canvas engine responsibility.

Deliverable:

- App shell contract for `apps/web` and `apps/macos`: account/session if needed, MCP connection state, persistence/API, panels, inspector, template picker, export drawer, companion dock, and spectator controls.

Verify:

- The app shell can be replaced per platform while canvas/core remains stable.

Depends on: P0 review gate

Parallel wave: B

Stop or ask if:

- Product business logic needs to move into renderer adapters to make a feature work.

## Phase P2: Universal Primitive Editing Slice

Goal: Replace engineering-only frontend assumptions with a small but general set of canvas primitives and direct-editing operations.

Why now: Templates must be composed from real primitives. If primitives are too narrow, every template becomes a special case.

Tasks: T2.1, T2.2, T2.3, T2.4, T2.5

Verify or evaluate:

- A user can draw, multi-select, label, connect, group, arrange, and annotate simple objects without choosing a specialized template.
- Grouping, adding objects to a group, removing objects from a group, nested grouping, and ungrouping feel direct and reversible.
- Primitive operations are actor-tracked and serializable.

Review gate:

- `D1 Primitive Sufficiency Review`: verify that planned primitives can build the first template set.

### T2.1 Primitive Object Model

Outcome: Extend the scene model conceptually from engineering nodes to universal primitives.

Deliverable:

- Primitive model for shape, text note, edge/connector, frame/group, image/artifact preview, comment marker, and actor marker.
- Mapping from current `Group`, `Node`, `Edge`, `Tag`, `Comment`, and `Artifact` to the new primitive vocabulary.
- Migration treatment for the old Shape/project concept and structured ADR-specific components: remove them from the primitive layer, map existing data to frames/groups, text/cards, labels/tags, comments, artifacts, and export presets.

Verify:

- Current group/node/edge workflows can still be represented.
- Plain note, box-and-arrow diagram, and slide-like frame can be represented without special template fields.
- ADR/design content can be represented as primitive canvas objects plus template/export metadata, not a dedicated core component.

Depends on: P1

Parallel wave: C

Stop or ask if:

- Existing canonical schema must be broken instead of evolved.

### T2.2 Minimal Figma-Like Editing Tools

Outcome: Provide the lowest useful set of direct manipulation tools.

Deliverable:

- Tool plan for select, pan, zoom, create shape, create text, create edge, frame/group, resize, rotate only if required, align/distribute, z-order, duplicate, copy/paste, comment, and delete.
- Multi-select plan for marquee selection, modifier-click selection, select all in frame/group, selection bounds, batch move/resize where safe, batch z-order, and batch delete.

Verify:

- A simple presentation slide, todo board, wiki note cluster, and architecture box diagram can be assembled manually.
- Multiple objects can be selected and moved together without creating a permanent group.

Depends on: T2.1

Parallel wave: D

Stop or ask if:

- The tool list drifts into full design-tool parity instead of minimum working canvas composition.

### T2.3 Text On Everything

Outcome: Treat labels as a shared capability, not a card-only feature.

Deliverable:

- Text attachment model for shapes, edges, frames, comments, and artifact previews.
- Editing behavior for direct text, edge labels, shape labels, and frame titles.

Verify:

- Korean, mixed text, multiline, no-space long text, and labels on edges/shapes pass the same text-quality gates as the renderer replacement plan.

Depends on: T2.1

Parallel wave: D

Stop or ask if:

- Label behavior requires a separate text engine per primitive type.

### T2.4 Grouping And Labeling Foundation

Outcome: Make grouping and labeling generic canvas operations rather than project/ADR-specific structure.

Deliverable:

- Group/ungroup operation model for temporary selection groups, persistent frames/groups, nested groups, group membership updates, and group-level transforms.
- Interaction model for intuitive group creation, adding new or existing objects into a group, pulling objects out of a group, moving objects between nested groups, and ungrouping without losing object identity.
- Group geometry model for rectangular frames and freeform group hulls that can hug member objects more closely than a rigid bounding box.
- Hull update behavior for smooth shrink/expand when members are added, moved, resized, or removed.
- Selection and hit-testing rules for entering a group, selecting the group vs selecting a member, selecting nested groups, and editing a member without accidental regrouping.
- Drag/drop and creation affordances: dropping an object into a group can attach it, creating inside an active group can include it, and dragging out can detach it when the gesture is clear.
- Label/tag model for object labels, group labels, tag chips, color labels, and template-provided suggested labels.
- Rules for how labels appear at different zoom levels without becoming semantic replacement LOD.
- Migration note for old project/Shape labels and ADR fields into generic labels, text fields, export metadata, or template metadata.

Verify:

- A mixed selection of shapes, notes, edges, comments, and artifact previews can be grouped, labeled, moved, ungrouped, and exported without changing object identity.
- Nested groups can be created and edited without requiring fixed project/task/decision semantics.
- Adding, creating, moving, and removing objects changes group membership predictably and can be undone.
- Freeform hulls visually track member geometry and animate shrink/expand without jarring jumps.
- Todo/wiki/ADR/server-diagram templates can use the same labeling model.
- Label/tag changes produce normal operation events.

Depends on: T2.1, T2.2, T2.3

Parallel wave: E

Stop or ask if:

- Grouping needs a private project model or ADR model to preserve meaning.
- Nested grouping starts enforcing a fixed hierarchy instead of staying a flexible canvas structure.
- Freeform hull behavior requires complex boolean/vector editing beyond group visualization and hit testing.
- Labels become a separate taxonomy system before the primitive labeling behavior is proven.

### T2.5 Collaboration-Ready Operation Model

Outcome: Make local direct edits and MCP edits use the same operation vocabulary while avoiding mutation choices that would make future realtime collaboration a rewrite.

Deliverable:

- Operation vocabulary for create, move, resize, edit text, connect, multi-select, group, ungroup, label, tag, comment, export, accept proposal, reject proposal, and delete.
- Required metadata: operationId, actorId, actorType, clientId, targetIds, timestamp, baseRevision or previousRevision, source tool call if any.
- Document-state boundary for canonical objects, text, geometry, style, comments, artifacts, exports, proposals, and accepted operations.
- Ephemeral-state boundary for viewport, hover, active tool, local selection focus, follow mode, companion icon animation, transient read cursor, and temporary trace effects.
- Local operation log or equivalent history model that can drive undo/redo, MCP trace, audit display, and future broadcast without treating raw snapshots as the primary edit unit.

Verify:

- Human edits and MCP writes both produce operation events without introducing remote collaboration behavior.
- No normal editor action mutates canonical scene state through an untracked blob overwrite.
- Undo/redo or recent-history inspection can be explained from operations, not only from full-scene snapshots.
- Ephemeral viewport/follow/hover state can be reset without changing persisted document state.

Depends on: T2.1, T2.4

Parallel wave: F

Stop or ask if:

- Operations become a raw scene blob patch without semantic target information.
- The implementation path requires adding CRDT, realtime transport, remote auth, or remote cursor UI now instead of keeping only collaboration-ready foundations.

## Decision D1: Primitive Sufficiency Review

Question: Can the first product templates be built from the primitive set without hidden special cases?

Approve if:

- Todo, wiki note, ADR/design, server architecture diagram, presentation outline, and investigation map all compose from shared primitives.
- ADR/design and old Shape/project content no longer require canonical project/ADR-specific components in the primitive layer.
- Missing needs are promoted to shared primitives or shared components.

Redirect if:

- Any template needs a private renderer, private object model, or revived project/ADR component before the shared primitive layer is usable.

## Phase P3: Good LOD And Infinite Object Performance

Goal: Render and navigate an effectively unbounded canvas without changing object identity during zoom.

Why now: The product only works as a visual filesystem if large amounts of heterogeneous data remain spatially explorable.

Tasks: T3.1, T3.2, T3.3, T3.4

Verify or evaluate:

- Large scenes keep stable object identity during zoom.
- Far objects degrade visually instead of becoming unrelated semantic replacements.
- Performance evidence includes frame time, memory, object count, text/cache stats, and interaction latency.

### T3.1 Good LOD Policy

Outcome: Define the allowed LOD transitions.

Deliverable:

- Policy for screen-size thresholds: full detail, compact detail, shape-only, density/overview, and minimap-like extreme zoom.
- Explicit rule that object id, position, bounds, selection identity, and hit-test identity stay stable.
- Group hull LOD policy: hulls may simplify visually at distance, but group membership, containment identity, and nested group readability must remain stable.

Verify:

- A reviewer can distinguish visual degradation from semantic replacement.
- A reviewer can zoom through nested/freeform groups without losing track of which objects belong together.

Depends on: P1

Parallel wave: C

Stop or ask if:

- Product direction changes to aggregate-first graph analytics.

### T3.2 Renderer Cache And Streaming Plan

Outcome: Plan how infinite data enters the viewport without full-detail rendering.

Deliverable:

- Strategy for spatial index, viewport query, tile/cache levels, text cache, glyph atlas, edge cache, density texture, prefetch radius, and memory budget.

Verify:

- The plan does not require all scene objects to be mounted as DOM or loaded at full detail.

Depends on: T3.1

Parallel wave: D

Stop or ask if:

- SQLite/API query shape cannot support spatial paging and zoom-aware fetches.

### T3.3 Benchmark Fixtures For Heterogeneous Work Objects

Outcome: Extend benchmark coverage beyond engineering node graphs.

Deliverable:

- Fixture plan for mixed notes, shapes, edges, frames, todos, wiki cards, slide frames, architecture diagrams, comments, artifacts, and active agent markers.

Verify:

- Benchmarks include at least the existing 1k+ card/edge scale plus heterogeneous object mixes.

Depends on: T2.1, T3.1

Parallel wave: E

Stop or ask if:

- Benchmark data becomes too synthetic to reflect real workspaces.

### T3.4 Interaction Under Load

Outcome: Preserve direct manipulation and spectator tracking in large scenes.

Deliverable:

- Test/evaluation plan for pan, zoom, select, drag, text edit overlay, follow agent, and operation trace under large-scene load.

Verify:

- Interaction latency remains within the approved benchmark target while visual detail degrades gracefully.

Depends on: T3.2, T3.3

Parallel wave: F

Stop or ask if:

- Follow/spectator animation competes with editor input responsiveness.

## Phase P4: Template System On Primitives

Goal: Build common AI-agent work surfaces as templates over shared primitives.

Why now: Templates prove the universal canvas model without turning shape.ai into many separate apps.

Tasks: T4.1, T4.2, T4.3, T4.4, T4.5

Verify or evaluate:

- Templates create editable primitive groups, not opaque widgets.
- Users can break apart, modify, connect, and export template content.

### T4.1 Template Contract

Outcome: Define templates as recipes over primitives and operations.

Deliverable:

- Contract for template metadata, primitive recipe, default layout, allowed exports, suggested tags, and optional AI prompt hints.

Verify:

- Applying a template results in normal canvas objects.

Depends on: D1

Parallel wave: G

Stop or ask if:

- A template requires a private editing mode to be useful.

### T4.2 Todo And Task Board Template

Outcome: Create a todo/task planning surface from primitives.

Deliverable:

- Template plan using frames, cards, status labels, dependency edges, owner/priority badges, and comments.

Verify:

- Tasks can be edited manually and by MCP operations using the shared operation model.

Depends on: T4.1

Parallel wave: H

Stop or ask if:

- Todo ordering requires a separate list database instead of canvas operations.

### T4.3 Wiki Note And Idea Board Templates

Outcome: Create knowledge and ideation surfaces from primitives.

Deliverable:

- Template plan for wiki note clusters, idea boards, source/evidence cards, and reference edges.

Verify:

- Notes can be reorganized spatially without losing text, tags, comments, or source refs.

Depends on: T4.1

Parallel wave: H

Stop or ask if:

- Long-form document editing becomes the primary surface instead of canvas-backed note/detail editing.

### T4.4 ADR And Architecture Diagram Templates

Outcome: Create engineering design surfaces without hard-coding the whole app to engineering.

Deliverable:

- Template plan for ADR, decision map, server architecture diagram, dependency diagram, and investigation map as primitive recipes.
- Export preset plan for ADR formats that reads primitive text, labels, comments, edges, and artifacts instead of requiring structured ADR core components.

Verify:

- The same shape/text/edge/frame primitives carry the content; engineering semantics live in template metadata, labels, comments, and export rules.
- Existing structured ADR content has a migration path into primitive objects plus ADR export metadata.

Depends on: T4.1

Parallel wave: H

Stop or ask if:

- Architecture or ADR objects need behavior that should be promoted to a generic primitive.

### T4.5 Presentation Template

Outcome: Support lightweight presentation/storytelling composition from the same canvas.

Deliverable:

- Template plan for slide frames, title/body text, image/artifact preview, connectors, speaker-note cards, and export outline.

Verify:

- Users can build simple slide-like frames with the same editing tools used for shapes and notes.

Depends on: T4.1

Parallel wave: H

Stop or ask if:

- The feature drifts into a full PowerPoint clone instead of minimum presentation composition.

## Phase P5: MCP Companion Activity Tracker

Goal: Make MCP clients visible as agent companions that show what they read, write, and produce on the canvas.

Why now: This is the AI-native differentiator. It turns headless MCP tool calls into observable canvas activity.

Tasks: T5.1, T5.2, T5.3, T5.4

Verify or evaluate:

- Each connected MCP client has a visible identity.
- Active clients move to the real target location for reads/writes.
- Users can click an icon to inspect or follow the agent's current work.

### T5.1 MCP Client Identity And Dock

Outcome: Show connected MCP clients as companion icons.

Deliverable:

- Client identity model and dock behavior for idle, active, error, disconnected, and muted states.
- Visual rule for active "jumping" or motion state without distracting from editing.

Verify:

- Multiple clients are distinguishable by label/color/icon.

Depends on: P1

Parallel wave: C

Stop or ask if:

- MCP transport cannot provide stable client/session identity.

### T5.2 Tool Call To Canvas Target Mapping

Outcome: Resolve MCP operations to visible canvas targets.

Deliverable:

- Mapping for query_scene viewport, get_group, patch_scene, set_selection, add_comment, export_group, tag updates, and future template operations.
- Fallback behavior when a tool call has no concrete target.

Verify:

- Tool calls can move the companion marker to a group, object, selection, viewport, or artifact preview.

Depends on: T2.5

Parallel wave: G

Stop or ask if:

- Tool payloads lack enough target information to visualize intent.

### T5.3 Spectator And Follow Mode

Outcome: Let users observe one agent's canvas activity.

Deliverable:

- Interaction plan for click-to-follow, pinned follow, pause/resume, jump-to-current-target, recent trail, and handoff back to manual viewport control.

Verify:

- Follow mode never steals control unexpectedly while the user is editing.

Depends on: T5.1, T5.2

Parallel wave: E

Stop or ask if:

- Spectator mode conflicts with local direct manipulation.

### T5.4 Operation Trace And Write Preview

Outcome: Show what the agent did without making the canvas noisy.

Deliverable:

- Trace model for reading, writing, commenting, exporting, proposal creation, proposal acceptance/rejection, and errors.
- Ghost diff/write preview behavior for risky writes.

Verify:

- Users can answer "what did this agent just read or change?" from the canvas and activity trail.

Depends on: T5.2

Parallel wave: E

Stop or ask if:

- Trace events become permanent visual clutter instead of inspectable history.

## Phase P6: Product Integration And Migration Plan

Goal: Decide how the universal canvas, templates, and companion activity tracker become production without disrupting the current replacement work.

Why now: Existing work already has a Rust canvas replacement track. This phase prevents duplicate or conflicting migrations.

Tasks: T6.1, T6.2, T6.3

Verify or evaluate:

- The plan explains how current `src/`, `poc/`, future `apps/`, and future `canvas/` fit together.
- No current production behavior is lost without an acceptance gate.

### T6.1 Current Model Compatibility Review

Outcome: Compare current `Scene` schema with the proposed primitive/template model.

Deliverable:

- Compatibility report: evolve in place, add a primitive layer, or migrate to a new canonical object model.
- Migration note for operation metadata, base revisions, actor records, and document-vs-ephemeral state boundaries in local SQLite.
- Migration note for removing/demoting old Shape/project and structured ADR components into generic frames/groups, labels, primitive objects, template metadata, and export presets.

Verify:

- Existing MCP/API/export/comment flows have an explicit compatibility path.
- Current local-first behavior remains sufficient without realtime collaboration infrastructure.
- No compatibility path keeps project/Shape or ADR-specific structures as privileged canvas primitives.

Depends on: P2, P4, P5

Parallel wave: I

Stop or ask if:

- Migration would break existing local SQLite data without a recovery path.

### T6.2 Production Migration Sequence

Outcome: Define the order for moving from current app layout to future package layout.

Deliverable:

- Migration sequence for promoting validated POC/engine code into `canvas/` and moving app shell toward `apps/web` only after contracts are stable.

Verify:

- Build/test commands remain available throughout the migration.

Depends on: T6.1

Parallel wave: J

Stop or ask if:

- The migration needs a repo/package manager change beyond the approved scope.

### T6.3 Final Direction Review

Outcome: Reconfirm the AI companion canvas product direction before broad implementation.

Deliverable:

- Review brief summarizing product scope, core architecture, primitive sufficiency, LOD evidence, template coverage, MCP activity tracker behavior, and remaining deferrals.

Verify:

- A reviewer can choose to approve implementation, split scope, or redirect product positioning.

Depends on: T6.2

Parallel wave: K

Stop or ask if:

- The product direction shifts back to a narrow engineering-only architecture canvas.

## Decision D2: AI Companion Canvas Direction Review

Question: Is the next implementation wave still the universal AI companion canvas, rather than a narrow engineering-only diagram editor or a generic design-tool clone?

Approve if:

- Package boundaries still support web, macOS, and future iOS.
- Primitive/template separation still covers the first templates without special-case renderers.
- Good LOD keeps object identity stable while making large scenes performant.
- MCP companion tracker still provides useful agent observability instead of only decorative animation.

Redirect if:

- The product should narrow back to engineering architecture work only.
- The product should prioritize full Figma-like vector editing over AI-agent work memory.
- The package split creates migration cost without improving portability.

## Waves

- Wave A: T0.1, T0.2, T0.3
- Wave B: T1.1, T1.2, T1.3, T1.4
- Wave C: T2.1, T3.1, T5.1
- Wave D: T2.2, T2.3, T3.2
- Wave E: T2.4, T3.3
- Wave F: T2.5, T3.4
- Wave G: T4.1, T5.2
- Wave H: T4.2, T4.3, T4.4, T4.5, T5.3, T5.4
- Wave I: T6.1
- Wave J: T6.2
- Wave K: T6.3

## Next Unblocked

- T0.1 Define Product Object Vocabulary
- T0.2 Confirm Package And App Boundaries
- T0.3 Add Placeholder Structure

## Deferred

- iOS app scaffold: defer until `canvas/metal` contract is approved and the macOS shell direction is clearer.
- Remote collaboration: out of current scope. Keep the operation model collaboration-ready, but revisit realtime transport, remote users, conflict handling, and remote cursors only after the local-first canvas, operation trace, and MCP companion tracker are stable.
- Full document editor, full presentation editor, full vector design editor: defer unless a primitive/template gap proves one of these is actually required.
