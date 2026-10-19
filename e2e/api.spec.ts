import { test, expect } from "@playwright/test";

// API E2E: the same Library UI against the real Rust read API
// (`fotoflow --serve --demo` seeds 8 assets under /photos: 6 photos +
// 2 videos; the seed is idempotent so a stale DB file is fine).
// Run via `pnpm test:e2e:api`, which sets FOTOFLOW_API and boots both
// the Vite dev server and the API server (see playwright.api.config.ts).
// No mock-vs-API distinguisher is asserted here — the value is
// real-backend coverage of the read paths.

const API = process.env.FOTOFLOW_API;

test.beforeEach(async ({ page }) => {
  test.skip(!API, "FOTOFLOW_API not set — run via pnpm test:e2e:api");
  await page.goto(`/?api=${encodeURIComponent(API!)}`);
});

test("library shows seeded assets with counts", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(page.getByText("VID_20261007_192001.mp4")).toBeVisible();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("filename search narrows to one asset", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search library" }).fill("185817");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});

test("months dropdown contains 2025-10 and filters", async ({ page }) => {
  const month = page.getByLabel("Filter by month");
  await expect(month).toBeVisible();
  await expect(page.locator('select[aria-label="Filter by month"] option[value="2025-10"]')).toBeAttached();
  await month.selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("stats header shows 6 photos + 2 videos", async ({ page }) => {
  await expect(page.getByRole("button", { name: /6 photos/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /2 videos/ })).toBeVisible();
});

test("root-assets lists every id under /photos", async ({ page }) => {
  const ids = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/root-assets?root=${encodeURIComponent("/photos")}`);
    return (await r.json()) as number[];
  }, API!);
  expect(ids).toHaveLength(8);
});
