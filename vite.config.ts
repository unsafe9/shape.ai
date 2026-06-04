import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

const clientPort = Number(process.env.CHARRETTE_CLIENT_PORT ?? 5173);
const apiPort = Number(process.env.CHARRETTE_PORT ?? 8787);

export default defineConfig({
  plugins: [react()],
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
