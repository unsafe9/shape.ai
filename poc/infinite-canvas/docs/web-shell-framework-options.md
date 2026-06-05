# Web Shell Framework Options

## Current Shape

The POC web shell is canvas-first. `ShapeCanvasEngine` owns pointer handling, retained scene drawing, DOM text overlay placement, Rust/WASM/WebGPU integration, and benchmark frame production. The React layer in `web/src/main.tsx` mostly renders controls, panels, status, metrics, fixture loading, comments, and export previews.

This means the web shell does not need a large app framework for routing, SSR, data loading, or deep component composition. It needs predictable DOM updates around an imperative canvas engine.

## Recommendation

Use vanilla TypeScript with a small local store for the POC shell if the goal is the lightest dependency surface.

Why:

- The canvas engine is already imperative, so declarative reconciliation is not carrying the hardest part of the UI.
- The POC has no routing, SSR, or app-wide component library need.
- State can be centralized in one `PocShellState` object and rendered through small DOM update functions.
- It removes `@vitejs/plugin-react`, React Fast Refresh coupling, `react`, `react-dom`, `@types/react`, and `@types/react-dom` from the isolated POC path.

Do not migrate the production app shell on this evidence. Production still uses React and `lucide-react`; changing that is a separate app architecture decision.

## Candidate Ranking

1. Vanilla TypeScript + local store
   - Best fit for the isolated POC.
   - Lowest runtime and dependency cost.
   - Requires more discipline around DOM patching and event cleanup.

2. Preact
   - Best low-risk drop-in if keeping JSX/components is important.
   - Official docs support direct browser usage, Vite usage, and React aliasing.
   - Still keeps a component framework, just smaller.

3. Solid
   - Good fit if stats/control panels become highly reactive.
   - Fine-grained updates match frequent metrics and small UI regions.
   - Migration is more involved than Preact because React hooks are not drop-in.

4. Svelte
   - Good for a standalone compiled shell.
   - Adds `.svelte` component syntax and a second component language beside the production app.

5. Lit
   - Good if the shell should become framework-neutral Web Components embeddable from multiple hosts.
   - More ceremony than needed for the current POC control panel.

## Suggested Migration Plan

1. Split `main.tsx` responsibilities first without changing framework:
   - `shellState.ts`: POC state and derived selectors.
   - `shellActions.ts`: scene/app/API mutations.
   - `metricsView.tsx` and panel files if React stays temporarily.
2. If the shell remains POC-only, rewrite `main.tsx` to `main.ts` with:
   - static HTML template sections,
   - delegated event handlers,
   - explicit `renderShell(state)` and focused `renderMetrics`, `renderSelection`, `renderExportPreview` functions.
3. Remove React dependencies from the POC Vite config only after `npm run poc:verify` and browser smoke pass.
4. Keep the production app dependencies unchanged until there is a separate product-shell replacement plan.

## External Source Notes

- Preact's guide documents no-build browser usage, Vite setup, and React aliasing paths.
- Solid's docs describe fine-grained reactivity and targeted DOM updates.
- Svelte's docs describe compiling declarative components into optimized JavaScript.
- Lit's docs frame it as a lightweight Web Components library.
- Vite supports TypeScript transpilation and framework-light HTML entrypoints, but type checking should stay in a separate `tsc --noEmit` step.
