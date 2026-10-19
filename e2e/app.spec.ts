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

test("picker search narrows candidates", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("Searchable");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Searchable").first()).toBeVisible();

  await page.getByRole("button", { name: "Add photos" }).click();
  const picker = page.getByRole("dialog", { name: /Add photos to/ });
  await expect(picker).toBeVisible();
  await expect(picker.getByText("8 of 8")).toBeVisible();
  await picker.getByLabel("Search photos by filename").fill("185817");
  await expect(picker.getByText("1 of 8")).toBeVisible();
  await expect(picker.getByRole("button", { name: "IMG_20261007_185817.jpg" })).toBeVisible();
  await expect(picker.getByRole("button", { name: "IMG_20261007_185822.jpg" })).toBeHidden();
});

test("picker inline new-album creates and selects", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("First");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("First").first()).toBeVisible();

  await page.getByRole("button", { name: "Add photos" }).click();
  const picker = page.getByRole("dialog", { name: /Add photos to/ });
  await expect(picker).toBeVisible();
  await picker.getByRole("button", { name: "＋ New album" }).click();
  await picker.getByLabel("New album name").fill("Second");
  await picker.getByRole("button", { name: "Create" }).click();
  await expect(picker.getByRole("heading", { name: /Second/ })).toBeVisible();
  await picker.getByRole("button", { name: "IMG_20261007_185817.jpg" }).click();
  await picker.getByRole("button", { name: /Add 1 photo/ }).click();
  await expect(page.getByText("Added 1 photo to “Second”")).toBeVisible();
});

test("batch-bar inline new-album creates and adds", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByRole("button", { name: "＋ New album" }).click();
  await batchBar.getByLabel("New album name").fill("BatchAlbum");
  await batchBar.getByRole("button", { name: "Create" }).click();
  await expect(batchBar.getByLabel("Add selected to album")).toBeVisible();
  await batchBar.getByRole("button", { name: "Add to album" }).click();
  await expect(page.getByText("Added 1 photo to the album")).toBeVisible();
  await page.getByRole("button", { name: /^Albums/ }).click();
  await expect(page.getByText("BatchAlbum").first()).toBeVisible();
});

// ---------------------------------------------------------------------------
// Area 1: New library addition (mock importFolder / scanRoots)
// ---------------------------------------------------------------------------

test("library import updates stats and renders new assets", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  // New batch renders in the grid.
  await expect(page.getByText("IMPORTED_100.jpg").first()).toBeVisible();
  await expect(page.getByText("IMPORTED_103.jpg").first()).toBeVisible();
  // Stats chips reflect 10 photos (6 seed + 4 imported) + 2 videos.
  await expect(page.getByRole("button", { name: /10 photos/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /2 videos/ })).toBeVisible();
  // Import progress reports the deterministic 4-new batch.
  await expect(page.getByText(/12 scanned.*4 new.*8 unchanged/)).toBeVisible();
});

test("imported batch lands in a new month entry", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  const month = page.getByLabel("Filter by month");
  await expect(month).toBeVisible();
  // Seed month keeps its 8; the mock import batch is pinned to Nov 2025.
  await expect(page.locator('select[aria-label="Filter by month"] option[value="2025-10"]')).toBeAttached();
  await expect(page.locator('select[aria-label="Filter by month"] option[value="2025-11"]')).toBeAttached();
  await month.selectOption("2025-11");
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  await expect(page.getByText("IMPORTED_100.jpg").first()).toBeVisible();
  await month.selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByText("IMPORTED_100.jpg")).toBeHidden();
});

test("rescanning the same root does not duplicate history", async ({ page }) => {
  const path = page.getByPlaceholder("…or paste a folder path");
  await path.fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  await path.fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 16 of 16")).toBeVisible();
  // History lists each root once; the rescanned root now holds 8 assets.
  const roots = page.getByRole("list");
  await expect(roots.getByText("/mock/vacation", { exact: true })).toHaveCount(1);
  await expect(roots.getByText("/photos", { exact: true })).toHaveCount(1);
  await expect(page.getByText("8 assets", { exact: false }).nth(1)).toBeVisible();
});

