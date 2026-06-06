import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  // Register the Svelte plugin so future component tests can import .svelte
  // files; the existing node tests import only .ts modules and are unaffected.
  plugins: [svelte({ hot: false })],
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"]
  }
});
