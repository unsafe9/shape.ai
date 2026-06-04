import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  testMatch: /.*\.spec\.ts/,
  timeout: 30_000,
  fullyParallel: false,
  use: {
    baseURL: "http://127.0.0.1:5174",
    trace: "on-first-retry"
  },
  webServer: [
    {
      command: "SHAPE_AI_PORT=8788 npm run dev:server",
      url: "http://127.0.0.1:8788/api/health",
      reuseExistingServer: false,
      timeout: 30_000
    },
    {
      command: "SHAPE_AI_CLIENT_PORT=5174 SHAPE_AI_PORT=8788 npm run dev:client",
      url: "http://127.0.0.1:5174",
      reuseExistingServer: false,
      timeout: 30_000
    }
  ],
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } } },
    { name: "mobile", use: { ...devices["Pixel 7"] } }
  ]
});
