import type {
  AlbumRow,
  AssetRow,
  CleanupStats,
  CleanupProgress,
  ClusterEventsSummary,
  DuplicateGroup,
  DuplicateMember,
  EnrichProgress,
  EnrichSummary,
  EventRow,
  EventAssetRow,
  ExportSummary,
  FaceClusterRow,
  FaceRow,
  GeocodeProgress,
  HighlightResult,
  ImportProgress,
  ImportSummary,
  LibraryFilters,
  LibraryMonth,
  LibraryStats,
  PipelineProgress,
  PipelineSummary,
  ResolvePlacesSummary,
  ScanRoot,
  SceneTag,
  SceneTagCount,
  TagProgress,
  TagScenesSummary,
  ClusterProgress
} from "./library";

/**
 * Web-mode stand-in for the Tauri bridge: same function surface as
 * library.ts + engine.ts (+ the folder-picker `open` from
 * @tauri-apps/plugin-dialog), backed by an in-memory store so the UI can be
 * exercised in a plain browser (Playwright) without the engine.
 *
 * Signatures intentionally mirror library.ts one-to-one; bridge.ts selects
 * this module when `?web=1` (or VITE_FOTOFLOW_WEB=1) is set.
 */

type DialogOptions = { title?: string; directory?: boolean; multiple?: boolean };

