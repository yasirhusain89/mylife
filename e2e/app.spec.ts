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
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await expect(page.getByRole("heading", { name: "Cleanup" })).toBeVisible();
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible();
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Events" })).toBeVisible();
  await page.getByRole("tab", { name: "Map", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Map" })).toBeVisible();
  await expect(page.locator(".leaflet-container")).toBeVisible();
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Albums" })).toBeVisible();
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Engine" })).toBeVisible();
});

test("tabs expose tablist semantics with arrow-key navigation", async ({ page }) => {
  const tablist = page.getByRole("tablist", { name: "Library sections" });
  await expect(tablist).toBeVisible();
  const libraryTab = page.getByRole("tab", { name: "Library", exact: true });
  await expect(libraryTab).toHaveAttribute("aria-selected", "true");
  await libraryTab.focus();
  await page.keyboard.press("ArrowRight");
  const cleanupTab = page.getByRole("tab", { name: /^Cleanup/ });
  await expect(cleanupTab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("heading", { name: "Cleanup" })).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(libraryTab).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("heading", { name: "Library" })).toBeVisible();
});

test("confirm dialog traps focus and cancels with Esc and scrim click", async ({ page }) => {
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByRole("button", { name: "Trash", exact: true }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  // Initial focus lands on Cancel; Shift+Tab wraps to the last action.
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(dialog.getByRole("button", { name: "Move to Trash" })).toBeFocused();
  // Esc cancels and keeps the selection.
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(batchBar.getByText("1 selected")).toBeVisible();
  // Scrim click cancels too.
  await batchBar.getByRole("button", { name: "Trash", exact: true }).click();
  await expect(dialog).toBeVisible();
  await page.locator(".scrim").click({ position: { x: 5, y: 5 } });
  await expect(dialog).toBeHidden();
  await expect(batchBar.getByText("1 selected")).toBeVisible();
});

test("focus lands inside each dialog", async ({ page }) => {
  // Confirm (destructive).
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".batch-bar").getByRole("button", { name: "Trash", exact: true }).click();
  const confirm = page.getByRole("alertdialog");
  await expect(confirm).toBeVisible();
  await expect(confirm.locator(":focus")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(confirm).toBeHidden();
  // Person name dialog.
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Cluster 1", exact: true }).click();
  const nameDialog = page.getByRole("dialog", { name: "Person name" });
  await expect(nameDialog).toBeVisible();
  await expect(nameDialog.getByLabel("Person name")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(nameDialog).toBeHidden();
  // New-album dialog then the photo picker.
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const albumDialog = page.getByRole("dialog", { name: "New album" });
  await expect(albumDialog).toBeVisible();
  await expect(albumDialog.locator(":focus")).toHaveCount(1);
  await albumDialog.getByLabel("New album").fill("FocusAlbum");
  await albumDialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("FocusAlbum").first()).toBeVisible();
  await page.getByRole("button", { name: "Add photos" }).click();
  const picker = page.getByRole("dialog", { name: /Add photos to/ });
  await expect(picker).toBeVisible();
  await expect(picker.getByLabel("Search photos by filename")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(picker).toBeHidden();
  // Delete-album confirm.
  await page.getByRole("button", { name: "Delete album" }).click();
  const deleteDialog = page.getByRole("dialog", { name: "Delete album" });
  await expect(deleteDialog).toBeVisible();
  await expect(deleteDialog.locator(":focus")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(deleteDialog).toBeHidden();
  // Indexed-folder removal confirm.
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await page.getByRole("button", { name: "Remove /mock/vacation from the index" }).click();
  const rootDialog = page.getByRole("dialog", { name: "Remove indexed folder" });
  await expect(rootDialog).toBeVisible();
  await expect(rootDialog.locator(":focus")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(rootDialog).toBeHidden();
});

test("engine check passes against the mock sidecar", async ({ page }) => {
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await page.getByRole("button", { name: "Check engine" }).click();
  await expect(page.getByText("mock 0.0")).toBeVisible();
});

test("cleanup surfaces the seeded duplicate group", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  await expect(page.getByText("IMG_20261007_191434.jpg")).toBeVisible();
});

test("similar photos surface the seeded near-duplicate group", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
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

test("batch delete honors Flag-only mode and Cancel keeps selection", async ({ page }) => {
  // Flag-only mode is set in Cleanup but must hold in the Library batch bar.
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  // The button reflects flag mode instead of silently flipping to trash.
  await batchBar.getByRole("button", { name: "Flag", exact: true }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByText("Flag only — nothing on disk changes.")).toBeVisible();
  // Cancel preserves the selection.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(batchBar.getByText("1 selected")).toBeVisible();
  // Confirm flags without touching disk.
  await batchBar.getByRole("button", { name: "Flag", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Flag", exact: true }).click();
  await expect(page.getByText("flagged only")).toBeVisible();
  // Confirm drops only the acted-on id from the selection.
  await expect(batchBar.getByText("1 selected")).toBeHidden();
});

test("batch delete in trash mode moves to Trash and clears selection", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await expect(batchBar.getByText("1 selected")).toBeVisible();
  // The batch button reflects the default trash mode.
  await batchBar.getByRole("button", { name: "Trash", exact: true }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByText("moves the file to the macOS Trash")).toBeVisible();
  // Cancel keeps the selection.
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(batchBar.getByText("1 selected")).toBeVisible();
  // Confirm trashes and clears the selection.
  await batchBar.getByRole("button", { name: "Trash", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.getByRole("status")).toContainText("moved to Trash");
  await expect(batchBar.getByText("1 selected")).toBeHidden();
  await expect(batchBar.getByText("0 selected")).toBeVisible();
});

test("cleanup duplicate single delete honors trash mode", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  const group = page.locator("article.group", { hasText: "IMG_20261007_191434_copy.jpg" });
  await group.getByRole("button", { name: "Remove" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByText("moves the file to the macOS Trash")).toBeVisible();
  await dialog.getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.getByRole("status")).toContainText("moved to Trash");
});

test("cleanup blurred remove in flag mode shows the flagged badge", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");
  const card = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await card.getByRole("button", { name: "remove" }).click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog.getByText("Flag only — nothing on disk changes.")).toBeVisible();
  await dialog.getByRole("button", { name: "Flag", exact: true }).click();
  await expect(page.getByRole("status")).toContainText("flagged only");
  await expect(card.getByRole("button", { name: "flagged" })).toBeVisible();
});

test("flags survive a reload", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");
  const card = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await card.getByRole("button", { name: "remove" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Flag", exact: true }).click();
  await expect(card.getByRole("button", { name: "flagged" })).toBeVisible();

  await page.reload();
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  const reloaded = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await expect(reloaded.getByRole("button", { name: "flagged" })).toBeVisible();
});

test("flag undo unflags the photo", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");
  const card = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await card.getByRole("button", { name: "remove" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Flag", exact: true }).click();
  await expect(card.getByRole("button", { name: "flagged" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toContainText("Undid flag");
  await expect(card.getByRole("button", { name: "remove" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Undo" })).toBeHidden();
});

test("trash undo restores the asset", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".batch-bar").getByRole("button", { name: "Trash", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.getByRole("status")).toContainText("moved to Trash");
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toContainText("Restored");
  await expect(page.getByRole("button", { name: "Undo" })).toBeHidden();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});

test("unassign undo reattaches the face", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  await expect(page.getByText("4 faces")).toBeVisible();
  await page.getByRole("button", { name: "Remove this face from Cluster 1" }).first().click();
  await expect(page.getByText("Removed that face from this person")).toBeVisible();
  await expect(page.getByText("3 faces")).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByText("Reattached that face")).toBeVisible();
  await expect(page.getByText("4 faces")).toBeVisible();
});

test("merge undo splits the clusters back apart", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("checkbox", { name: "Select cluster 1 for merge" }).check();
  await page.getByRole("checkbox", { name: "Select cluster 2 for merge" }).check();
  await page.getByRole("button", { name: "Merge into #1" }).click();
  await expect(page.getByText("Cluster 2")).toBeHidden();
  await expect(page.getByText("6 faces", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByText("Split the merge back apart")).toBeVisible();
  await expect(page.getByText("4 faces")).toBeVisible();
  await expect(page.getByText("2 faces")).toBeVisible();
});

test("a new destructive action replaces the pending undo", async ({ page }) => {
  // Flag first: Undo is pending for the flagged photo.
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");
  const card = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await card.getByRole("button", { name: "remove" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Flag", exact: true }).click();
  await expect(card.getByRole("button", { name: "flagged" })).toBeVisible();
  // Trash second: the pending flag undo is discarded.
  await page.getByLabel(/Deleting moves files/).selectOption("trash");
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".batch-bar").getByRole("button", { name: "Trash", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.getByRole("status")).toContainText("moved to Trash");
  // Undo now restores the trash; the flag stands.
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByRole("status")).toContainText("Restored");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await expect(page.locator("figure", { hasText: "IMG_20261007_185824.jpg" }).getByRole("button", { name: "flagged" })).toBeVisible();
});

test("Clear empties the batch selection", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185822.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await expect(batchBar.getByText("2 selected")).toBeVisible();
  await batchBar.getByRole("button", { name: "Clear", exact: true }).click();
  await expect(batchBar.getByText("0 selected")).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" })).not.toBeChecked();
  await expect(page.getByRole("checkbox", { name: "Select IMG_20261007_185822.jpg" })).not.toBeChecked();
});

test("Done exits select mode and drops the selection", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await expect(page.locator(".batch-bar").getByText("1 selected")).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.locator(".batch-bar")).toBeHidden();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await expect(page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" })).not.toBeChecked();
});

test("grid arrow keys move focus across the library", async ({ page }) => {
  const thumbs = page.locator(".grid figure button.thumb");
  await thumbs.first().focus();
  await expect(thumbs.first()).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await expect(thumbs.nth(1)).toBeFocused();
  await page.keyboard.press("ArrowLeft");
  await expect(thumbs.first()).toBeFocused();
});

test("shift-range selects a run in select mode", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  const thumbs = page.locator(".grid figure button.thumb");
  await thumbs.first().focus();
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.locator(".batch-bar").getByText("2 selected")).toBeVisible();
  await page.keyboard.press("Shift+ArrowRight");
  await expect(page.locator(".batch-bar").getByText("3 selected")).toBeVisible();
});

test("select-all selects the visible set in select mode", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.locator(".grid figure button.thumb").first().focus();
  await page.keyboard.press("ControlOrMeta+a");
  await expect(page.locator(".batch-bar").getByText("8 selected")).toBeVisible();
});

test("Delete opens the confirm dialog for the selection", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".grid figure button.thumb").first().focus();
  await page.keyboard.press("Delete");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("moves the file to the macOS Trash")).toBeVisible();
});

test("S stars the focused photo", async ({ page }) => {
  await page.locator(".grid figure button.thumb").first().focus();
  await page.keyboard.press("s");
  await expect(page.locator(".grid figure").first().getByRole("button", { name: /^Unstar / })).toBeVisible();
});

test("people can be named and merged", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("checkbox", { name: "Select cluster 1 for merge" }).check();
  await page.getByRole("checkbox", { name: "Select cluster 2 for merge" }).check();
  await page.getByRole("button", { name: "Merge into #1" }).click();
  await expect(page.getByText("Cluster 2")).toBeHidden();
});

