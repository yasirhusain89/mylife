import { convertFileSrc, invoke } from "@tauri-apps/api/core";
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
  /** First non-null event place (null when the photo is in no placed event). */
  place: string | null;
  /** Sorted named people on this photo (empty when no one is tagged). */
  people: string[];
  /** EXIF camera body make/model/lens (null when the file carries none). */
  cameraMake: string | null;
  cameraModel: string | null;
  lens: string | null;
  /** GPS coordinates from EXIF (null when the file carries none). */
  latitude: number | null;
  longitude: number | null;
  /** Exposure settings from EXIF (null when the file carries none). */
  iso: number | null;
  fNumber: number | null;
  /** Shutter speed in seconds (0.004 = 1/250). */
  exposureTime: number | null;
  /** Focal length in mm (actual). */
  focalLength: number | null;
  /** 35mm-equivalent focal length in mm. */
  focalLength35: number | null;
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

// SSD ingest: mirror a camera card / device folder to a destination with
// progress. Desktop-only (no HTTP route): the browser cannot read disk.
export type IngestProgress = {
  copied: number;
  skipped: number;
  failed: string[];
  bytes: number;
  total: number;
  totalBytes: number;
  dest: string;
};

export type IngestSummary = IngestProgress;

export function ingestCopy(src: string, dest: string): Promise<IngestSummary> {
  return invoke("ingest_copy", { src, dest });
}

