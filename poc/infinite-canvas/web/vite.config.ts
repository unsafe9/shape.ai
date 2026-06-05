import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const pocPort = Number(process.env.SHAPE_AI_POC_PORT ?? 5174);
const apiTarget = process.env.SHAPE_AI_API_TARGET ?? "http://127.0.0.1:8787";

export default defineConfig({
  root: "poc/infinite-canvas/web",
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: pocPort,
    proxy: {
      "/api": {
        target: apiTarget,
        changeOrigin: true
      }
    }
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true
  }
});
