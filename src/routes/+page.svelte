<script lang="ts">
  import { onMount } from "svelte";
  import { open, isWebMode, assetUrl } from "$lib/bridge";  import {
    blurredAssets,
    cleanupStats,
    clustersList,
    renameCluster,
    mergeClusters,
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
    enrichAssets,
    findDuplicates,
    keepCopy,
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
    setFavorite,
    tagScenes,
    onTagProgress,
    sceneTags,
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
    type EventRow,
    type EventAssetRow,
    type GeocodeProgress,
    type FaceRow,
    type DuplicateGroup,
    type EnrichProgress,
    type ImportProgress,
    type LibraryStats,
    type ScanRoot,
    type SceneTag,
    type SceneTagCount,
    type TagProgress
  } from "$lib/bridge";
  import { engineCall, engineSysInfo, type SysInfo } from "$lib/bridge";

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
  let filter = $state<"all" | "image" | "video">("all");
  let favFilter = $state(false);
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
      sceneTag: sceneFilter ?? undefined,
      personCluster: personFilter ?? undefined,
      hasLocation: locFilter === "all" ? undefined : locFilter === "with",
      capturedMin,
      capturedMax
    };
  }
  let root = $state<string | null>(null);
  let importing = $state(false);
  let progress = $state<ImportProgress | null>(null);
  let error = $state<string | null>(null);
  let thumbs = $state<Record<string, string>>({});
  let people = $state<FaceClusterRow[]>([]);
  let mergeSelect = $state<Set<number>>(new Set());
  let typedPath = $state("");
  let enriching = $state(false);
  let enrichState = $state<EnrichProgress | null>(null);
  let cleanup = $state<CleanupStats | null>(null);
  let groups = $state<DuplicateGroup[]>([]);
  let blurred = $state<AssetRow[]>([]);
  let hashing = $state(false);
  let scoring = $state(false);
  let hashState = $state<CleanupProgress | null>(null);
  let qualityState = $state<CleanupProgress | null>(null);
  let deleteMode = $state<"trash" | "flag">("trash");
  let confirm = $state<{ ids: number[]; label: string } | null>(null);
  let tab = $state<"library" | "cleanup" | "people" | "events" | "albums" | "settings">("library");
  let flagged = $state<Set<number>>(new Set());
  let peopleLoading = $state(false);
  let events = $state<EventRow[]>([]);
  let eventsLoading = $state(false);
  let resolvingPlaces = $state(false);
  let geocodeState = $state<GeocodeProgress | null>(null);
  // Lightbox viewer over the current library page (index into `assets`).
  let viewer = $state<{ index: number } | null>(null);
  let viewerSrc = $state<string | null>(null);
  let viewerError = $state<string | null>(null);
  let viewerTags = $state<SceneTag[]>([]);
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
  function openViewer(index: number) {
    if (index < 0 || index >= assets.length) return;
    viewer = { index };
    viewerSrc = null;
    viewerError = null;
    viewerTags = [];
    void loadViewer(assets[index]);
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
    if (!viewer || assets.length === 0) return;
    const next = (viewer.index + delta + assets.length) % assets.length;
    viewer = { index: next };
    void loadViewer(assets[next]);
  }

  function closeViewer() {
    viewer = null;
    viewerSrc = null;
    viewerError = null;
  }

  function viewerKey(e: KeyboardEvent) {
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
      void runEnrich();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      importing = false;
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

  async function doRenameCluster(cluster: FaceClusterRow) {
    const name = window.prompt("Person name", cluster.name ?? "");
    if (name === null || name.trim() === "" || name.trim() === (cluster.name ?? "")) return;
    try {
      await renameCluster(cluster.id, name.trim());
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

  // MARK: - Albums

  let albums = $state<AlbumRow[]>([]);
  let albumsLoading = $state(false);
  let openAlbumId = $state<number | null>(null);
  let albumMembers = $state<Record<number, EventAssetRow[]>>({});
  let picker = $state<{ albumId: number; name: string } | null>(null);
  let pickerAssets = $state<AssetRow[]>([]);
  let pickerSelected = $state<Set<number>>(new Set());

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

  function toggleAlbum(album: AlbumRow) {
    openAlbumId = openAlbumId === album.id ? null : album.id;
    if (openAlbumId !== null) void loadAlbumMembers(album.id);
  }

  async function newAlbum() {
    const name = window.prompt("Album name");
    if (name === null || name.trim() === "") return;
    try {
      await albumCreate(name.trim());
      await loadAlbums();
      toast = `Created “${name.trim()}”`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function renameAlbum(album: AlbumRow) {
    const name = window.prompt("Album name", album.name);
    if (name === null || name.trim() === "" || name === album.name) return;
    try {
      await albumRename(album.id, name.trim());
      await loadAlbums();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function deleteAlbum(album: AlbumRow) {
    if (!window.confirm(`Delete album “${album.name}”? Photos stay in the library.`)) return;
    try {
      await albumDelete(album.id);
      await loadAlbums();
      toast = "Album deleted";
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  function openPicker(album: AlbumRow) {
    picker = { albumId: album.id, name: album.name };
    pickerSelected = new Set();
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

  async function loadFaceThumb(f: FaceRow) {
    const key = `face:${f.id}`;
    if (thumbs[key]) return;
    try {
      const { image } = await faceThumb(f.path, f.x, f.y, f.width, f.height, 160);
      if (image) thumbs[key] = thumbSrc(image);
    } catch {
      /* unreadable face crop */
    }
  }

  async function loadEvents() {
    resetThumbs();
    eventsLoading = true;
    try {
      events = await eventsList();
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function runClusterEvents() {
    eventsLoading = true;
    try {
      await clusterEvents(4, 500);
      events = await eventsList();
      toast = `Grouped into ${events.length} event${events.length === 1 ? "" : "s"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
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

  async function doRename(event: EventRow) {
    const name = window.prompt("Event name", event.name);
    if (name === null || name.trim() === "" || name === event.name) return;
    try {
      await renameEvent(event.id, name.trim());
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

  onMount(() => {
    if (!inApp && !web) {
      error = "Open this inside the fotoflow app window — the dev-server tab has no engine bridge.";
      return;
    }
    const offs = [
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
      onClusterProgress((p) => {
        if (peopleLoading && clusterState) {
          clusterState = { processed: p.processed, total: p.total };
        }
      }),
      onGeocodeProgress((p) => {
        if (resolvingPlaces) geocodeState = p;
      }),
      onGeocodeError((m) => {
        error = m;
      }),
      onTagProgress((p) => {
        if (tagging) tagState = p;
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
    <button class="tab" class:on={tab === "people"} onclick={() => { tab = "people"; void loadPeople(); }}>
      People{#if people.length > 0}&nbsp;{people.length}{/if}
    </button>
    <button class="tab" class:on={tab === "events"} onclick={() => { tab = "events"; void loadEvents(); }}>
      Events{#if events.length > 0}&nbsp;{events.length}{/if}
    </button>
    <button class="tab" class:on={tab === "albums"} onclick={() => { tab = "albums"; void loadAlbums(); }}>
      Albums{#if albums.length > 0}&nbsp;{albums.length}{/if}
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
      </div>

      {#if hashState}
        <p class="hint">{hashState.hashed ?? 0} hashed across {hashState.buckets ?? 0} size buckets</p>
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
      <h3 class="sub">Indexed folders</h3>
      <ul class="roots">
        {#each roots as r (r.root)}
          <li>
            <span class="root" title={r.root}>{r.root}</span>
            <span class="hint">
              {r.assets} asset{r.assets === 1 ? "" : "s"}{#if r.missing > 0} · {r.missing} unreachable{/if}
              · scanned {r.scans} time{r.scans === 1 ? "" : "s"} · {fmtDate(r.lastStartedAt)}
            </span>
          </li>
        {/each}
      </ul>
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
        <button class="tiny danger" disabled={selected.size === 0} onclick={batchTrash}>Trash</button>
        <button class="ghost" disabled={selected.size === 0} onclick={() => (selected = new Set())}>Clear</button>
      </div>
    {/if}

    {#if assets.length}
      <div class="grid">
        {#each assets as asset, i (asset.id)}
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
              onclick={() => { if (selectMode) toggleSelect(asset.id); else openViewer(i); }}
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
      </div>
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
            {#if peopleFaces[cluster.id]?.length}
              <div class="members">
                {#each peopleFaces[cluster.id] as face (face.id)}
                  <figure>
                    <div class="thumb small">
                      {#if thumbs[`face:${face.id}`]}
                        <img src={thumbs[`face:${face.id}`]} alt="detected face" loading="lazy" />
                      {:else}
                        <span class="placeholder">…</span>
                      {/if}
                    </div>
                    <figcaption>
                      <span class="name" title={face.path}>{face.path.split("/").pop()}</span>
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
    </section>
  {/if}

  {#if tab === "events"}
    <section class="card" aria-labelledby="events-heading">
      <div class="card-head">
        <h2 id="events-heading">Events</h2>
        <button onclick={runClusterEvents} disabled={eventsLoading}>
          {eventsLoading ? "Clustering…" : "Cluster events by time & place"}
        </button>
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
              <button class="ename" title="Click to rename" onclick={() => void doRename(event)}>{event.name}</button>
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
        <div class="members picker-grid">
          {#each pickerAssets as asset (asset.id)}
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
    {@const current = assets[viewer.index]}
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
            <span class="hint">{viewer.index + 1} of {assets.length}</span>
            <span class="viewer-name" title={current.name}>{current.name}</span>
            <button
              type="button"
              class="star"
              class:on={current.favorite !== 0}
              title={current.favorite !== 0 ? "Unstar" : "Star"}
              aria-pressed={current.favorite !== 0}
              onclick={() => void toggleFavorite(current)}
            >★</button>
            <button class="tiny" onclick={closeViewer} aria-label="Close viewer">✕</button>
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
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(6rem, 100%), 1fr));
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
  .dialog.wide {
    width: min(52rem, 96vw);
    max-height: 82vh;
    overflow: auto;
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
