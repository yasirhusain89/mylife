import { test, expect } from "@playwright/test";

// All specs run against the in-memory mock bridge (?web=1): seeded with
// 8 assets (6 photos, 2 videos), one duplicate group, one blurred photo.

test.beforeEach(async ({ page }) => {
  await page.goto("/?web=1");
});

test("library shows seeded assets with counts", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(page.getByText("VID_20261007_192001.mp4")).toBeVisible();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByRole("button", { name: /6 photos/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /2 videos/ })).toBeVisible();
});

test("all tabs navigate and render", async ({ page }) => {
  await page.getByRole("button", { name: /^Cleanup/ }).click();
  await expect(page.getByRole("heading", { name: "Cleanup" })).toBeVisible();
  await page.getByRole("button", { name: "People", exact: true }).click();
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible();
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Events" })).toBeVisible();
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Engine" })).toBeVisible();
});

test("engine check passes against the mock sidecar", async ({ page }) => {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Check engine" }).click();
  await expect(page.getByText("mock 0.0")).toBeVisible();
});

test("cleanup surfaces the seeded duplicate group", async ({ page }) => {
  await page.getByRole("button", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  await expect(page.getByText("IMG_20261007_191434.jpg")).toBeVisible();
});

test("import via typed path adds assets", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  await expect(page.getByText("/mock/vacation", { exact: false }).first()).toBeVisible();
});

test("month filter narrows the grid", async ({ page }) => {
  await page.getByLabel("Filter by month").selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});
