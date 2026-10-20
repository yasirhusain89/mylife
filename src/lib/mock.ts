import type {
  AlbumRow,
  AssetRow,
  CleanupStats,
  CleanupProgress,
  ClusterEventsSummary,
  DuplicateGroup,
  DuplicateMember,
  EmbedProgress,
  EmbedSummary,
  NearDupeSummary,
  NearGroup,
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
  MapPin,
  PetProgress,
  PetSighting,
  DetectPetsSummary,
  PipelineProgress,
  PipelineSummary,
  RemoveRootSummary,
  ResolvePlacesSummary,
  ScanRoot,
  SceneTag,
  SceneTagCount,
  SimilarHit,
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
    place: null,
    people: [],
    cameraMake: null,
    cameraModel: null,
    lens: null,
    latitude: null,
    longitude: null,
    iso: null,
    fNumber: null,
    exposureTime: null,
    focalLength: null,
    focalLength35: null,
    ...opts
  };
}

const DUPLICATE_SHA = "a".repeat(64);
const seed: AssetRow[] = [
  asset(1, "IMG_20261007_185817.jpg", "image", {
    cameraMake: "Apple",
    cameraModel: "iPhone 16 Pro",
    iso: 80,
    fNumber: 1.78,
    exposureTime: 1 / 120,
    focalLength: 6.86,
    focalLength35: 24,
    latitude: 37.7749,
    longitude: -122.4194
  }),
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
// Session-undo shelf: trashAssets moves rows here, restoreAssets moves them
// back. Mirrors the backend keeping DB rows with missing=1.
let trashedStore = new Map<number, AssetRow>();
let scanHistory: string[] = ["/photos"];

// AssetRow carries lat/lng for the rare fully-tagged fixture; the mock
// still tracks "has GPS" separately so the With/No-location filter chips
// have something to chew on.
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

let nearGroups: NearGroup[] = [
  {
    id: 1,
    wasteBytes: 1_237_000,
    keptId: 3,
    members: [
      { ...memberOf(assets[2]), qualityScore: 120 },
      { ...memberOf(assets[3]), qualityScore: 210 }
    ]
  }
];

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
// Faces detached by unassignFace wait here until assignFace reattaches them
// (or restoreCluster pulls them into a recreated cluster).
let unassignedFaces = new Map<number, FaceRow>();
let clusterNames = new Map<number, string>();
let faceId = 1;

let events: EventRow[] = [];
let eventId = 1;

type MockAlbum = { id: number; name: string; createdAt: number; members: number[] };
let albums: MockAlbum[] = [];
let albumId = 1;

export async function importFolder(root: string, _recursive = true): Promise<ImportSummary> {
  // Deterministic web-mock batch: 4 images in a distinct month (Nov 2025) so
  // the months dropdown gains an entry after import. Roots containing
  // "missing" simulate an ejected volume: those assets are flagged missing so
  // the Check-files / unreachable UI has something to report.
  const NOV_BASE = Date.UTC(2025, 10, 15, 12, 0, 0) / 1000;
  const missingRoot = root.toLowerCase().includes("missing");
  const added: AssetRow[] = [];
  for (let i = 0; i < 4; i++) {
    const id = nextId;
    added.push(
      asset(id, `IMPORTED_${id}.jpg`, "image", {
        path: `${root}/IMPORTED_${id}.jpg`,
        capturedAt: NOV_BASE + id * 60,
        missing: missingRoot ? 1 : 0
      })
    );
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
  if (opts.hasPet !== undefined) {
    out = out.filter((a) =>
      opts.hasPet
        ? sightings.some((s) => s.path === a.path)
        : !sightings.some((s) => s.path === a.path),
    );
  }
  if (opts.sceneTag !== undefined) {
    out = out.filter((a) => (assetTagMap.get(a.id) ?? []).some((t) => t.tag === opts.sceneTag));
  }
  if (opts.query !== undefined) {
    const needle = opts.query.trim().toLowerCase();
    if (needle !== "") {
      const eventHit = events.some(
        (e) => e.name.toLowerCase().includes(needle) || (e.place ?? "").toLowerCase().includes(needle),
      );
      out = out.filter(
        (a) =>
          a.name.toLowerCase().includes(needle) ||
          a.path.toLowerCase().includes(needle) ||
          (assetTagMap.get(a.id) ?? []).some((t) => t.tag.includes(needle)) ||
          eventHit ||
          sightings.some((s) => s.path === a.path && s.label.toLowerCase().includes(needle)) ||
          [...clusterMembers.entries()].some(
            ([cid, ids]) => ids.has(a.id) && (clusterNames.get(cid) ?? "").toLowerCase().includes(needle),
          ),
      );
    }
  }
  if (opts.personCluster !== undefined) {
    const members = clusterMembers.get(opts.personCluster);
    out = members ? out.filter((a) => members.has(a.id)) : [];
  }
  // Recently Saved: insertion order (id) stands in for first_seen_at.
  if (opts.sortAdded) out = [...out].sort((a, b) => b.id - a.id);
  return out;
}

export async function libraryList(opts: LibraryFilters = {}): Promise<AssetRow[]> {
  const rows = applyFilters(assets, opts);
  const offset = opts.offset ?? 0;
  const limit = opts.limit ?? 60;
  // The real backend annotates each row with its first event place and its
  // named people; the mock derives the same shape from its located set and
  // named clusters so grouping behaves identically.
  return rows.slice(offset, offset + limit).map((a) => ({
    ...a,
    place: located.has(a.id) ? "San Francisco, California" : null,
    people: [...clusterMembers.entries()]
      .filter(([, ids]) => ids.has(a.id))
      .map(([cid]) => clusterNames.get(cid))
      .filter((n): n is string => !!n)
      .sort()
  }));
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
    missing: assets.filter((a) => a.missing !== 0).length,
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

export async function assetsInRoot(root: string): Promise<number[]> {
  const base = root.endsWith("/") ? root.slice(0, -1) : root;
  return assets
    .filter((a) => a.path === root || a.path === base || a.path.startsWith(`${base}/`))
    .map((a) => a.id);
}

export async function removeRoot(root: string): Promise<RemoveRootSummary> {
  const base = root.endsWith("/") ? root.slice(0, -1) : root;
  const under = (p: string) => p === root || p === base || p.startsWith(`${base}/`);
  const gone = new Set(assets.filter((a) => under(a.path)).map((a) => a.id));
  const gonePaths = new Set(assets.filter((a) => under(a.path)).map((a) => a.path));
  assets = assets.filter((a) => !gone.has(a.id));
  scanHistory = scanHistory.filter((r) => r !== root);
  const goneSightings = sightings.filter((s) => under(s.path)).length;
  sightings = sightings.filter((s) => !under(s.path));
  let goneFaces = 0;
  let pruned = 0;
  // Drop cluster memberships that lost every member; names survive on the
  // remaining clusters exactly like the backend's empty-cluster prune.
  for (const [cid, ids] of [...clusterMembers]) {
    const rest = new Set([...ids].filter((id) => !gone.has(id)));
    if (rest.size === 0) {
      clusterMembers.delete(cid);
      clusterNames.delete(cid);
      goneFaces += facesByCluster.get(cid)?.length ?? 0;
      facesByCluster.delete(cid);
      clusters = clusters.filter((c) => c.id !== cid);
      pruned++;
    } else {
      clusterMembers.set(cid, rest);
      const kept = (facesByCluster.get(cid) ?? []).filter((f) => !gonePaths.has(f.path));
      goneFaces += (facesByCluster.get(cid)?.length ?? 0) - kept.length;
      facesByCluster.set(cid, kept);
      clusters = clusters.map((c) => (c.id === cid ? { ...c, faceCount: rest.size } : c));
    }
  }
  return {
    root,
    removedAssets: gone.size,
    removedFaces: goneFaces,
    removedSightings: goneSightings,
    prunedClusters: pruned,
    removedScans: 1
  };
}

// Web-mock pins: the "located" assets get deterministic coords around San
// Francisco so the map view has something to plot without a backend.
export async function mapPins(): Promise<MapPin[]> {
  return [...located]
    .sort((a, b) => a - b)
    .map((id, i) => {
      const a = assets.find((x) => x.id === id)!;
      return {
        id: a.id,
        path: a.path,
        name: a.name,
        latitude: 37.7749 + i * 0.002,
        longitude: -122.4194 + i * 0.0015
      };
    });
}

// Web-mock tiles: a labeled grid data URL — no network in tests.
// Fill/label vary by source so layer-toggle tests can tell them apart.
export async function mapTile(
  z: number,
  x: number,
  y: number,
  source: "osm" | "topo" = "osm"
): Promise<string> {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256">` +
    `<rect width="256" height="256" fill="${source === "topo" ? "#2a3038" : "#1a2030"}"/>` +
    `<text x="128" y="128" fill="#5a6a8a" font-size="16" text-anchor="middle">${source} ${z}/${x}/${y}</text></svg>`;
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
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

export async function findSimilar(): Promise<NearDupeSummary> {
  return { hashed: 6, failed: 0, groups: nearGroups.length, wasteBytes: 1_237_000 };
}

export async function nearGroupsList(): Promise<NearGroup[]> {
  return nearGroups;
}

export async function keepNearCopy(groupId: number, keepId: number): Promise<NearGroup[]> {
  nearGroups = nearGroups.map((g) => (g.id === groupId ? { ...g, keptId: keepId } : g));
  return nearGroups;
}

export function onNearProgress(_cb: (p: CleanupProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

/**
 * Deterministic placeholder for visual similarity: the web mock has no
 * engine vectors, so neighbours rank by id distance to the query (closest
 * ids first, ties broken by id). The query photo itself is always excluded.
 */
export async function similarTo(assetId: number, limit = 12): Promise<SimilarHit[]> {
  return assets
    .filter((a) => a.id !== assetId)
    .map((a) => ({
      id: a.id,
      path: a.path,
      name: a.name,
      kind: a.kind,
      score: 1 / (1 + Math.abs(a.id - assetId))
    }))
    .sort((x, y) => y.score - x.score || x.id - y.id)
    .slice(0, limit);
}

export async function embedLibrary(): Promise<EmbedSummary> {
  return { embedded: assets.length, failed: 0 };
}

export function onEmbedProgress(_cb: (p: EmbedProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
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
  const found = assets.filter((a) => drop.has(a.id));
  for (const a of found) trashedStore.set(a.id, a);
  assets = assets.filter((a) => !drop.has(a.id));
  return { trashed: found.length, failed: [] };
}

export async function restoreAssets(ids: number[]): Promise<{ restored: number; failed: string[] }> {
  const failed: string[] = [];
  let restored = 0;
  for (const id of ids) {
    const row = trashedStore.get(id);
    if (!row) {
      failed.push(`${id}: already emptied or renamed — restore from Finder`);
      continue;
    }
    trashedStore.delete(id);
    assets = [...assets, { ...row, missing: 0 }];
    restored++;
  }
  assets.sort((x, y) => x.id - y.id);
  return { restored, failed };
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
  return { enriched: 8, hashed: 8, scored: 8, events: 1, tagged: 2, embedded: 8, faces: 8, places: 0, failed: 0 };
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

export async function verifyLibrary(): Promise<{ missing: number; restored: number; unreadable: number; unreadablePaths: string[] }> {
  const missing = assets.filter((a) => a.missing !== 0).length;
  return { missing, restored: 0, unreadable: 0, unreadablePaths: [] };
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

export async function unassignFace(faceId: number): Promise<number> {
  for (const [cid, list] of facesByCluster) {
    const hit = list.find((f) => f.id === faceId);
    if (hit) {
      facesByCluster.set(
        cid,
        list.filter((f) => f.id !== faceId)
      );
      unassignedFaces.set(faceId, hit);
      // The asset stays linked while a sibling face in the same cluster
      // still shows the same photo.
      const siblingKeeps = (facesByCluster.get(cid) ?? []).some((f) => f.path === hit.path);
      if (!siblingKeeps) {
        const assetId = [...assets, ...trashedStore.values()].find((a) => a.path === hit.path)?.id;
        if (assetId !== undefined) clusterMembers.get(cid)?.delete(assetId);
      }
      clusters = clusters.map((c) => (c.id === cid ? { ...c, faceCount: c.faceCount - 1 } : c));
      return cid;
    }
  }
  throw new Error(`face ${faceId} is not assigned to any cluster`);
}

export async function assignFace(faceId: number, clusterId: number): Promise<void> {
  if (!clusters.some((c) => c.id === clusterId)) {
    throw new Error(`cluster ${clusterId} not found`);
  }
  const face = unassignedFaces.get(faceId);
  if (!face) throw new Error(`face ${faceId} not found`);
  unassignedFaces.delete(faceId);
  facesByCluster.set(clusterId, [...(facesByCluster.get(clusterId) ?? []), face]);
  const assetId = assets.find((a) => a.path === face.path)?.id;
  if (assetId !== undefined) {
    const ids = clusterMembers.get(clusterId) ?? new Set<number>();
    ids.add(assetId);
    clusterMembers.set(clusterId, ids);
  }
  clusters = clusters.map((c) =>
    c.id === clusterId ? { ...c, faceCount: (facesByCluster.get(clusterId) ?? []).length } : c,
  );
}

export async function restoreCluster(name: string | null, faceIds: number[]): Promise<number> {
  if (faceIds.length === 0) throw new Error("no faces to restore");
  const wanted = new Set(faceIds);
  const moving: FaceRow[] = [];
  for (const [cid, list] of [...facesByCluster]) {
    facesByCluster.set(
      cid,
      list.filter((f) => {
        if (wanted.has(f.id)) {
          moving.push(f);
          return false;
        }
        return true;
      }),
    );
  }
  // Detached faces (unassigned since the merge) rejoin too.
  for (const fid of faceIds) {
    const f = unassignedFaces.get(fid);
    if (f) {
      unassignedFaces.delete(fid);
      moving.push(f);
    }
  }
  if (moving.length !== faceIds.length) throw new Error("one of the faces does not exist");
  const nid = Math.max(0, ...clusters.map((c) => c.id)) + 1;
  const clean = name?.trim() ? name.trim() : null;
  if (clean) clusterNames.set(nid, clean);
  facesByCluster.set(nid, moving);
  const memberIds = new Set<number>();
  for (const f of moving) {
    const assetId = [...assets, ...trashedStore.values()].find((a) => a.path === f.path)?.id;
    if (assetId !== undefined) {
      memberIds.add(assetId);
      for (const ids of clusterMembers.values()) ids.delete(assetId);
    }
  }
  clusterMembers.set(nid, memberIds);
  clusters = clusters.map((c) => ({
    ...c,
    faceCount: (facesByCluster.get(c.id) ?? []).length,
  }));
  clusters = [
    ...clusters,
    { id: nid, faceCount: moving.length, representativePath: moving[0]?.path ?? null, name: clean },
  ];
  return nid;
}

export async function facesCount(): Promise<number> {
  let total = 0;
  for (const list of facesByCluster.values()) total += list.length;
  return total;
}

let sightings: PetSighting[] = [];

export async function detectPets(): Promise<DetectPetsSummary> {
  const images = assets.filter((a) => a.kind === "image");
  sightings = [
    {
      id: 1,
      path: "/photos/IMG_20261007_191433.jpg",
      label: "Dog",
      x: 0.2,
      y: 0.3,
      width: 0.4,
      height: 0.4,
      confidence: 0.93
    }
  ];
  return { images: images.length, sightings: sightings.length, missing: 0 };
}

export function onPetProgress(_cb: (p: PetProgress) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export function onPetError(_cb: (message: string) => void): Promise<() => void> {
  return Promise.resolve(() => {});
}

export async function sightingsList(): Promise<PetSighting[]> {
  return sightings;
}

export async function faceThumb(
  path: string,
  _x: number,
  _y: number,
  _width: number,
  _height: number,
  _maxPx = 256,
): Promise<{ image: string }> {
  // Mirrors the engine's "cannot decode image" failure for files on dead
  // volumes so the unreadable-face UI state is testable in web mode.
  if (path.includes("UNREADABLE")) throw new Error(`cannot decode image: ${path}`);
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
  if (cmd === "cached_thumb") {
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
