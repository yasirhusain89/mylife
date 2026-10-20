<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open, isWebMode, isApiMode, assetUrl } from "$lib/bridge";  import {
    blurredAssets,
    cleanupStats,
    clustersList,
    renameCluster,
    mergeClusters,
    unassignFace,
    detectPets,
    onPetProgress,
    onPetError,
    sightingsList,
    clusterEvents,
    eventsList,
    eventAssets,
    renameEvent,
    resolvePlaces,
    onGeocodeProgress,
    onGeocodeError,
    albumsList,
    albumCreate,
    albumRename,
    albumDelete,
    albumAssets,
    albumAdd,
    albumRemove,
    type AlbumRow,
    exportHighlight,
    exportBundle,
    type ExportSummary,
    type HighlightResult,
    faceThumb,
    facesInCluster,
    duplicatesList,
    engineThumb,
    findSimilar,
    similarTo,
    nearGroupsList,
    keepNearCopy,
    onNearProgress,
    enrichAssets,
    findDuplicates,
    runPostImport,
    onPipelineProgress,    keepCopy,
    onHashProgress,
    onQualityProgress,
    scoreQuality,
    trashAssets,
    importFolder,
    onEnrichProgress,
    libraryList,
    libraryCount,
    libraryMonths,
    type LibraryMonth,
    libraryStats,
    scanRoots,
    assetsInRoot,
    setFavorite,
    tagScenes,
    onTagProgress,    sceneTags,
    embedLibrary,
    onEmbedProgress,
    assetTags,
    onImportError,
    onImportProgress,
    onClusterProgress,
    runFaceClustering,
    verifyLibrary,
    type AssetRow,
    type CleanupProgress,
    type CleanupStats,
    type FaceClusterRow,
    type PetProgress,
    type PetSighting,
    type EventRow,
    type EventAssetRow,
    type GeocodeProgress,
    type FaceRow,
    type DuplicateGroup,
    type NearGroup,
    type EnrichProgress,
    type ImportProgress,
    type LibraryStats,
    type SimilarHit,
    type PipelineProgress,
    type ScanRoot,
    type SceneTag,
    type SceneTagCount,
    type TagProgress,
    type EmbedProgress,
    mapPins,
    mapTile,
    type MapPin,
    removeRoot,
    type RemoveRootSummary
  } from "$lib/bridge";
  import { engineCall, engineSysInfo, type SysInfo } from "$lib/bridge";
  import "leaflet/dist/leaflet.css";
  import * as L from "leaflet";
  import type { Coords, DoneCallback, LayerGroup, Map as LeafletMap, MapOptions } from "leaflet";

  type Row = { key: string; val: string };
  type EngineStatus =
    | { state: "idle" }
    | { state: "checking" }
    | { state: "up"; rows: Row[]; ms: number }
    | { state: "down"; error: string };

  const PAGE = 60;

  let status = $state<EngineStatus>({ state: "idle" });
  let assets = $state<AssetRow[]>([]);
  let stats = $state<LibraryStats | null>(null);
  let roots = $state<ScanRoot[]>([]);
  const RECENT_SEARCH_KEY = "fotoflow-recent-searches";
  const FOLDERS_OPEN_KEY = "fotoflow-folders-open";

  function loadFoldersOpen(): boolean {
    try {
      if (typeof localStorage === "undefined") return true;
      const raw = localStorage.getItem(FOLDERS_OPEN_KEY);
      return raw === null ? true : JSON.parse(raw) !== false;
    } catch {
      return true;
    }
  }

  function saveFoldersOpen(open: boolean) {
    try {
      localStorage.setItem(FOLDERS_OPEN_KEY, JSON.stringify(open));
    } catch {
      /* storage unavailable: preference just doesn't persist */
    }
  }

  let foldersOpen = $state(loadFoldersOpen());

  type GroupMode = "none" | "date" | "location" | "person" | "media";
  const GROUP_BY_KEY = "fotoflow-group-by";
  const GROUP_MODES: GroupMode[] = ["none", "date", "location", "person", "media"];

  function loadGroupBy(): GroupMode {
    try {
      if (typeof localStorage === "undefined") return "none";
      const raw = localStorage.getItem(GROUP_BY_KEY);
      return GROUP_MODES.includes(raw as GroupMode) ? (raw as GroupMode) : "none";
    } catch {
      return "none";
    }
  }

  function saveGroupBy(g: GroupMode) {
    try {
      localStorage.setItem(GROUP_BY_KEY, g);
    } catch {
      /* storage unavailable: preference just doesn't persist */
    }
  }

  let groupBy = $state<GroupMode>(loadGroupBy());

  function monthKeyOf(a: AssetRow): string {
    const t = a.capturedAt ?? a.modified;
    if (!t) return "";
    const d = new Date(t * 1000);
    return `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}`;
  }

  function monthLabelOf(key: string): string {
    if (!key) return "Unknown date";
    const [y, m] = key.split("-").map(Number);
    return new Date(Date.UTC(y, m - 1, 1)).toLocaleString(undefined, {
      month: "long",
      year: "numeric",
      timeZone: "UTC"
    });
  }

  function groupKeyOf(a: AssetRow): string {
    switch (groupBy) {
      case "date": return monthKeyOf(a);
      case "location": return a.place ?? "";
      case "person": return a.people[0] ?? "";
      case "media": return a.kind;
      default: return "";
    }
  }

  function groupLabelOf(a: AssetRow): string {
    switch (groupBy) {
      case "date": return monthLabelOf(monthKeyOf(a));
      case "location": return a.place ?? "No location";
      case "person": return a.people[0] ?? "No one tagged";
      case "media": return a.kind === "image" ? "Photos" : "Videos";
      default: return "";
    }
  }

  // Display order when grouped: sections sorted (newest month first,
  // names A–Z, unknowns last), newest photo first inside each section.
  // Carries the flat-list index so the viewer, selection and Similar
  // keep working on the underlying order.
  let displayAssets = $derived.by(() => {
    const items = assets.map((asset, index) => ({ asset, index }));
    if (groupBy === "none") return items;
    const dir = groupBy === "date" ? -1 : 1;
    // Unknowns sort last in every mode ("" is already last descending).
    const sortKey = (k: string) => (k === "" && groupBy !== "date" ? "\uffff" : k);
    return items.sort((a, b) => {
      const ka = sortKey(groupKeyOf(a.asset));
      const kb = sortKey(groupKeyOf(b.asset));
      if (ka < kb) return -dir;
      if (ka > kb) return dir;
      const ta = a.asset.capturedAt ?? a.asset.modified ?? 0;
      const tb = b.asset.capturedAt ?? b.asset.modified ?? 0;
      return tb - ta || b.asset.id - a.asset.id;
    });
  });

  let groupCounts = $derived.by(() => {
    const m = new Map<string, number>();
    if (groupBy === "none") return m;
    for (const a of assets) {
      const k = groupKeyOf(a);
      m.set(k, (m.get(k) ?? 0) + 1);
    }
    return m;
  });

  function loadRecentSearches(): string[] {
    try {
      if (typeof localStorage === "undefined") return [];
      const raw = localStorage.getItem(RECENT_SEARCH_KEY);
      const arr = raw ? JSON.parse(raw) : [];
      return Array.isArray(arr) ? arr.filter((s) => typeof s === "string").slice(0, 5) : [];
    } catch {
      return [];
    }
  }

  let filter = $state<"all" | "image" | "video">("all");
  let favFilter = $state(false);
  let petFilter = $state(false);
  let searchText = $state("");
  let searchQuery = $state<string | null>(null);
  let recentSearches = $state<string[]>(loadRecentSearches());
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  function saveRecentSearch(q: string) {
    recentSearches = [q, ...recentSearches.filter((s) => s !== q)].slice(0, 5);
    try {
      localStorage.setItem(RECENT_SEARCH_KEY, JSON.stringify(recentSearches));
    } catch {
      /* storage unavailable: recents just don't persist */
    }
  }

  function queueSearch() {
    if (searchTimer) clearTimeout(searchTimer);
    const text = searchText;
    searchTimer = setTimeout(() => void applySearch(text), 300);
  }

  async function applySearch(raw: string) {
    const q = raw.trim();
    searchText = q;
    searchQuery = q === "" ? null : q;
    if (q !== "") saveRecentSearch(q);
    await refresh();
  }

  function clearSearch() {
    if (searchTimer) clearTimeout(searchTimer);
    searchText = "";
    searchQuery = null;
    void refresh();
  }
  let sceneFilter = $state<string | null>(null);
  let tagOptions = $state<SceneTagCount[]>([]);
  let tagging = $state(false);
  let tagState = $state<TagProgress | null>(null);
  let monthFilter = $state<string | null>(null);
  let locFilter = $state<"all" | "with" | "without">("all");
  let personFilter = $state<number | null>(null);
  let months = $state<LibraryMonth[]>([]);
  let personOptions = $state<FaceClusterRow[]>([]);
  let filteredTotal = $state(0);

  function monthBounds(m: string): [number, number] {
    const [y, mo] = m.split("-").map(Number);
    return [Date.UTC(y, mo - 1, 1) / 1000, Date.UTC(y, mo, 1) / 1000];
  }

  function currentFilters() {
    const [capturedMin, capturedMax] = monthFilter ? monthBounds(monthFilter) : [undefined, undefined];
    return {
      kind: filter === "all" ? undefined : filter,
      favoriteOnly: favFilter || undefined,
      hasPet: petFilter || undefined,
      sceneTag: sceneFilter ?? undefined,
      query: searchQuery ?? undefined,
      personCluster: personFilter ?? undefined,
      hasLocation: locFilter === "all" ? undefined : locFilter === "with",
      capturedMin,
      capturedMax
    };
  }
  let root = $state<string | null>(null);
  let importing = $state(false);
  let progress = $state<ImportProgress | null>(null);
  let pipelining = $state(false);
  let pipelineState = $state<PipelineProgress | null>(null);
  let error = $state<string | null>(null);
  let thumbs = $state<Record<string, string>>({});
  let people = $state<FaceClusterRow[]>([]);
  let mergeSelect = $state<Set<number>>(new Set());
  let openClusterId = $state<number | null>(null);
  let pets = $state<PetSighting[]>([]);
  let petsLoading = $state(false);
  let petState = $state<PetProgress | null>(null);
  let typedPath = $state("");
  let enriching = $state(false);
  let enrichState = $state<EnrichProgress | null>(null);
  let cleanup = $state<CleanupStats | null>(null);
  let groups = $state<DuplicateGroup[]>([]);
  let nearGroups = $state<NearGroup[]>([]);
  let nearing = $state(false);
  let nearState = $state<CleanupProgress | null>(null);
  let blurred = $state<AssetRow[]>([]);
  let hashing = $state(false);
  let scoring = $state(false);
  let hashState = $state<CleanupProgress | null>(null);
  let qualityState = $state<CleanupProgress | null>(null);
  let deleteMode = $state<"trash" | "flag">("trash");
  let confirm = $state<{ ids: number[]; label: string } | null>(null);
  let tab = $state<"library" | "cleanup" | "people" | "events" | "albums" | "map" | "settings">("library");
  let flagged = $state<Set<number>>(new Set());
  let peopleLoading = $state(false);
  let events = $state<EventRow[]>([]);
  let eventsLoading = $state(false);
  let gapHours = $state(4);
  let openEventId = $state<number | null>(null);
  let eventMembers = $state<Record<number, EventAssetRow[]>>({});
  let resolvingPlaces = $state(false);
  let geocodeState = $state<GeocodeProgress | null>(null);
  // Lightbox viewer over the current library page (index into `assets`),
  // or over an explicit list (cluster review mode).
  let viewer = $state<{ index: number } | null>(null);
  let viewerList = $state<AssetRow[] | null>(null);
  let reviewCluster = $state<{ id: number; name: string } | null>(null);
  let viewerSrc = $state<string | null>(null);
  let viewerError = $state<string | null>(null);
  let viewerTags = $state<SceneTag[]>([]);
  // Image-query visual neighbours ("more like this photo").
  let similar = $state<{ assetId: number; name: string; rows: SimilarHit[] } | null>(null);
  let similarLoading = $state(false);
  let embedState = $state<EmbedProgress | null>(null);
  // Library multi-select: checkbox set + batch actions.
  let selectMode = $state(false);
  let selected = $state<Set<number>>(new Set());
  let albumOptions = $state<AlbumRow[]>([]);
  let batchAlbum = $state<number | null>(null);
  let toast = $state<string | null>(null);

  async function checkEngine() {
    status = { state: "checking" };
    const t0 = performance.now();
    try {
      const pong = await engineCall<{ pong: boolean }>("ping");
      if (!pong.pong) throw new Error("engine did not answer pong");
      const info = await engineSysInfo();
      status = {
        state: "up",
        ms: Math.round(performance.now() - t0),
        rows: [
          { key: "OS", val: `${info.os}` },
          { key: "Architecture", val: `${info.arch} · ${info.cpuCount} cores` },
          { key: "Memory", val: `${info.memoryGB.toFixed(1)} GB unified` },
          { key: "Swift engine", val: info.swift }
        ]
      };
    } catch (e) {
      status = { state: "down", error: String(e).replace(/^Error: /, "") };
    }
  }

  function fmtBytes(n: number): string {
    if (n <= 0) return "0 B";
    const units = ["B", "KB", "MB", "GB", "TB"];
    const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
    return `${(n / 1024 ** i).toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
  }

  function fmtDate(epoch: number): string {
    return new Date(epoch * 1000).toLocaleDateString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric"
    });
  }

  async function refresh() {
    resetThumbs();
    const f = currentFilters();
    assets = await libraryList({ ...f, limit: PAGE });
    filteredTotal = await libraryCount(f);
    stats = await libraryStats();
    months = await libraryMonths();
    try {
      roots = await scanRoots();
      await ensureAlbumOptions();
    } catch {
      roots = [];
    }
    try {
      tagOptions = await sceneTags();
    } catch {
      tagOptions = [];
    }
  }

  async function loadPersonOptions() {
    try {
      personOptions = await clustersList();
    } catch {
      personOptions = [];
    }
  }

  function libraryTotal(): number {
    return filteredTotal;
  }

  async function loadMore() {
    try {
      const next = await libraryList({
        ...currentFilters(),
        limit: PAGE,
        offset: assets.length
      });
      if (next.length === 0) return;
      assets = [...assets, ...next];
      void loadThumbs(next);
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  // Thumbnail pipeline: viewport-driven, pooled, cancellable.
  //
  // Engine thumbs arrive as raw base64; the web mock returns full data URIs.
  function thumbSrc(image: string): string {
    return image.startsWith("data:") ? image : `data:image/jpeg;base64,${image}`;
  }
  //
  // Library/picker figures register through `observeThumb`; the observer
  // enqueues only on-screen paths (200px overscan prefetch). Up to
  // THUMB_CONCURRENCY engine calls run at once instead of strictly
  // serially. resetThumbs() drops pending work and invalidates in-flight
  // results on tab switches; whatever is still visible is requeued.
  const THUMB_CONCURRENCY = 6;
  let thumbQueue: string[] = [];
  let thumbQueued = new Set<string>();
  let visiblePaths = new Set<string>();
  let thumbActive = 0;
  let thumbGen = 0;
  let thumbObserver: IntersectionObserver | null = null;

  function queueThumb(path: string) {
    if (thumbs[path] || thumbQueued.has(path)) return;
    thumbQueued.add(path);
    thumbQueue.push(path);
    void pumpThumbs();
  }

  async function pumpThumbs() {
    const gen = thumbGen;
    while (thumbActive < THUMB_CONCURRENCY && thumbQueue.length > 0) {
      const path = thumbQueue.shift() as string;
      thumbActive++;
      engineThumb(path, 256).then(
        ({ image }) => {
          if (gen === thumbGen && image) thumbs[path] = thumbSrc(image);
        },
        () => {
          /* unreadable asset: leave the placeholder */
        }
      ).finally(() => {
        thumbActive--;
        thumbQueued.delete(path);
        void pumpThumbs();
      });
    }
  }

  function resetThumbs() {
    thumbGen++;
    thumbQueue = [];
    thumbQueued.clear();
    for (const p of visiblePaths) queueThumb(p);
  }

  // Eager variant for bounded surfaces (cleanup lists, album members):
  // same pool, no viewport gating.
  function loadThumbs(rows: { path: string }[]) {
    for (const a of rows) queueThumb(a.path);
  }

  function ensureThumbObserver(): IntersectionObserver | null {
    if (thumbObserver) return thumbObserver;
    if (typeof IntersectionObserver === "undefined") return null;
    thumbObserver = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const p = (e.target as HTMLElement).dataset.thumbPath;
          if (!p) continue;
          if (e.isIntersecting) {
            visiblePaths.add(p);
            queueThumb(p);
          } else {
            visiblePaths.delete(p);
          }
        }
      },
      { rootMargin: "200px" }
    );
    return thumbObserver;
  }

  function observeThumb(node: HTMLElement, path: string) {
    node.dataset.thumbPath = path;
    const obs = ensureThumbObserver();
    if (obs) obs.observe(node);
    else queueThumb(path); // no viewport API (prerender): load eagerly
    return {
      destroy() {
        visiblePaths.delete(path);
        thumbObserver?.unobserve(node);
      }
    };
  }

  async function toggleFavorite(a: AssetRow) {
    const next = a.favorite === 0;
    try {
      await setFavorite(a.id, next);
      for (let i = 0; i < assets.length; i++) {
        if (assets[i].id === a.id) {
          assets[i] = { ...assets[i], favorite: next ? 1 : 0 };
          break;
        }
      }
      // An unstarred photo drops out of the Favorites view.
      if (favFilter && !next) await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function toggleSelect(id: number) {
    const next = new Set(selected);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selected = next;
  }

  function groupIds(key: string): number[] {
    const out: number[] = [];
    for (const { asset } of displayAssets) {
      if (groupKeyOf(asset) === key) out.push(asset.id);
    }
    return out;
  }

  // One-go group selection: selects every loaded photo in the section
  // (or clears just the section when all of it is already selected).
  // Additive across groups; entering select mode so the batch bar appears.
  function toggleGroup(key: string) {
    const ids = groupIds(key);
    const next = new Set(selected);
    const all = ids.length > 0 && ids.every((id) => next.has(id));
    if (all) for (const id of ids) next.delete(id);
    else for (const id of ids) next.add(id);
    selected = next;
    selectMode = true;
  }

  async function enterSelectMode() {
    selectMode = true;
    selected = new Set();
    try {
      albumOptions = await albumsList();
      batchAlbum = albumOptions.length > 0 ? albumOptions[0].id : null;
    } catch {
      albumOptions = [];
      batchAlbum = null;
    }
  }

  async function batchFavorite() {
    const ids = [...selected];
    try {
      for (const id of ids) await setFavorite(id, true);
      toast = `Starred ${ids.length} asset${ids.length === 1 ? "" : "s"}`;
      selected = new Set();
      await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function batchAddToAlbum() {
    if (batchAlbum === null || selected.size === 0) return;
    try {
      const r = await albumAdd(batchAlbum, [...selected]);
      toast = `Added ${r.added} photo${r.added === 1 ? "" : "s"} to the album`;
      selected = new Set();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function batchTrash() {
    if (selected.size === 0) return;
    deleteMode = "trash";
    confirm = { ids: [...selected], label: `${selected.size} selected assets` };
    selected = new Set();
  }

  // Lightbox viewer: full-size image via the engine, video via asset URL.
  function viewerRows(): AssetRow[] {
    return viewerList ?? assets;
  }

  function openViewer(index: number) {
    if (index < 0 || index >= assets.length) return;
    viewer = { index };
    viewerList = null;
    reviewCluster = null;
    viewerSrc = null;
    viewerError = null;
    viewerTags = [];
    void loadViewer(assets[index]);
  }

  // Cluster review: step through every source photo full-size to accept
  // (tag once on the card) or reject (per-photo removal below) the person.
  function openReview(cluster: FaceClusterRow) {
    const paths = clusterSourcePaths(cluster.id);
    if (paths.length === 0) {
      error = "No photos to review in this cluster yet — expand Show photos first.";
      return;
    }
    viewerList = paths.map((path) => {
      const row = assets.find((a) => a.path === path);
      if (row) return row;
      const name = path.split("/").pop() ?? path;
      return {
        id: -1, path, name, ext: name.split(".").pop() ?? "", kind: "image" as const,
        mime: null, sizeBytes: 0, modified: 0, capturedAt: null, width: null,
        height: null, duration: null, favorite: 0, missing: 0, sha256: null,
        place: null, people: []
      };
    });
    reviewCluster = { id: cluster.id, name: cluster.name ?? `Cluster ${cluster.id}` };
    viewer = { index: 0 };
    viewerSrc = null;
    viewerError = null;
    viewerTags = [];
    void loadViewer(viewerList[0]);
  }

  async function removePhotoFromReview() {
    const list = viewerList;
    const rc = reviewCluster;
    if (!viewer || !list || !rc) return;
    const photo = list[viewer.index];
    try {
      const faces = (peopleFaces[rc.id] ?? []).filter((f) => f.path === photo.path);
      let removed = 0;
      for (const f of faces) {
        await unassignFace(f.id);
        removed++;
      }
      people = await clustersList();
      peopleFaces = { ...peopleFaces, [rc.id]: await facesInCluster(rc.id) };
      void loadPersonOptions();
      const next = list.filter((a) => a.path !== photo.path);
      if (next.length === 0) {
        closeViewer();
        toast = "Removed the last photo — cluster is empty";
        return;
      }
      viewerList = next;
      viewer = { index: Math.min(viewer.index, next.length - 1) };
      viewerSrc = null;
      viewerError = null;
      viewerTags = [];
      void loadViewer(next[viewer.index]);
      toast = `Removed ${photo.name} (${removed} face${removed === 1 ? "" : "s"}) from ${rc.name}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function loadViewer(a: AssetRow) {
    viewerSrc = null;
    viewerError = null;
    viewerTags = [];
    try {
      if (a.kind === "video") {
        const url = assetUrl(a.path);
        // Empty in web mode: the markup falls back to the thumbnail poster.
        if (url) viewerSrc = url;
      } else {
        const { image } = await engineThumb(a.path, 1600);
        if (!image) throw new Error("engine returned no image");
        viewerSrc = thumbSrc(image);
      }
      try {
        viewerTags = await assetTags(a.id);
      } catch {
        viewerTags = [];
      }
    } catch (e) {
      viewerError = String(e).replace(/^Error: /, "");
    }
  }

  function stepViewer(delta: number) {
    const rows = viewerRows();
    if (!viewer || rows.length === 0) return;
    const next = (viewer.index + delta + rows.length) % rows.length;
    viewer = { index: next };
    void loadViewer(rows[next]);
  }

  function closeViewer() {
    viewer = null;
    viewerList = null;
    reviewCluster = null;
    viewerSrc = null;
    viewerError = null;
  }

  // Picasa-style slideshow: fullscreen theater overlay advancing on a timer.
  // Fixed overlay (not the Fullscreen API) so it works in the Tauri WebView
  // everywhere; videos pause the timer since they play on their own clock.
  const SLIDE_MS = 3000;
  let show = $state<{
    items: AssetRow[];
    index: number;
    playing: boolean;
    src: string | null;
    error: string | null;
  } | null>(null);
  let showTimer: ReturnType<typeof setInterval> | null = null;

  function stopShowTimer() {
    if (showTimer !== null) {
      clearInterval(showTimer);
      showTimer = null;
    }
  }

  function showCurrent(): AssetRow | null {
    return show && show.items.length > 0 ? show.items[show.index] : null;
  }

  async function loadShow(a: AssetRow) {
    if (!show) return;
    show.src = null;
    show.error = null;
    try {
      if (a.kind === "video") {
        const url = assetUrl(a.path);
        if (url) show.src = url;
        // A video holds the screen: pause auto-advance until the user moves on.
        stopShowTimer();
        if (show) show.playing = false;
      } else {
        const { image } = await engineThumb(a.path, 1600);
        if (!image) throw new Error("engine returned no image");
        if (show) show.src = thumbSrc(image);
      }
    } catch (e) {
      if (show) show.error = String(e).replace(/^Error: /, "");
    }
  }

  function restartShowTimer() {
    stopShowTimer();
    if (show?.playing) showTimer = setInterval(() => stepShow(1), SLIDE_MS);
  }

  function startShow(items: AssetRow[], index = 0) {
    if (items.length === 0) return;
    closeViewer();
    stopShowTimer();
    const at = Math.min(Math.max(index, 0), items.length - 1);
    show = { items, index: at, playing: true, src: null, error: null };
    void loadShow(items[at]);
    showTimer = setInterval(() => stepShow(1), SLIDE_MS);
  }

  function stepShow(delta: number) {
    if (!show || show.items.length === 0) return;
    const next = (show.index + delta + show.items.length) % show.items.length;
    show.index = next;
    void loadShow(show.items[next]);
    // Manual steps restart the countdown so the new slide gets full time.
    if (show.playing) restartShowTimer();
  }

  function toggleShowPlay() {
    if (!show) return;
    show.playing = !show.playing;
    restartShowTimer();
    // Resuming on a video would stall instantly; step to the next photo.
    if (show.playing && showCurrent()?.kind === "video") stepShow(1);
  }

  function closeShow() {
    stopShowTimer();
    show = null;
  }

  async function openSimilar(a: AssetRow) {
    // Videos never get vectors (embed backfill is images-only by design),
    // so fail fast with a friendly message instead of the raw error.
    if (a.kind !== "image") {
      error = "Similarity search works on photos — videos are not supported.";
      return;
    }
    similarLoading = true;
    embedState = null;
    similar = { assetId: a.id, name: a.name, rows: [] };
    try {
      try {
        similar = { assetId: a.id, name: a.name, rows: await similarTo(a.id, 12) };
      } catch (e) {
        // First run on a library imported before vectors existed: embed
        // everything once (one-time cost, progress shown), then retry.
        if (!/has no '.*' vector yet/.test(String(e))) throw e;
        const summary = await embedLibrary();
        similar = { assetId: a.id, name: a.name, rows: await similarTo(a.id, 12) };
        toast = `Embedded ${summary.embedded} photo${summary.embedded === 1 ? "" : "s"} for similarity`;
      }
      void loadThumbs(similar.rows.map((m) => ({ path: m.path }) as AssetRow));
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
      similar = null;
    } finally {
      similarLoading = false;
      embedState = null;
    }
  }

  function viewerKey(e: KeyboardEvent) {
    if (show) {
      if (e.key === "Escape") closeShow();
      else if (e.key === "ArrowRight") { e.preventDefault(); stepShow(1); }
      else if (e.key === "ArrowLeft") { e.preventDefault(); stepShow(-1); }
      else if (e.key === " ") { e.preventDefault(); toggleShowPlay(); }
      return;
    }
    if (!viewer) return;
    if (e.key === "Escape") closeViewer();
    else if (e.key === "ArrowRight") { e.preventDefault(); stepViewer(1); }
    else if (e.key === "ArrowLeft") { e.preventDefault(); stepViewer(-1); }
  }

  async function runImport(target: string) {
    error = null;
    root = target;
    importing = true;
    progress = null;
    try {
      const summary = await importFolder(target, true);
      progress = {
        scanId: summary.scanId,
        scanned: summary.scanned,
        imported: summary.imported,
        unchanged: summary.unchanged,
        bytes: summary.bytes
      };
      await refresh();
      void runPipeline();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      importing = false;
    }
  }

  async function runPipeline() {
    if (pipelining) return;
    pipelining = true;
    pipelineState = null;
    try {
      const summary = await runPostImport();
      const bits = [
        `${summary.enriched} enriched`,
        `${summary.hashed} hashed`,
        `${summary.scored} scored`,
        `${summary.events} events`,
        `${summary.tagged} tagged`,
        `${summary.embedded} embedded`,
        `${summary.faces} faces`,
        summary.places > 0 ? `${summary.places} places` : null,
        summary.failed > 0 ? `${summary.failed} failed` : null
      ].filter((b) => b !== null);
      toast = `Library organized: ${bits.join(" · ")}`;
      await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      pipelining = false;
    }
  }

  async function refreshCleanup() {
    resetThumbs();
    cleanup = await cleanupStats();
    groups = await duplicatesList();
    blurred = await blurredAssets(36);
    void loadThumbs(blurred);
    for (const g of groups) {
      for (const m of g.members) void loadThumbs([{ path: m.path } as AssetRow]);
    }
    try {
      nearGroups = await nearGroupsList();
    } catch {
      nearGroups = [];
    }
    for (const g of nearGroups) {
      for (const m of g.members) void loadThumbs([{ path: m.path } as AssetRow]);
    }
  }

  let highlightBusy = $state<string | null>(null);
  let highlightResult = $state<HighlightResult | null>(null);
  let highlightKey = $state<string | null>(null);

  let exportBusy = $state<string | null>(null);
  let exportResult = $state<ExportSummary | null>(null);
  let exportKey = $state<string | null>(null);

  function sourceKey(src: { eventId?: number; albumId?: number }): string {
    return src.eventId != null ? `e${src.eventId}` : `a${src.albumId}`;
  }

  async function runExport(
    src: { eventId?: number; albumId?: number },
    name: string,
    mode: "drive" | "whatsapp",
  ) {
    const dest = await open({ directory: true, multiple: false, title: "Choose export folder" });
    if (typeof dest !== "string" || dest === "") return;
    const key = `${sourceKey(src)}-${mode}`;
    exportBusy = key;
    exportResult = null;
    exportKey = key;
    try {
      exportResult = await exportBundle(src, name, mode, dest);
      toast = `Exported ${exportResult.exported} photos for ${mode === "drive" ? "Drive" : "WhatsApp"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      exportBusy = null;
    }
  }

  async function makeHighlight(src: { eventId?: number; albumId?: number }, name: string) {
    const key = sourceKey(src);
    highlightBusy = key;
    highlightResult = null;
    highlightKey = key;
    try {
      highlightResult = await exportHighlight(src, name.replace(/\s+/g, "-").toLowerCase());
      toast = "Highlight video exported";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      highlightBusy = null;
    }
  }

  let peopleFaces = $state<Record<number, FaceRow[]>>({});
  let deadFaces = $state<Set<number>>(new Set());
  let clusterState = $state<{ processed: number; total: number } | null>(null);

  async function runVerify() {
    try {
      const r = await verifyLibrary();
      await refresh();
      toast =
        r.missing === 0 && r.restored === 0
          ? "All indexed files are present"
          : `${r.missing} missing · ${r.restored} restored`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function loadPeople() {
    resetThumbs();
    peopleLoading = true;
    people = [];
    try {
      people = await clustersList();
      void loadClusterFaces();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      peopleLoading = false;
    }
  }

  function doRenameCluster(cluster: FaceClusterRow) {
    const known = [...new Set(people.map((p) => p.name).filter((n): n is string => !!n && n !== cluster.name))];
    askName("Person name", cluster.name ?? "", "Rename", (n) => void renameClusterTo(cluster.id, n), known);
  }

  async function renameClusterTo(clusterId: number, name: string) {
    try {
      await renameCluster(clusterId, name);
      people = await clustersList();
      void loadPersonOptions();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function toggleMerge(id: number) {
    const next = new Set(mergeSelect);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    mergeSelect = next;
  }

  async function toggleClusterPhotos(cluster: FaceClusterRow) {
    openClusterId = openClusterId === cluster.id ? null : cluster.id;
    if (openClusterId === null) return;
    try {
      const faces = await facesInCluster(cluster.id);
      peopleFaces = { ...peopleFaces, [cluster.id]: faces };
      // Fresh attempt: drop stale unreadable flags for these faces (a
      // reconnected volume reads fine again).
      const alive = new Set(deadFaces);
      for (const f of faces) alive.delete(f.id);
      deadFaces = alive;
      const seen = new Set<string>();
      const rows = faces
        .filter((f) => {
          if (seen.has(f.path)) return false;
          seen.add(f.path);
          return true;
        })
        .map((f) => ({ path: f.path }) as AssetRow);
      for (const f of faces) void loadFaceThumb(f);
      void loadThumbs(rows);
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function clusterSourcePaths(clusterId: number): string[] {
    const seen = new Set<string>();
    for (const f of peopleFaces[clusterId] ?? []) seen.add(f.path);
    return [...seen];
  }

  function viewPersonInLibrary(clusterId: number) {
    personFilter = clusterId;
    tab = "library";
    void refresh();
  }

  async function runMerge() {
    if (mergeSelect.size < 2) return;
    const ids = [...mergeSelect].sort((a, b) => a - b);
    const into = ids[0];
    try {
      for (let k = 1; k < ids.length; k++) await mergeClusters(ids[k], into);
      mergeSelect = new Set();
      people = await clustersList();
      void loadClusterFaces();
      void loadPersonOptions();
      toast = `Merged into one person (${people.find((c) => c.id === into)?.faceCount ?? 0} faces)`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function removeFaceFromCluster(faceId: number, clusterId: number) {
    try {
      await unassignFace(faceId);
      people = await clustersList();
      // Preserve the card's display mode: a collapsed card previews the
      // first 8 faces (like loadClusterFaces); refetching the full list
      // here would explode the grid from 8 tiles to the whole cluster.
      const all = await facesInCluster(clusterId);
      const shown = openClusterId === clusterId ? all : all.slice(0, 8);
      peopleFaces = { ...peopleFaces, [clusterId]: shown };
      void loadPersonOptions();
      toast = "Removed that face from this person";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  // In-app dialogs: the app WebView suppresses native prompt()/confirm(),
  // which made album create/delete and every rename silently do nothing.
  type NameDialog = { title: string; initial: string; okLabel: string; run: (name: string) => void; suggestions: string[] };
  let nameDialog = $state<NameDialog | null>(null);
  let nameValue = $state("");
  let deleteAlbumTarget = $state<AlbumRow | null>(null);

  function askName(title: string, initial: string, okLabel: string, run: (name: string) => void, suggestions: string[] = []) {
    nameDialog = { title, initial, okLabel, run, suggestions };
    nameValue = initial;
  }

  // MARK: - Albums

  let albums = $state<AlbumRow[]>([]);
  let albumsLoading = $state(false);
  let openAlbumId = $state<number | null>(null);
  let albumMembers = $state<Record<number, EventAssetRow[]>>({});
  let picker = $state<{ albumId: number; name: string } | null>(null);
  let pickerAssets = $state<AssetRow[]>([]);
  let pickerSelected = $state<Set<number>>(new Set());
  let pickerSearch = $state("");
  let pickerNewOpen = $state(false);
  let pickerNewName = $state("");
  let batchNewOpen = $state(false);
  let batchNewName = $state("");
  let inlineBusy = $state(false);
  let eventAlbumChoice = $state<Record<number, number | null>>({});
  let eventAlbumBusy = $state<number | null>(null);
  let rootAlbumChoice = $state<Record<string, number | null>>({});
  let rootAlbumBusy = $state<string | null>(null);
  let pickerFiltered = $derived(
    pickerSearch.trim() === ""
      ? pickerAssets
      : pickerAssets.filter((a) =>
          a.name.toLowerCase().includes(pickerSearch.trim().toLowerCase())
        )
  );

  async function loadAlbums() {
    resetThumbs();
    albumsLoading = true;
    try {
      albums = await albumsList();
      if (openAlbumId !== null && !albums.some((a) => a.id === openAlbumId)) openAlbumId = null;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      albumsLoading = false;
    }
  }

  async function loadAlbumMembers(albumId: number) {
    try {
      const rows = await albumAssets(albumId);
      albumMembers = { ...albumMembers, [albumId]: rows };
      void loadThumbs(rows.map((m) => ({ path: m.path } as AssetRow)));
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  // Map tab: pins from the backend, tiles from the Rust disk cache
  // (mock/API: inline grid data URLs / plain OSM URLs — see mapTile). The
  // map object is rebuilt when its container remounts after a tab switch.
  // Cache-first tiles: the Rust command serves disk-cached OSM PNGs as data
  // URLs so repeat pans cost no network.
  class CachedTiles extends L.GridLayer {
    createTile(coords: Coords, done: DoneCallback): HTMLImageElement {
      const img = document.createElement("img");
      img.alt = "";
      mapTile(coords.z, coords.x, coords.y).then(
        (url) => {
          img.src = url;
          done(undefined, img);
        },
        (err) => done(err instanceof Error ? err : new Error(String(err)), img)
      );
      return img;
    }
  }

  let mapEl = $state<HTMLDivElement | null>(null);
  let mapObj: LeafletMap | null = null;
  let pinLayer: LayerGroup | null = null;
  let mapPinsRows = $state<MapPin[]>([]);
  let mapLoading = $state(false);
  let mapError = $state<string | null>(null);

  async function loadMap() {
    if (mapLoading) return;
    mapLoading = true;
    mapError = null;
    try {
      mapPinsRows = await mapPins();
      loadThumbs(mapPinsRows.slice(0, 500));
    } catch (e) {
      mapError = String(e).replace(/^Error: /, "");
      return;
    } finally {
      mapLoading = false;
    }
    // The tab section mounts on the same click that calls loadMap; wait a
    // render pass so bind:this has run before touching the container.
    await tick();
    if (!mapEl) return;
    if (mapObj && !mapObj.getContainer().isConnected) {
      mapObj.remove();
      mapObj = null;
      pinLayer = null;
    }
    if (!mapObj) {
      const opts: MapOptions = { attributionControl: true };
      mapObj = L.map(mapEl, opts).setView([20, 0], 2);
      new CachedTiles({ attribution: "© OpenStreetMap contributors", tileSize: 256 }).addTo(mapObj);
    }
    renderPins();
    requestAnimationFrame(() => mapObj?.invalidateSize());
  }

  function renderPins() {
    if (!mapObj) return;
    pinLayer?.remove();
    pinLayer = L.layerGroup().addTo(mapObj);
    for (const p of mapPinsRows) {
      const m = L.circleMarker([p.latitude, p.longitude], {
        radius: 7,
        color: "#2f6bff",
        weight: 2,
        fillColor: "#2f6bff",
        fillOpacity: 0.65
      });
      m.bindPopup(() => pinPopupEl(p));
      pinLayer.addLayer(m);
    }
    if (mapPinsRows.length > 0) {
      mapObj.fitBounds(
        L.latLngBounds(mapPinsRows.map((p) => [p.latitude, p.longitude] as [number, number])),
        { padding: [30, 30] }
      );
    }
  }

  function pinPopupEl(pin: MapPin): HTMLElement {
    const el = document.createElement("div");
    el.className = "map-popup";
    const thumb = thumbs[pin.path];
    if (thumb) {
      const img = document.createElement("img");
      img.src = thumb;
      img.alt = pin.name;
      el.appendChild(img);
    }
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = pin.name;
    el.appendChild(name);
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "tiny";
    btn.textContent = "Show in Library";
    btn.onclick = () => {
      mapObj?.closePopup();
      tab = "library";
      void applySearch(pin.name);
    };
    el.appendChild(btn);
    return el;
  }

  function toggleAlbum(album: AlbumRow) {
    openAlbumId = openAlbumId === album.id ? null : album.id;
    if (openAlbumId !== null) void loadAlbumMembers(album.id);
  }

  async function createAlbum(name: string) {
    try {
      await albumCreate(name);
      await loadAlbums();
      try {
        albumOptions = await albumsList();
        if (batchAlbum === null && albumOptions.length > 0) batchAlbum = albumOptions[0].id;
      } catch {
        /* album dropdown refresh is best-effort */
      }
      toast = `Created “${name}”`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function ensureAlbumOptions() {
    try {
      albumOptions = await albumsList();
      if (albumOptions.length > 0) {
        if (batchAlbum === null || !albumOptions.some((a) => a.id === batchAlbum)) {
          batchAlbum = albumOptions[0].id;
        }
        const next: Record<number, number | null> = { ...eventAlbumChoice };
        for (const e of events) {
          if (next[e.id] == null || !albumOptions.some((a) => a.id === next[e.id])) {
            next[e.id] = albumOptions[0].id;
          }
        }
        eventAlbumChoice = next;
        const nextRoots: Record<string, number | null> = { ...rootAlbumChoice };
        for (const r of roots) {
          if (nextRoots[r.root] == null || !albumOptions.some((a) => a.id === nextRoots[r.root])) {
            nextRoots[r.root] = albumOptions[0].id;
          }
        }
        rootAlbumChoice = nextRoots;
      }
    } catch {
      /* dropdown options are best-effort */
    }
  }

  async function inlineCreateAlbum(rawName: string, origin: "picker" | "batch") {
    const name = rawName.trim();
    if (name === "" || inlineBusy) return;
    inlineBusy = true;
    try {
      const id = await albumCreate(name);
      const list = await albumsList();
      albums = list;
      albumOptions = list;
      if (origin === "picker") {
        picker = { albumId: id, name };
        pickerNewOpen = false;
        pickerNewName = "";
      } else {
        batchAlbum = id;
        batchNewOpen = false;
        batchNewName = "";
      }
      toast = `Created “${name}”`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      inlineBusy = false;
    }
  }

  async function addEventToAlbum(eventId: number) {
    const albumId = eventAlbumChoice[eventId];
    const members = eventMembers[eventId] ?? [];
    if (albumId == null || members.length === 0 || eventAlbumBusy !== null) return;
    eventAlbumBusy = eventId;
    try {
      const r = await albumAdd(albumId, members.map((m) => m.id));
      const target = albumOptions.find((a) => a.id === albumId);
      await loadAlbums();
      toast =
        target != null
          ? `Added ${r.added} photo${r.added === 1 ? "" : "s"} to “${target.name}”`
          : `Added ${r.added} photo${r.added === 1 ? "" : "s"} to the album`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventAlbumBusy = null;
    }
  }

  async function addRootToAlbum(root: string) {
    const albumId = rootAlbumChoice[root];
    if (albumId == null || rootAlbumBusy !== null) return;
    rootAlbumBusy = root;
    try {
      const ids = await assetsInRoot(root);
      if (ids.length === 0) {
        toast = `No assets under ${root}`;
        return;
      }
      const r = await albumAdd(albumId, ids);
      const target = albumOptions.find((a) => a.id === albumId);
      await loadAlbums();
      toast =
        target != null
          ? `Added ${r.added} photo${r.added === 1 ? "" : "s"} from ${root} to “${target.name}”`
          : `Added ${r.added} photo${r.added === 1 ? "" : "s"} to the album`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      rootAlbumBusy = null;
    }
  }

  function newAlbum() {
    askName("New album", "", "Create", (n) => void createAlbum(n));
  }

  function renameAlbum(album: AlbumRow) {
    askName("Album name", album.name, "Rename", (n) => void renameAlbumTo(album.id, n));
  }

  async function renameAlbumTo(albumId: number, name: string) {
    try {
      await albumRename(albumId, name);
      await loadAlbums();
      try {
        albumOptions = await albumsList();
      } catch {
        /* best-effort */
      }
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function deleteAlbum(album: AlbumRow) {
    deleteAlbumTarget = album;
  }

  async function confirmDeleteAlbum() {
    const target = deleteAlbumTarget;
    deleteAlbumTarget = null;
    if (!target) return;
    try {
      await albumDelete(target.id);
      await loadAlbums();
      try {
        albumOptions = await albumsList();
        if (batchAlbum !== null && !albumOptions.some((a) => a.id === batchAlbum)) {
          batchAlbum = albumOptions.length > 0 ? albumOptions[0].id : null;
        }
      } catch {
        /* best-effort */
      }
      toast = "Album deleted";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  // Indexed-folder removal: forgets the root's index rows (files on disk
  // are untouched). Confirm dialog mirrors the album delete below.
  let removeRootTarget = $state<ScanRoot | null>(null);
  let removingRoot = $state(false);

  function askRemoveRoot(root: ScanRoot) {
    removeRootTarget = root;
  }

  async function confirmRemoveRoot() {
    const target = removeRootTarget;
    removeRootTarget = null;
    if (!target || removingRoot) return;
    removingRoot = true;
    try {
      const s = await removeRoot(target.root);
      await refresh();
      toast = `Removed ${target.root} — ${s.removedAssets} asset${s.removedAssets === 1 ? "" : "s"} forgotten`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      removingRoot = false;
    }
  }

  function openPicker(album: AlbumRow) {
    picker = { albumId: album.id, name: album.name };
    pickerSelected = new Set();
    pickerSearch = "";
    pickerNewOpen = false;
    pickerNewName = "";
    libraryList({ limit: 500 })
      .then((rows) => {
        pickerAssets = rows;
      })
      .catch((e) => (error = String(e).replace(/^Error: /, "")));
  }

  function togglePicker(assetId: number) {
    const next = new Set(pickerSelected);
    if (next.has(assetId)) next.delete(assetId);
    else next.add(assetId);
    pickerSelected = next;
  }

  async function addPicked() {
    const target = picker;
    if (!target) return;
    const ids = [...pickerSelected];
    picker = null;
    resetThumbs();
    if (ids.length === 0) return;
    try {
      const r = await albumAdd(target.albumId, ids);
      await loadAlbums();
      if (openAlbumId === target.albumId) void loadAlbumMembers(target.albumId);
      toast = `Added ${r.added} photo${r.added === 1 ? "" : "s"} to “${target.name}”`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function removeFromAlbum(albumId: number, assetId: number) {
    try {
      await albumRemove(albumId, assetId);
      await loadAlbums();
      void loadAlbumMembers(albumId);
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  /// Runs the engine's face detector + clusterer over the whole library.
  async function findPeople() {
    peopleLoading = true;
    clusterState = { processed: 0, total: 0 };
    try {
      await runFaceClustering();
      people = await clustersList();
      void loadClusterFaces();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      peopleLoading = false;
      clusterState = null;
    }
  }

  async function loadClusterFaces() {
    const next: Record<number, FaceRow[]> = {};
    deadFaces = new Set();
    for (const c of people) {
      try {
        next[c.id] = (await facesInCluster(c.id)).slice(0, 8);
      } catch {
        /* cluster listing is best-effort */
      }
    }
    peopleFaces = next;
    for (const faces of Object.values(next)) {
      for (const f of faces) void loadFaceThumb(f);
    }
  }

  async function loadPets() {
    resetThumbs();
    petsLoading = true;
    pets = [];
    try {
      pets = await sightingsList();
      for (const s of pets) {
        const key = `pet:${s.id}`;
        if (thumbs[key]) continue;
        try {
          const { image } = await faceThumb(s.path, s.x, s.y, s.width, s.height, 160);
          if (image) thumbs[key] = thumbSrc(image);
        } catch {
          /* unreadable pet crop */
        }
      }
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      petsLoading = false;
    }
  }

  async function findPets() {
    petsLoading = true;
    petState = { processed: 0, total: 0, sightings: 0 };
    try {
      await detectPets();
      await loadPets();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      petsLoading = false;
      petState = null;
    }
  }

  async function loadFaceThumb(f: FaceRow) {
    const key = `face:${f.id}`;
    if (thumbs[key]) return;
    try {
      const { image } = await faceThumb(f.path, f.x, f.y, f.width, f.height, 160);
      if (image) thumbs[key] = thumbSrc(image);
    } catch {
      // Source file unreadable (dead volume): remember so the tile says so
      // instead of looking forever like a pending load.
      deadFaces = new Set(deadFaces).add(f.id);
    }
  }

  async function loadEvents() {
    resetThumbs();
    eventsLoading = true;
    try {
      events = await eventsList();
      await ensureAlbumOptions();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function runClusterEvents() {
    eventsLoading = true;
    try {
      await clusterEvents(gapHours, 500);
      events = await eventsList();
      await ensureAlbumOptions();
      toast = `Grouped into ${events.length} event${events.length === 1 ? "" : "s"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function loadEventMembers(eventId: number) {
    try {
      const rows = await eventAssets(eventId);
      eventMembers = { ...eventMembers, [eventId]: rows };
      void loadThumbs(rows.map((m) => ({ path: m.path }) as AssetRow));
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function toggleEvent(event: EventRow) {
    openEventId = openEventId === event.id ? null : event.id;
    if (openEventId !== null) void loadEventMembers(event.id);
  }

  async function runResolvePlaces() {
    resolvingPlaces = true;
    geocodeState = null;
    try {
      const summary = await resolvePlaces();
      events = await eventsList();
      toast =
        summary.total === 0
          ? "Every located event already has a place name"
          : `Resolved ${summary.resolved + summary.cached} of ${summary.total} place name${summary.total === 1 ? "" : "s"}` +
            (summary.failed > 0 ? ` (${summary.failed} failed — retry later)` : "");
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      resolvingPlaces = false;
    }
  }

  function doRename(event: EventRow) {
    askName("Event name", event.name, "Rename", (n) => void renameEventTo(event.id, n));
  }

  async function renameEventTo(eventId: number, name: string) {
    try {
      await renameEvent(eventId, name);
      events = await eventsList();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function groupEventsByMonth(list: EventRow[]): { month: string; events: EventRow[] }[] {
    const map = new Map<string, EventRow[]>();
    for (const e of list) {
      const d = new Date(e.startedAt * 1000);
      const key = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
      const bucket = map.get(key);
      if (bucket) bucket.push(e);
      else map.set(key, [e]);
    }
    return [...map.entries()].map(([month, bucket]) => ({ month, events: bucket }));
  }

  function monthLabel(m: string): string {
    const [y, mo] = m.split("-").map(Number);
    return new Date(Date.UTC(y, mo - 1, 1)).toLocaleDateString("en-US", {
      month: "long",
      year: "numeric",
      timeZone: "UTC"
    });
  }

  async function runFindDuplicates() {
    hashing = true;
    hashState = null;
    try {
      cleanup = await findDuplicates();
      await refreshCleanup();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      hashing = false;
    }
  }

  async function runFindSimilar() {
    nearing = true;
    nearState = null;
    try {
      const summary = await findSimilar();
      toast =
        summary.groups === 0
          ? "No similar photos found"
          : `${summary.groups} similar group${summary.groups === 1 ? "" : "s"} · ${fmtBytes(summary.wasteBytes)} recoverable`;
      await refreshCleanup();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      nearing = false;
    }
  }

  async function chooseNearKeeper(groupId: number, keepId: number) {
    try {
      nearGroups = await keepNearCopy(groupId, keepId);
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function runScoreQuality() {
    scoring = true;
    qualityState = null;
    try {
      cleanup = await scoreQuality();
      await refreshCleanup();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      scoring = false;
    }
  }

  async function chooseKeeper(groupId: number, keepId: number) {
    groups = await keepCopy(groupId, keepId);
  }

  async function applyDeletion() {
    if (!confirm) return;
    const { ids, label } = confirm;
    confirm = null;
    try {
      if (deleteMode === "trash") {
        const result = await trashAssets(ids);
        if (result.failed.length > 0) {
          error = `${result.trashed} moved to Trash, ${result.failed.length} failed`;
        }
      } else {
        // Flag only: nothing on disk changes, the choice stays reversible.
        for (const id of ids) flagged.add(id);
        flagged = flagged;
      }
      await refreshCleanup();
      toast = `${label} · ${deleteMode === "trash" ? "moved to Trash" : "flagged only"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function runEnrich() {
    if (enriching) return;
    enriching = true;
    enrichState = { processed: 0, enriched: 0, failed: 0, pending: 0 };
    try {
      const summary = await enrichAssets(3);
      enrichState = {
        processed: summary.processed,
        enriched: summary.enriched,
        failed: summary.failed,
        pending: summary.pending
      };
      await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      enriching = false;
    }
  }

  async function runTagScenes() {
    if (tagging) return;
    tagging = true;
    tagState = null;
    try {
      const summary = await tagScenes();
      toast =
        summary.total === 0
          ? "Every photo already has scene tags"
          : `Tagged ${summary.tagged} of ${summary.total} photos` +
            (summary.failed > 0 ? ` (${summary.failed} unreadable)` : "");
      await refresh();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      tagging = false;
    }
  }

  async function chooseFolder() {
    try {
      const picked = await open({ directory: true, multiple: false, title: "Add photo folder" });
      if (typeof picked !== "string" || picked === "") return;
      await runImport(picked);
    } catch (e) {
      error = `Folder picker unavailable: ${String(e).replace(/^Error: /, "")} — paste a path below instead.`;
    }
  }

  async function importTypedPath() {
    const target = typedPath.trim();
    if (target === "") return;
    await runImport(target);
  }

  // The dev server is also reachable from a plain browser tab, where the Tauri
  // IPC bridge does not exist. Skip registration there instead of throwing.
  const inApp = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  const web = isWebMode();
  const api = isApiMode();

  onMount(() => {
    if (!inApp && !web && !api) {
      error = "Open this inside the fotoflow app window — the dev-server tab has no engine bridge.";
      return;
    }
    // API thin slice serves reads only (no progress streams); the Tauri
    // `listen` calls have no HTTP equivalent and would reject under plain
    // vite, so skip registration there and rely on refresh().
    const offs = api
      ? []
      : [
      onImportProgress((p) => {
        if (importing) progress = p;
      }),
      onImportError((m) => {
        error = m;
      }),
      onEnrichProgress((p) => {
        if (enriching) enrichState = p;
      }),
      onHashProgress((p) => {
        if (hashing) hashState = p;
      }),
      onQualityProgress((p) => {
        if (scoring) qualityState = p;
      }),
      onNearProgress((p) => {
        if (nearing) nearState = p;
      }),
      onClusterProgress((p) => {
        if (peopleLoading && clusterState) {
          clusterState = { processed: p.processed, total: p.total };
        }
      }),
      onPetProgress((p) => {
        if (petsLoading) petState = p;
      }),
      onPetError((m) => {
        error = m;
      }),
      onGeocodeProgress((p) => {
        if (resolvingPlaces) geocodeState = p;
      }),
      onGeocodeError((m) => {
        error = m;
      }),
      onTagProgress((p) => {
        if (tagging) tagState = p;
      }),
      onEmbedProgress((p) => {
        if (similarLoading) embedState = p;
      }),
      onPipelineProgress((p) => {
        if (pipelining) pipelineState = p;
      })
    ];
    void refresh();
    void loadPersonOptions();
    return () => {
      for (const off of offs) void off.then((f) => f());
      thumbObserver?.disconnect();
      thumbObserver = null;
    };
  });
</script>

<svelte:window onkeydown={viewerKey} />
<main class="container">
  <header>
    <h1>fotoflow</h1>
    <p class="tagline">on-device library management for photo and video</p>
  </header>

  <nav class="tabs">
    <button class="tab" class:on={tab === "library"} onclick={() => { tab = "library"; void refresh(); }}>Library</button>
    <button class="tab" class:on={tab === "cleanup"} onclick={() => { tab = "cleanup"; void refreshCleanup(); }}>
      Cleanup{#if cleanup && cleanup.duplicateGroups > 0}&nbsp;{cleanup.duplicateGroups}{/if}
    </button>
    <button class="tab" class:on={tab === "people"} onclick={() => { tab = "people"; void loadPeople(); void loadPets(); }}>
      People{#if people.length > 0}&nbsp;{people.length}{/if}
    </button>
    <button class="tab" class:on={tab === "events"} onclick={() => { tab = "events"; void loadEvents(); }}>
      Events{#if events.length > 0}&nbsp;{events.length}{/if}
    </button>
    <button class="tab" class:on={tab === "albums"} onclick={() => { tab = "albums"; void loadAlbums(); }}>
      Albums{#if albums.length > 0}&nbsp;{albums.length}{/if}
    </button>
    <button class="tab" class:on={tab === "map"} onclick={() => { tab = "map"; void loadMap(); }}>
      Map
    </button>
    <button class="tab" class:on={tab === "settings"} onclick={() => { tab = "settings"; }}>
      Settings
    </button>
  </nav>

  {#if error}
    <div class="error global-error" role="alert">
      <span>{error}</span>
      <button class="tiny" onclick={() => (error = null)} aria-label="Dismiss error">Dismiss</button>
    </div>
  {/if}

  {#if tab === "cleanup"}
    <section class="card" aria-labelledby="cleanup-heading">
      <div class="card-head">
        <h2 id="cleanup-heading">Cleanup</h2>
      </div>

      <div class="actions">
        <button onclick={() => void runFindDuplicates()} disabled={hashing}>
          {hashing ? "Hashing…" : "Find exact duplicates"}
        </button>
        <button onclick={() => void runScoreQuality()} disabled={scoring}>
          {scoring ? "Scoring…" : "Score sharpness"}
        </button>
        <button onclick={() => void runFindSimilar()} disabled={nearing}>
          {nearing ? "Comparing…" : "Find similar photos"}
        </button>
      </div>

      {#if hashState}
        <p class="hint">{hashState.hashed ?? 0} hashed across {hashState.buckets ?? 0} size buckets</p>
      {/if}
      {#if nearState}
        <p class="hint">{nearState.hashed ?? 0} perceptual hashes computed</p>
      {/if}
      {#if qualityState}
        <p class="hint">{qualityState.scored ?? 0} scored · {qualityState.blurred ?? 0} look blurred</p>
      {/if}

      {#if cleanup}
        <div class="stats">
          <span class="chip static">{cleanup.duplicateGroups} duplicate groups</span>
          <span class="chip static">{fmtBytes(cleanup.wasteBytes)} recoverable</span>
          <span class="chip static">{cleanup.blurred} blurred</span>
        </div>
      {/if}

      <label class="mode">
        Deleting moves files to the macOS Trash
        <select bind:value={deleteMode}>
          <option value="trash">Move to Trash (reversible)</option>
          <option value="flag">Flag only (nothing changes)</option>
        </select>
      </label>

      {#if groups.length}
        <h3 class="sub">Duplicate groups</h3>
        {#each groups as group (group.id)}
          <article class="group">
            <header>
              <span>{group.members.length} copies · {fmtBytes(group.sizeBytes)} each</span>
              <span class="waste">keep 1, free {fmtBytes(group.wasteBytes)}</span>
            </header>
            <div class="members">
              {#each group.members as member (member.id)}
                <figure class:keeper={member.id === group.keptId}>
                  <div class="thumb small">
                    {#if thumbs[member.path]}
                      <img src={thumbs[member.path]} alt={member.name} loading="lazy" />
                    {:else}
                      <span class="placeholder">IMG</span>
                    {/if}
                  </div>
                  <figcaption>
                    <span class="name" title={member.path}>{member.name}</span>
                    {#if member.id === group.keptId}
                      <span class="badge keep">keeper</span>
                    {:else}
            <div class="row-actions">
                        <button class="tiny" onclick={() => void chooseKeeper(group.id, member.id)}>
                          Keep this
                        </button>
                        <button class="tiny danger" onclick={() => (confirm = { ids: [member.id], label: member.name })}>
                          Remove
                        </button>
                      </div>
                    {/if}
                  </figcaption>
                </figure>
              {/each}
            </div>
            {#if group.members.length > 1}
              <button
                class="tiny danger"
                onclick={() => (confirm = { ids: group.members.filter((m) => m.id !== group.keptId).map((m) => m.id), label: `${group.members.length - 1} duplicates` })}
              >
                Move {group.members.length - 1} extra {group.members.length - 1 === 1 ? "copy" : "copies"} to Trash
              </button>
            {/if}
          </article>
        {/each}
      {/if}

      {#if nearGroups.length}
        <h3 class="sub">Similar photos</h3>
        <p class="hint">Perceptual matches (resaves, crops, resizes) — review before trashing.</p>
        {#each nearGroups as group (group.id)}
          <article class="group">
            <header>
              <span>{group.members.length} similar · keep 1</span>
              <span class="waste">free {fmtBytes(group.wasteBytes)}</span>
            </header>
            <div class="members">
              {#each group.members as member (member.id)}
                <figure class:keeper={member.id === group.keptId}>
                  <div class="thumb small">
                    {#if thumbs[member.path]}
                      <img src={thumbs[member.path]} alt={member.name} loading="lazy" />
                    {:else}
                      <span class="placeholder">IMG</span>
                    {/if}
                  </div>
                  <figcaption>
                    <span class="name" title={member.path}>{member.name}</span>
                    {#if member.id === group.keptId}
                      <span class="badge keep">keeper</span>
                    {:else}
                      <span class="row-actions">
                        <button class="tiny" onclick={() => void chooseNearKeeper(group.id, member.id)}>
                          Keep this
                        </button>
                        <button class="tiny danger" onclick={() => (confirm = { ids: [member.id], label: member.name })}>
                          Trash
                        </button>
                      </span>
                    {/if}
                  </figcaption>
                </figure>
              {/each}
            </div>
            {#if group.members.length > 1}
              <button
                class="tiny danger"
                onclick={() => (confirm = { ids: group.members.filter((m) => m.id !== group.keptId).map((m) => m.id), label: `${group.members.length - 1} similar photos` })}
              >
                Move {group.members.length - 1} extra {group.members.length - 1 === 1 ? "copy" : "copies"} to Trash
              </button>
            {/if}
          </article>
        {/each}
      {/if}

      {#if blurred.length}
        <h3 class="sub">Likely blurred</h3>
        <div class="grid">
          {#each blurred as asset (asset.id)}
            <figure>
              <div class="thumb">
                {#if thumbs[asset.path]}
                  <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
                {:else}
                  <span class="placeholder">{asset.ext.toUpperCase()}</span>
                {/if}
              </div>
              <figcaption>
                <span class="name" title={asset.name}>{asset.name}</span>
                <button class="tiny danger" onclick={() => (confirm = { ids: [asset.id], label: asset.name })}>
                  {flagged.has(asset.id) ? "flagged" : "remove"}
                </button>
              </figcaption>
            </figure>
          {/each}
        </div>
      {/if}
    </section>
  {:else if tab === "library"}
  <section class="card" aria-labelledby="library-heading">
    <div class="card-head">
      <h2 id="library-heading">Library</h2>
      <button onclick={chooseFolder} disabled={importing}>
        {importing ? "Importing…" : "Add folder"}
      </button>
      {#if selectMode}
        <button class="ghost" onclick={() => { selectMode = false; selected = new Set(); }}>Done</button>
      {:else}
        <button class="ghost" onclick={() => void enterSelectMode()}>Select</button>
      {/if}
    </div>

    <div class="path-entry" role="search">
      <input
        type="search"
        placeholder="Search names, tags, people, places…"
        aria-label="Search library"
        bind:value={searchText}
        oninput={queueSearch}
      />
      {#if searchText !== ""}
        <button type="button" class="ghost" onclick={clearSearch}>Clear</button>
      {/if}
    </div>
    {#if searchText === "" && recentSearches.length > 0}
      <div class="enrich">
        <span class="hint">Recent:</span>
        {#each recentSearches as r (r)}
          <button class="chip" onclick={() => void applySearch(r)}>{r}</button>
        {/each}
      </div>
    {/if}

    <form
      class="path-entry"
      onsubmit={(e) => {
        e.preventDefault();
        void importTypedPath();
      }}
    >
      <input
        type="text"
        placeholder="…or paste a folder path"
        bind:value={typedPath}
        disabled={importing}
      />
      <button type="submit" disabled={importing || typedPath.trim() === ""}>Scan</button>
    </form>

    {#if root}
      <p class="root" title={root}>{root}</p>
    {/if}

    {#if roots.length}
      <div class="roots-head">
        <h3 class="sub">Indexed folders</h3>
        <button
          type="button"
          class="tiny"
          aria-expanded={foldersOpen}
          aria-controls="indexed-folders"
          onclick={() => {
            foldersOpen = !foldersOpen;
            saveFoldersOpen(foldersOpen);
          }}
        >
          {foldersOpen ? "Hide" : `Show (${roots.length})`}
        </button>
      </div>
      {#if foldersOpen}
      <ul class="roots" id="indexed-folders">
        {#each roots as r (r.root)}
          <li>
            <span class="root" title={r.root}>{r.root}</span>
            <span class="hint">
              {r.assets} asset{r.assets === 1 ? "" : "s"}{#if r.missing > 0} · {r.missing} unreachable{/if}
              · scanned {r.scans} time{r.scans === 1 ? "" : "s"} · {fmtDate(r.lastStartedAt)}
            </span>
            {#if albumOptions.length > 0}
              <span class="row-actions">
                <select
                  aria-label={`Add everything under ${r.root} to album`}
                  onchange={(e) => {
                    rootAlbumChoice = {
                      ...rootAlbumChoice,
                      [r.root]: e.currentTarget.value === "" ? null : Number(e.currentTarget.value)
                    };
                  }}
                >
                  {#each albumOptions as a (a.id)}
                    <option
                      value={a.id}
                      selected={(rootAlbumChoice[r.root] ?? albumOptions[0]?.id) === a.id}
                    >{a.name}</option>
                  {/each}
                </select>
                <button
                  class="tiny"
                  disabled={r.assets === 0 || (rootAlbumChoice[r.root] ?? albumOptions[0]?.id) == null || rootAlbumBusy !== null}
                  onclick={() => void addRootToAlbum(r.root)}
                >
                  {rootAlbumBusy === r.root ? "Adding…" : "＋ All to album"}
                </button>
              </span>
            {/if}
            <span class="row-actions">
              <button
                type="button"
                class="tiny danger"
                title={`Forget ${r.root} (files on disk are untouched)`}
                aria-label={`Remove ${r.root} from the index`}
                disabled={removingRoot}
                onclick={() => askRemoveRoot(r)}
              >
                Remove
              </button>
            </span>
          </li>
        {/each}
      </ul>
      {/if}
    {/if}

    {#if progress}
      <div class="progress">
        <div class="bar" class:live={importing}><span></span></div>
        <p class="hint">
          {progress.scanned} scanned · {progress.imported} new · {progress.unchanged} unchanged ·
          {fmtBytes(progress.bytes)}
        </p>
      </div>
    {/if}

    {#if pipelining}
      <div class="progress">
        <div class="bar live"><span></span></div>
        <p class="hint">
          Organizing after import{pipelineState ? `: ${pipelineState.stage} (${pipelineState.processed}${pipelineState.total > 0 ? ` of ${pipelineState.total}` : ""})` : "…"}
        </p>
      </div>
    {/if}

    {#if stats}
      <div class="stats">
        <button class="chip" class:on={filter === "all"} onclick={() => { filter = "all"; void refresh(); }}>
          All
        </button>
        <button class="chip" class:on={filter === "image"} onclick={() => { filter = "image"; void refresh(); }}>
          {stats.photos} photos
        </button>
        <button class="chip" class:on={filter === "video"} onclick={() => { filter = "video"; void refresh(); }}>
          {stats.videos} videos
        </button>
        <button class="chip" class:on={favFilter} onclick={() => { favFilter = !favFilter; void refresh(); }} aria-pressed={favFilter}>
          ★ Favorites
        </button>
        <button class="chip" class:on={petFilter} onclick={() => { petFilter = !petFilter; void refresh(); }} aria-pressed={petFilter}>
          Pets
        </button>
        <span class="total">{fmtBytes(stats.bytes)} indexed</span>
      </div>

      <div class="filters">
        <select
          aria-label="Filter by month"
          onchange={(e) => { monthFilter = e.currentTarget.value || null; void refresh(); }}
        >
          <option value="">All dates</option>
          {#each months as m (m.month)}
            <option value={m.month} selected={monthFilter === m.month}>
              {m.month} ({m.count})
            </option>
          {/each}
        </select>
        <button
          class="chip"
          class:on={locFilter === "all"}
          onclick={() => { locFilter = "all"; void refresh(); }}
        >
          Anywhere
        </button>
        <button
          class="chip"
          class:on={locFilter === "with"}
          onclick={() => { locFilter = "with"; void refresh(); }}
        >
          With location
        </button>
        <button
          class="chip"
          class:on={locFilter === "without"}
          onclick={() => { locFilter = "without"; void refresh(); }}
        >
          No location
        </button>
        {#if personOptions.length > 0}
          <select
            aria-label="Filter by person"
            onchange={(e) => {
              personFilter = e.currentTarget.value === "" ? null : Number(e.currentTarget.value);
              void refresh();
            }}
          >
            <option value="">Everyone</option>
            {#each personOptions as c (c.id)}
              <option value={c.id} selected={personFilter === c.id}>
                {c.name ?? `Person ${c.id}`} ({c.faceCount})
              </option>
            {/each}
          </select>
        {/if}
        {#if tagOptions.length > 0}
          <select
            aria-label="Filter by scene"
            onchange={(e) => {
              sceneFilter = e.currentTarget.value || null;
              void refresh();
            }}
          >
            <option value="">Every scene</option>
            {#each tagOptions as t (t.tag)}
              <option value={t.tag} selected={sceneFilter === t.tag}>
                {t.tag} ({t.count})
              </option>
            {/each}
          </select>
        {/if}
        <select
          aria-label="Group by"
          onchange={(e) => {
            groupBy = e.currentTarget.value as GroupMode;
            saveGroupBy(groupBy);
          }}
        >
          <option value="none" selected={groupBy === "none"}>No grouping</option>
          <option value="date" selected={groupBy === "date"}>Date</option>
          <option value="location" selected={groupBy === "location"}>Location</option>
          <option value="person" selected={groupBy === "person"}>Person</option>
          <option value="media" selected={groupBy === "media"}>Media type</option>
        </select>
      </div>

      {#if stats.missing > 0}
        <p class="error">
          {stats.missing} file{stats.missing === 1 ? "" : "s"} unreachable (ejected volume or deleted) —
          <button class="tiny" onclick={() => void runVerify()}>Check files</button>
        </p>
      {:else}
        <p class="hint">
          <button class="tiny" onclick={() => void runVerify()}>Check files</button>
          <span> verify that indexed files still exist on disk</span>
        </p>
      {/if}

      {#if (stats.photos + stats.videos) > 0}
        <div class="enrich">
          <button class="ghost" onclick={() => void runEnrich()} disabled={enriching}>
            {enriching ? "Reading metadata…" : "Refresh metadata"}
          </button>
          <button class="ghost" onclick={() => void runTagScenes()} disabled={tagging || enriching}>
            {tagging ? "Tagging scenes…" : "Tag scenes"}
          </button>
          {#if enrichState && enrichState.processed > 0}
            <span class="hint">
              {enrichState.enriched} enriched · {enrichState.failed} unreadable
              {#if enrichState.pending > 0}· {enrichState.pending} pending{/if}
            </span>
          {/if}
          {#if tagState && tagging}
            <span class="hint">
              {tagState.processed} of {tagState.total} photos · {tagState.tagged} tagged
            </span>
          {/if}
        </div>
      {/if}
    {/if}

    {#if selectMode}
      <div class="enrich batch-bar">
        <span class="hint">{selected.size} selected</span>
        <button class="ghost" disabled={selected.size === 0} onclick={() => void batchFavorite()}>★ Star</button>
        <button
          class="ghost"
          disabled={selected.size === 0}
          title="Play the selected photos fullscreen"
          onclick={() => startShow(assets.filter((a) => selected.has(a.id)))}
        >▶ Play</button>
        {#if albumOptions.length > 0}
          <select
            aria-label="Add selected to album"
            onchange={(e) => { batchAlbum = e.currentTarget.value === "" ? null : Number(e.currentTarget.value); }}
          >
            {#each albumOptions as a (a.id)}
              <option value={a.id} selected={batchAlbum === a.id}>{a.name}</option>
            {/each}
          </select>
          <button class="ghost" disabled={selected.size === 0 || batchAlbum === null} onclick={() => void batchAddToAlbum()}>
            Add to album
          </button>
        {/if}
        <button class="ghost" onclick={() => { batchNewOpen = !batchNewOpen; }}>
          ＋ New album
        </button>
        {#if batchNewOpen}
          <form
            class="path-entry inline-new"
            onsubmit={(e) => {
              e.preventDefault();
              void inlineCreateAlbum(batchNewName, "batch");
            }}
          >
            <input
              type="text"
              placeholder="New album name"
              aria-label="New album name"
              bind:value={batchNewName}
              maxlength={80}
            />
            <button type="submit" disabled={batchNewName.trim() === "" || inlineBusy}>
              {inlineBusy ? "Creating…" : "Create"}
            </button>
          </form>
        {/if}
        <button class="tiny danger" disabled={selected.size === 0} onclick={batchTrash}>Trash</button>
        <button class="ghost" disabled={selected.size === 0} onclick={() => (selected = new Set())}>Clear</button>
      </div>
    {/if}

    {#if assets.length}
      <div class="grid">
        {#each displayAssets as { asset, index }, di (asset.id)}
          {#if groupBy !== "none" && (di === 0 || groupKeyOf(displayAssets[di - 1].asset) !== groupKeyOf(asset))}
            {@const gkey = groupKeyOf(asset)}
            {@const gids = groupIds(gkey)}
            <h4 class="group-head">
              <input
                type="checkbox"
                class="selbox-inline"
                aria-label={`Select ${groupLabelOf(asset)}`}
                checked={gids.length > 0 && gids.every((id) => selected.has(id))}
                onchange={() => toggleGroup(gkey)}
              />
              {groupLabelOf(asset)} ({groupCounts.get(gkey) ?? 0})
            </h4>
          {/if}
          <figure class:selected={selected.has(asset.id)}>
            {#if selectMode}
              <input
                type="checkbox"
                class="selbox"
                checked={selected.has(asset.id)}
                aria-label={`Select ${asset.name}`}
                onchange={() => toggleSelect(asset.id)}
              />
            {/if}
            <button
              type="button"
              class="thumb clickable"
              use:observeThumb={asset.path}
              title={selectMode ? `Select ${asset.name}` : `Open ${asset.name}`}
              onclick={() => { if (selectMode) toggleSelect(asset.id); else openViewer(index); }}
            >
              {#if thumbs[asset.path]}
                <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
              {:else}
                <span class="placeholder">{asset.ext.toUpperCase()}</span>
              {/if}
            </button>
            <figcaption>
              <span class="caption-row">
                <button
                  type="button"
                  class="star"
                  class:on={asset.favorite !== 0}
                  aria-label={asset.favorite !== 0 ? `Unstar ${asset.name}` : `Star ${asset.name}`}
                  title={asset.favorite !== 0 ? "Unstar" : "Star"}
                  aria-pressed={asset.favorite !== 0}
                  onclick={() => void toggleFavorite(asset)}
                >★</button>
                <span class="name" title={asset.name}>{asset.name}</span>
              </span>
              <span class="meta">
                {asset.capturedAt ?? asset.modified ? fmtDate(asset.capturedAt ?? asset.modified) : "—"}
                {#if asset.width && asset.height} · {asset.width}×{asset.height}{/if}
                · {fmtBytes(asset.sizeBytes)}
              </span>
              <span class="row-actions">
                {#if asset.kind === "image"}
                <button
                  type="button"
                  class="tiny"
                  aria-label={`Similar to ${asset.name}`}
                  title="More like this photo"
                  onclick={() => void openSimilar(asset)}
                >Similar</button>
                {/if}
              </span>
            </figcaption>
          </figure>
        {/each}
      </div>
    {:else if !importing}
      <p class="hint">No assets yet — add a folder to build the index.</p>
    {/if}

    {#if stats && assets.length > 0}
      <div class="enrich">
        <span class="hint">Showing {assets.length} of {libraryTotal()}</span>
        {#if assets.length < libraryTotal()}
          <button class="ghost" onclick={() => void loadMore()}>Load more</button>
        {/if}
      </div>
    {/if}
  </section>

  {/if}

  {#if tab === "people"}
    <section class="card" aria-labelledby="people-heading">
      <div class="card-head">
        <h2 id="people-heading">People</h2>
        <button onclick={findPeople} disabled={peopleLoading}>
          {peopleLoading ? "Scanning faces…" : "Find people in library"}
        </button>
        <button class="ghost" onclick={() => void findPets()} disabled={petsLoading}>
          {petsLoading ? "Sniffing…" : "Find pets"}
        </button>
      </div>
      {#if petState && petsLoading}
        <p class="hint">{petState.processed} of {petState.total} images · {petState.sightings} pets</p>
      {/if}
      {#if clusterState}
        <p class="hint">{clusterState.processed}/{clusterState.total} images scanned…</p>
      {/if}
      {#if stats && stats.missing > 0}
        <p class="error">
          {stats.missing} indexed file{stats.missing === 1 ? " is" : "s are"} unreadable —
          the volume may be ejected. Reconnect it, then press “Check files” in the Library tab.
        </p>
      {/if}
      {#if people.length}
        <p class="hint">Tick two clusters to merge them · click a name to tag it · ✕ removes a face that doesn't belong.</p>
        {#if mergeSelect.size >= 2}
          <div class="enrich batch-bar">
            <span class="hint">{mergeSelect.size} selected</span>
            <button onclick={() => void runMerge()}>
              Merge into #{[...mergeSelect].sort((a, b) => a - b)[0]}
            </button>
            <button class="ghost" onclick={() => (mergeSelect = new Set())}>Clear</button>
          </div>
        {/if}
        {#each people as cluster (cluster.id)}
          <article class="group">
            <header>
              <span class="cluster-head">
                <input
                  type="checkbox"
                  class="selbox-inline"
                  checked={mergeSelect.has(cluster.id)}
                  aria-label={`Select cluster ${cluster.id} for merge`}
                  onchange={() => toggleMerge(cluster.id)}
                />
                <button
                  class="ename"
                  title="Click to name this person"
                  onclick={() => void doRenameCluster(cluster)}
                >{cluster.name ?? `Cluster ${cluster.id}`}</button>
              </span>
              <span class="waste">{cluster.faceCount} face{cluster.faceCount === 1 ? "" : "s"}</span>
            </header>
            <div class="row-actions">
              <button class="tiny" onclick={() => void toggleClusterPhotos(cluster)}>
                {openClusterId === cluster.id ? "Hide photos" : "Show photos"}
              </button>
              <button
                class="tiny"
                title="Step through every photo full-size to accept or reject"
                onclick={() => void openReview(cluster)}
              >
                Review all
              </button>
              <button class="tiny" onclick={() => void doRenameCluster(cluster)}>
                {cluster.name ? "Rename" : "Tag name"}
              </button>
              <button class="tiny" onclick={() => viewPersonInLibrary(cluster.id)}>
                View in Library
              </button>
            </div>
            {#if openClusterId === cluster.id}
              <div class="members">
                {#each clusterSourcePaths(cluster.id) as path (path)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[path]}
                        <img src={thumbs[path]} alt={path.split("/").pop()} loading="lazy" />
                      {:else}
                        <span class="placeholder">IMG</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={path}>{path.split("/").pop()}</span>
                    </figcaption>
                  </figure>
                {/each}
              </div>
            {/if}
            {#if peopleFaces[cluster.id]?.length}
              <div class="members">
                {#each peopleFaces[cluster.id] as face (face.id)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[`face:${face.id}`]}
                        <img src={thumbs[`face:${face.id}`]} alt="detected face" loading="lazy" />
                      {:else}
                        <span
                          class="placeholder"
                          title={deadFaces.has(face.id) ? "Source file unreadable" : "Loading face…"}
                        >{deadFaces.has(face.id) ? "unreadable" : "…"}</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={face.path}>{face.path.split("/").pop()}</span>
                      <button
                        class="tiny ghost"
                        aria-label={`Remove this face from ${cluster.name ?? `Cluster ${cluster.id}`}`}
                        title="Not this person — remove from cluster"
                        onclick={() => void removeFaceFromCluster(face.id, cluster.id)}
                      >✕</button>
                    </figcaption>
                  </figure>
                {/each}
              </div>
            {/if}
          </article>
        {/each}
      {:else if !peopleLoading}
        <p class="hint">No people clusters yet — scan the library to detect faces.</p>
      {/if}
      {#if pets.length}
        <h3 class="sub">Pets</h3>
        <div class="members">
          {#each pets as s (s.id)}
            <figure>
              <div class="thumb small">
                {#if thumbs[`pet:${s.id}`]}
                  <img src={thumbs[`pet:${s.id}`]} alt={`${s.label} in ${s.path}`} loading="lazy" />
                {:else}
                  <span class="placeholder">…</span>
                {/if}
              </div>
              <figcaption>
                <span class="name">{s.label} · {Math.round(s.confidence * 100)}%</span>
                <span class="meta" title={s.path}>{s.path.split("/").pop()}</span>
              </figcaption>
            </figure>
          {/each}
        </div>
      {:else if !petsLoading}
        <p class="hint">No pets spotted yet — dogs and cats only; birds stay tag-level.</p>
      {/if}
    </section>
  {/if}

  {#if tab === "events"}
    <section class="card" aria-labelledby="events-heading">
      <div class="card-head">
        <h2 id="events-heading">Events</h2>
        <button onclick={runClusterEvents} disabled={eventsLoading}>
          {eventsLoading ? "Clustering…" : "Cluster events by time & place"}
        </button>
        <label class="gap-label" title="Photos closer than this stay in one event — raise it to group multi-day trips">
          Max gap
          <input
            type="number"
            class="gap-input"
            min="1"
            max="168"
            step="1"
            bind:value={gapHours}
            disabled={eventsLoading}
            aria-label="Maximum hours between photos in one event"
          />
          h
        </label>
        <button onclick={runResolvePlaces} disabled={resolvingPlaces || eventsLoading}>
          {resolvingPlaces ? "Resolving…" : "Resolve place names"}
        </button>
      </div>
      {#if geocodeState && resolvingPlaces}
        <p class="hint">
          {geocodeState.processed} of {geocodeState.total} places ·
          {geocodeState.cached} cached{geocodeState.failed > 0 ? ` · ${geocodeState.failed} failed` : ""}
        </p>
      {/if}
      <p class="hint">Place names come from OpenStreetMap and are cached on this device.</p>
      {#if events.length}
        <div class="timeline">
          {#each groupEventsByMonth(events) as g (g.month)}
            <h3 class="sub timeline-month">{monthLabel(g.month)}</h3>
            {#each g.events as event (event.id)}
          <article class="group event-card">
            <header>
              <button class="ename" title="Click to show photos" onclick={() => toggleEvent(event)}>{event.name}</button>
              <span class="waste">
                {new Date(event.startedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                – {new Date(event.endedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
              </span>
            </header>
            <p class="meta">
              {event.assetCount} asset{event.assetCount === 1 ? "" : "s"} ·
              {event.faceCount} face{event.faceCount === 1 ? "" : "s"} ·
              {event.place ?? (event.latitude !== null && event.longitude !== null
                ? `${event.latitude.toFixed(4)}, ${event.longitude.toFixed(4)}`
                : "no location")}
            </p>
            <div class="row-actions">
              <button class="tiny" onclick={() => toggleEvent(event)}>
                {openEventId === event.id ? "Hide photos" : "Show photos"}
              </button>
              <button class="tiny" onclick={() => void doRename(event)}>Rename</button>
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void runExport({ eventId: event.id }, event.name, "drive")}>
                {exportBusy === `e${event.id}-drive` ? "Copying…" : "For Google Drive"}
              </button>
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void runExport({ eventId: event.id }, event.name, "whatsapp")}>
                {exportBusy === `e${event.id}-whatsapp` ? "Compressing…" : "For WhatsApp"}
              </button>
              <button class="tiny" disabled={event.assetCount === 0 || exportBusy !== null} onclick={() => void makeHighlight({ eventId: event.id }, event.name)}>
                {highlightBusy === `e${event.id}` ? "Exporting…" : "Highlight video"}
              </button>
              {#if albumOptions.length > 0}
                <select
                  aria-label={`Add ${event.name} to album`}
                  onchange={(e) => {
                    eventAlbumChoice = {
                      ...eventAlbumChoice,
                      [event.id]: e.currentTarget.value === "" ? null : Number(e.currentTarget.value)
                    };
                  }}
                >
                  {#each albumOptions as a (a.id)}
                    <option
                      value={a.id}
                      selected={(eventAlbumChoice[event.id] ?? albumOptions[0]?.id) === a.id}
                    >{a.name}</option>
                  {/each}
                </select>
                <button
                  class="tiny"
                  disabled={(eventMembers[event.id] ?? []).length === 0 || (eventAlbumChoice[event.id] ?? albumOptions[0]?.id) == null || eventAlbumBusy !== null}
                  onclick={() => void addEventToAlbum(event.id)}
                >
                  {eventAlbumBusy === event.id ? "Adding…" : "＋ Album"}
                </button>
              {/if}
            </div>
            {#if highlightResult && highlightKey === `e${event.id}`}
              <p class="hint">
                Highlight: {highlightResult.seconds.toFixed(1)}s · {fmtBytes(highlightResult.bytes)}
                · {highlightResult.photos} photo{highlightResult.photos === 1 ? "" : "s"}{#if highlightResult.videosIncluded > 0} · {highlightResult.videosIncluded} video clip{highlightResult.videosIncluded === 1 ? "" : "s"}{/if}
                {#if highlightResult.videosSkipped > 0}· {highlightResult.videosSkipped} unreadable skipped{/if}
                <br />{highlightResult.output}
              </p>
            {/if}
            {#if exportResult && exportKey === `e${event.id}`}
              <p class="hint">
                Exported {exportResult.exported} to {exportResult.dir}
                {#if exportResult.skipped > 0}· {exportResult.skipped} skipped{/if}
              </p>
            {/if}
            {#if openEventId === event.id && eventMembers[event.id]?.length}
              <div class="members">
                {#each eventMembers[event.id] as member (member.id)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[member.path]}
                        <img src={thumbs[member.path]} alt={member.name} loading="lazy" />
                      {:else}
                        <span class="placeholder">{member.kind === "video" ? "VID" : "IMG"}</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={member.path}>{member.name}</span>
                    </figcaption>
                  </figure>
                {/each}
              </div>
            {:else if openEventId === event.id}
              <p class="hint">No readable photos in this event.</p>
            {/if}
          </article>
            {/each}
          {/each}
        </div>
      {:else if !eventsLoading}
        <p class="hint">No events yet — cluster the library by time and place.</p>
      {/if}
    </section>
  {/if}

  {#if tab === "albums"}
    <section class="card" aria-labelledby="albums-heading">
      <div class="card-head">
        <h2 id="albums-heading">Albums</h2>
        <button onclick={newAlbum}>New album</button>
      </div>

      {#if albums.length}
        {#each albums as album (album.id)}
          <article class="group album-card">
            <header>
              <button class="ename" title="Click to rename" onclick={() => void renameAlbum(album)}>
                {#if album.coverPath && thumbs[album.coverPath]}
                  <img src={thumbs[album.coverPath]} alt="" loading="lazy" />
                {/if}
                {album.name}
              </button>
              <span class="waste">{album.assetCount} photo{album.assetCount === 1 ? "" : "s"}</span>
            </header>
            <div class="row-actions">
              <button class="tiny" onclick={() => toggleAlbum(album)}>
                {openAlbumId === album.id ? "Hide photos" : "Show photos"}
              </button>
              <button class="tiny" onclick={() => openPicker(album)}>Add photos</button>
              <button class="tiny" disabled={album.assetCount === 0 || exportBusy !== null} onclick={() => void runExport({ albumId: album.id }, album.name, "drive")}>
                {exportBusy === `a${album.id}-drive` ? "Copying…" : "For Google Drive"}
              </button>
              <button class="tiny" disabled={album.assetCount === 0 || exportBusy !== null} onclick={() => void runExport({ albumId: album.id }, album.name, "whatsapp")}>
                {exportBusy === `a${album.id}-whatsapp` ? "Compressing…" : "For WhatsApp"}
              </button>
              <button class="tiny" disabled={album.assetCount === 0 || highlightBusy !== null} onclick={() => void makeHighlight({ albumId: album.id }, album.name)}>
                {highlightBusy === `a${album.id}` ? "Exporting…" : "Highlight video"}
              </button>
              <button class="tiny danger" onclick={() => void deleteAlbum(album)}>Delete album</button>
            </div>
            {#if highlightResult && highlightKey === `a${album.id}`}
              <p class="hint">
                Highlight: {highlightResult.seconds.toFixed(1)}s · {fmtBytes(highlightResult.bytes)}
                · {highlightResult.photos} photo{highlightResult.photos === 1 ? "" : "s"}{#if highlightResult.videosIncluded > 0} · {highlightResult.videosIncluded} video clip{highlightResult.videosIncluded === 1 ? "" : "s"}{/if}
                {#if highlightResult.videosSkipped > 0}· {highlightResult.videosSkipped} unreadable skipped{/if}
                <br />{highlightResult.output}
              </p>
            {/if}
            {#if exportResult && exportKey?.startsWith(`a${album.id}-`)}
              <p class="hint">
                Exported {exportResult.exported} to {exportResult.dir}
                {#if exportResult.skipped > 0}· {exportResult.skipped} skipped{/if}
              </p>
            {/if}
            {#if openAlbumId === album.id && albumMembers[album.id]?.length}
              <div class="members">
                {#each albumMembers[album.id] as member (member.id)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[member.path]}
                        <img src={thumbs[member.path]} alt={member.name} loading="lazy" />
                      {:else}
                        <span class="placeholder">{member.kind === "video" ? "VID" : "IMG"}</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={member.path}>{member.name}</span>
                      <button class="tiny danger" onclick={() => void removeFromAlbum(album.id, member.id)}>
                        Remove
                      </button>
                    </figcaption>
                  </figure>
                {/each}
              </div>
            {:else if openAlbumId === album.id}
              <p class="hint">No photos in this album yet — use “Add photos”.</p>
            {/if}
          </article>
        {/each}
      {:else if !albumsLoading}
        <p class="hint">No albums yet — create one and hand-pick the photos for an audience.</p>
      {/if}
    </section>
  {/if}

  {#if tab === "map"}
  <section class="card" aria-labelledby="map-heading">
    <div class="card-head">
      <h2 id="map-heading">Map</h2>
      <span class="hint">{mapPinsRows.length} geotagged photo{mapPinsRows.length === 1 ? "" : "s"}</span>
    </div>
    {#if mapLoading}
      <p class="hint">Loading pins…</p>
    {:else if mapError}
      <p class="error">{mapError}</p>
    {:else if mapPinsRows.length === 0}
      <p class="hint">No geotagged photos yet — photos with GPS land here after Refresh metadata.</p>
    {/if}
    <div class="map-wrap" bind:this={mapEl} role="application" aria-label="Photo map"></div>
    <p class="hint">Tiles © OpenStreetMap contributors · cached on this device · select a pin, then “Show in Library”.</p>
  </section>
  {/if}

  {#if tab === "settings"}
  <section class="card" aria-labelledby="engine-heading">
    <div class="card-head">
      <h2 id="engine-heading">Engine</h2>
      <button onclick={checkEngine} disabled={status.state === "checking"}>
        {status.state === "checking" ? "Checking…" : "Check engine"}
      </button>
    </div>

    <div class="status">
      <span class="sink" class:up={status.state === "up"} class:down={status.state === "down"}></span>
      <span class="state">
        {#if status.state === "idle"}Not verified
        {:else if status.state === "checking"}Connecting to engine…
        {:else if status.state === "up"}Connected · round-trip {status.ms} ms
        {:else}Engine error{/if}
      </span>
    </div>

    {#if status.state === "up"}
      <dl>
        {#each status.rows as row (row.key)}
          <div class="row">
            <dt>{row.key}</dt>
            <dd>{row.val}</dd>
          </div>
        {/each}
      </dl>
    {:else if status.state === "down"}
      <p class="error">{status.error}</p>
    {/if}

    <p class="hint">Storage: library index and thumbnails stay on this device. Only reverse-geocoding lookups leave the machine.</p>
  </section>
  {/if}

  {#if confirm}
    <div class="scrim">
      <div class="dialog" role="alertdialog" aria-labelledby="confirm-heading">
        <h3 id="confirm-heading">Remove {confirm.label}?</h3>
        <p class="hint">
          {#if deleteMode === "trash"}
            This moves the file to the macOS Trash. You can put it back from Finder until the
            Trash is emptied.
          {:else}
            Flag only — nothing on disk changes.
          {/if}
        </p>
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (confirm = null)}>Cancel</button>
          <button class="danger-solid" onclick={() => void applyDeletion()}>
            {deleteMode === "trash" ? "Move to Trash" : "Flag"}
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if picker}
    <div class="scrim" role="dialog" aria-labelledby="picker-heading">
      <div class="dialog wide">
        <h3 id="picker-heading">Add photos to “{picker.name}”</h3>
        <div class="path-entry">
          <input
            type="search"
            placeholder="Search by filename…"
            aria-label="Search photos by filename"
            bind:value={pickerSearch}
          />
        </div>
        <p class="hint">{pickerFiltered.length} of {pickerAssets.length}</p>
        <div class="enrich">
          <button class="ghost" onclick={() => { pickerNewOpen = !pickerNewOpen; }}>
            ＋ New album
          </button>
          {#if pickerNewOpen}
            <form
              class="path-entry inline-new"
              onsubmit={(e) => {
                e.preventDefault();
                void inlineCreateAlbum(pickerNewName, "picker");
              }}
            >
              <input
                type="text"
                placeholder="New album name"
                aria-label="New album name"
                bind:value={pickerNewName}
                maxlength={80}
              />
              <button type="submit" disabled={pickerNewName.trim() === "" || inlineBusy}>
                {inlineBusy ? "Creating…" : "Create"}
              </button>
            </form>
          {/if}
        </div>
        <div class="members picker-grid">
          {#each pickerFiltered as asset (asset.id)}
            <figure class:keeper={pickerSelected.has(asset.id)}>
              <button
                type="button"
                class="thumb small clickable"
                use:observeThumb={asset.path}
                aria-pressed={pickerSelected.has(asset.id)}
                title={asset.name}
                onclick={() => togglePicker(asset.id)}
              >
                {#if thumbs[asset.path]}
                  <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
                {:else}
                  <span class="placeholder">{asset.ext.toUpperCase()}</span>
                {/if}
              </button>
              <figcaption><span class="name" title={asset.name}>{asset.name}</span></figcaption>
            </figure>
          {/each}
        </div>
        {#if pickerFiltered.length === 0}
          <p class="hint">No photos match “{pickerSearch.trim()}”.</p>
        {/if}
        <div class="dialog-actions">
          <button class="ghost" onclick={() => { picker = null; resetThumbs(); }}>Cancel</button>
          <button disabled={pickerSelected.size === 0} onclick={() => void addPicked()}>
            Add {pickerSelected.size} photo{pickerSelected.size === 1 ? "" : "s"}
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if viewer}
    {@const rows = viewerRows()}
    {@const current = rows[viewer.index]}
    {#if current}
      <div
        class="scrim"
        role="dialog"
        aria-modal="true"
        aria-label={current.name}
        tabindex={-1}
      >
        <div class="viewer">
          <div class="viewer-bar">
            <span class="hint">{viewer.index + 1} of {rows.length}{reviewCluster ? ` · reviewing ${reviewCluster.name}` : ""}</span>
            <span class="viewer-name" title={current.name}>{current.name}</span>
            <button
              type="button"
              class="star"
              class:on={current.favorite !== 0}
              title={current.id < 0 ? "Reopen from the Library to star" : current.favorite !== 0 ? "Unstar" : "Star"}
              aria-pressed={current.favorite !== 0}
              disabled={current.id < 0}
              onclick={() => void toggleFavorite(current)}
            >★</button>
            {#if !reviewCluster}
              {#if current.kind === "image"}
              <button
                type="button"
                class="tiny"
                aria-label={`Similar to ${current.name}`}
                title="More like this photo"
                onclick={() => void openSimilar(current)}
              >Similar</button>
              {/if}
            {:else}
              <button
                type="button"
                class="tiny"
                title={`Remove this photo from ${reviewCluster.name}`}
                onclick={() => void removePhotoFromReview()}
              >Not {reviewCluster.name} — remove</button>
            {/if}
            <button class="tiny" onclick={closeViewer} aria-label="Close viewer">✕</button>
            <button
              class="tiny"
              title="Play slideshow from here"
              aria-label="Play slideshow"
              onclick={() => startShow(viewerRows(), viewer?.index ?? 0)}
            >▶ Play</button>
          </div>
          <div class="viewer-stage">
            <button class="viewer-nav" onclick={() => stepViewer(-1)} aria-label="Previous">‹</button>
            {#if current.kind === "video" && viewerSrc}
              {#key viewerSrc}
                <!-- svelte-ignore a11y_media_has_caption: library files ship no caption tracks -->
                <video src={viewerSrc} controls autoplay playsinline></video>
              {/key}
            {:else if current.kind === "video"}
              {#if thumbs[current.path]}
                <img src={thumbs[current.path]} alt={current.name} />
              {/if}
              <p class="hint">Video playback needs the desktop app — the web preview has no file to play.</p>
            {:else if viewerSrc}
              <img src={viewerSrc} alt={current.name} />
            {:else if viewerError}
              <p class="error">{viewerError}</p>
            {:else}
              <p class="hint">Loading full size…</p>
            {/if}
            <button class="viewer-nav" onclick={() => stepViewer(1)} aria-label="Next">›</button>
          </div>
          <p class="hint viewer-meta">
            {current.capturedAt ?? current.modified ? fmtDate(current.capturedAt ?? current.modified) : "—"}
            {#if current.width && current.height} · {current.width}×{current.height}{/if}
            · {fmtBytes(current.sizeBytes)}
            {#if viewerTags.length > 0} · {viewerTags.map((t) => t.tag).join(", ")}{/if}
          </p>
        </div>
      </div>
    {/if}
  {/if}

  {#if show && showCurrent()}
    {@const sc = showCurrent()!}
    <div class="theater" role="dialog" aria-modal="true" aria-label="Slideshow" tabindex={-1}>
      <div class="theater-top">
        <span class="hint">{show.index + 1} of {show.items.length} · {sc.name}</span>
        <div class="theater-controls">
          <button class="tiny" onclick={() => stepShow(-1)} aria-label="Previous">‹</button>
          <button class="tiny" onclick={toggleShowPlay} aria-label={show.playing ? "Pause" : "Play"}>
            {show.playing ? "❚❚" : "▶"}
          </button>
          <button class="tiny" onclick={() => stepShow(1)} aria-label="Next">›</button>
          <button class="tiny" onclick={closeShow} aria-label="Close slideshow">✕</button>
        </div>
      </div>
      <div class="theater-stage">
        {#if sc.kind === "video" && show.src}
          {#key show.src}
            <!-- svelte-ignore a11y_media_has_caption: library files ship no caption tracks -->
            <video src={show.src} controls autoplay playsinline></video>
          {/key}
        {:else if sc.kind === "video"}
          {#if thumbs[sc.path]}
            <img src={thumbs[sc.path]} alt={sc.name} />
          {/if}
          <p class="hint">Video playback needs the desktop app — the web preview has no file to play.</p>
        {:else if show.src}
          {#key show.src}
            <img src={show.src} alt={sc.name} />
          {/key}
        {:else if show.error}
          <p class="error">{show.error}</p>
        {:else}
          <p class="hint">Loading full size…</p>
        {/if}
      </div>
      <p class="hint theater-hint">‹ › move · space plays/pauses · esc closes</p>
    </div>
  {/if}

  {#if similar}
    <div
      class="scrim"
      role="dialog"
      aria-modal="true"
      aria-label={`Similar to ${similar.name}`}
      tabindex={-1}
    >
      <div class="dialog wide">
        <h3>Similar to {similar.name}</h3>
        {#if similarLoading}
          <p class="hint">
            {#if embedState}
              First run: embedding library for similarity… {embedState.processed} processed
            {:else}
              Finding similar photos…
            {/if}
          </p>
        {:else if similar.rows.length}
          <div class="members">
            {#each similar.rows as m (m.id)}
              <figure>
                <div class="thumb small">
                  {#if thumbs[m.path]}
                    <img src={thumbs[m.path]} alt={m.name} loading="lazy" />
                  {:else}
                    <span class="placeholder">IMG</span>
                  {/if}
                </div>
                <figcaption>
                  <span class="name" title={m.path}>{m.name}</span>
                  <span class="meta">{Math.round(m.score * 100)}% match</span>
                </figcaption>
              </figure>
            {/each}
          </div>
        {:else}
          <p class="hint">No similar photos yet — they appear after the library is embedded.</p>
        {/if}
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (similar = null)}>Back to Library</button>
        </div>
      </div>
    </div>
  {/if}

  {#if nameDialog}
    <div class="scrim" role="dialog" aria-modal="true" aria-label={nameDialog.title} tabindex={-1}>
      <form
        class="dialog"
        onsubmit={(e) => {
          e.preventDefault();
          const v = nameValue.trim();
          const d = nameDialog;
          if (v === "" || !d) return;
          nameDialog = null;
          d.run(v);
        }}
      >
        <h3>{nameDialog.title}</h3>
        <!-- svelte-ignore a11y_autofocus: modal dialog takes focus on open -->
        <input
          class="name-input"
          type="text"
          bind:value={nameValue}
          aria-label={nameDialog.title}
          maxlength={80}
          autofocus
        />
        {#if nameDialog.suggestions.length > 0}
          <p class="hint">Known people — select instead of retyping:</p>
          <div class="suggest-row">
            {#each nameDialog.suggestions as s (s)}
              <button type="button" class="tiny ghost" onclick={() => (nameValue = s)}>{s}</button>
            {/each}
          </div>
        {/if}
        <div class="dialog-actions">
          <button type="button" class="ghost" onclick={() => (nameDialog = null)}>Cancel</button>
          <button type="submit" disabled={nameValue.trim() === ""}>{nameDialog.okLabel}</button>
        </div>
      </form>
    </div>
  {/if}

  {#if deleteAlbumTarget}
    <div class="scrim" role="dialog" aria-modal="true" aria-label="Delete album" tabindex={-1}>
      <div class="dialog">
        <h3 id="delete-album-heading">Delete “{deleteAlbumTarget.name}”?</h3>
        <p class="hint">Photos stay in the library.</p>
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (deleteAlbumTarget = null)}>Cancel</button>
          <button class="danger-solid" onclick={() => void confirmDeleteAlbum()}>Delete</button>
        </div>
      </div>
    </div>
  {/if}

  {#if removeRootTarget}
    <div class="scrim" role="dialog" aria-modal="true" aria-label="Remove indexed folder" tabindex={-1}>
      <div class="dialog">
        <h3>Forget “{removeRootTarget.root}”?</h3>
        <p class="hint">
          {removeRootTarget.assets} asset{removeRootTarget.assets === 1 ? "" : "s"} leave the index.
          Files on disk are untouched — rescan to bring them back.
        </p>
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (removeRootTarget = null)}>Cancel</button>
          <button class="danger-solid" onclick={() => void confirmRemoveRoot()}>Remove</button>
        </div>
      </div>
    </div>
  {/if}

  {#if toast}
    <p class="toast" role="status">{toast}</p>
  {/if}
</main>

<style>
  :global(*) {
    box-sizing: border-box;
  }
  :global(body) {
    margin: 0;
    min-height: 100vh;
    background: #0d0f14;
    color: #e7eaf0;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", sans-serif;
  }
  .container {
    max-width: 120rem;
    width: 100%;
    margin: 0 auto;
    padding: 1.5rem clamp(1rem, 4vw, 3rem) 3rem;
  }
  header h1 {
    font-size: 2rem;
    margin: 0;
    letter-spacing: -0.02em;
  }
  .tagline {
    margin: 0.25rem 0 1.5rem;
    color: #8a93a5;
  }
  .card {
    background: #151820;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 1rem 1.25rem;
    margin-bottom: 1rem;
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
  }
  h2 {
    font-size: 1rem;
    margin: 0;
  }
  button {
    background: #2f6bff;
    color: #fff;
    border: 0;
    border-radius: 8px;
    padding: 0.4rem 0.9rem;
    font: inherit;
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .path-entry {
    display: flex;
    gap: 0.4rem;
    margin-bottom: 0.6rem;
  }
  .path-entry input {
    flex: 1;
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.85rem;
    padding: 0.35rem 0.6rem;
  }
  .path-entry input:focus {
    outline: none;
    border-color: #2f6bff;
  }
  .root {
    margin: 0 0 0.6rem;
    color: #8a93a5;
    font-size: 0.8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .roots-head {
    display: flex;
    align-items: baseline;
    gap: 0.8rem;
  }
  .roots-head .sub {
    flex: 1;
  }
  .roots {
    list-style: none;
    margin: 0 0 0.6rem;
    padding: 0;
  }
  .roots li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem 0.8rem;
    align-items: baseline;
    padding: 0.3rem 0;
    border-top: 1px solid #232733;
  }
  .roots li .root {
    flex: 1;
    min-width: 12rem;
    margin: 0;
  }
  .progress {
    margin-bottom: 0.6rem;
  }
  .bar {
    height: 4px;
    background: #232733;
    border-radius: 2px;
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: #2f6bff;
    transition: width 0.2s ease;
  }
  /* Scan size is unknown up front, so the live bar sweeps instead of filling. */
  .bar.live span {
    width: 35%;
    animation: sweep 1.1s ease-in-out infinite;
  }
  @keyframes sweep {
    0% { margin-left: 0; }
    50% { margin-left: 65%; }
    100% { margin-left: 0; }
  }
  .hint {
    margin: 0.4rem 0 0;
    color: #8a93a5;
    font-size: 0.8rem;
  }
  .stats {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.6rem 0;
    flex-wrap: wrap;
  }
  .chip {
    background: #1c212c;
    color: #b6bdc9;
    border: 1px solid #232733;
    border-radius: 999px;
    padding: 0.2rem 0.7rem;
    font-size: 0.8rem;
  }
  .chip.on {
    background: #2f6bff;
    border-color: #2f6bff;
    color: #fff;
  }
  .total {
    margin-left: auto;
    color: #8a93a5;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
  }
  .enrich {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    flex-wrap: wrap;
    margin: 0.2rem 0 0.4rem;
  }
  .enrich .hint {
    margin: 0;
  }
  button.ghost {
    background: transparent;
    color: #b6bdc9;
    border: 1px solid #232733;
    font-size: 0.8rem;
    padding: 0.25rem 0.7rem;
  }
  button.ghost:hover:not(:disabled) {
    border-color: #3a445c;
  }
  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-bottom: 0.8rem;
  }
  .tab {
    background: transparent;
    color: #8a93a5;
    border: 1px solid transparent;
    font-size: 0.85rem;
    padding: 0.3rem 0.8rem;
  }
  .tab.on {
    background: #1c212c;
    border-color: #232733;
    color: #e7eaf0;
  }
  .actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-bottom: 0.5rem;
  }
  .sub {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #8a93a5;
    margin: 1rem 0 0.5rem;
  }
  .chip.static {
    cursor: default;
  }
  .mode {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.8rem;
    color: #8a93a5;
    margin: 0.5rem 0;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    flex-wrap: wrap;
    margin: 0 0 0.6rem;
  }
  .filters select {
    background: #0d0f14;
    color: #e7eaf0;
    border: 1px solid #232733;
    border-radius: 999px;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.2rem 0.5rem;
    max-width: 12rem;
  }
  .mode select {
    background: #0d0f14;
    color: #e7eaf0;
    border: 1px solid #232733;
    border-radius: 6px;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.2rem 0.4rem;
  }
  .group {
    background: #10131a;
    border: 1px solid #232733;
    border-radius: 10px;
    padding: 0.6rem;
    margin-bottom: 0.6rem;
  }
  .group > header {
    display: flex;
    justify-content: space-between;
    font-size: 0.78rem;
    color: #b6bdc9;
    margin-bottom: 0.5rem;
    min-width: 0;
  }
  .waste {
    color: #7b8496;
  }
  .members {
    /* auto-fill (not fit): empty tracks are kept, so a one-photo group
       stays one small tile instead of stretching full-width. */
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(6rem, 100%), 1fr));
    gap: 0.5rem;
    margin-bottom: 0.5rem;
  }
  .members figure {
    min-width: 0;
  }
  .members figure {
    border: 1px solid transparent;
    border-radius: 8px;
    padding: 0.25rem;
  }
  .members figure.keeper {
    border-color: #2f6bff;
    background: rgba(47, 107, 255, 0.08);
  }
  .thumb.small {
    aspect-ratio: 1;
  }
  .event-card .meta {
    margin: 0 0 0.5rem;
  }
  .ename {
    background: transparent;
    border: 0;
    padding: 0;
    color: #e7eaf0;
    font-size: inherit;
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .ename:hover {
    text-decoration: underline;
  }
  .badge.keep {
    display: inline-block;
    font-size: 0.65rem;
    color: #2f6bff;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .row-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    margin-top: 0.2rem;
  }
  button.tiny {
    background: #1c212c;
    color: #b6bdc9;
    border: 1px solid #232733;
    font-size: 0.68rem;
    padding: 0.15rem 0.4rem;
    border-radius: 6px;
  }
  button.tiny.danger {
    color: #e5484d;
  }
  button.danger-solid {
    background: #e5484d;
    color: #fff;
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(5, 7, 10, 0.72);
    display: grid;
    place-items: center;
    padding: 1rem;
  }
  .dialog {
    background: #151820;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 1rem 1.2rem;
    width: min(24rem, 92vw);
  }
  .dialog h3 {
    margin: 0 0 0.5rem;
    font-size: 0.95rem;
  }
  .map-wrap {
    height: 60vh;
    min-height: 24rem;
    border: 1px solid #232733;
    border-radius: 12px;
    overflow: hidden;
    background: #0b0e14;
  }
  .map-wrap :global(.leaflet-container) {
    height: 100%;
    background: #0b0e14;
    font: inherit;
  }
  :global(.map-popup) {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    align-items: flex-start;
  }
  :global(.map-popup img) {
    width: 160px;
    border-radius: 6px;
  }
  :global(.map-popup .name) {
    font-size: 0.75rem;
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .theater {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: #000;
    display: flex;
    flex-direction: column;
    padding: 0.8rem 1rem;
  }
  .theater-top {
    display: flex;
    align-items: center;
    gap: 0.8rem;
  }
  .theater-top .hint {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .theater-controls {
    display: flex;
    gap: 0.4rem;
  }
  .theater-stage {
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    overflow: hidden;
  }
  .theater-stage img,
  .theater-stage video {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .theater-hint {
    text-align: center;
    margin: 0.4rem 0 0;
  }
  .dialog.wide {
    width: min(52rem, 96vw);
    max-height: 82vh;
    overflow: auto;
  }
  .gap-label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: #8a93a5;
  }
  .gap-input {
    width: 3.2rem;
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.3rem 0.4rem;
  }
  .timeline {
    border-left: 2px solid #232733;
    margin: 0.6rem 0 0 0.35rem;
    padding-left: 0.9rem;
  }
  .timeline-month {
    margin-top: 0.75rem;
  }
  .timeline-month:first-child {
    margin-top: 0.25rem;
  }
  .picker-grid {
    max-height: 58vh;
    overflow: auto;
    padding-right: 0.2rem;
  }
  .thumb.clickable {
    cursor: pointer;
  }
  button.thumb {
    display: block;
    width: 100%;
    padding: 0;
    background: #0d0f14;
    border: 0;
    color: inherit;
  }
  .viewer {
    background: #101319;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 0.8rem 1rem;
    width: min(64rem, 96vw);
    max-height: 92vh;
    overflow: auto;
  }
  .viewer-bar {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    margin-bottom: 0.6rem;
  }
  .viewer-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 600;
  }
  .viewer-stage {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .viewer-stage img,
  .viewer-stage video {
    flex: 1;
    min-width: 0;
    max-height: 74vh;
    width: 100%;
    object-fit: contain;
    border-radius: 8px;
    background: #000;
  }
  button.viewer-nav {
    background: transparent;
    border: 1px solid #232733;
    border-radius: 8px;
    font-size: 1.6rem;
    line-height: 1;
    padding: 0.6rem 0.8rem;
    flex: none;
  }
  .viewer-meta {
    margin: 0.5rem 0 0;
  }
  .thumb.clickable:hover {
    outline: 2px solid #2f6bff;
  }
  .ename img {
    width: 1.6rem;
    height: 1.6rem;
    object-fit: cover;
    border-radius: 5px;
    vertical-align: -0.45rem;
    margin-right: 0.4rem;
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.9rem;
  }
  .name-input {
    width: 100%;
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.9rem;
    padding: 0.4rem 0.6rem;
  }
  .suggest-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.5rem;
  }
  .toast {
    position: fixed;
    bottom: 1rem;
    left: 50%;
    transform: translateX(-50%);
    background: #1c212c;
    border: 1px solid #232733;
    border-radius: 999px;
    padding: 0.35rem 0.9rem;
    font-size: 0.8rem;
    color: #b6bdc9;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(8.5rem, 100%), 1fr));
    gap: 0.6rem;
    margin-top: 0.6rem;
  }
  .grid > figure {
    min-width: 0;
  }
  .grid > figure.selected {
    outline: 2px solid #2f6bff;
    border-radius: 8px;
  }
  .group-head {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
    font-weight: 600;
    color: #cfd5e1;
    margin: 0.8rem 0 0.1rem;
  }
  .group-head:first-child {
    margin-top: 0;
  }
  .grid > figure {
    position: relative;
  }
  .selbox {
    position: absolute;
    top: 0.35rem;
    left: 0.35rem;
    z-index: 1;
    width: 1.1rem;
    height: 1.1rem;
    accent-color: #2f6bff;
  }
  .selbox-inline {
    width: 1rem;
    height: 1rem;
    accent-color: #2f6bff;
    flex: none;
  }
  .cluster-head {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .batch-bar {
    align-items: center;
    flex-wrap: wrap;
  }
  .batch-bar select {
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.3rem 0.5rem;
  }
  .row-actions select {
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 6px;
    color: #e7eaf0;
    font: inherit;
    font-size: 0.68rem;
    padding: 0.15rem 0.4rem;
    max-width: 10rem;
  }
  .inline-new {
    flex: 1;
    min-width: 12rem;
    margin: 0;
  }
  figure {
    margin: 0;
  }
  .thumb {
    aspect-ratio: 1;
    background: #0d0f14;
    border-radius: 8px;
    overflow: hidden;
    display: grid;
    place-items: center;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .placeholder {
    color: #4a5162;
    font-size: 0.75rem;
    letter-spacing: 0.05em;
  }
  figcaption {
    margin-top: 0.25rem;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .caption-row {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
  }
  .caption-row .name {
    flex: 1;
    min-width: 0;
  }
  button.star {
    flex: none;
    background: transparent;
    border: 0;
    padding: 0 0.1rem;
    font-size: 0.85rem;
    line-height: 1;
    color: #4a5160;
  }
  button.star.on {
    color: #f5b83d;
  }
  .meta {
    color: #7b8496;
    font-size: 0.68rem;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 0.75rem;
    font-size: 0.9rem;
    color: #b6bdc9;
  }
  .sink {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #555a66;
  }
  .sink.up {
    background: #35c773;
  }
  .sink.down {
    background: #e5484d;
  }
  dl {
    margin: 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    padding: 0.35rem 0;
    border-top: 1px solid #232733;
    font-size: 0.9rem;
    min-width: 0;
  }
  dt {
    color: #8a93a5;
  }
  dd {
    margin: 0;
    min-width: 0;
    font-variant-numeric: tabular-nums;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error {
    margin: 0;
    color: #e5484d;
    font-size: 0.85rem;
  }
  .global-error {
    background: rgba(229, 72, 77, 0.08);
    border: 1px solid rgba(229, 72, 77, 0.35);
    border-radius: 8px;
    padding: 0.5rem 0.8rem;
    margin-bottom: 0.8rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.8rem;
  }
</style>
