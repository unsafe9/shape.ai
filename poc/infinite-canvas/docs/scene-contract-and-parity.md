# Shape Scene Contract And Parity

## Contract

The POC render scene is defined in `poc/infinite-canvas/web/src/scene.ts`.

Renderer-owned fields:

- `SceneSnapshot.version`
- `sceneId`
- `CameraState`
- `RenderGroup`
- `RenderCard`
- `RenderEdge`
- `SceneStyleToken`
- current `SceneSelection`
- fixture/source metadata

App-owned fields excluded from the renderer:

- tag registry names/colors/descriptions
- comments
- artifacts
- export/proposal state
- MCP workflow state
- confidence/evidence refs/child decision ids
- edge rationale/confidence

The exclusion list is executable through `excludedBusinessFields()` in `poc/infinite-canvas/web/src/adapter.ts`.

## Adapter

`src/shared/renderScene.ts` owns the production-side render snapshot contract.

`shapeSceneToRenderSnapshot(scene)` maps:

- group -> frame
- node -> card
- edge -> route input
- selection -> renderer selection seed
- tag ids -> group filter/input metadata only

It does not map comments, artifacts, or export state.

`createShapeSceneFixture()` provides a deterministic app-level `Scene` for the POC harness. The harness can load it through the adapter without touching the production DOM/SVG canvas path.

## Patch Contract

POC patches:

- `create-group`
- `delete-group`
- `move-group`
- `move-card`
- `set-card-z-index`
- `edit-card-text`
- `create-card`
- `delete-card`
- `create-edge`
- `delete-edge`
- `select`

`validateScenePatch()` rejects unknown cards, duplicate cards, unknown groups, missing targets, invalid card bounds, and self-edges before the renderer snapshot accepts a mutation.

`applyRenderPatchToShapeScene()` translates accepted renderer patches back into app `ScenePatch` semantics:

- card drag -> updated node position and node selection
- group drag -> app `translateGroups`, moved group bounds, moved group nodes, and group selection
- z-order -> updated node `zIndex` and node selection
- text edit -> updated node title/summary/detail and node selection
- group create -> new app group with app-owned defaults and group selection
- group delete -> `removeGroupIds`, group node cleanup, incident `removeEdgeIds`, and canvas selection
- card create/duplicate/paste -> new app node with app-owned defaults and node selection
- card delete -> `removeNodeIds`, incident `removeEdgeIds`, and canvas selection
- edge create -> new app edge with default product fields and edge selection
- edge delete -> `removeEdgeIds` and canvas selection
- select -> app selection update

The POC harness keeps fixture app scenes in memory and reloads a render snapshot after accepted app patches. In `Real scene` mode, it can load `/api/scene` through the POC Vite proxy and send group translate/card drag/edit/group create/delete/node create/delete/duplicate/copy/paste/z-order/edge app patches through the real `/api/scene` `PATCH` route. Selection is mirrored in memory in the POC harness to avoid racing selection saves against drag mutation saves; the renderer still receives compact `select` patches so visible WebGPU can update selected group/card/edge outlines through dirty slot writes.

`shapeSceneToFilteredRenderSnapshot()` applies current app tag filtering before snapshot projection: a group is visible only when it includes every active tag id, and hidden node/edge selections are reset to canvas selection.

`updateShapeSceneGroupTags()` keeps tag attach in the app-owned shell: it updates app `SceneGroup.tagIds`, keeps tag registry data out of the renderer snapshot, and selects the updated group. In `Real scene` mode the POC uses `/api/groups/:id/tags` for the persistent path.

`addShapeSceneComment()` and the POC Product Export panel are also app-shell paths. Comments and artifacts stay in app `Scene`; export previews are generated from `sceneGraphForGroup()` and `generateLocalExport()`, not from renderer snapshots. In `Real scene` mode comments use `/api/comments` and exports use `/api/groups/:id/export`.

## Current App Parity Evidence

Covered by POC:

- group frame rendering
- card rendering
- text snippet rendering
- edge rendering
- pan/zoom/fit
- selection and WebGPU selected group/card/edge outline updates
- group tag filtering before render snapshot projection
- selected group tag attach through the app shell
- selected object comments through the app shell
- product export preview from app `Scene` graph data
- group drag / `translateGroups` semantics
- group creation/deletion, including contained node and incident edge cleanup on delete
- card drag
- card creation/deletion/duplicate/copy/paste, including incident edge cleanup on delete
- z-order actions through node `zIndex` patches
- edge creation
- edge deletion
- DOM inline edit overlay
- snapshot export for fixture comparison
- benchmark stats
- deterministic Shape `Scene` fixture loaded through the render adapter
- in-memory app scene patch loop for group translate, card drag, edit, group create/delete, node create/delete/duplicate/copy/paste, z-order, edge create/delete, and selection
- optional real app scene load/save path through `/api/scene` when the production backend is running
- export compatibility check proving app graph/export helpers still operate from `Scene`, while render snapshots exclude artifact/comment/export state

Covered by existing production tests, not reimplemented in POC:

- MCP API semantics

App-owned export path checked by POC tests:

- `sceneGraphForGroup()` derives deterministic export graph data from app `Scene`.
- `selectedSubgraph()` keeps selected node/edge export scope independent from renderer selection internals.
- `generateLocalExport()` continues to produce Mermaid/export content without reading render snapshot fields.
- Renderer snapshot export remains a POC diagnostic artifact, not the product MADR/YADR/Mermaid/image prompt export source.

Parity conclusion:

The POC proves the renderer boundary can carry the current editor surface and can translate core interaction patches back into app scene semantics. The render snapshot adapter now lives in shared production source without mounting the new renderer. The API-backed real scene path is terminal-verified through the POC proxy for drag/text/edge-equivalent patches, but production replacement is not ready until the same path is rendered-browser-verified against real persisted scenes and the Rust/wgpu backend reaches product quality.
