import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// T6.2: minimal Svelte config for the additive Svelte web shell. vitePreprocess
// lets <script lang="ts"> blocks use TypeScript without a separate toolchain.
export default {
  preprocess: vitePreprocess()
};
