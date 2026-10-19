import { defineConfig, devices } from "@playwright/test";

// API E2E: same Vite dev server plus the real Rust read API
// (`fotoflow --serve --demo` seeds 8 assets under /photos; the seed is
// idempotent so a stale DB file is fine). Specs opt in via ?api=.
export default defineConfig({
  testDir: "./e2e",
  testMatch: "api.spec.ts",
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  use: {
    baseURL: "http://127.0.0.1:1422",
    trace: "on-first-failure"
  },
  webServer: [
    {
      command: "pnpm exec vite dev --port 1422 --strictPort --host 127.0.0.1",
      port: 1422,
      reuseExistingServer: !process.env.CI,
      timeout: 120_000
    },
    {
      command:
        "cargo run -q -p fotoflow -- --serve --port 1423 --db /tmp/ff-api-test.db --demo",
      cwd: "src-tauri",
      port: 1423,
      reuseExistingServer: false,
      timeout: 120_000
    }
  ],
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }]
});