test("cluster cards expand and jump to the library", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await page.getByRole("button", { name: "View in Library" }).first().click();
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
});

test("pet detection surfaces sightings and filters", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find pets" }).click();
  await expect(page.getByRole("heading", { name: "Pets" })).toBeVisible();
  await expect(page.getByText("Dog · 93%")).toBeVisible();
  await page.getByRole("tab", { name: "Library", exact: true }).click();
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

test("indexed folders collapse, expand, and remember across reload", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Indexed folders" })).toBeVisible();
  await expect(page.getByText("/photos", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Hide" }).click();
  await expect(page.getByText("/photos", { exact: true })).toHaveCount(0);
  const show = page.getByRole("button", { name: /^Show \(\d+\)$/ });
  await expect(show).toBeVisible();
  await page.reload();
  await expect(page.getByText("/photos", { exact: true })).toHaveCount(0);
  await show.click();
  await expect(page.getByText("/photos", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Hide" })).toBeVisible();
});

test("group by sections the grid and clears back to flat", async ({ page }) => {
  const group = page.getByLabel("Group by");
  await group.selectOption("media");
  await expect(page.getByRole("heading", { name: "Photos (6)" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Videos (2)" })).toBeVisible();
  await group.selectOption("date");
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await group.selectOption("location");
  await expect(page.getByRole("heading", { name: "San Francisco, California (4)" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "No location (4)" })).toBeVisible();
  await group.selectOption("none");
  await expect(page.locator(".group-head")).toHaveCount(0);
  await expect(page.locator(".grid figure")).toHaveCount(8);
});

test("group by person sections named clusters from untagged", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Cluster 1", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Person name" });
  await dialog.getByLabel("Person name").fill("Ada");
  await dialog.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByLabel("Group by").selectOption("person");
  await expect(page.getByRole("heading", { name: "Ada (4)" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "No one tagged (4)" })).toBeVisible();
});

test("group headers select and clear the whole section in one go", async ({ page }) => {
  await page.getByLabel("Group by").selectOption("media");
  const photos = page.getByRole("checkbox", { name: "Select Photos" });
  await photos.check();
  await expect(page.getByText("6 selected")).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Select Videos" })).not.toBeChecked();
  // Clearing the section leaves the other section's selection alone.
  await page.getByRole("checkbox", { name: "Select Videos" }).check();
  await expect(page.getByText("8 selected")).toBeVisible();
  await photos.uncheck();
  await expect(page.getByText("2 selected")).toBeVisible();
});

test("slideshow plays from the viewer with pause and step", async ({ page }) => {
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  const viewer = page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" });
  await expect(viewer).toBeVisible();
  await viewer.getByRole("button", { name: "Play slideshow" }).click();
  const show = page.getByRole("dialog", { name: "Slideshow" });
  await expect(show).toBeVisible();
  await expect(show.getByText("1 of 8")).toBeVisible();
  // Freeze autoplay, then stepping is deterministic.
  await show.getByRole("button", { name: "Pause" }).click();
  await expect(show.getByRole("button", { name: "Play" })).toBeVisible();
  await show.getByRole("button", { name: "Next" }).click();
  await expect(show.getByText("2 of 8")).toBeVisible();
  await show.getByRole("button", { name: "Close slideshow" }).click();
  await expect(show).toBeHidden();
});

test("slideshow plays just the current selection", async ({ page }) => {
  await page.getByLabel("Group by").selectOption("media");
  await page.getByRole("checkbox", { name: "Select Photos" }).check();
  await expect(page.getByText("6 selected")).toBeVisible();
  await page.getByRole("button", { name: "▶ Play" }).click();
  const show = page.getByRole("dialog", { name: "Slideshow" });
  await expect(show).toBeVisible();
  await expect(show.getByText("1 of 6")).toBeVisible();
  await show.getByRole("button", { name: "Pause" }).click();
  await show.getByRole("button", { name: "Close slideshow" }).click();
  await expect(show).toBeHidden();
  // Selection survives the show.
  await expect(page.getByText("6 selected")).toBeVisible();
});

test("slideshow locks page scroll with no theater scrollbar", async ({ page }) => {
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  const viewer = page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" });
  await viewer.getByRole("button", { name: "Play slideshow" }).click();
  const show = page.getByRole("dialog", { name: "Slideshow" });
  await expect(show).toBeVisible();
  await show.getByRole("button", { name: "Pause" }).click();
  // Body scroll is locked while the theater is open…
  await expect.poll(() => page.evaluate(() => document.body.style.overflow)).toBe("hidden");
  // …and the theater itself never scrolls.
  await expect(show).toHaveCSS("overflow", "hidden");
  await show.getByRole("button", { name: "Close slideshow" }).click();
  await expect(show).toBeHidden();
  // …and the lock is released on close.
  await expect.poll(() => page.evaluate(() => document.body.style.overflow)).toBe("");
});

test("slideshow zooms with buttons, keys and wheel, resets on advance", async ({ page }) => {
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  const viewer = page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" });
  await viewer.getByRole("button", { name: "Play slideshow" }).click();
  const show = page.getByRole("dialog", { name: "Slideshow" });
  await expect(show).toBeVisible();
  await show.getByRole("button", { name: "Pause" }).click();
  const reset = show.getByRole("button", { name: "Reset zoom" });
  await expect(reset).toHaveText("100%");
  await show.getByRole("button", { name: "Zoom in" }).click();
  await expect(reset).toHaveText("125%");
  await expect(show.locator(".theater-stage img")).toHaveCSS("transform", /1\.25/);
  await page.keyboard.press("-");
  await expect(reset).toHaveText("100%");
  await show.locator(".theater-stage").hover();
  await page.mouse.wheel(0, -120);
  await expect(reset).toHaveText("125%");
  // Drag pans 1:1 with the cursor (matrix tx equals the drag distance).
  const box = await show.locator(".theater-stage img").boundingBox();
  const cx = box!.x + box!.width / 2, cy = box!.y + box!.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx + 60, cy);
  await page.mouse.up();
  await expect(show.locator(".theater-stage img")).toHaveCSS("transform", /matrix\(1\.25, 0, 0, 1\.25, 60/);
  // Advancing to the next slide resets zoom.
  await show.getByRole("button", { name: "Next" }).click();
  await expect(reset).toHaveText("100%");
  await show.getByRole("button", { name: "Close slideshow" }).click();
  await expect(show).toBeHidden();
});

test("map tab plots located photos and jumps to the library", async ({ page }) => {
  await page.getByRole("tab", { name: "Map", exact: true }).click();
  const map = page.locator(".leaflet-container");
  await expect(map).toBeVisible();
  await expect(map.locator(".leaflet-control-attribution")).toHaveCount(0);
  await expect.poll(() => map.locator(".leaflet-tile-pane img").count()).toBeGreaterThan(0);
  // Pins render on canvas (smooth per-frame redraw while panning/zooming).
  await expect(map.locator("canvas")).toBeAttached();
  // Drive a real click through the first pin pixel: popup opens, jump works.
  const box = await map.boundingBox();
  const pt = await page.evaluate(() => {
    const w = window as unknown as {
      __ffMap?: { latLngToContainerPoint(a: [number, number]): { x: number; y: number } };
      __ffMapPins?: [number, number][];
    };
    return w.__ffMap!.latLngToContainerPoint(w.__ffMapPins![0]);
  });
  await page.mouse.click(box!.x + pt.x, box!.y + pt.y);
  await page.getByRole("button", { name: "Show in Library" }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
});

test("map clusters nearby pins when zoomed out and dissolves on click", async ({ page }) => {
  await page.getByRole("tab", { name: "Map", exact: true }).click();
  const map = page.locator(".leaflet-container");
  await expect(map).toBeVisible();
  await expect.poll(() => map.locator(".leaflet-tile-pane img").count()).toBeGreaterThan(0);
  // Fitted to the pins: every photo plots individually, no clusters.
  await expect(map.locator(".map-cluster")).toHaveCount(0);
  // Pull back to a city view: the 4 nearby located pins merge into one badge.
  // animate:false: headless never settles Leaflet's zoom animation, so the
  // test drives synchronous transitions (production keeps the animation).
  await page.evaluate(() => {
    const w = window as unknown as {
      __ffMap?: { setView(a: [number, number], z: number, o?: { animate?: boolean }): void };
    };
    w.__ffMap!.setView([37.78, -122.42], 5, { animate: false });
  });
  const cluster = map.locator(".map-cluster");
  await expect(cluster).toHaveCount(1);
  await expect(cluster).toHaveText("4");
  // Clicking the badge zooms toward street level.
  const before = await page.evaluate(
    () => (window as unknown as { __ffMap?: { getZoom(): number } }).__ffMap!.getZoom()
  );
  await cluster.click();
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __ffMap?: { getZoom(): number } }).__ffMap!.getZoom())
    )
    .toBeGreaterThan(before);
  // Let the click's zoom animation settle: teleporting mid-flight leaves
  // the animated zoom fighting the synchronous one.
  await expect
    .poll(() =>
      page.evaluate(() => (window as unknown as { __ffMap?: { getZoom(): number } }).__ffMap!.getZoom())
    )
    .toBe(before + 2);
  await page.waitForTimeout(400);
  // Back at street level the badge is gone and pins plot individually.
  // z17: the cluster cell (0.0007°) drops below the closest pin spacing.
  await page.evaluate(() => {
    const w = window as unknown as {
      __ffMap?: { setView(a: [number, number], z: number, o?: { animate?: boolean }): void };
    };
    w.__ffMap!.setView([37.78, -122.42], 17, { animate: false });
  });
  await expect(map.locator(".map-cluster")).toHaveCount(0);
});

test("map layer toggle switches streets and terrain", async ({ page }) => {
  await page.getByRole("tab", { name: "Map", exact: true }).click();
  const map = page.locator(".leaflet-container");
  await expect(map).toBeVisible();
  await expect(page.getByRole("button", { name: "Streets" })).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Terrain" }).click();
  await expect(page.getByRole("button", { name: "Terrain" })).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByText("Topo tiles © OpenStreetMap contributors")).toBeVisible();
  await expect
    .poll(() => map.locator(".leaflet-tile-pane img").first().getAttribute("src"))
    .toContain("topo");
  await page.getByRole("button", { name: "Streets" }).click();
  await expect(page.getByText("Tiles © OpenStreetMap contributors")).toBeVisible();
  await expect
    .poll(() => map.locator(".leaflet-tile-pane img").first().getAttribute("src"))
    .not.toContain("topo");
});

