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

export type LibraryFilters = {
  kind?: "image" | "video";
  offset?: number;
  limit?: number;
  personCluster?: number;
  hasLocation?: boolean;
  capturedMin?: number;
  capturedMax?: number;
  favoriteOnly?: boolean;
  sceneTag?: string;
  query?: string;
  hasPet?: boolean;
};

export function libraryList(opts: LibraryFilters = {}): Promise<AssetRow[]> {
  return invoke("library_list", {
    kind: opts.kind,
    offset: opts.offset,
    limit: opts.limit,
    personCluster: opts.personCluster ?? null,
    hasLocation: opts.hasLocation ?? null,
    capturedMin: opts.capturedMin ?? null,
    capturedMax: opts.capturedMax ?? null,
    favoriteOnly: opts.favoriteOnly ?? null,
    sceneTag: opts.sceneTag ?? null,
    query: opts.query?.trim() ? opts.query.trim() : null,
    hasPet: opts.hasPet ?? null
  });
}

export function libraryCount(opts: Omit<LibraryFilters, "offset" | "limit"> = {}): Promise<number> {
  return invoke("library_count", {
    kind: opts.kind,
    personCluster: opts.personCluster ?? null,
    hasLocation: opts.hasLocation ?? null,
    capturedMin: opts.capturedMin ?? null,
    capturedMax: opts.capturedMax ?? null,
    favoriteOnly: opts.favoriteOnly ?? null,
    sceneTag: opts.sceneTag ?? null,
    query: opts.query?.trim() ? opts.query.trim() : null,
    hasPet: opts.hasPet ?? null
  });
}

export function setFavorite(assetId: number, favorite: boolean): Promise<void> {
  return invoke("set_favorite", { assetId, favorite });
}

export type PetSighting = {
  id: number;
  path: string;
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
  confidence: number;
};

export type PetProgress = {
  processed: number;
  total: number;
  sightings: number;
};

export type DetectPetsSummary = {
  images: number;
  sightings: number;
  missing: number;
};

export function detectPets(): Promise<DetectPetsSummary> {
  return invoke("detect_pets");
}

export function onPetProgress(cb: (p: PetProgress) => void): Promise<() => void> {
  return listen<PetProgress>("pet-progress", (e) => cb(e.payload));
}

export function onPetError(cb: (message: string) => void): Promise<() => void> {
  return listen<string>("pet-error", (e) => cb(e.payload));
}

export function sightingsList(): Promise<PetSighting[]> {
  return invoke("sightings_list");
}

export type TagProgress = {
  processed: number;
  total: number;
  tagged: number;
  failed: number;
};

export type TagScenesSummary = {
  total: number;
  tagged: number;
  failed: number;
};

export type SceneTagCount = {
  tag: string;
  count: number;
};

export type SceneTag = {
  tag: string;
  confidence: number;
};

export function tagScenes(): Promise<TagScenesSummary> {
  return invoke("tag_scenes");
}

export function onTagProgress(cb: (p: TagProgress) => void): Promise<() => void> {
  return listen<TagProgress>("tag-progress", (e) => cb(e.payload));
}

export function sceneTags(): Promise<SceneTagCount[]> {
  return invoke("scene_tags");
}

export function assetTags(assetId: number): Promise<SceneTag[]> {
  return invoke("asset_tags", { assetId });
}

export type PipelineProgress = {
  stage: string;
  processed: number;
  total: number;
};

export type PipelineSummary = {
  enriched: number;
  hashed: number;
  scored: number;
  events: number;
  tagged: number;
  embedded: number;
  places: number;
  failed: number;
};

export function runPostImport(): Promise<PipelineSummary> {
  return invoke("run_post_import_cmd");
}

export function onPipelineProgress(cb: (p: PipelineProgress) => void): Promise<() => void> {
  return listen<PipelineProgress>("pipeline-progress", (e) => cb(e.payload));
}

export type LibraryMonth = { month: string; count: number };

export function libraryMonths(): Promise<LibraryMonth[]> {
  return invoke("library_months");
}

export function libraryStats(): Promise<LibraryStats> {
  return invoke("library_stats");
}

export type ScanRoot = {
  root: string;
  scans: number;
  lastStartedAt: number;
  assets: number;
  missing: number;
};

export function scanRoots(): Promise<ScanRoot[]> {
  return invoke("scan_roots");
}

