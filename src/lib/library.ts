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

export type EnrichSummary = {
  processed: number;
  enriched: number;
  failed: number;
  pending: number;
  elapsed: number;
};

export type EnrichProgress = {
  processed: number;
  enriched: number;
  failed: number;
  pending: number;
  exhausted?: number;
};

export function enrichAssets(maxAttempts = 3): Promise<EnrichSummary> {
  return invoke("enrich_assets", { maxAttempts });
}

export function onEnrichProgress(cb: (p: EnrichProgress) => void): Promise<() => void> {
  return listen<EnrichProgress>("enrich-progress", (e) => cb(e.payload));
}

export type DuplicateMember = {
  id: number;
  path: string;
  name: string;
  isDuplicate: boolean;
  capturedAt: number | null;
  qualityScore: number | null;
};

export type DuplicateGroup = {
  id: number;
  sha256: string;
  sizeBytes: number;
  wasteBytes: number;
  keptId: number | null;
  members: DuplicateMember[];
};

export type CleanupStats = {
  duplicateGroups: number;
  wasteBytes: number;
  blurred: number;
};

export type CleanupProgress = {
  hashed?: number;
  failed?: number;
  buckets?: number;
  scored?: number;
  blurred?: number;
};

export function findDuplicates(): Promise<CleanupStats> {
  return invoke("hash_candidates_run");
}

export function scoreQuality(): Promise<CleanupStats> {
  return invoke("score_quality");
}

export function duplicatesList(): Promise<DuplicateGroup[]> {
  return invoke("duplicates_list");
}

export function keepCopy(groupId: number, keepId: number): Promise<DuplicateGroup[]> {
  return invoke("duplicates_keep", { groupId, keepId });
}

export function cleanupStats(): Promise<CleanupStats> {
  return invoke("cleanup_stats_cmd");
}

export function blurredAssets(limit = 60): Promise<AssetRow[]> {
  return invoke("library_list", { kind: "image", blurredOnly: true, limit, offset: 0 });
}

export function trashAssets(ids: number[]): Promise<{ trashed: number; failed: string[] }> {
  return invoke("trash_assets", { ids });
}

export type FaceRow = {
  id: number;
  path: string;
  clusterId: number | null;
  x: number;
  y: number;
  width: number;
  height: number;
  confidence: number;
  detectedAt: number;
};

export type FaceClusterRow = {
  id: number;
  faceCount: number;
  representativePath: string | null;
};

export function faceThumb(
  path: string,
  x: number,
  y: number,
  width: number,
  height: number,
  maxPx = 256,
): Promise<{ image: string }> {
  return invoke("engine_call", {
    cmd: "face_thumb",
    params: { path, x, y, width, height, maxPx },
  });
}

export function clustersList(): Promise<FaceClusterRow[]> {
  return invoke("clusters_list");
}

export function facesInCluster(clusterId: number): Promise<FaceRow[]> {
  return invoke("faces_in_cluster", { clusterId });
}

export function facesCount(): Promise<number> {
  return invoke("faces_count");
}

export type EventRow = {
  id: number;
  name: string;
  startedAt: number;
  endedAt: number;
  latitude: number | null;
  longitude: number | null;
  place: string | null;
  assetCount: number;
  faceCount: number;
};

export type EventAssetRow = {
  id: number;
  path: string;
  name: string;
  kind: string;
};

export type ClusterEventsSummary = {
  events: number;
  assets: number;
};

export function clusterEvents(
  gapHours = 4,
  maxDistanceM = 500,
): Promise<ClusterEventsSummary> {
  return invoke("cluster_events", { gapHours, maxDistanceM });
}

export function eventsList(): Promise<EventRow[]> {
  return invoke("events_list");
}

export function eventAssets(eventId: number): Promise<EventAssetRow[]> {
  return invoke("event_assets", { eventId });
}

export function renameEvent(eventId: number, name: string): Promise<void> {
  return invoke("rename_event", { event_id: eventId, name });
}

export type HighlightResult = {
  output: string;
  bytes: number;
  seconds: number;
  photos: number;
  videosSkipped: number;
};

export type ExportSummary = {
  dir: string;
  exported: number;
  skipped: number;
  bytes: number;
};

export function exportBundle(
  eventId: number,
  eventName: string,
  mode: "drive" | "whatsapp",
  destDir: string,
): Promise<ExportSummary> {
  return invoke("export_bundle", { eventId, eventName, mode, destDir });
}

export function exportHighlight(
  eventId: number,
  name?: string,
): Promise<HighlightResult> {
  return invoke("export_highlight", { eventId, name });
}

export function onHashProgress(cb: (p: CleanupProgress) => void): Promise<() => void> {
  return listen<CleanupProgress>("hash-progress", (e) => cb(e.payload));
}

export function onQualityProgress(cb: (p: CleanupProgress) => void): Promise<() => void> {
  return listen<CleanupProgress>("quality-progress", (e) => cb(e.payload));
}

export function engineThumb(path: string, maxPx = 256): Promise<{ path: string; image: string }> {
  return invoke("engine_call", { cmd: "thumb", params: { path, maxPx } });
}
