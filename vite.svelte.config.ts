import { cpSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig, type Plugin } from "vite";

// T6.2: a SECOND, additive Vite entry that builds ONLY the Svelte web shell to
// dist/client-svelte. The React entry (vite.config.ts → dist/client) is left
// untouched. Both reuse the framework-neutral renderer/* + lib/api.ts + the
// copyRendererWasm() plugin verbatim.

const root = fileURLToPath(new URL(".", import.meta.url));
const clientPort = Number(process.env.SHAPE_AI_SVELTE_CLIENT_PORT ?? 5174);
const apiPort = Number(process.env.SHAPE_AI_PORT ?? 8787);

export default defineConfig({
  plugins: [svelte(), copyRendererWasm()],
  server: {
    host: "127.0.0.1",
    port: clientPort,
    proxy: {
      "/api": `http://127.0.0.1:${apiPort}`
    }
  },
  build: {
    outDir: "dist/client-svelte",
    rollupOptions: {
      input: resolve(root, "index-svelte.html")
    }
  }
});

function copyRendererWasm(): Plugin {
  return {
    name: "copy-shape-renderer-wasm",
    apply: "build" as const,
    closeBundle() {
      const sourceDir = resolve(root, "src/client/renderer/wasm");
      const requiredFiles = ["shape_canvas_core.js", "shape_canvas_core_bg.wasm"];
      const missingFile = requiredFiles.find((fileName) => !existsSync(join(sourceDir, fileName)));
      if (missingFile) {
        this.error(`Rust renderer WASM package is missing ${missingFile}; run npm run renderer:wasm:build before vite build.`);
      }
      const targetDir = resolve(root, "dist/client-svelte/assets/wasm");
      mkdirSync(targetDir, { recursive: true });
      for (const fileName of readdirSync(sourceDir)) {
        if (fileName === ".gitignore") continue;
        cpSync(join(sourceDir, fileName), join(targetDir, fileName), { recursive: true });
      }
    }
  };
}