test("indexed folder removal forgets the root and can be cancelled", async ({ page }) => {
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  const folders = page.locator("#indexed-folders");
  await expect(folders.getByText("/mock/vacation", { exact: true })).toBeVisible();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
  // Cancel leaves everything alone.
  await page.getByRole("button", { name: "Remove /mock/vacation from the index" }).click();
  const dialog = page.getByRole("dialog", { name: "Remove indexed folder" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(folders.getByText("/mock/vacation", { exact: true })).toBeVisible();
  // Confirm forgets the root's assets and drops the row.
  await page.getByRole("button", { name: "Remove /mock/vacation from the index" }).click();
  await dialog.getByRole("button", { name: "Remove" }).click();
  await expect(folders.getByText("/mock/vacation", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
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
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Resolve place names" }).click();
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("searchbox", { name: "Search library" }).fill("san francisco");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
});

test("event place names resolve from coordinates", async ({ page }) => {
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
  await expect(page.getByText("37.7749, -122.4194")).toBeVisible();
  await page.getByRole("button", { name: "Resolve place names" }).click();
  await expect(page.getByText("San Francisco, California")).toBeVisible();
});

test("highlight video reports photos and clips", async ({ page }) => {
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Highlight video" }).click();
  await expect(page.getByText("3 photos")).toBeVisible();
  await expect(page.getByText("1 video clip")).toBeVisible();
});

test("events render as a month-grouped timeline", async ({ page }) => {
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByRole("heading", { name: "October 2025" })).toBeVisible();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
});

test("album workflow: create, add photos, show, rename, delete", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
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
  await page.getByRole("tab", { name: "Events", exact: true }).click();
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
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
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
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
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
  await page.getByRole("tab", { name: /^Albums/ }).click();
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
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await expect(page.getByRole("heading", { name: "People" })).toBeVisible();
  await expect(page.getByText("No people clusters yet — scan the library to detect faces.")).toBeVisible();
  await expect(page.getByText("Cluster 1")).toBeHidden();
});

test("face clustering renders two clusters with face counts", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await expect(page.getByText("Cluster 2")).toBeVisible();
  // Seed has 6 photos: first four in cluster 1, last two in cluster 2.
  await expect(page.getByText("4 faces")).toBeVisible();
  await expect(page.getByText("2 faces")).toBeVisible();
});

test("renaming a cluster persists in the UI", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: "Cluster 1", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Person name" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Person name").fill("Ada");
  await dialog.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
  await expect(page.getByText("Cluster 1")).toBeHidden();
  // Rename survives leaving and returning to the tab.
  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("tab", { name: /^People/ }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
});