export function assetsInRoot(root: string): Promise<number[]> {
  return invoke("assets_in_root", { root });
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

export type NearGroup = {
  id: number;
  wasteBytes: number;
  keptId: number | null;
  members: DuplicateMember[];
};

export type NearDupeSummary = {
  hashed: number;
  failed: number;
  groups: number;
  wasteBytes: number;
};

export function findSimilar(): Promise<NearDupeSummary> {
  return invoke("near_dupes_run");
}

export function nearGroupsList(): Promise<NearGroup[]> {
  return invoke("near_groups_list");
}

export function keepNearCopy(groupId: number, keepId: number): Promise<NearGroup[]> {
  return invoke("near_keep", { groupId, keepId });
}

export function onNearProgress(cb: (p: CleanupProgress) => void): Promise<() => void> {
  return listen<CleanupProgress>("near-progress", (e) => cb(e.payload));
}

// MARK: - Visual similarity (similar-photos v1: image-query neighbours)

export type SimilarHit = {
  id: number;
  path: string;
  name: string;
  kind: string;
  score: number;
};

export type EmbedSummary = {
  embedded: number;
  failed: number;
};

export type EmbedProgress = {
  processed: number;
  embedded: number;
  failed: number;
};

export function similarTo(assetId: number, limit = 12): Promise<SimilarHit[]> {
  return invoke("similar_to", { assetId, limit });
}

export function embedLibrary(): Promise<EmbedSummary> {
  return invoke("embed_library");
}

export function onEmbedProgress(cb: (p: EmbedProgress) => void): Promise<() => void> {
  return listen<EmbedProgress>("embed-progress", (e) => cb(e.payload));
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
  name: string | null;
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

export type ClusterProgress = {
  processed: number;
  total: number;
  faces: number;
};

export function runFaceClustering(): Promise<{
  images: number;
  faces: number;
  clusters: number;
  missing: number;
}> {
  return invoke("cluster_faces", {});
}

export function verifyLibrary(): Promise<{ missing: number; restored: number }> {
  return invoke("verify_library");
}

export function onClusterProgress(cb: (p: ClusterProgress) => void): Promise<() => void> {
  return listen<ClusterProgress>("cluster-progress", (e) => cb(e.payload));
}

export function clustersList(): Promise<FaceClusterRow[]> {
  return invoke("clusters_list");
}

export function renameCluster(clusterId: number, name: string): Promise<void> {
  return invoke("rename_cluster", { clusterId, name });
}

export function mergeClusters(fromId: number, intoId: number): Promise<{ merged: number }> {
  return invoke("merge_clusters", { fromId, intoId });
}

export function facesInCluster(clusterId: number): Promise<FaceRow[]> {
  return invoke("faces_in_cluster", { clusterId });
}

export function unassignFace(faceId: number): Promise<number> {
  return invoke("unassign_face", { faceId });
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

export type GeocodeProgress = {
  processed: number;
  total: number;
  resolved: number;
  cached: number;
  failed: number;
};

export type ResolvePlacesSummary = {
  total: number;
  resolved: number;
  cached: number;
  failed: number;
};

export function resolvePlaces(): Promise<ResolvePlacesSummary> {
  return invoke("resolve_places");
}

export function onGeocodeProgress(cb: (p: GeocodeProgress) => void): Promise<() => void> {
  return listen<GeocodeProgress>("geocode-progress", (e) => cb(e.payload));
}

export function onGeocodeError(cb: (message: string) => void): Promise<() => void> {
  return listen<string>("geocode-error", (e) => cb(e.payload));
}

// MARK: - Albums (user-curated lists, usable as export sources)

export type AlbumRow = {
  id: number;
  name: string;
  createdAt: number;
  assetCount: number;
  coverPath: string | null;
};

export function albumsList(): Promise<AlbumRow[]> {
  return invoke("albums_list");
}

export function albumCreate(name: string): Promise<number> {
  return invoke("album_create", { name });
}

export function albumRename(albumId: number, name: string): Promise<void> {
  return invoke("album_rename", { albumId, name });
}

export function albumDelete(albumId: number): Promise<{ removed: number }> {
  return invoke("album_delete", { albumId });
}

export function albumAssets(albumId: number): Promise<EventAssetRow[]> {
  return invoke("album_assets", { albumId });
}

export function albumAdd(albumId: number, assetIds: number[]): Promise<{ added: number }> {
  return invoke("album_add", { albumId, assetIds });
}

export function albumRemove(albumId: number, assetId: number): Promise<void> {
  return invoke("album_remove", { albumId, assetId });
}

// Highlight / bundle exports accept either source; pass exactly one id.
export function exportBundle(
  source: { eventId?: number; albumId?: number },
  eventName: string,
  mode: "drive" | "whatsapp",
  destDir: string,
): Promise<ExportSummary> {
  return invoke("export_bundle", {
    eventId: source.eventId ?? null,
    albumId: source.albumId ?? null,
    eventName,
    mode,
    destDir
  });
}

export function exportHighlight(
  source: { eventId?: number; albumId?: number },
  name?: string,
): Promise<HighlightResult> {
  return invoke("export_highlight", {
    eventId: source.eventId ?? null,
    albumId: source.albumId ?? null,
    name
  });
}

export type HighlightResult = {
  output: string;
  bytes: number;
  seconds: number;
  photos: number;
  videosIncluded: number;
  videosSkipped: number;
};

export type ExportSummary = {
  dir: string;
  exported: number;
  skipped: number;
  bytes: number;
};

export function onHashProgress(cb: (p: CleanupProgress) => void): Promise<() => void> {
  return listen<CleanupProgress>("hash-progress", (e) => cb(e.payload));
}

export function onQualityProgress(cb: (p: CleanupProgress) => void): Promise<() => void> {
  return listen<CleanupProgress>("quality-progress", (e) => cb(e.payload));
}

export function engineThumb(path: string, maxPx = 256): Promise<{ path: string; image: string }> {
  return invoke("cached_thumb", { path, maxPx });
}