function svgThumb(label: string): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256">` +
    `<rect width="100%" height="100%" fill="#1c212c"/>` +
    `<text x="50%" y="50%" dominant-baseline="middle" text-anchor="middle" ` +
    `fill="#8a93a5" font-size="12" font-family="sans-serif">${label}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}

function asset(
  id: number,
  name: string,
  kind: "image" | "video",
  opts: Partial<AssetRow> = {},
): AssetRow {
  return {
    id,
    path: `/photos/${name}`,
    name,
    ext: name.split(".").pop() ?? "jpg",
    kind,
    mime: kind === "image" ? "image/jpeg" : "video/mp4",
    sizeBytes: 1_200_000 + id * 37_000,
    modified: 1_760_000_000 + id * 60,
    capturedAt: 1_760_000_000 + id * 60,
    width: kind === "image" ? 4032 : 1920,
    height: kind === "image" ? 3024 : 1080,
    duration: kind === "video" ? 12.5 : null,
    favorite: 0,
    missing: 0,
    sha256: null,
    ...opts
  };
}

const DUPLICATE_SHA = "a".repeat(64);
const seed: AssetRow[] = [
  asset(1, "IMG_20261007_185817.jpg", "image"),
  asset(2, "IMG_20261007_185822.jpg", "image"),
  asset(3, "IMG_20261007_185824.jpg", "image"),
  asset(4, "IMG_20261007_191433.jpg", "image"),
  asset(5, "IMG_20261007_191434.jpg", "image", { sha256: DUPLICATE_SHA }),
  asset(6, "IMG_20261007_191434_copy.jpg", "image", { sha256: DUPLICATE_SHA }),
  asset(7, "VID_20261007_192001.mp4", "video"),
  asset(8, "VID_20261007_204512.mp4", "video")
];

let assets: AssetRow[] = [...seed];
let nextId = 100;
let scanHistory: string[] = ["/photos"];

// AssetRow carries no location column; the mock tracks "has GPS" separately
// so the With/No-location filter chips have something to chew on.
let located = new Set<number>([1, 2, 4, 7]);

// personCluster -> asset ids, filled by runFaceClustering.
let clusterMembers = new Map<number, Set<number>>();

let groups: DuplicateGroup[] = [
  {
    id: 1,
    sha256: DUPLICATE_SHA,
    sizeBytes: seed[4].sizeBytes,
    wasteBytes: seed[4].sizeBytes,
    keptId: 5,
    members: [
      memberOf(seed[4]),
      memberOf(seed[5])
    ]
  }
];

let blurredList: AssetRow[] = [assets[2]];

function memberOf(a: AssetRow): DuplicateMember {
  return {
    id: a.id,
    path: a.path,
    name: a.name,
    isDuplicate: true,
    capturedAt: a.capturedAt,
    qualityScore: 180
  };
}

let clusters: FaceClusterRow[] = [];
let facesByCluster = new Map<number, FaceRow[]>();
let clusterNames = new Map<number, string>();
let faceId = 1;

let events: EventRow[] = [];
let eventId = 1;

type MockAlbum = { id: number; name: string; createdAt: number; members: number[] };
let albums: MockAlbum[] = [];
let albumId = 1;

export async function importFolder(root: string, _recursive = true): Promise<ImportSummary> {
  const added: AssetRow[] = [];
  for (let i = 0; i < 4; i++) {
    added.push(asset(nextId, `IMPORTED_${nextId}.jpg`, "image", { path: `${root}/IMPORTED_${nextId}.jpg` }));
    nextId++;
  }
  assets = [...assets, ...added];
  if (!scanHistory.includes(root)) scanHistory.push(root);
  return {
    root,
    scanId: 1,
    scanned: assets.length,
    imported: added.length,
    unchanged: assets.length - added.length,
    skipped: 0,
    bytes: assets.reduce((s, a) => s + a.sizeBytes, 0),
    elapsed: 0.4
  };
}

function applyFilters(rows: AssetRow[], opts: LibraryFilters): AssetRow[] {
  let out = rows;
  if (opts.kind) out = out.filter((a) => a.kind === opts.kind);
  if (opts.hasLocation !== undefined) {
    out = out.filter((a) => (opts.hasLocation ? located.has(a.id) : !located.has(a.id)));
  }
  if (opts.capturedMin !== undefined) out = out.filter((a) => (a.capturedAt ?? 0) >= opts.capturedMin!);
  if (opts.capturedMax !== undefined) out = out.filter((a) => (a.capturedAt ?? 0) < opts.capturedMax!);
  if (opts.favoriteOnly) out = out.filter((a) => a.favorite !== 0);
  if (opts.sceneTag !== undefined) {
    out = out.filter((a) => (assetTagMap.get(a.id) ?? []).some((t) => t.tag === opts.sceneTag));
  }
  if (opts.personCluster !== undefined) {
    const members = clusterMembers.get(opts.personCluster);
    out = members ? out.filter((a) => members.has(a.id)) : [];
  }
  return out;
}

export async function libraryList(opts: LibraryFilters = {}): Promise<AssetRow[]> {
  const rows = applyFilters(assets, opts);
  const offset = opts.offset ?? 0;
  const limit = opts.limit ?? 60;
  return rows.slice(offset, offset + limit);
}

export async function libraryCount(
  opts: Omit<LibraryFilters, "offset" | "limit"> = {},
): Promise<number> {
  return applyFilters(assets, opts).length;
}

export async function libraryMonths(): Promise<LibraryMonth[]> {
  const counts = new Map<string, number>();
  for (const a of assets) {
    if (a.capturedAt == null) continue;
    const d = new Date(a.capturedAt * 1000);
    const key = `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}`;
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([month, count]) => ({ month, count }))
    .sort((a, b) => (a.month < b.month ? 1 : -1));
}

export async function libraryStats(): Promise<LibraryStats> {  const photos = assets.filter((a) => a.kind === "image").length;
  return {
    photos,
    videos: assets.filter((a) => a.kind === "video").length,
    bytes: assets.reduce((s, a) => s + a.sizeBytes, 0),
    missing: 0,
    hashed: assets.filter((a) => a.sha256 !== null).length
  };
}

export async function scanRoots(): Promise<ScanRoot[]> {
  return scanHistory.map((root) => {
    const members = assets.filter((a) => a.path === root || a.path.startsWith(`${root}/`));
    return {
      root,
      scans: 1,
      lastStartedAt: 1_760_000_000,
      assets: members.length,
      missing: members.filter((a) => a.missing !== 0).length
    };
  });
}

export async function enrichAssets(_maxAttempts = 3): Promise<EnrichSummary> {
  return { processed: assets.length, enriched: assets.length, failed: 0, pending: 0, elapsed: 0.2 };
}

export function onEnrichProgress(_cb: (p: EnrichProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function onImportProgress(_cb: (p: ImportProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function onImportError(_cb: (message: string) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

function cleanupSnapshot(): CleanupStats {
  return {
    duplicateGroups: groups.length,
    wasteBytes: groups.reduce((s, g) => s + g.wasteBytes, 0),
    blurred: blurredList.length
  };
}

export async function findDuplicates(): Promise<CleanupStats> {
  return cleanupSnapshot();
}

export async function scoreQuality(): Promise<CleanupStats> {
  return cleanupSnapshot();
}

export async function duplicatesList(): Promise<DuplicateGroup[]> {
  return groups;
}

export async function keepCopy(groupId: number, keepId: number): Promise<DuplicateGroup[]> {
  groups = groups.map((g) => (g.id === groupId ? { ...g, keptId: keepId } : g));
  return groups;
}

export async function cleanupStats(): Promise<CleanupStats> {
  return cleanupSnapshot();
}

export async function blurredAssets(limit = 60): Promise<AssetRow[]> {
  return blurredList.slice(0, limit);
}

export async function trashAssets(ids: number[]): Promise<{ trashed: number; failed: string[] }> {
  const drop = new Set(ids);
  assets = assets.filter((a) => !drop.has(a.id));
  return { trashed: ids.length, failed: [] };
}

export async function setFavorite(assetId: number, favorite: boolean): Promise<void> {
  assets = assets.map((a) => (a.id === assetId ? { ...a, favorite: favorite ? 1 : 0 } : a));
}

let assetTagMap = new Map<number, SceneTag[]>([
  [1, [{ tag: "mountain", confidence: 0.91 }, { tag: "lake", confidence: 0.62 }]],
  [4, [{ tag: "flower", confidence: 0.83 }]]
]);

export async function tagScenes(): Promise<TagScenesSummary> {
  return { total: assets.length, tagged: assetTagMap.size, failed: 0 };
}

export function onTagProgress(_cb: (p: TagProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export async function sceneTags(): Promise<SceneTagCount[]> {
  const counts = new Map<string, number>();
  for (const tags of assetTagMap.values()) {
    for (const t of tags) counts.set(t.tag, (counts.get(t.tag) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([tag, count]) => ({ tag, count }))
    .sort((a, b) => b.count - a.count);
}

export async function assetTags(assetId: number): Promise<SceneTag[]> {
  return assetTagMap.get(assetId) ?? [];
}

export async function runPostImport(): Promise<PipelineSummary> {
  return { enriched: 8, hashed: 8, scored: 8, events: 1, tagged: 2, places: 0, failed: 0 };
}

export function onPipelineProgress(_cb: (p: PipelineProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export async function runFaceClustering(): Promise<{
  images: number;
  faces: number;
  clusters: number;
  missing: number;
}> {
  const images = assets.filter((a) => a.kind === "image");
  if (images.length === 0) {
    clusters = [];
    facesByCluster = new Map();
    clusterMembers = new Map();
    return { images: 0, faces: 0, clusters: 0, missing: 0 };
  }

  const faceFor = (a: AssetRow): FaceRow => {
    const id = faceId++;
    return { id, path: a.path, clusterId: null, x: 0.3, y: 0.2, width: 0.2, height: 0.25, confidence: 0.9, detectedAt: 0 };
  };

  // Two people: most photos get the first face; the last two images get a second.
  const clusterA = images.slice(0, Math.max(3, images.length - 2)).map(faceFor);
  const clusterB = images.slice(-2).map(faceFor);
  const list = [
    { id: 1, faceCount: clusterA.length, representativePath: clusterA[0]?.path ?? null, name: clusterNames.get(1) ?? null },
    { id: 2, faceCount: clusterB.length, representativePath: clusterB[0]?.path ?? null, name: clusterNames.get(2) ?? null }
  ];
  clusters = list;
  facesByCluster = new Map([
    [1, clusterA],
    [2, clusterB]
  ]);
  const idsA = new Set(images.slice(0, Math.max(3, images.length - 2)).map((a) => a.id));
  const idsB = new Set(images.slice(-2).map((a) => a.id));
  clusterMembers = new Map([
    [1, idsA],
    [2, idsB]
  ]);
  return { images: images.length, faces: clusterA.length + clusterB.length, clusters: list.length, missing: 0 };
}

export function onClusterProgress(_cb: (p: ClusterProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export async function verifyLibrary(): Promise<{ missing: number; restored: number }> {
  return { missing: 0, restored: 0 };
}

export async function clustersList(): Promise<FaceClusterRow[]> {
  return clusters;
}

export async function renameCluster(clusterId: number, name: string): Promise<void> {
  const clean = name.trim();
  if (!clean) throw new Error("person name cannot be empty");
  clusterNames.set(clusterId, clean);
  clusters = clusters.map((c) => (c.id === clusterId ? { ...c, name: clean } : c));
}

export async function mergeClusters(fromId: number, intoId: number): Promise<{ merged: number }> {
  if (fromId === intoId) throw new Error("cannot merge a cluster into itself");
  const from = facesByCluster.get(fromId) ?? [];
  const into = facesByCluster.get(intoId) ?? [];
  facesByCluster.set(intoId, [...into, ...from]);
  facesByCluster.delete(fromId);
  const idsA = clusterMembers.get(fromId) ?? new Set<number>();
  const idsB = clusterMembers.get(intoId) ?? new Set<number>();
  clusterMembers.set(intoId, new Set([...idsB, ...idsA]));
  clusterMembers.delete(fromId);
  if (!clusterNames.get(intoId) && clusterNames.get(fromId)) {
    clusterNames.set(intoId, clusterNames.get(fromId)!);
  }
  clusterNames.delete(fromId);
  const merged = into.length + from.length;
  clusters = clusters
    .filter((c) => c.id !== fromId)
    .map((c) => (c.id === intoId ? { ...c, faceCount: merged, name: clusterNames.get(intoId) ?? null } : c));
  return { merged };
}

export async function facesInCluster(clusterId: number): Promise<FaceRow[]> {
  return facesByCluster.get(clusterId) ?? [];
}

export async function facesCount(): Promise<number> {
  let total = 0;
  for (const list of facesByCluster.values()) total += list.length;
  return total;
}

export async function faceThumb(
  _path: string,
  _x: number,
  _y: number,
  _width: number,
  _height: number,
  _maxPx = 256,
): Promise<{ image: string }> {
  return { image: svgThumb("face") };
}

export async function engineThumb(path: string, _maxPx = 256): Promise<{ path: string; image: string }> {
  return { path, image: svgThumb(path.split("/").pop() ?? "?") };
}

export async function clusterEvents(
  _gapHours = 4,
  _maxDistanceM = 500,
): Promise<ClusterEventsSummary> {
  if (events.length === 0) {
    const start = assets[0]?.capturedAt ?? 1_760_000_000;
    events = [
      {
        id: eventId++,
        name: "InlawsVisit2026",
        startedAt: start,
        endedAt: start + 3600 * 3,
        latitude: 37.7749,
        longitude: -122.4194,
        place: null,
        assetCount: assets.length,
        faceCount: 0
      }
    ];
  }
  return { events: events.length, assets: assets.length };
}

export async function eventsList(): Promise<EventRow[]> {
  return events;
}

export async function eventAssets(eventId: number): Promise<EventAssetRow[]> {
  return events
    .filter((e) => e.id === eventId)
    .flatMap(() => assets.map((a) => ({ id: a.id, path: a.path, name: a.name, kind: a.kind })));
}

export async function renameEvent(eventId: number, name: string): Promise<void> {
  events = events.map((e) => (e.id === eventId ? { ...e, name } : e));
}

export async function resolvePlaces(): Promise<ResolvePlacesSummary> {
  const missing = events.filter((e) => e.place === null && e.latitude !== null && e.longitude !== null);
  events = events.map((e) =>
    e.place === null && e.latitude !== null && e.longitude !== null
      ? { ...e, place: "San Francisco, California" }
      : e,
  );
  return { total: missing.length, resolved: missing.length, cached: 0, failed: 0 };
}

export function onGeocodeProgress(_cb: (p: GeocodeProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function onGeocodeError(_cb: (message: string) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

function albumRow(a: MockAlbum): AlbumRow {
  const cover = a.members.length > 0
    ? (assets.find((x) => x.id === a.members[0])?.path ?? null)
    : null;
  return { id: a.id, name: a.name, createdAt: a.createdAt, assetCount: a.members.length, coverPath: cover };
}

export async function albumsList(): Promise<AlbumRow[]> {
  return albums.map(albumRow);
}

export async function albumCreate(name: string): Promise<number> {
  const id = albumId++;
  albums = [...albums, { id, name, createdAt: Math.floor(Date.now() / 1000), members: [] }];
  return id;
}

export async function albumRename(albumId: number, name: string): Promise<void> {
  albums = albums.map((a) => (a.id === albumId ? { ...a, name } : a));
}

export async function albumDelete(albumId: number): Promise<{ removed: number }> {
  const found = albums.find((a) => a.id === albumId);
  albums = albums.filter((a) => a.id !== albumId);
  return { removed: found ? found.members.length : 0 };
}

export async function albumAssets(albumId: number): Promise<EventAssetRow[]> {
  const album = albums.find((a) => a.id === albumId);
  if (!album) return [];
  const members = new Set(album.members);
  return assets
    .filter((a) => members.has(a.id))
    .map((a) => ({ id: a.id, path: a.path, name: a.name, kind: a.kind }));
}

export async function albumAdd(albumId: number, assetIds: number[]): Promise<{ added: number }> {
  let added = 0;
  albums = albums.map((a) => {
    if (a.id !== albumId) return a;
    const members = [...a.members];
    for (const id of assetIds) {
      if (!members.includes(id)) {
        members.push(id);
        added++;
      }
    }
    return { ...a, members };
  });
  return { added };
}

export async function albumRemove(albumId: number, assetId: number): Promise<void> {
  albums = albums.map((a) =>
    a.id === albumId ? { ...a, members: a.members.filter((id) => id !== assetId) } : a,
  );
}

export async function exportBundle(
  _source: { eventId?: number; albumId?: number },
  eventName: string,
  _mode: "drive" | "whatsapp",
  destDir: string,
): Promise<ExportSummary> {
  return {
    dir: `${destDir}/${eventName}`,
    exported: assets.length,
    skipped: 0,
    bytes: assets.reduce((s, a) => s + a.sizeBytes, 0)
  };
}

export async function exportHighlight(
  _source: { eventId?: number; albumId?: number },
  _name?: string,
): Promise<HighlightResult> {
  return { output: "/tmp/highlight.mp4", bytes: 587_000, seconds: 9, photos: 3, videosIncluded: 1, videosSkipped: 0 };
}

export function onHashProgress(_cb: (p: CleanupProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function onQualityProgress(_cb: (p: CleanupProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export async function open(_opts: DialogOptions = {}): Promise<string | null> {
  return "/mock/picked-folder";
}

export interface SysInfo {
  os: string;
  arch: string;
  cpuCount: number;
  memoryGB: number;
  swift: string;
}

export async function engineCall<T>(cmd: string, params?: object): Promise<T> {
  if (cmd === "ping") return { pong: true } as T;
  if (cmd === "sysinfo") {
    return { os: "web-mock", arch: "aarch64", cpuCount: 12, memoryGB: 64, swift: "mock 0.0" } as T;
  }
  if (cmd === "thumb") {
    const p = (params ?? {}) as { path?: string; maxPx?: number };
    const path = p.path ?? "?";
    return { path, image: svgThumb(path.split("/").pop() ?? "?") } as T;
  }
  if (cmd === "face_thumb") return { image: svgThumb("face") } as T;
  throw new Error(`web mode: engine command ${cmd} not mocked`);
}

export async function engineSysInfo(): Promise<SysInfo> {
  return { os: "web-mock", arch: "aarch64", cpuCount: 12, memoryGB: 64, swift: "mock 0.0" };
}
