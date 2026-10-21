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

test("favorite persists and the favorites filter shows it", async ({ page }) => {
  const name = "IMG_20261007_185817.jpg";
  // Reset via HTTP so a stale DB file can't leave the star pre-set, then
  // reload so the UI starts unstarred; the starring itself stays UI-driven.
  await page.evaluate(async (base) => {
    const rows = (await (
      await fetch(`${base}/api/library?limit=500`)
    ).json()) as { id: number }[];
    await Promise.all(
      rows.map((a) =>
        fetch(`${base}/api/favorites`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ id: a.id, favorite: false })
        })
      )
    );
  }, API!);
  await page.reload();
  await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
  await page.getByRole("button", { name: `Star ${name}` }).first().click();
  await page.getByRole("button", { name: "★ Favorites" }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText(name).first()).toBeVisible();
});

test("album create+add via UI then Albums tab shows the count", async ({ page }) => {
  const name = `e2e-album-${Date.now()}`;
  await page.getByRole("tab", { name: /^Albums/ }).click();
  await page.getByRole("button", { name: "New album", exact: true }).click();
  await page.getByRole("textbox", { name: "New album" }).fill(name);
  await page.getByRole("button", { name: "Create" }).click();
  // Scope to this run's album card: the shared DB file keeps albums
  // from earlier runs, so bare lookups would match several cards.
  const card = page.locator("article", { hasText: name });
  await expect(card.getByRole("button", { name, exact: true })).toBeVisible();
  await card.getByRole("button", { name: "Add photos" }).click();
  await page.locator('button[title="IMG_20261007_185817.jpg"]').click();
  await page.getByRole("button", { name: "Add 1 photo" }).click();
  await expect(page.getByText(/Added 1 photo to/)).toBeVisible();
  await expect(card.getByText("1 photo")).toBeVisible();
});

test("tag-run returns a summary without error", async ({ page }) => {
  const summary = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/tags/run`, { method: "POST" });
    if (!r.ok) throw new Error(`tags/run failed: ${r.status}`);
    return (await r.json()) as { total: number; tagged: number; failed: number };
  }, API!);
  expect(typeof summary.total).toBe("number");
  expect(typeof summary.tagged).toBe("number");
  expect(typeof summary.failed).toBe("number");
  // UI trigger resolves too (no progress stream over HTTP — the button
  // just settles back with a summary toast).
  await page.getByRole("button", { name: "Tag scenes" }).click();
  await expect(page.getByText(/Tagged \d+ of \d+ photos|already has scene tags/)).toBeVisible();
});

test("path import via Scan adds a second root and grows counts", async ({ page }) => {
  const fs = await import("node:fs");
  const os = await import("node:os");
  const path = await import("node:path");
  const tmpRoot = path.join(
    os.tmpdir(),
    `ff-e2e-import-${Date.now()}-${Math.floor(Math.random() * 100000)}`
  );
  fs.mkdirSync(tmpRoot, { recursive: true });
  for (let i = 0; i < 2; i++) {
    fs.writeFileSync(path.join(tmpRoot, `e2e_${i}.jpg`), Buffer.alloc(2048 + i, 0x78));
  }

  const before = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/library/count`);
    return ((await r.json()) as { count: number }).count;
  }, API!);

  await page.getByPlaceholder("…or paste a folder path").fill(tmpRoot);
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText(tmpRoot, { exact: true }).first()).toBeVisible({ timeout: 30_000 });

  const after = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/library/count`);
    return ((await r.json()) as { count: number }).count;
  }, API!);
  expect(after).toBeGreaterThan(before);
  expect(after).toBeGreaterThanOrEqual(before + 2);

  const roots = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/roots`);
    return (await r.json()) as { root: string }[];
  }, API!);
  expect(roots.some((r) => r.root === tmpRoot)).toBe(true);

  // Removing the root returns the library to its seeded counts, so the
  // shared DB file stays clean for the next suite run.
  const removed = await page.evaluate(async ({ base, root }) => {
    const r = await fetch(`${base}/api/roots`, {
      method: "DELETE",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ root })
    });
    return (await r.json()) as { removedAssets: number; removedScans: number };
  }, { base: API!, root: tmpRoot });
  expect(removed.removedAssets).toBeGreaterThanOrEqual(2);
  const restored = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/library/count`);
    return ((await r.json()) as { count: number }).count;
  }, API!);
  expect(restored).toBe(before);
});

test("map pins endpoint serves an array", async ({ page }) => {
  const pins = await page.evaluate(async (base) => {
    const r = await fetch(`${base}/api/map-pins`);
    return (await r.json()) as unknown;
  }, API!);
  expect(Array.isArray(pins)).toBe(true);
});
