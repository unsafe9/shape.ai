import { cpSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig, type Plugin } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const clientPort = Number(process.env.SHAPE_AI_CLIENT_PORT ?? 5173);
const apiPort = Number(process.env.SHAPE_AI_PORT ?? 8787);

export default defineConfig({
  plugins: [svelte(), copyRendererWasm(), copySceneCoreWasm()],
  server: {
    host: "127.0.0.1",
    port: clientPort,
    proxy: {
      "/api": `http://127.0.0.1:${apiPort}`
    }
  },
  build: {
    outDir: "dist/client"
  }
});

function copyRendererWasm(): Plugin {
  return {
    name: "copy-shape-renderer-wasm",
    apply: "build" as const,
    closeBundle() {
      const sourceDir = resolve(root, "platforms/web/bridge/wasm");
      const requiredFiles = ["shape_canvas_core.js", "shape_canvas_core_bg.wasm"];
      const missingFile = requiredFiles.find((fileName) => !existsSync(join(sourceDir, fileName)));
      if (missingFile) {
        this.error(`Rust renderer WASM package is missing ${missingFile}; run npm run renderer:wasm:build before vite build.`);
      }
      const targetDir = resolve(root, "dist/client/assets/wasm");
      mkdirSync(targetDir, { recursive: true });
      for (const fileName of readdirSync(sourceDir)) {
        if (fileName === ".gitignore") continue;
        cpSync(join(sourceDir, fileName), join(targetDir, fileName), { recursive: true });
      }
    }
  };
}

// MG-7a: the scene-core op-apply WASM is loaded by sceneCoreWasm.ts via a
// @vite-ignore'd `./wasm/shape_scene_core.js` import, so (like the renderer wasm)
// vite does not bundle it — it must sit next to the emitted chunk at runtime. The
// emitted shell chunk lives under assets/, so the bridge resolves `./wasm/...` to
// assets/wasm/. We copy the scene-core package into the SAME assets/wasm dir the
// renderer uses. The package is checked in, so a missing build is a warning (the
// loader falls back to the TS op-apply), not a hard build failure.
function copySceneCoreWasm(): Plugin {
  return {
    name: "copy-shape-scene-core-wasm",
    apply: "build" as const,
    closeBundle() {
      const sourceDir = resolve(root, "platforms/web/bridge/wasm");
      const requiredFiles = ["shape_scene_core.js", "shape_scene_core_bg.wasm"];
      const missingFile = requiredFiles.find((fileName) => !existsSync(join(sourceDir, fileName)));
      if (missingFile) {
        this.warn(`scene-core WASM package is missing ${missingFile}; the shell will fall back to the TS op-apply.`);
        return;
      }
      const targetDir = resolve(root, "dist/client/assets/wasm");
      mkdirSync(targetDir, { recursive: true });
      for (const fileName of readdirSync(sourceDir)) {
        if (fileName === ".gitignore") continue;
        cpSync(join(sourceDir, fileName), join(targetDir, fileName), { recursive: true });
      }
    }
  };
}