test("missing-volume import surfaces unreachable state and check-files report", async ({
  page
}) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/missing-drive");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  // Stats error + per-root hint expose the ejected volume (mock flags the
  // 4 imported assets missing when the root contains "missing").
  await expect(page.getByText(/4 files? unreachable/)).toBeVisible();
  await expect(page.getByText(/unreachable/).first()).toBeVisible();
  await page.getByRole("button", { name: "Check files" }).click();
  await expect(page.getByText("4 missing · 0 restored")).toBeVisible();
});

test("check-files reports a healthy library when nothing is missing", async ({ page }) => {
  await page.getByRole("button", { name: "Check files" }).click();
  await expect(page.getByText("All indexed files are present")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Area 2: Person identification + merging
// ---------------------------------------------------------------------------

test("people empty state before clustering", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible();
  await expect(page.getByText("No people clusters yet — scan the library to detect faces.")).toBeVisible();
  await expect(page.getByText("Cluster 1")).toBeHidden();
});

test("face clustering renders two clusters with face counts", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await expect(page.getByText("Cluster 2")).toBeVisible();
  // Seed has 6 photos: first four in cluster 1, last two in cluster 2.
  await expect(page.getByText("4 faces")).toBeVisible();
  await expect(page.getByText("2 faces")).toBeVisible();
});

test("renaming a cluster persists in the UI", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: "Cluster 1" }).click();
  const dialog = page.getByRole("dialog", { name: "Person name" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Person name").fill("Ada");
  await dialog.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
  await expect(page.getByText("Cluster 1")).toBeHidden();
  // Rename survives leaving and returning to the tab.
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: /^People/ }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
});

test("cluster source photos expand", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  // Cluster 1 holds the first four seed photos; paths dedupe to filenames.
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByText("IMG_20261007_185822.jpg").first()).toBeVisible();
  await page.getByRole("button", { name: "Hide photos" }).first().click();
  // Collapsing keeps the cluster card but the member grid is gone.
  await expect(page.getByText("Cluster 1")).toBeVisible();
});

test("merging clusters adds counts and drops the loser", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await expect(page.getByText("Cluster 2")).toBeVisible();
  await page.getByRole("checkbox", { name: "Select cluster 1 for merge" }).check();
  await page.getByRole("checkbox", { name: "Select cluster 2 for merge" }).check();
  await page.getByRole("button", { name: "Merge into #1" }).click();
  await expect(page.getByText("Cluster 2")).toBeHidden();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await expect(page.getByText("6 faces", { exact: true })).toBeVisible();
  await expect(page.getByText(/Merged into one person \(6 faces\)/)).toBeVisible();
});

test("view-in-library filters to the selected person", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "View in Library" }).first().click();
  // Cluster 1 = first four seed photos.
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByText("IMG_20261007_191434_copy.jpg")).toBeHidden();
});

// ---------------------------------------------------------------------------
// Area 3: Adding to album by every supported gathering path
// ---------------------------------------------------------------------------

test("library multi-select adds a batch to an existing album", async ({ page }) => {
  // Arrange: an album to target.
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("BatchTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("BatchTarget").first()).toBeVisible();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185822.jpg" }).check();
  await expect(page.getByText("2 selected")).toBeVisible();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByLabel("Add selected to album").selectOption({ label: "BatchTarget" });
  await batchBar.getByRole("button", { name: "Add to album" }).click();
  await expect(page.getByText("Added 2 photos to the album")).toBeVisible();

  await page.getByRole("button", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "BatchTarget" });
  await expect(card.getByText("2 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("IMG_20261007_185822.jpg")).toBeVisible();
});

