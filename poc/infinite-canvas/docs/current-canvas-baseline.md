# Current Canvas Workflow Baseline

## Scope

현재 production 앱은 DOM/SVG 기반 scene canvas를 유지한다. 이번 POC는 production canvas path를 수정하지 않고, `poc/infinite-canvas/` 아래에서 대체 가능성을 검증한다.

## Evidence Sources

- `README.md`: Web UI graph editing, local commands, API, MCP, export, performance model.
- `src/shared/schema.ts`: canonical `Scene`, `Group`, `Node`, `Edge`, `Tag`, `Comment`, `Artifact`, `selection`.
- `src/shared/graph.ts`: viewport bounds, LOD visibility, node bounds, graph export helpers.
- `src/client/App.tsx`: pan/zoom, fit, fullscreen, selection, node drag, group drag, inline edit, linked node creation, copy/paste, z-order, comments, export drawer flow.
- `tests/app.spec.ts`: end-to-end coverage for group creation, tag attach/filter, node edit, comments, copy/paste, z-order, export.
- `tests/performance.test.ts`: 10k-object storage/viewport/LOD performance contract.

## Preserved Workflows

- Scene navigation: pan, wheel zoom, fit scene, fullscreen, viewport query.
- Scene rendering: group frame, node card, selected state, edge curve/label, z-order, performance HUD.
- Graph editing: node select, inline edit, linked node creation, delete, duplicate, copy/paste, z-order move.
- Edge editing: edge selection, source/target validation, linked edge creation, deletion.
- Product shell: group creation, tag attach/filter, comments, deterministic exports, MCP/API semantics.

## Renderer-Specific Behavior That Can Be Replaced

- DOM card mounting strategy.
- SVG edge layer implementation.
- Current zoom-level DOM culling thresholds.
- Current compact/detail DOM card split.
- Current CSS-only visual implementation details.

These are implementation details, not product requirements. The new engine must preserve user-visible workflow semantics, stable ids, persistence, selection/export behavior, and edit affordances.

## Baseline Measurement Method

Use the existing app for behavioral baseline:

```bash
npm run typecheck
npm run test:unit
npm run build
npm run test:e2e
```

Use the POC for renderer comparison:

```bash
npm run poc:verify
npm run poc:dev
```

Manual baseline checklist:

- Pan/zoom remains smooth while groups/cards/edges stay visually continuous.
- Selecting a group, node, or edge returns the same stable id used by persistence/export.
- Inline text editing feels like editing inside a card, including IME/copy/paste behavior.
- Dragging a card persists a position patch.
- Creating/deleting an edge keeps graph export deterministic.
- Selection state does not break group/node/edge/selection export scopes.