test("rename offers known people to select instead of retyping", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  // Name cluster 1 first so cluster 2's dialog has a suggestion.
  await page.getByRole("button", { name: "Cluster 1", exact: true }).click();
  let dialog = page.getByRole("dialog", { name: "Person name" });
  await dialog.getByLabel("Person name").fill("Ada");
  await dialog.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Ada").first()).toBeVisible();
  // Cluster 2's dialog suggests Ada; selecting it fills the input.
  await page.getByRole("button", { name: "Cluster 2", exact: true }).click();
  dialog = page.getByRole("dialog", { name: "Person name" });
  await expect(dialog.getByText("Known people — select instead of retyping:")).toBeVisible();
  await dialog.getByRole("button", { name: "Ada" }).click();
  await expect(dialog.getByLabel("Person name")).toHaveValue("Ada");
  await dialog.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("Cluster 2")).toBeHidden();
});

test("cluster source photos expand", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
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
  await page.getByRole("tab", { name: "People", exact: true }).click();
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

test("removing a face that doesn't belong shrinks the cluster", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  await expect(page.getByText("4 faces")).toBeVisible();
  await page.getByRole("button", { name: "Remove this face from Cluster 1" }).first().click();
  await expect(page.getByText("Removed that face from this person")).toBeVisible();
  await expect(page.getByText("3 faces")).toBeVisible();
  await expect(page.getByText("Cluster 1")).toBeVisible();
});

test("removing a face keeps the collapsed preview compact", async ({ page }) => {
  // Two imports put 12 faces in cluster 1; the collapsed card previews 8.
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/vacation");
  await page.getByRole("button", { name: "Scan" }).click();
  await page.getByPlaceholder("…or paste a folder path").fill("/mock/trip");
  await page.getByRole("button", { name: "Scan" }).click();
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("12 faces")).toBeVisible();
  const card = page.locator("article", { hasText: "Cluster 1" });
  await expect(card.locator("figure")).toHaveCount(8);
  // Removing one must not explode the preview to the full list.
  await card.getByRole("button", { name: "Remove this face from Cluster 1" }).first().click();
  await expect(page.getByText("Removed that face from this person")).toBeVisible();
  await expect(page.getByText("11 faces")).toBeVisible();
  await expect(card.locator("figure")).toHaveCount(8);
});

test("cluster review steps through every photo with per-photo reject", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Review all" }).first().click();
  await expect(page.getByText("1 of 4 · reviewing Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.getByText("2 of 4 · reviewing Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: /Not Cluster 1/ }).click();
  await expect(page.getByText(/Removed .* from Cluster 1/)).toBeVisible();
  await expect(page.getByText("2 of 3 · reviewing Cluster 1")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByText("3 faces")).toBeVisible();
});

test("faces on unreadable files say so instead of loading forever", async ({ page }) => {
  // Import from a root whose files cannot be decoded (mock mirrors the
  // engine's "cannot decode image" failure for UNREADABLE paths).
  await page.getByPlaceholder("…or paste a folder path").fill("/UNREADABLE");
  await page.getByRole("button", { name: "Scan" }).click();
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();

  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  // Face grids render for every loaded cluster; all 4 UNREADABLE faces (2 in
  // cluster 1, 2 in cluster 2) report unreadable while healthy faces render.
  await expect(page.getByTitle("Source file unreadable")).toHaveCount(4);
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});

