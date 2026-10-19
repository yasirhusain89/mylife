import { defineConfig, devices } from "@playwright/test";

// E2E runs the app in web mode (`?web=1` selects the in-memory mock bridge,
// so no Tauri window or Swift engine is needed). The dev server gets its own
// port so a running `tauri dev` on 1420 never conflicts.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  use: {
    baseURL: "http://127.0.0.1:1422",
    trace: "on-first-retry"
  },
  webServer: {
    command: "pnpm exec vite dev --port 1422 --strictPort --host 127.0.0.1",
    port: 1422,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }]
});
