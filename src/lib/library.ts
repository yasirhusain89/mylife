import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type AssetRow = {
  id: number;
  path: string;
  name: string;
  ext: string;
  kind: "image" | "video";
  mime: string | null;
  sizeBytes: number;
  modified: number;
  capturedAt: number | null;
  width: number | null;
  height: number | null;
  duration: number | null;
  favorite: number;
  missing: number;
  sha256: string | null;
};

export type LibraryStats = {
  photos: number;
  videos: number;
  bytes: number;
  missing: number;
  hashed: number;
};

export type ImportSummary = {
  root: string;
  scanId: number;
  scanned: number;
  imported: number;
  unchanged: number;
  skipped: number;
  bytes: number;
  elapsed: number;
};

export type ImportProgress = {
  scanId: number;
  scanned: number;
  imported: number;
  unchanged: number;
  bytes: number;
  walking?: boolean;
};

export function importFolder(root: string, recursive = true): Promise<ImportSummary> {
  return invoke("import_folder", { root, recursive });
}

export function libraryList(
  opts: { kind?: "image" | "video"; offset?: number; limit?: number } = {},
): Promise<AssetRow[]> {
  return invoke("library_list", opts);
}

export function libraryStats(): Promise<LibraryStats> {
  return invoke("library_stats");
}

export function onImportProgress(cb: (p: ImportProgress) => void): Promise<() => void> {
  return listen<ImportProgress>("import-progress", (e) => cb(e.payload));
}

export function onImportError(cb: (message: string) => void): Promise<() => void> {
  return listen<string>("import-error", (e) => cb(e.payload));
}

export function engineThumb(path: string, maxPx = 256): Promise<{ path: string; image: string }> {
  return invoke("engine_call", { cmd: "thumb", params: { path, maxPx } });
}