test("picker search-toggle-confirm adds to an album", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("PickerTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("PickerTarget").first()).toBeVisible();

  await page.getByRole("button", { name: "Add photos" }).click();
  const picker = page.getByRole("dialog", { name: /Add photos to/ });
  await expect(picker).toBeVisible();
  await expect(picker.getByText("8 of 8")).toBeVisible();
  // Search narrows candidates, toggle selects, clearing restores the grid.
  await picker.getByLabel("Search photos by filename").fill("185817");
  await expect(picker.getByText("1 of 8")).toBeVisible();
  await picker.getByRole("button", { name: "IMG_20261007_185817.jpg" }).click();
  await picker.getByLabel("Search photos by filename").fill("");
  await expect(picker.getByText("8 of 8")).toBeVisible();
  await picker.getByRole("button", { name: "IMG_20261007_185822.jpg" }).click();
  await picker.getByRole("button", { name: "Add 2 photos" }).click();
  await expect(page.getByText('Added 2 photos to “PickerTarget”')).toBeVisible();

  const card = page.locator(".album-card", { hasText: "PickerTarget" });
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("IMG_20261007_185822.jpg")).toBeVisible();
});

test("event member grid adds to an album via ＋Album", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("EventTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("EventTarget").first()).toBeVisible();

  await page.getByRole("button", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
  await page.getByRole("button", { name: "Show photos" }).click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await page.getByLabel("Add InlawsVisit2026 to album").selectOption({ label: "EventTarget" });
  await page.getByRole("button", { name: "＋ Album" }).click();
  await expect(page.getByText('Added 8 photos to “EventTarget”')).toBeVisible();

  await page.getByRole("button", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "EventTarget" });
  await expect(card.getByText("8 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("VID_20261007_192001.mp4")).toBeVisible();
});

test("indexed folder adds everything to an album via ＋ All to album", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("FolderTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("FolderTarget").first()).toBeVisible();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  const row = page.locator(".roots li", { hasText: "/photos" });
  await row.getByLabel("Add everything under /photos to album").selectOption({ label: "FolderTarget" });
  await row.getByRole("button", { name: "＋ All to album" }).click();
  await expect(page.getByText("Added 8 photos from /photos to “FolderTarget”")).toBeVisible();

  await page.getByRole("button", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "FolderTarget" });
  await expect(card.getByText("8 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("VID_20261007_192001.mp4")).toBeVisible();
});

test("filtered library selection adds only the visible set", async ({ page }) => {
  await page.getByRole("button", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("FilteredTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("FilteredTarget").first()).toBeVisible();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("searchbox", { name: "Search library" }).fill("185817");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByLabel("Add selected to album").selectOption({ label: "FilteredTarget" });
  await batchBar.getByRole("button", { name: "Add to album" }).click();
  await expect(page.getByText("Added 1 photo to the album")).toBeVisible();

  await page.getByRole("button", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "FilteredTarget" });
  await expect(card.getByText("1 photo")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("IMG_20261007_185822.jpg")).toBeHidden();
});

// ---------------------------------------------------------------------------
// Area 4: Search and filter
// ---------------------------------------------------------------------------

test("filename search is case-insensitive with counts", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search library" }).fill("img_20261007_185817");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(1);
});

test("kind filter separates photos and videos", async ({ page }) => {
  await page.getByRole("button", { name: /6 photos/ }).click();
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByText("VID_20261007_192001.mp4")).toBeHidden();

  await page.getByRole("button", { name: /2 videos/ }).click();
  await expect(page.getByText("Showing 2 of 2")).toBeVisible();
  await expect(page.getByText("VID_20261007_192001.mp4").first()).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg")).toBeHidden();

  await page.getByRole("button", { name: "All", exact: true }).click();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("month filter isolates the seeded month", async ({ page }) => {
  const month = page.getByLabel("Filter by month");
  await expect(month).toBeVisible();
  await month.selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await month.selectOption("");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("person filter narrows to one cluster", async ({ page }) => {
  // The Library person dropdown only appears once personOptions load; a
  // rename triggers that reload (Find alone does not refresh the options).
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: "Cluster 1" }).click();
  const rename = page.getByRole("dialog", { name: "Person name" });
  await rename.getByLabel("Person name").fill("FilterMe");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("FilterMe").first()).toBeVisible();

  await page.getByRole("button", { name: "Library", exact: true }).click();
  const person = page.getByLabel("Filter by person");
  await expect(person).toBeVisible();
  await person.selectOption("1");
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByText("IMG_20261007_191434_copy.jpg")).toBeHidden();
  await person.selectOption("");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("scene tag filter narrows to tagged assets", async ({ page }) => {
  const scene = page.getByLabel("Filter by scene");
  await expect(scene).toBeVisible();
  await scene.selectOption("mountain");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(1);
  await scene.selectOption("flower");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_191433.jpg").first()).toBeVisible();
  await scene.selectOption("");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("pet filter needs detection then narrows", async ({ page }) => {
  await page.getByRole("button", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find pets" }).click();
  await expect(page.getByText("Dog · 93%")).toBeVisible();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Pets", exact: true }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_191433.jpg").first()).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(1);
});

test("favorite filter and unstar dropout", async ({ page }) => {
  await page.getByRole("button", { name: "Star IMG_20261007_185817.jpg" }).click();
  await page.getByRole("button", { name: "★ Favorites" }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  // Unstarring drops the photo out of the Favorites view.
  await page.getByRole("button", { name: /Unstar IMG_20261007_185817/ }).click();
  await expect(page.getByText("No assets yet — add a folder to build the index.")).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(0);
});

test("location filter splits located vs unlocated", async ({ page }) => {
  await page.getByRole("button", { name: "With location" }).click();
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await expect(page.getByText("VID_20261007_192001.mp4").first()).toBeVisible();

  await page.getByRole("button", { name: "No location" }).click();
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185824.jpg").first()).toBeVisible();

  await page.getByRole("button", { name: "Anywhere" }).click();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("combined kind plus tag filters intersect", async ({ page }) => {
  await page.getByRole("button", { name: /6 photos/ }).click();
  await page.getByLabel("Filter by scene").selectOption("mountain");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  // Videos carry no mountain tag: the same tag with kind=video is empty.
  await page.getByRole("button", { name: /2 videos/ }).click();
  await expect(page.getByText("No assets yet — add a folder to build the index.")).toBeVisible();
});

test("empty search result shows the no-assets hint", async ({ page }) => {
  await page.getByRole("searchbox", { name: "Search library" }).fill("zzz-no-such-photo-123");
  await expect(page.getByText("No assets yet — add a folder to build the index.")).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(0);
});

// ---------------------------------------------------------------------------
// Area 5: Visual similarity (image-query "more like this photo")
// ---------------------------------------------------------------------------

test("similar button opens results that exclude the query photo", async ({ page }) => {
  await page.getByRole("button", { name: "Similar to IMG_20261007_185817.jpg" }).first().click();
  const dialog = page.getByRole("dialog", { name: "Similar to IMG_20261007_185817.jpg" });
  await expect(dialog).toBeVisible();
  // Neighbours render; the query photo itself is never a result.
  await expect(dialog.getByText("IMG_20261007_185822.jpg")).toBeVisible();
  await expect(dialog.locator("figure", { hasText: "IMG_20261007_185817.jpg" })).toHaveCount(0);
  await expect(dialog.locator("figure")).toHaveCount(7);
  await dialog.getByRole("button", { name: "Back to Library" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("viewer similar button opens results and closing returns", async ({ page }) => {
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  const viewer = page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" });
  await expect(viewer).toBeVisible();
  await viewer.getByRole("button", { name: "Similar to IMG_20261007_185817.jpg" }).click();
  const dialog = page.getByRole("dialog", { name: "Similar to IMG_20261007_185817.jpg" });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator("figure")).toHaveCount(7);
  await dialog.getByRole("button", { name: "Back to Library" }).click();
  await expect(dialog).toBeHidden();
  // The viewer underneath is untouched; Escape still closes it.
  await page.keyboard.press("Escape");
  await expect(viewer).toBeHidden();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});