export function onIngestProgress(cb: (p: IngestProgress) => void): Promise<() => void> {
  return listen<IngestProgress>("ingest-progress", (e) => cb(e.payload));
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
  /** Newest-first by first_seen_at (Pinned "Recently Saved"). */
  sortAdded?: boolean;
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
    hasPet: opts.hasPet ?? null,
    sortAdded: opts.sortAdded ?? null
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
  near: number;
  events: number;
  tagged: number;
  embedded: number;
  faces: number;
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

export type RemoveRootSummary = {
  root: string;
  removedAssets: number;
  removedFaces: number;
  removedSightings: number;
  prunedClusters: number;
  removedScans: number;
};

/** Forgets one indexed root (index rows only — files on disk are untouched). */
export function removeRoot(root: string): Promise<RemoveRootSummary> {
  return invoke("remove_root", { root });
}

export type MapPin = {
  id: number;
  path: string;
  name: string;
  latitude: number;
  longitude: number;
};

export function mapPins(): Promise<MapPin[]> {
  return invoke("map_pins");
}

/** Map tile source: `osm` streets or `topo` terrain relief. */
export type TileSource = "osm" | "topo";

/**
 * One cached map tile as a browser-loadable URL (desktop only — see api.ts
 * for the HTTP fallback). The backend returns an app-data file path;
 * convertFileSrc turns it into an asset-protocol URL the browser caches
 * natively, so tiles cost no base64/IPC per view.
 */
export function mapTile(z: number, x: number, y: number, source: TileSource): Promise<string> {
  return invoke<string>("map_tile", { z, x, y, source }).then((p) => convertFileSrc(p));
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
  favorite: boolean;
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

export type RestoreSummary = { restored: number; failed: string[] };

/** Session-undo for trash_assets: moves files back from ~/.Trash. */
export function restoreAssets(ids: number[]): Promise<RestoreSummary> {
  return invoke("restore_assets", { ids });
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
  return invoke("cached_face_thumb", { path, x, y, width, height, maxPx });
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

export type VerifySummary = {
  missing: number;
  restored: number;
  unreadable: number;
  unreadablePaths: string[];
};

export function verifyLibrary(): Promise<VerifySummary> {
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

/** Session-undo for unassign_face: reattaches a detached face. */
export function assignFace(faceId: number, clusterId: number): Promise<void> {
  return invoke("assign_face", { faceId, clusterId });
}

/** Session-undo for merge_clusters: recreates a cluster around these faces. */
export function restoreCluster(name: string | null, faceIds: number[]): Promise<number> {
  return invoke("restore_cluster", { name, faceIds });
}

export function facesCount(): Promise<number> {
  return invoke("faces_count");
}

export function searchText(text: string, limit?: number): Promise<SearchHit[]> {
  return invoke("search_text", { text, limit });
}

/** One semantic-search hit: a photo (`tSeconds` null) or a video moment. */
export type SearchHit = {
  assetId: number;
  path: string;
  name: string;
  kind: string;
  score: number;
  tSeconds: number | null;
};

export type ClipModelsStatus = {
  ready: boolean;
  bytesTotal: number;
  bytesOnDisk: number;
};

export type ClipDownloadProgress = {
  name: string;
  received: number;
  total: number;
};

export function clipModelsPresent(): Promise<ClipModelsStatus> {
  return invoke("clip_models_present");
}

export function downloadClipModels(): Promise<ClipModelsStatus> {
  return invoke("download_clip_models");
}

export function onClipDownloadProgress(cb: (p: ClipDownloadProgress) => void): Promise<() => void> {
  return listen<ClipDownloadProgress>("clip-download-progress", (e) => cb(e.payload));
}

export function embedClipBackfill(): Promise<{ embedded: number; failed: number }> {
  return invoke("embed_clip_backfill");
}

export function reindexClip(): Promise<{ embedded: number; failed: number }> {
  return invoke("reindex_clip");
}

export function cancelClipBackfill(): Promise<void> {
  return invoke("cancel_clip_backfill");
}

export function removeClipModels(): Promise<ClipModelsStatus> {
  return invoke("remove_clip_models");
}

export type ReviewClusterRef = {
  id: number | null;
  name: string | null;
  faceCount: number;
};

export type ReviewFaceRef = {
  id: number;
  path: string;
  x: number;
  y: number;
  width: number;
  height: number;
};

/** One People-review item. `merge`: two clusters that may be one person;
 *  `assign`: a lone face that may belong to `assignTo`. `faceB` is always
 *  the target side. */
export type ReviewCandidate = {
  kind: "merge" | "assign";
  distance: number;
  clusterA: ReviewClusterRef;
  clusterB: ReviewClusterRef;
  faceA: ReviewFaceRef;
  faceB: ReviewFaceRef;
  assignTo: number | null;
};

export function reviewCandidates(): Promise<ReviewCandidate[]> {
  return invoke("review_candidates");
}

export function dismissReviewCandidate(faceA: number, faceB: number): Promise<void> {
  return invoke("dismiss_review_candidate", { faceA, faceB });
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

export type TopPerson = {
  name: string;
  faces: number;
};

export type TripRow = {
  id: string;
  name: string;
  place: string | null;
  startedAt: number;
  endedAt: number;
  stopCount: number;
  assetCount: number;
  eventIds: number[];
  kind: string;
  summary: string;
  dayCount: number;
  photoCount: number;
  videoCount: number;
  places: string[];
  topPeople: TopPerson[];
};

/** Trip → stop hierarchy over the event list (dynamic, display keys only). */
export function tripsList(): Promise<TripRow[]> {
  return invoke("trips_list");
}

/** Balanced coverage curation across several events into an album. */
export function curatePicksToAlbum(eventIds: number[], albumId: number, limit = 24): Promise<number> {
  return invoke("curate_picks_to_album", { eventIds, albumId, limit });
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

/** Best-of-place pick: balanced coverage curation (day × stop quotas, favorites, named faces, diversity, video mix) into an album. */
export function addEventPicksToAlbum(eventId: number, albumId: number, limit = 12): Promise<number> {
  return invoke("add_event_picks_to_album", { eventId, albumId, limit });
}

/** One-click family share pack: memory video + Day-organized handpicked media + offline gallery. */
export type SharePackSummary = {
  dir: string;
  video: string;
  exported: number;
  skipped: number;
  bytes: number;
};

/**
 * Long-export progress ("export-progress" events). `total` 0 = indeterminate
 * phase (planning/gallery); otherwise an honest finished-unit count, not wall
 * time — a heavy video holds `done` still, then it jumps. Consumers filter on
 * `job`, the caller-chosen per-run key echoed back by the backend.
 */
export type ExportProgress = {
  job: string;
  phase: "planning" | "packing" | "rendering" | "gallery" | "done";
  done: number;
  total: number;
  label: string;
};

export function onExportProgress(cb: (p: ExportProgress) => void): Promise<() => void> {
  return listen<ExportProgress>("export-progress", (e) => cb(e.payload));
}

export function exportSharePack(
  source: { eventIds?: number[]; albumId?: number },
  name: string,
  mode: "drive" | "compact",
  stripGps: boolean,
  destDir: string,
  job: string,
): Promise<SharePackSummary> {
  return invoke("export_share_pack", {
    eventIds: source.eventIds ?? null,
    albumId: source.albumId ?? null,
    name,
    mode,
    stripGps,
    destDir,
    job
  });
}

// Recently Deleted: trashed assets beyond the session-undo window.
export type TrashEntry = {
  assetId: number;
  path: string;
  name: string;
  trashedAt: number;
};

export function trashList(): Promise<TrashEntry[]> {
  return invoke("trash_list");
}

export function clearTrashLog(): Promise<number> {
  return invoke("clear_trash_log");
}

export function albumRemove(albumId: number, assetId: number): Promise<void> {
  return invoke("album_remove", { albumId, assetId });
}

// Highlight / bundle exports accept one source: an event, an album, or an
// adhoc id list (space-saver bundles of dupes + blurred).
export function exportBundle(
  source: { eventId?: number; albumId?: number; assetIds?: number[] },
  eventName: string,
  mode: "drive" | "whatsapp" | "compact",
  destDir: string,
): Promise<ExportSummary> {
  return invoke("export_bundle", {
    eventId: source.eventId ?? null,
    albumId: source.albumId ?? null,
    assetIds: source.assetIds ?? null,
    eventName,
    mode,
    destDir
  });
}

export function exportHighlight(
  source: { eventId?: number; albumId?: number },
  name?: string,
  job?: string,
  opts?: { maxSeconds?: number; musicPath?: string },
): Promise<HighlightResult> {
  return invoke("export_highlight", {
    eventId: source.eventId ?? null,
    albumId: source.albumId ?? null,
    name,
    job: job ?? "",
    maxSeconds: opts?.maxSeconds ?? null,
    musicPath: opts?.musicPath ?? null
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
