// T6.2: ambient declaration so the React-oriented `tsc --noEmit` typecheck can
// resolve `import X from "./Foo.svelte"` in the additive Svelte shell entry
// (svelte/main.ts). Svelte components are compiled by vite-plugin-svelte, not by
// tsc; this keeps the type checker quiet without pulling Svelte into tsc's graph.
declare module "*.svelte" {
  import type { Component } from "svelte";
  const component: Component;
  export default component;
}
