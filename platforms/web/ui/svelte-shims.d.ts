// Ambient declaration so `tsc --noEmit` can resolve `import X from "./Foo.svelte"`. Svelte components
// are compiled by vite-plugin-svelte, not tsc; this keeps the checker quiet without pulling Svelte into tsc's graph.
declare module "*.svelte" {
  import type { Component } from "svelte";
  const component: Component;
  export default component;
}