test("view-in-library filters to the selected person", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
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
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("BatchTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("BatchTarget").first()).toBeVisible();

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185822.jpg" }).check();
  await expect(page.getByText("2 selected")).toBeVisible();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByLabel("Add selected to album").selectOption({ label: "BatchTarget" });
  await batchBar.getByRole("button", { name: "Add to album" }).click();
  await expect(page.getByText("Added 2 photos to the album")).toBeVisible();

  await page.getByRole("tab", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "BatchTarget" });
  await expect(card.getByText("2 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("IMG_20261007_185822.jpg")).toBeVisible();
});

test("picker search-toggle-confirm adds to an album", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
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
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("EventTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("EventTarget").first()).toBeVisible();

  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await expect(page.getByText("InlawsVisit2026")).toBeVisible();
  await page.getByRole("button", { name: "Show photos" }).click();
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
  await page.getByLabel("Add InlawsVisit2026 to album").selectOption({ label: "EventTarget" });
  await page.getByRole("button", { name: "＋ Album" }).click();
  await expect(page.getByText('Added 8 photos to “EventTarget”')).toBeVisible();

  await page.getByRole("tab", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "EventTarget" });
  await expect(card.getByText("8 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("VID_20261007_192001.mp4")).toBeVisible();
});

test("indexed folder adds everything to an album via ＋ All to album", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("FolderTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("FolderTarget").first()).toBeVisible();

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  const row = page.locator(".roots li", { hasText: "/photos" });
  await row.getByLabel("Add everything under /photos to album").selectOption({ label: "FolderTarget" });
  await row.getByRole("button", { name: "＋ All to album" }).click();
  await expect(page.getByText("Added 8 photos from /photos to “FolderTarget”")).toBeVisible();

  await page.getByRole("tab", { name: /^Albums/ }).click();
  const card = page.locator(".album-card", { hasText: "FolderTarget" });
  await expect(card.getByText("8 photos")).toBeVisible();
  await card.getByRole("button", { name: "Show photos" }).click();
  await expect(card.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  await expect(card.getByText("VID_20261007_192001.mp4")).toBeVisible();
});

test("filtered library selection adds only the visible set", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("FilteredTarget");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("FilteredTarget").first()).toBeVisible();

  await page.getByRole("tab", { name: "Library", exact: true }).click();
  await page.getByRole("searchbox", { name: "Search library" }).fill("185817");
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  const batchBar = page.locator(".batch-bar");
  await batchBar.getByLabel("Add selected to album").selectOption({ label: "FilteredTarget" });
  await batchBar.getByRole("button", { name: "Add to album" }).click();
  await expect(page.getByText("Added 1 photo to the album")).toBeVisible();

  await page.getByRole("tab", { name: /^Albums/ }).click();
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
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await expect(page.getByText("Cluster 1")).toBeVisible();
  await page.getByRole("button", { name: "Cluster 1", exact: true }).click();
  const rename = page.getByRole("dialog", { name: "Person name" });
  await rename.getByLabel("Person name").fill("FilterMe");
  await rename.getByRole("button", { name: "Rename" }).click();
  await expect(page.getByText("FilterMe").first()).toBeVisible();

  await page.getByRole("tab", { name: "Library", exact: true }).click();
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
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find pets" }).click();
  await expect(page.getByText("Dog · 93%")).toBeVisible();
  await page.getByRole("tab", { name: "Library", exact: true }).click();
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

test("videos offer no Similar button (similarity is photos-only)", async ({ page }) => {
  await expect(page.getByRole("button", { name: "Similar to VID_20261007_192001.mp4" })).toHaveCount(0);
  await page.getByTitle("Open VID_20261007_192001.mp4").click();
  const viewer = page.getByRole("dialog", { name: "VID_20261007_192001.mp4" });
  await expect(viewer).toBeVisible();
  await expect(viewer.getByRole("button", { name: /^Similar to / })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(viewer).toBeHidden();
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

// ---------------------------------------------------------------------------
// Issue #6 minor roll-up
// ---------------------------------------------------------------------------

test("toast auto-dismisses when no undo is pending", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("ToastAlbum");
  await dialog.getByRole("button", { name: "Create" }).click();
  const toast = page.getByRole("status");
  await expect(toast).toContainText("Created");
  await expect(toast).toBeHidden({ timeout: 10_000 });
});

test("toast persists while an undo is pending", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByLabel(/Deleting moves files/).selectOption("flag");
  const card = page.locator("figure", { hasText: "IMG_20261007_185824.jpg" });
  await card.getByRole("button", { name: "remove" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Flag", exact: true }).click();
  const toast = page.getByRole("status");
  await expect(toast).toContainText("flagged only");
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
  // Past the ~6s auto-dismiss: the undo toast must still be up.
  await page.waitForTimeout(7_000);
  await expect(toast).toBeVisible();
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
});

test("kind filter persists across reload with a summary", async ({ page }) => {
  await page.getByRole("button", { name: /6 photos/ }).click();
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  await expect(page.getByText(/Filters:/)).toContainText("photos");
  await page.reload();
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  await expect(page.getByText(/Filters:/)).toContainText("photos");
});

test("person jump shows a filter chip that clear-all resets", async ({ page }) => {
  await page.getByRole("tab", { name: "People", exact: true }).click();
  await page.getByRole("button", { name: "Find people in library" }).click();
  await page.getByRole("button", { name: "View in Library" }).first().click();
  await expect(page.getByText("Showing 4 of 4")).toBeVisible();
  // The person filter is active even though the person dropdown has no
  // options yet — the summary chip makes it visible.
  await expect(page.getByText(/Filters:/)).toBeVisible();
  await page.getByRole("button", { name: "Clear all filters" }).click();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByText(/Filters:/)).toBeHidden();
});

test("outside-app error explains the missing bridge in plain language", async ({ page }) => {
  // Engine/sidecar failures (ENOENT, dead sidecar, trash partials) cannot be
  // produced by the in-memory mock — this covers the plain-language path.
  await page.goto("/");
  await expect(page.getByRole("alert")).toContainText("fotoflow app window");
});

// ---------------------------------------------------------------------------
// Frozen layout shell: sidebar rail + density
// ---------------------------------------------------------------------------

test("sidebar rail renders all 7 tabs, collapses, and persists width", async ({ page }) => {
  const tablist = page.getByRole("tablist", { name: "Library sections" });
  await expect(tablist).toBeVisible();
  await expect(page.getByRole("tab", { name: "Library", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: /^Cleanup/ })).toBeVisible();
  await expect(page.getByRole("tab", { name: /^People/ })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Events", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Map", exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: /^Albums/ })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Settings", exact: true })).toBeVisible();

  // Width persists across reload.
  await page.evaluate(() => localStorage.setItem("fotoflow-sidebar-width", "280"));
  await page.reload();
  const sidebar = page.locator(".sidebar");
  await expect(sidebar).toBeVisible();
  await expect(sidebar).toHaveCSS("width", "280px");

  // Collapse works and persists.
  await page.getByRole("button", { name: "Collapse sidebar" }).click();
  await expect(page.getByRole("button", { name: "Expand sidebar" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("button", { name: "Expand sidebar" })).toBeVisible();
  await page.getByRole("button", { name: "Expand sidebar" }).click();
  await expect(page.getByRole("button", { name: "Collapse sidebar" })).toBeVisible();
  await expect(page.locator(".sidebar")).toHaveCSS("width", "280px");
});

test("density toggle switches grid minima without resetting filters", async ({ page }) => {
  const grid = page.locator(".grid").first();
  await expect(grid).toHaveAttribute("data-density", "comfortable");
  // Narrow to photos, then switch density: the filter must survive.
  await page.getByRole("button", { name: /6 photos/ }).click();
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  await page.getByRole("button", { name: "Compact", exact: true }).click();
  await expect(grid).toHaveAttribute("data-density", "compact");
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  // Density persists across reload along with the filter.
  await page.reload();
  await expect(page.locator(".grid").first()).toHaveAttribute("data-density", "compact");
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
  await page.getByRole("button", { name: "Comfortable", exact: true }).click();
  await expect(page.locator(".grid").first()).toHaveAttribute("data-density", "comfortable");
  await expect(page.getByText("Showing 6 of 6")).toBeVisible();
});

test("details toggle swaps captioned cards and wall-to-wall photos", async ({ page }) => {
  const grid = page.locator(".grid").first();
  const details = page.getByRole("button", { name: "Toggle photo details" });
  // Details on by default: names, stars and meta under photos.
  await expect(details).toHaveAttribute("aria-pressed", "true");
  await expect(grid).toHaveAttribute("data-details", "true");
  await expect(grid.getByText("IMG_20261007_185817.jpg")).toBeVisible();
  // Off: wall-to-wall photos, selection still works.
  await details.click();
  await expect(details).toHaveAttribute("aria-pressed", "false");
  await expect(grid).toHaveAttribute("data-details", "false");
  await expect(grid.getByText("IMG_20261007_185817.jpg")).toBeHidden();
  await expect(grid.getByRole("button", { name: /Star|Unstar/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await expect(page.locator(".batch-bar").getByText("1 selected")).toBeVisible();
  // Persists across reload, then back on.
  await page.reload();
  await expect(page.locator(".grid").first()).toHaveAttribute("data-details", "false");
  await page.getByRole("button", { name: "Toggle photo details" }).click();
  await expect(page.locator(".grid").first().getByText("IMG_20261007_185817.jpg")).toBeVisible();
});

// ---------------------------------------------------------------------------
// Issue #7: side-by-side duplicate review + uniform cleanup preview sizes
// ---------------------------------------------------------------------------

test("compare panels render keeper and candidate panes with names", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  await page.getByRole("button", { name: "Find similar photos" }).click();
  // Duplicate group: keeper pane labeled Keeper, candidate pane named.
  const dup = page.locator("article.group", { hasText: "IMG_20261007_191434_copy.jpg" });
  const dupCompare = dup.locator("[data-compare]");
  await expect(dupCompare.locator('[data-pane="keeper"]')).toContainText("Keeper");
  await expect(dupCompare.locator('[data-pane="keeper"] img')).toHaveAttribute(
    "alt",
    "IMG_20261007_191434.jpg"
  );
  await expect(dupCompare.locator('[data-pane="candidate"]')).toContainText("IMG_20261007_191434_copy.jpg");
  await expect(dupCompare.locator('[data-pane="keeper"] img')).toBeVisible();
  await expect(dupCompare.locator('[data-pane="candidate"] img')).toBeVisible();
  // Similar group: same two-pane shape with its own keeper and candidate.
  const near = page.locator("article.group").filter({ hasText: "similar" });
  const nearCompare = near.locator("[data-compare]");
  await expect(nearCompare.locator('[data-pane="keeper"]')).toContainText("Keeper");
  await expect(nearCompare.locator('[data-pane="keeper"] img')).toHaveAttribute(
    "alt",
    "IMG_20261007_185824.jpg"
  );
  await expect(nearCompare.locator('[data-pane="candidate"]')).toContainText("IMG_20261007_191433.jpg");
  await expect(nearCompare.locator('[data-pane="candidate"] img')).toBeVisible();
});

test("picking another member updates the candidate pane", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  const dup = page.locator("article.group", { hasText: "IMG_20261007_191434_copy.jpg" });
  const cmp = dup.locator("[data-compare]");
  await expect(cmp.locator('[data-pane="candidate"]')).toContainText("IMG_20261007_191434_copy.jpg");
  // Strip buttons are keyboard-focusable.
  const keeperStrip = dup.getByRole("button", { name: "Show IMG_20261007_191434.jpg in compare" });
  await keeperStrip.focus();
  await expect(keeperStrip).toBeFocused();
  await keeperStrip.click();
  await expect(cmp.locator('[data-pane="candidate"]')).toContainText("IMG_20261007_191434.jpg");
});

test("keep-swap updates the keeper pane", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  const dup = page.locator("article.group", { hasText: "IMG_20261007_191434_copy.jpg" });
  const cmp = dup.locator("[data-compare]");
  await expect(cmp.locator('[data-pane="keeper"] img')).toHaveAttribute("alt", "IMG_20261007_191434.jpg");
  await cmp.getByRole("button", { name: "Keep this one instead" }).click();
  await expect(cmp.locator('[data-pane="keeper"] img')).toHaveAttribute(
    "alt",
    "IMG_20261007_191434_copy.jpg"
  );
});

test("cleanup previews share one uniform size", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Find exact duplicates" }).click();
  await page.getByRole("button", { name: "Find similar photos" }).click();
  const dup = page.locator("article.group", { hasText: "IMG_20261007_191434_copy.jpg" });
  const near = page.locator("article.group").filter({ hasText: "similar" });
  const blurredH3 = page.getByRole("heading", { name: "Likely blurred" });
  await expect(dup.locator(".cleanup-preview").first()).toBeVisible();
  await expect(near.locator(".cleanup-preview").first()).toBeVisible();
  await expect(blurredH3).toBeVisible();
  const heights = await page.evaluate(() => {
    const first = (root: ParentNode | null) => root?.querySelector(".cleanup-preview") ?? null;
    const dupEl = first(document.querySelector("article.group"));
    const groups = [...document.querySelectorAll("article.group")];
    const nearEl = groups.length > 1 ? first(groups[groups.length - 1]) : null;
    const h3 = [...document.querySelectorAll("h3.sub")].find((h) =>
      h.textContent?.includes("Likely blurred")
    );
    const blurredEl = h3?.nextElementSibling ? first(h3.nextElementSibling) : null;
    return [dupEl, nearEl, blurredEl].map((el) => el?.getBoundingClientRect().height ?? -1);
  });
  expect(heights.every((h) => h > 0)).toBe(true);
  expect(Math.max(...heights) - Math.min(...heights)).toBeLessThanOrEqual(2);
});

// ---------------------------------------------------------------------------
// Issue #8: Apple Photos-style library zoom + infinite scroll
// ---------------------------------------------------------------------------

test("library zoom cycles all to year with correct headers and disabled ends", async ({ page }) => {
  const zoomOut = page.getByRole("button", { name: "Zoom out timeline" });
  const zoomIn = page.getByRole("button", { name: "Zoom in timeline" });
  // "all" is the current ungrouped grid: no section headers.
  await expect(page.locator(".group-head")).toHaveCount(0);
  await expect(zoomOut).toBeDisabled();
  await expect(zoomIn).toBeEnabled();
  // all → day: one section per UTC calendar day.
  await zoomIn.click();
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(zoomOut).toBeEnabled();
  // day → month.
  await zoomIn.click();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  // month → year.
  await zoomIn.click();
  await expect(page.getByRole("heading", { name: "2025 (8)" })).toBeVisible();
  await expect(zoomIn).toBeDisabled();
  // Back down with no wrap: year → month → day → all.
  await zoomOut.click();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await zoomOut.click();
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(zoomIn).toBeEnabled();
  await zoomOut.click();
  await expect(page.locator(".group-head")).toHaveCount(0);
  await expect(zoomOut).toBeDisabled();
});

test("library zoom granularity persists across reload", async ({ page }) => {
  const zoomIn = page.getByRole("button", { name: "Zoom in timeline" });
  await zoomIn.click();
  await zoomIn.click();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Zoom out timeline" })).toBeEnabled();
});

test("ctrl+wheel over the grid zooms the timeline, plain wheel only scrolls", async ({ page }) => {
  const grid = page.locator(".grid");
  await expect(page.locator(".group-head")).toHaveCount(0);
  // Ctrl+wheel up = zoom in (all → day), matching the + button.
  await grid.dispatchEvent("wheel", { deltaY: -80, ctrlKey: true, bubbles: true, cancelable: true });
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await page.waitForTimeout(350);
  // Ctrl+wheel down = zoom out (day → all).
  await grid.dispatchEvent("wheel", { deltaY: 80, ctrlKey: true, bubbles: true, cancelable: true });
  await expect(page.locator(".group-head")).toHaveCount(0);
  // Plain wheel never changes zoom.
  await grid.dispatchEvent("wheel", { deltaY: 400, bubbles: true, cancelable: true });
  await expect(page.locator(".group-head")).toHaveCount(0);
});

test("library zoom keyboard shortcuts move levels and respect viewer and dialogs", async ({ page }) => {
  // Wait for the app to be interactive so the first keypress isn't lost.
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.locator(".group-head")).toHaveCount(0);
  // Ctrl/Cmd+= zooms in (all → day), Ctrl/Cmd+− zooms back out.
  await page.keyboard.press("ControlOrMeta+=");
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+=");
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+-");
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  // The open viewer swallows the shortcut: the grid behind stays put.
  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  await expect(page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+=");
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  // An open confirm dialog swallows it too.
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".batch-bar").getByRole("button", { name: "Trash", exact: true }).click();
  await expect(page.getByRole("alertdialog")).toBeVisible();
  await page.keyboard.press("ControlOrMeta+=");
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toHaveCount(0);
  await page.keyboard.press("Escape");
});

test("library infinite scroll appends past the first page", async ({ page }) => {
  await page.goto("/?web=1&pageSize=4");
  await expect(page.getByText("Showing 4 of 8")).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(4);
  await page.locator("#grid-sentinel").scrollIntoViewIfNeeded();
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(8);
});

test("zoomed group headers select the whole section and filters still apply", async ({ page }) => {
  await page.getByRole("button", { name: "Zoom in timeline" }).click();
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  // One-go group selection works at day granularity.
  await page.getByRole("checkbox", { name: "Select October 9, 2025" }).check();
  await expect(page.getByText("8 selected")).toBeVisible();
  await page.getByRole("checkbox", { name: "Select October 9, 2025" }).uncheck();
  await expect(page.getByText("0 selected")).toBeVisible();
  // The month filter narrows the zoomed grid without breaking sections.
  await page.getByLabel("Filter by month").selectOption("2025-10");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
});

test("month zoom shows a capped highlights reel with true counts", async ({ page }) => {
  await page.goto("/?web=1&highlights=3");
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  const zoomIn = page.getByRole("button", { name: "Zoom in timeline" });
  await zoomIn.click(); // day: the full grid, uncapped.
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(8);
  await zoomIn.click(); // month: curated reel, true header count.
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(3);
  await expect(page.getByRole("button", { name: "Show all October 2025 (8)" })).toBeVisible();
});

test("show-all drills into the period and back restores the reel", async ({ page }) => {
  await page.goto("/?web=1&highlights=3");
  const zoomIn = page.getByRole("button", { name: "Zoom in timeline" });
  await zoomIn.click();
  await zoomIn.click(); // month reel.
  await expect(page.locator(".grid figure")).toHaveCount(3);
  await page.getByRole("button", { name: "Show all October 2025 (8)" }).click();
  // Drilled to the month's days: full grid plus a back banner.
  await expect(page.getByRole("heading", { name: "October 9, 2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(8);
  await expect(page.getByText("Showing 8 of 8")).toBeVisible();
  const banner = page.locator(".drill-banner");
  await expect(banner).toContainText("October 2025");
  await banner.getByRole("button", { name: "Back to months" }).click();
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(3);
});

test("year drill opens that year's months with highlights", async ({ page }) => {
  await page.goto("/?web=1&highlights=3");
  const zoomIn = page.getByRole("button", { name: "Zoom in timeline" });
  await zoomIn.click();
  await zoomIn.click();
  await zoomIn.click(); // year reel.
  await expect(page.getByRole("heading", { name: "2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(3);
  await page.getByRole("button", { name: "Show all 2025 (8)" }).click();
  // That year's months, still highlights; back returns to years.
  await expect(page.getByRole("heading", { name: "October 2025 (8)" })).toBeVisible();
  await expect(page.locator(".grid figure")).toHaveCount(3);
  const banner = page.locator(".drill-banner");
  await expect(banner).toContainText("2025");
  await banner.getByRole("button", { name: "Back to years" }).click();
  await expect(page.getByRole("heading", { name: "2025 (8)" })).toBeVisible();
});

// ---------------------------------------------------------------------------
// Pinned shortcuts (Favorites / Recently Saved / Map / Videos / Screenshots /
// People) + the photo info panel (camera, exposure, GPS, file facts)
// ---------------------------------------------------------------------------

test("pinned shortcuts jump to tabs and library presets", async ({ page }) => {
  // Star one photo so Favorites has something to find.
  await page.getByRole("button", { name: "Star IMG_20261007_185817.jpg" }).click();
  await page.getByRole("button", { name: "Favorites", exact: true }).click();
  await expect(page.getByText("Showing 1 of 1")).toBeVisible();
  await expect(page.getByText("Filters: favorites")).toBeVisible();
  // Recently Saved: newest first_seen_at first (mock: highest id).
  await page.getByRole("button", { name: "Recently Saved", exact: true }).click();
  await expect(page.getByText("Filters: recently added")).toBeVisible();
  await expect(page.locator(".grid figure").first()).toContainText("VID_20261007_204512.mp4");
  // Videos: kind preset.
  await page.getByRole("button", { name: "Videos", exact: true }).click();
  await expect(page.getByText("Showing 2 of 2")).toBeVisible();
  await expect(page.getByText("Filters: videos")).toBeVisible();
  // Screenshots: saved name search (no fixture matches — wiring check).
  await page.getByRole("button", { name: "Screenshots", exact: true }).click();
  await expect(page.getByLabel("Search library")).toHaveValue("screenshot");
  await expect(page.locator(".grid figure")).toHaveCount(0);
  // Map + People jump tabs.
  await page.getByRole("button", { name: "Map", exact: true }).click();
  await expect(page.locator(".leaflet-container")).toBeVisible();
  await page.getByRole("button", { name: "People", exact: true }).click();
  await expect(page.getByRole("tab", { name: /^People/ })).toHaveAttribute("aria-selected", "true");
});

test("photo info shows camera, exposure, gps and file facts", async ({ page }) => {  await page.getByTitle("Open IMG_20261007_185817.jpg").click();
  await expect(page.getByRole("dialog", { name: "IMG_20261007_185817.jpg" })).toBeVisible();
  await page.getByRole("button", { name: "Photo info" }).click();
  const info = page.locator(".info");
  await expect(info).toContainText("iPhone 16 Pro");
  await expect(info).toContainText("ƒ/1.78");
  await expect(info).toContainText("1/120");
  await expect(info).toContainText("ISO 80");
  await expect(info).toContainText("24 mm");
  await expect(info).toContainText("37.7749, -122.4194");
  await expect(info).toContainText("4032×3024");
  // The I key toggles the panel.
  await page.keyboard.press("i");
  await expect(page.locator(".info")).toHaveCount(0);
  await page.keyboard.press("i");
  await expect(page.locator(".info")).toHaveCount(1);
  await page.keyboard.press("Escape");
  // A photo without EXIF shows file facts only — no camera rows. The
  // inspector stays open across photos, so no second click is needed.
  await page.getByTitle("Open IMG_20261007_185822.jpg").click();
  const bare = page.locator(".info");
  await expect(bare).toContainText("Dimensions");
  await expect(bare).toContainText("Size");
  await expect(bare).not.toContainText("iPhone");
  await expect(bare).not.toContainText("Camera");
});

// ---------------------------------------------------------------------------
// Cleanup review mode: keyboard triage queue + staged bulk apply
// ---------------------------------------------------------------------------

test("review triage stages with the keyboard and bulk-applies", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Review all (3)" }).click();
  const review = page.getByRole("region", { name: "Cleanup review" });
  await expect(review).toContainText("Duplicate group 1 of 3 · 2 photos");
  // D stages the extra copy and auto-advances to the similar group.
  await page.keyboard.press("d");
  await expect(review).toContainText("Similar group 2 of 3 · 2 photos");
  await expect(review.getByText("1 photo staged in 1 decision")).toBeVisible();
  // B goes back; D again re-stages and moves on.
  await page.keyboard.press("b");
  await expect(review).toContainText("Duplicate group 1 of 3 · 2 photos");
  await page.keyboard.press("d");
  await expect(review).toContainText("Similar group 2 of 3 · 2 photos");
  // S skips the similar group, landing on the blurred photo.
  await page.keyboard.press("s");
  await expect(review).toContainText("Blurred photo 3 of 3");
  await expect(review).toContainText("IMG_20261007_185824.jpg");
  await page.keyboard.press("d");
  await expect(review.getByText("2 photos staged in 2 decisions")).toBeVisible();
  // One bulk apply: single confirm, single undo entry.
  await review.getByRole("button", { name: "Apply all (A)" }).click();
  await expect(page.getByRole("alertdialog")).toContainText("Move 2 photos in 2 decisions to Trash?");
  await page.getByRole("alertdialog").getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.locator(".toast")).toContainText("moved to Trash");
  await expect(page.getByRole("button", { name: "Undo" })).toBeVisible();
  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.locator(".toast")).toContainText("Restored");
});

test("review keeper swap clears the staged verdict", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Review all (3)" }).click();
  const review = page.getByRole("region", { name: "Cleanup review" });
  await page.keyboard.press("d"); // stage the duplicate extra, advance.
  await expect(review).toContainText("Similar group 2 of 3 · 2 photos");
  // K makes the candidate the keeper: verdict cleared, swap offered back.
  await page.keyboard.press("k");
  await expect(review.getByRole("button", { name: "Keep this one instead" })).toBeVisible();
  await expect(review.getByText("Staged ·", { exact: false })).toHaveCount(0);
  // Re-decide under the new keeper: the other copy stages instead.
  await page.keyboard.press("d");
  await expect(review.getByText("2 photos staged in 2 decisions")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(review).toBeHidden();
  await expect(page.getByRole("heading", { name: "Duplicate groups" })).toBeVisible();
});

// ---------------------------------------------------------------------------
// Trip workflow: SSD ingest, best-of-place picks, space-saver compress
// ---------------------------------------------------------------------------

test("ingest copies to SSD then indexes the copy", async ({ page }) => {
  await page.getByRole("button", { name: "Copy to SSD…" }).click();
  const dialog = page.getByRole("dialog", { name: "Copy photos to fast storage" });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Copy photos from").fill("/mock/sd-card");
  await dialog.getByLabel("Copy photos to").fill("/mock/ssd/trip");
  await dialog.getByRole("button", { name: "Start copy" }).click();
  await expect(dialog).toContainText("Copied 8 · skipped 0 identical");
  await dialog.getByRole("button", { name: "Index this folder" }).click();
  await expect(dialog).toBeHidden();
  // Indexing the copy imports its photos into the library.
  await expect(page.getByText("Showing 12 of 12")).toBeVisible();
});

test("event top picks land in the chosen album", async ({ page }) => {
  await page.getByRole("tab", { name: "Albums", exact: true }).click();
  await page.getByRole("button", { name: "New album" }).click();
  const dialog = page.getByRole("dialog", { name: "New album" });
  await dialog.getByLabel("New album").fill("Trip Best");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByText("Trip Best").first()).toBeVisible();
  await page.getByRole("tab", { name: "Events", exact: true }).click();
  await page.getByRole("button", { name: "Cluster events by time & place" }).click();
  await page.getByRole("button", { name: "Show photos" }).first().click();
  await page.getByRole("button", { name: "＋ Top 12" }).first().click();
  await expect(page.locator(".toast")).toContainText("Added 8 top picks to “Trip Best”");
});

test("space saver compresses dup extras plus blurred", async ({ page }) => {
  await page.getByRole("tab", { name: /^Cleanup/ }).click();
  await page.getByRole("button", { name: "Compress extras (2)" }).click();
  await expect(page.locator(".toast")).toContainText("Compressed 2 photos");
  await expect(page.getByText("Compressed 2 to /mock/picked-folder/space-saver")).toBeVisible();
});

test("recently deleted lists trash and restores", async ({ page }) => {
  await page.getByRole("button", { name: "Select", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select IMG_20261007_185817.jpg" }).check();
  await page.locator(".batch-bar").getByRole("button", { name: "Trash", exact: true }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "Move to Trash" }).click();
  await expect(page.locator(".toast")).toContainText("moved to Trash");
  await page.getByRole("button", { name: "Recently Deleted", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Recently Deleted" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("IMG_20261007_185817.jpg");
  await dialog.getByRole("button", { name: "Restore", exact: true }).click();
  await expect(page.locator(".toast")).toContainText("Restored IMG_20261007_185817.jpg");
  await expect(page.getByText("IMG_20261007_185817.jpg").first()).toBeVisible();
});
