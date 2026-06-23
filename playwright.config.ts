import { defineConfig, devices } from "@playwright/test";

// Playwright config for scaffold smoke tests against the Vite dev server.
// Full Tauri IPC E2E requires tauri-driver/WebDriver; this config covers the
// frontend shell render. See tests/app.spec.ts.
export default defineConfig({
  testDir: "./tests",
  timeout: 30_000,
  fullyParallel: false,
  retries: 0,
  use: {
    baseURL: "http://127.0.0.1:1420",
    trace: "on-first-retry",
  },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
});
