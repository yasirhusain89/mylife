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

test("similar photos surface the seeded near-duplicate group", async ({ page }) => {
  await page.getByRole("button", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find similar photos" }).click();
  await expect(page.getByRole("heading", { name: "Similar photos" })).toBeVisible();
  await expect(page.getByText("IMG_20261007_185824.jpg").first()).toBeVisible();
});

test("import via typed path adds assets", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  await expect(page.getByText("/mock/vacation", { exact: false }).first()).toBeVisible();
});

test("post-import pipeline organizes automatically", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Library organized: 8 enriched", { exact: false })).toBeVisible();
});

test("starring filters the library to favorites", async ({ page }) => {
  await page.getByRole("button", { name: "Star IMG_20261007_185817.jpg" }).click();
  await page.getByRole("button", { name: "★ Favorites" }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});

test("multi-select stars a batch", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185822.jpg" }).check();
  await expect(page.getByText("2 selected")).toBeVisible();
  await page.getByRole("button", { name: "★ Star", exact: true }).click();
  await page.getByRole("button", { name: "★ Favorites" }).click();
  await expect(page.getByText("Showing 2 of 2")).toBeVisible();
});

test("people can be named and merged", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("checkbox", { name: "Select cluster 1 for merge" }).check();
  await page.getByRole("checkbox", { name: "Select cluster 2 for merge" }).check();
  await page.getByRole("button", { name: "Merge into #1" }).click();
  await expect(page.getByText("Cluster 2")).toBeHidden();
});

test("cluster cards expand and jump to the library", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await page.getByRole("button", { name: "View in Library" }).first().click();
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
});

test("pet detection surfaces sightings and filters", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find pets" }).click();
  await expect(page.getByRole("heading", { name: "Pets" })).toBeVisible();
  await expect(page.getByText("Dog · 93%")).toBeVisible();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Pets", exact: true }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await page.getByRole("searchbox", { name: "Search library" }).fill("dog");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
});

test("indexed folders list every scanned root", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Indexed folders" })).toBeVisible();
  await expect(page.getByText("/photos", { exact: true })).toBeVisible();
  await expect(page.getByText("8 assets", { exact: false }).first()).toBeVisible();
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  const roots = page.getByRole("list");
  await expect(roots.getByText("/mock/vacation", { exact: true })).toBeVisible();
});

test("month filter narrows the grid", async ({ page }) => {
  await page.getByLabel("Filter by month").selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("filename search narrows and clears", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search library" }).fill("185817");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await page.getByRole("button", { name: "Clear" }).click();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByText("Recent:")).toBeVisible();
  await page.getByRole("button", { name: "185817", exact: true }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
});

test("concept search finds tags and places", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search library" }).fill("mountain");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await page.getByRole("button", { name: "Clear" }).click();
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Resolve place names" }).click();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("searchbox", { name: "Search library" }).fill("san francisco");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("event place names resolve from coordinates", async ({ page }) => {
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
  await expect(page.getByText("37.7749, -122.4194")).toBeVisible();
  await page.getByRole("button", { name: "Resolve place names" }).click();
  await expect(page.getByText("San Francisco, California")).toBeVisible();
});

test("highlight video reports photos and clips", async ({ page }) => {
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Highlight video" }).click();
  await expect(page.getByText("3 photos")).toBeVisible();
  await expect(page.getByText("1 video clip")).toBeVisible();
});

test("events render as a month-grouped timeline", async ({ page }) => {
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByRole("heading", { name: "October 2025" })).toBeVisible();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
});

test("album workflow: create, add photos, show, rename, delete", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("New album").fill("Family");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Family").first()).toBeVisible();

  await page.getByRole("button", { name: "Add photos" }).click();
  const picker = page.getByRole("dialog", { name: /Add photos to/ });
  await expect(picker).toBeVisible();
  await picker.getByRole("button", { name: "IMG_20261007_185817.jpg" }).click();
  await picker.getByRole("button", { name: "IMG_20261007_185822.jpg" }).click();
  await picker.getByRole("button", { name: "Add 2 photos" }).click();
  await page.getByRole("button", { name: "Show photos" }).click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();

  await page.getByRole("button", { name: "Remove", exact: true }).first().click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeHidden();

  await page.getByRole("button", { name: "Family", exact: true }).click();
  const rename = page.getByRole("dialog", { name: "Album name" });
  await rename.getByLabel("Album name").fill("Kin");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Kin").first()).toBeVisible();

  await page.getByRole("button", { name: "Delete album" }).click();
  const confirm = page.getByRole("dialog", { name: "Delete album" });
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Delete" }).click();
  await expect(page.getByText("Family").first()).toBeHidden();
});

test("event cards expand to member photos", async ({ page }) => {
  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Show photos" }).click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByLabel("Maximum hours between photos in one event")).toHaveValue("4");
});

test("scene tags filter the library", async ({ page }) => {
  await page.getByRole("button", { name: "Tag scenes" }).click();
  await page.getByLabel("Filter by scene").selectOption("mountain");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});

test("clicking a photo opens the viewer with keyboard navigation", async ({ page }) => {
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("1 of 8")).toBeVisible();
  await expect(dialog.getByRole("img").first()).toBeVisible();
  await page.keyboard.press("ArrowRight");
  await expect(dialog.getByText("2 of 8")).toBeVisible();
  await expect(dialog.getByText("IMG_20261007_185822.jpg").first()).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
});

test("video shows a poster with a web-mode note", async ({ page }) => {
  await page.getByTitle("Open VID_20261007_192001.mp4").click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Video playback needs the desktop app")).toBeVisible();
});
