import { cpSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig, type Plugin } from "vite";

const root = fileURLToPath(new URL(".", import.meta.url));
const clientPort = Number(process.env.SHAPE_AI_CLIENT_PORT ?? 5173);
const apiPort = Number(process.env.SHAPE_AI_PORT ?? 8787);

export default defineConfig({
  plugins: [react(), copyRendererWasm()],
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
      const sourceDir = resolve(root, "src/client/renderer/wasm");
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
