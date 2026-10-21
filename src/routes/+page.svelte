<script lang="ts">
  import { onMount, tick } from "svelte";
  import { open, isWebMode, isApiMode, assetUrl } from "$lib/bridge";  import {
    blurredAssets,
    cleanupStats,
    clustersList,
    renameCluster,
    mergeClusters,
    restoreCluster,
    unassignFace,
    assignFace,
    detectPets,
    onPetProgress,
    onPetError,
    sightingsList,
    clusterEvents,
    eventsList,
    eventAssets,
    tripsList,
    curatePicksToAlbum,
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
    type SharePackSummary,
    exportSharePack,
    onExportProgress,
    type ExportProgress,
    revealInDir,
    openExternal,
    ingestCopy,
    onIngestProgress,
    type IngestProgress,
    type IngestSummary,
    addEventPicksToAlbum,
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
    restoreAssets,
    trashList,
    clearTrashLog,
    type TrashEntry,
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
    type TripRow,
    type GeocodeProgress,
    type FaceRow,
    type DuplicateGroup,
    type DuplicateMember,
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

  // Curated highlights per section at month/year zoom (Apple Photos-style:
  // the reel shows key photos, the header count stays the true total).
  // Test affordance (?highlights=N) mirrors ?pageSize=N.
  const HIGHLIGHTS = 8;

  function highlightCap(): number {
    try {
      if (typeof window === "undefined") return HIGHLIGHTS;
      const n = Number(new URLSearchParams(window.location.search).get("highlights"));
      if (Number.isInteger(n) && n > 0) return n;
    } catch {
      /* no URL API: fall through to the default */
    }
    return HIGHLIGHTS;
  }

  // Test affordance (?pageSize=N): additive like the __ffMap pattern —
  // production stays at PAGE unless a valid positive integer is given.
  function pageSize(): number {
    try {
      if (typeof window === "undefined") return PAGE;
      const n = Number(new URLSearchParams(window.location.search).get("pageSize"));
      if (Number.isInteger(n) && n > 0) return n;
    } catch {
      /* no URL API: fall through to the default */
    }
    return PAGE;
  }

  let status = $state<EngineStatus>({ state: "idle" });
  let assets = $state<AssetRow[]>([]);
  let stats = $state<LibraryStats | null>(null);
  let roots = $state<ScanRoot[]>([]);
  const RECENT_SEARCH_KEY = "fotoflow-recent-searches";
  const FOLDERS_OPEN_KEY = "fotoflow-folders-open";
  const FLAGGED_KEY = "fotoflow-flagged";

  function loadFlagged(): Set<number> {
    try {
      if (typeof localStorage === "undefined") return new Set();
      const raw = localStorage.getItem(FLAGGED_KEY);
      const arr = raw ? JSON.parse(raw) : [];
      if (!Array.isArray(arr)) return new Set();
      return new Set(arr.filter((id) => typeof id === "number"));
    } catch {
      return new Set();
    }
  }

  function saveFlagged(ids: Set<number>) {
    try {
      localStorage.setItem(FLAGGED_KEY, JSON.stringify([...ids]));
    } catch {
      /* storage unavailable: flags just don't persist */
    }
  }

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

  // Frozen shell: resizable sidebar (208–300px, fully hideable for
  // full-width photos) + thumbnail-size slider. Same load/save +
  // try/catch pattern as the flagged/folders/filter keys above.
  const SIDEBAR_WIDTH_KEY = "fotoflow-sidebar-width";
  const SIDEBAR_HIDDEN_KEY = "fotoflow-sidebar-hidden";
  const DENSITY_KEY = "fotoflow-density";
  const GRID_DETAILS_KEY = "fotoflow-grid-details";
  const SIDEBAR_MIN = 208;
  const SIDEBAR_MAX = 300;
  const SIDEBAR_DEFAULT = 248;

  function loadSidebarWidth(): number {
    try {
      if (typeof localStorage === "undefined") return SIDEBAR_DEFAULT;
      const raw = localStorage.getItem(SIDEBAR_WIDTH_KEY);
      const n = raw === null ? NaN : Number(raw);
      if (!Number.isFinite(n)) return SIDEBAR_DEFAULT;
      return Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(n)));
    } catch {
      return SIDEBAR_DEFAULT;
    }
  }

  function saveSidebarWidth(w: number) {
    try {
      localStorage.setItem(SIDEBAR_WIDTH_KEY, String(w));
    } catch {
      /* storage unavailable: width just doesn't persist */
    }
  }

  function loadSidebarHidden(): boolean {
    try {
      if (typeof localStorage === "undefined") return false;
      // Migrates the old 64px-rail flag: collapsed then means hidden now.
      if (localStorage.getItem(SIDEBAR_HIDDEN_KEY) === null) {
        return localStorage.getItem("fotoflow-sidebar-collapsed") === "true";
      }
      return localStorage.getItem(SIDEBAR_HIDDEN_KEY) === "true";
    } catch {
      return false;
    }
  }

  function saveSidebarHidden(v: boolean) {
    try {
      localStorage.setItem(SIDEBAR_HIDDEN_KEY, String(v));
    } catch {
      /* storage unavailable: preference just doesn't persist */
    }
  }

  // Thumbnail size continuum (Apple Photos-style slider, not steps):
  // grid tile minimum in rem, persisted. Migrates the old two-step flag.
  const GRID_MIN_DEFAULT = 8.5;
  const GRID_MIN_MIN = 4;
  const GRID_MIN_MAX = 12;

  function clampGridMin(n: number): number {
    return Math.min(GRID_MIN_MAX, Math.max(GRID_MIN_MIN, n));
  }

  function loadGridMin(): number {
    try {
      if (typeof localStorage === "undefined") return GRID_MIN_DEFAULT;
      const raw = localStorage.getItem(DENSITY_KEY);
      if (raw === "compact") return 6.5;
      if (raw !== null && raw !== "comfortable") {
        const n = Number(raw);
        if (Number.isFinite(n)) return clampGridMin(n);
      }
      return GRID_MIN_DEFAULT;
    } catch {
      return GRID_MIN_DEFAULT;
    }
  }

  function saveGridMin(v: number) {
    try {
      localStorage.setItem(DENSITY_KEY, String(v));
    } catch {
      /* storage unavailable: preference just doesn't persist */
    }
  }

  let sidebarWidth = $state(loadSidebarWidth());
  let sidebarHidden = $state(loadSidebarHidden());
  // Slider position never touches filters: no refresh(), no filter writes.
  let gridMin = $state(loadGridMin());

  // Wall-to-wall grid vs captioned cards (name, star, meta, Similar).
  // Default on: selection/star controls live in the caption.
  function loadGridDetails(): boolean {
    try {
      if (typeof localStorage === "undefined") return true;
      const raw = localStorage.getItem(GRID_DETAILS_KEY);
      return raw === null ? true : raw !== "false";
    } catch {
      return true;
    }
  }

  function saveGridDetails(v: boolean) {
    try {
      localStorage.setItem(GRID_DETAILS_KEY, String(v));
    } catch {
      /* storage unavailable: preference just doesn't persist */
    }
  }

  let gridDetails = $state(loadGridDetails());

  function toggleSidebar() {
    sidebarHidden = !sidebarHidden;
    saveSidebarHidden(sidebarHidden);
  }

  function onResizeStart(e: PointerEvent) {
    if (sidebarHidden) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = sidebarWidth;
    const move = (ev: PointerEvent) => {
      sidebarWidth = Math.min(SIDEBAR_MAX, Math.max(SIDEBAR_MIN, Math.round(startW + ev.clientX - startX)));
    };
    const up = () => {
      saveSidebarWidth(sidebarWidth);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

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

  type LibZoom = "all" | "day" | "month" | "year";
  const LIB_ZOOM_KEY = "fotoflow-lib-zoom";
  const LIB_ZOOMS: LibZoom[] = ["all", "day", "month", "year"];
  const LIB_ZOOM_LABELS: Record<LibZoom, string> = { all: "All", day: "Day", month: "Month", year: "Year" };

  function loadLibZoom(): LibZoom {
    try {
      if (typeof localStorage === "undefined") return "all";
      const raw = localStorage.getItem(LIB_ZOOM_KEY);
      return (LIB_ZOOMS as string[]).includes(raw as string) ? (raw as LibZoom) : "all";
    } catch {
      return "all";
    }
  }

  function saveLibZoom(z: LibZoom) {
    try {
      localStorage.setItem(LIB_ZOOM_KEY, z);
    } catch {
      /* storage unavailable: zoom just doesn't persist */
    }
  }

  let libZoom = $state<LibZoom>(loadLibZoom());

  // Apple Photos-style timeline zoom: all (flat) → day → month → year.
  // Direct level jump (segmented control), clamped steps (keys/wheel).
  // Manual zooming always exits a drill-down (see zoomDrill).
  function setLibZoom(z: LibZoom) {
    if (libZoom === z) return;
    libZoom = z;
    zoomDrill = null;
    saveLibZoom(z);
  }

  function zoomLib(dir: 1 | -1) {
    const i = LIB_ZOOMS.indexOf(libZoom);
    const next = Math.min(LIB_ZOOMS.length - 1, Math.max(0, i + dir));
    setLibZoom(LIB_ZOOMS[next]);
  }

  // Drill-down from a highlights section: month → that month's days,
  // year → that year's months. Transient (not persisted): Back restores
  // the zoom level it came from.
  let zoomDrill = $state<{ from: "month" | "year"; key: string } | null>(null);

  function drillInto(key: string) {
    const g = dateGranularity();
    if (g === "month") {
      libZoom = "day";
      zoomDrill = { from: "month", key };
    } else if (g === "year") {
      libZoom = "month";
      zoomDrill = { from: "year", key };
    } else {
      return;
    }
    saveLibZoom(libZoom);
  }

  function drillBack() {
    if (!zoomDrill) return;
    libZoom = zoomDrill.from;
    zoomDrill = null;
    saveLibZoom(libZoom);
  }

  function drillLabel(): string {
    if (!zoomDrill) return "";
    return zoomDrill.from === "month" ? monthLabelOf(zoomDrill.key) : yearLabelOf(zoomDrill.key);
  }

  // Assets visible under a drill: month drills match the month key,
  // year drills the year key. Everything else sees all loaded assets.
  let drilledAssets = $derived.by(() => {
    const drill = zoomDrill;
    if (!drill) return assets;
    return drill.from === "month"
      ? assets.filter((a) => monthKeyOf(a) === drill.key)
      : assets.filter((a) => yearKeyOf(a) === drill.key);
  });

  // Ctrl/Cmd+wheel (trackpad pinch) over the grid steps the timeline zoom;
  // plain wheel keeps scrolling. Accumulated with a cooldown so one gesture
  // moves exactly one level. Non-passive: must suppress the browser zoom.
  let gridWheelAcc = 0;
  let gridWheelCool = 0;

  function gridZoomWheel(node: HTMLElement) {
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      if (tab !== "library" || viewer || show) return;
      const now = performance.now();
      if (now < gridWheelCool) return;
      const delta = e.deltaY * (e.deltaMode === 1 ? 16 : 1);
      gridWheelAcc += delta;
      if (Math.abs(gridWheelAcc) < 50) return;
      // Wheel up (deltaY −) = zoom in, matching the + button (all → year).
      zoomLib(gridWheelAcc > 0 ? -1 : 1);
      gridWheelAcc = 0;
      gridWheelCool = now + 250;
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return { destroy() { node.removeEventListener("wheel", onWheel); } };
  }

  // Effective grouping: a zoomed timeline always sections by date at the
  // zoom granularity; at "all" the Group-by select owns the grid (its date
  // branch keeps the historical month granularity).
  let effGroup = $derived<GroupMode>(libZoom === "all" ? groupBy : "date");

  type DateGranularity = "day" | "month" | "year";

  function dateGranularity(): DateGranularity {
    return libZoom === "all" ? "month" : libZoom;
  }

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

  function dayKeyOf(a: AssetRow): string {
    const t = a.capturedAt ?? a.modified;
    if (!t) return "";
    const d = new Date(t * 1000);
    return `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;
  }

  function dayLabelOf(key: string): string {
    if (!key) return "Unknown date";
    const [y, m, d] = key.split("-").map(Number);
    return new Date(Date.UTC(y, m - 1, d)).toLocaleDateString(undefined, {
      month: "long",
      day: "numeric",
      year: "numeric",
      timeZone: "UTC"
    });
  }

  function yearKeyOf(a: AssetRow): string {
    const t = a.capturedAt ?? a.modified;
    if (!t) return "";
    return String(new Date(t * 1000).getUTCFullYear());
  }

  function yearLabelOf(key: string): string {
    return key === "" ? "Unknown date" : key;
  }

  function dateKeyOf(a: AssetRow): string {
    const g = dateGranularity();
    return g === "day" ? dayKeyOf(a) : g === "year" ? yearKeyOf(a) : monthKeyOf(a);
  }

  function dateLabelOf(key: string): string {
    const g = dateGranularity();
    return g === "day" ? dayLabelOf(key) : g === "year" ? yearLabelOf(key) : monthLabelOf(key);
  }

  function groupKeyOf(a: AssetRow): string {
    switch (effGroup) {
      case "date": return dateKeyOf(a);
      case "location": return a.place ?? "";
      case "person": return a.people[0] ?? "";
      case "media": return a.kind;
      default: return "";
    }
  }

  function groupLabelOf(a: AssetRow): string {
    switch (effGroup) {
      case "date": return dateLabelOf(dateKeyOf(a));
      case "location": return a.place ?? "No location";
      case "person": return a.people[0] ?? "No one tagged";
      case "media": return a.kind === "image" ? "Photos" : "Videos";
      default: return "";
    }
  }

  // Display order when grouped: sections sorted (newest month first,
  // names A–Z, unknowns last), newest photo first inside each section.
  // Carries the flat-list index so the viewer, selection and Similar
  // keep working on the underlying order. Honors an active drill-down.
  let sectionAssets = $derived.by(() => {
    // Index stays assets-based so the viewer, selection and Similar keep
    // working on the underlying order even under a drill-down filter.
    const items = assets
      .map((asset, index) => ({ asset, index }))
      .filter(({ asset }) =>
        !zoomDrill ||
        (zoomDrill.from === "month"
          ? monthKeyOf(asset) === zoomDrill.key
          : yearKeyOf(asset) === zoomDrill.key)
      );
    if (effGroup === "none") return items;
    const dir = effGroup === "date" ? -1 : 1;
    // Unknowns sort last in every mode ("" is already last descending).
    const sortKey = (k: string) => (k === "" && effGroup !== "date" ? "\uffff" : k);
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

  // Highlights mode: only at the Month/Year timeline zooms does the grid
  // show a curated reel per section (favorites → resolution → newest,
  // capped) instead of every photo. At "all" every section shows everything
  // — including Group-by Date month sections — so All never looks like Month.
  // Day zoom and the flat grid always show everything.
  let highlightsMode = $derived(
    libZoom !== "all" && effGroup === "date" && (dateGranularity() === "month" || dateGranularity() === "year")
  );

  function compareHighlights(a: AssetRow, b: AssetRow): number {
    const fa = a.favorite !== 0 ? 1 : 0;
    const fb = b.favorite !== 0 ? 1 : 0;
    if (fa !== fb) return fb - fa;
    const ra = (a.width ?? 0) * (a.height ?? 0);
    const rb = (b.width ?? 0) * (b.height ?? 0);
    if (ra !== rb) return rb - ra;
    const ta = a.capturedAt ?? a.modified ?? 0;
    const tb = b.capturedAt ?? b.modified ?? 0;
    return tb - ta || b.id - a.id;
  }

  let displayAssets = $derived.by(() => {
    // Structural backstop for the keyed grid: no producer may ever hand the
    // each block a duplicated id (see loadMore). First occurrence wins so
    // viewer indices stay stable.
    const uniq = (list: { asset: AssetRow; index: number }[]) => {
      const seen = new Set<number>();
      return list.filter(({ asset }) => {
        if (seen.has(asset.id)) return false;
        seen.add(asset.id);
        return true;
      });
    };
    if (!highlightsMode) return uniq(sectionAssets);
    const cap = highlightCap();
    const out: { asset: AssetRow; index: number }[] = [];
    let chunk: { asset: AssetRow; index: number }[] = [];
    let cur: string | null = null;
    const flush = () => {
      chunk.sort((a, b) => compareHighlights(a.asset, b.asset));
      for (const e of chunk.slice(0, cap)) out.push(e);
      chunk = [];
    };
    for (const e of sectionAssets) {
      const k = groupKeyOf(e.asset);
      if (cur === null) cur = k;
      if (k !== cur) {
        flush();
        cur = k;
      }
      chunk.push(e);
    }
    flush();
    return uniq(out);
  });

  let groupCounts = $derived.by(() => {
    const m = new Map<string, number>();
    if (effGroup === "none") return m;
    for (const a of drilledAssets) {
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

  const FILTERS_KEY = "fotoflow-filters";

  // Filter persistence (issue #6): same load/save + try/catch pattern as the
  // flagged/group-by keys above. Validated field-by-field so a stale or
  // hand-edited entry can never break the filter types.
  function loadSavedFilters(): {
    filter?: "all" | "image" | "video";
    favFilter?: boolean;
    petFilter?: boolean;
    monthFilter?: string | null;
    locFilter?: "all" | "with" | "without";
    personFilter?: number | null;
    sceneFilter?: string | null;
    sortAdded?: boolean;
  } {
    try {
      if (typeof localStorage === "undefined") return {};
      const raw = localStorage.getItem(FILTERS_KEY);
      if (!raw) return {};
      const p: unknown = JSON.parse(raw);
      if (typeof p !== "object" || p === null) return {};
      const q = p as Record<string, unknown>;
      return {
        ...(q.filter === "all" || q.filter === "image" || q.filter === "video" ? { filter: q.filter } : {}),
        ...(typeof q.favFilter === "boolean" ? { favFilter: q.favFilter } : {}),
        ...(typeof q.petFilter === "boolean" ? { petFilter: q.petFilter } : {}),
        ...(typeof q.monthFilter === "string" ? { monthFilter: q.monthFilter } : {}),
        ...(q.locFilter === "all" || q.locFilter === "with" || q.locFilter === "without" ? { locFilter: q.locFilter } : {}),
        ...(typeof q.personFilter === "number" ? { personFilter: q.personFilter } : {}),
        ...(typeof q.sceneFilter === "string" ? { sceneFilter: q.sceneFilter } : {}),
        ...(typeof q.sortAdded === "boolean" ? { sortAdded: q.sortAdded } : {})
      };
    } catch {
      return {};
    }
  }

  const savedFilters = loadSavedFilters();

  let filter = $state<"all" | "image" | "video">(savedFilters.filter ?? "all");
  let favFilter = $state(savedFilters.favFilter ?? false);
  let petFilter = $state(savedFilters.petFilter ?? false);
  // Recently Saved ordering (Pinned shortcut): newest first_seen_at first.
  let sortAdded = $state(savedFilters.sortAdded ?? false);
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
  let sceneFilter = $state<string | null>(savedFilters.sceneFilter ?? null);
  let tagOptions = $state<SceneTagCount[]>([]);
  let tagging = $state(false);
  let tagState = $state<TagProgress | null>(null);
  let monthFilter = $state<string | null>(savedFilters.monthFilter ?? null);
  let locFilter = $state<"all" | "with" | "without">(savedFilters.locFilter ?? "all");
  let personFilter = $state<number | null>(savedFilters.personFilter ?? null);
  let months = $state<LibraryMonth[]>([]);
  let personOptions = $state<FaceClusterRow[]>([]);
  let filteredTotal = $state(0);

  function saveFilters() {
    try {
      localStorage.setItem(
        FILTERS_KEY,
        JSON.stringify({ filter, favFilter, petFilter, monthFilter, locFilter, personFilter, sceneFilter, sortAdded })
      );
    } catch {
      /* storage unavailable: filters just don't persist */
    }
  }

  // Every filter assignment above flows through here (reads inside an
  // effect subscribe); writing storage never re-triggers the effect.
  $effect(() => {
    saveFilters();
  });

  function personFilterName(): string | null {
    if (personFilter === null) return null;
    const hit =
      personOptions.find((c) => c.id === personFilter) ?? people.find((c) => c.id === personFilter);
    return hit?.name ?? `Cluster ${personFilter}`;
  }

  // Active-filter summary (issue #6): one line like
  // "Filters: photos · October 2025 · Lake". Rendered whenever any of the
  // persisted filters is active, so a People→Library jump (which sets
  // personFilter while the person dropdown may have no options yet) stays
  // visible with a way back.
  let activeFilterParts = $derived.by(() => {
    const parts: string[] = [];
    if (filter === "image") parts.push("photos");
    else if (filter === "video") parts.push("videos");
    if (favFilter) parts.push("favorites");
    if (petFilter) parts.push("pets");
    if (monthFilter) parts.push(monthLabelOf(monthFilter));
    const pn = personFilterName();
    if (pn) parts.push(pn);
    if (sceneFilter) parts.push(sceneFilter);
    if (locFilter === "with") parts.push("with location");
    else if (locFilter === "without") parts.push("no location");
    if (sortAdded) parts.push("recently added");
    return parts;
  });

  function clearAllFilters() {
    filter = "all";
    favFilter = false;
    petFilter = false;
    monthFilter = null;
    locFilter = "all";
    personFilter = null;
    sceneFilter = null;
    sortAdded = false;
    void refresh();
  }

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
      capturedMax,
      sortAdded: sortAdded || undefined
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
  let flagged = $state<Set<number>>(loadFlagged());
  let peopleLoading = $state(false);
  let events = $state<EventRow[]>([]);
  let eventsLoading = $state(false);
  let gapHours = $state(4);
  let openEventId = $state<number | null>(null);
  let eventMembers = $state<Record<number, EventAssetRow[]>>({});
  // Trips: dynamic Trip → Stop grouping over events (Stage 2). Loaded with
  // events; per-trip album target mirrors the per-event dropdown.
  let trips = $state<TripRow[]>([]);
  let tripAlbumChoice = $state<Record<string, number | null>>({});
  let tripAlbumBusy = $state<string | null>(null);
  let resolvingPlaces = $state(false);
  let geocodeState = $state<GeocodeProgress | null>(null);
  // Lightbox viewer over the current library page (index into `assets`),
  // or over an explicit list (cluster review mode).
  let viewer = $state<{ index: number } | null>(null);
  let viewerList = $state<AssetRow[] | null>(null);
  let reviewCluster = $state<{ id: number; name: string } | null>(null);
  let viewerSrc = $state<string | null>(null);
  let viewerError = $state<string | null>(null);
  // Photo info panel (ⓘ): camera settings + GPS + file facts. Stays open
  // across steps like Apple's inspector; closed with the viewer/theater.
  let infoOpen = $state(false);
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

  // Session-only undo: the last destructive action (trash/flag/merge/
  // unassign). Replaced by the next destructive action, never persisted.
  type UndoAction = {
    kind: "trash" | "flag" | "merge" | "unassign";
    ids: number[];
    label: string;
    extra?: {
      faceId?: number;
      clusterId?: number;
      into?: number;
      snapshots?: { id: number; name: string | null; faceIds: number[] }[];
    };
  };
  let lastAction = $state<UndoAction | null>(null);

  // Toast auto-dismiss (issue #6): plain notices fade after ~6s. A pending
  // Undo keeps the toast up so the action stays reversible; assigning a new
  // toast replaces the old one and restarts the timer via effect cleanup.
  $effect(() => {
    if (toast && !lastAction) {
      const t = setTimeout(() => {
        toast = null;
      }, 6000);
      return () => clearTimeout(t);
    }
  });

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

  // Photo info panel: camera settings + GPS + file facts, Apple-style.
  // Every helper returns null when the file carries no such EXIF so the
  // panel simply omits that row instead of showing dashes.
  function trimNum(v: number): string {
    return String(Math.round(v * 100) / 100);
  }

  function fmtFNumber(v: number | null): string | null {
    if (v === null || !(v > 0)) return null;
    return `ƒ/${trimNum(v)}`;
  }

  function fmtExposure(v: number | null): string | null {
    if (v === null || !(v > 0)) return null;
    if (v >= 1) return `${trimNum(v)} s`;
    return `1/${Math.round(1 / v)}`;
  }

  function fmtFocal(a: AssetRow): string | null {
    const mm = a.focalLength35 ?? a.focalLength;
    if (mm === null || !(mm > 0)) return null;
    return `${trimNum(mm)} mm`;
  }

  function exposureParts(a: AssetRow): string[] {
    const parts: string[] = [];
    const f = fmtFNumber(a.fNumber);
    if (f) parts.push(f);
    const s = fmtExposure(a.exposureTime);
    if (s) parts.push(s);
    if (a.iso !== null && a.iso > 0) parts.push(`ISO ${a.iso}`);
    const fl = fmtFocal(a);
    if (fl) parts.push(fl);
    return parts;
  }

  function cameraLabel(a: AssetRow): string | null {
    const { cameraMake: make, cameraModel: model } = a;
    if (model && make && model.toLowerCase().startsWith(make.toLowerCase())) return model;
    const bits = [make, model].filter((x): x is string => !!x);
    return bits.length > 0 ? bits.join(" ") : null;
  }

  function gpsLabel(a: AssetRow): string | null {
    if (a.latitude === null || a.longitude === null) return null;
    return `${a.latitude.toFixed(4)}, ${a.longitude.toFixed(4)}`;
  }

  // Plain-language errors (issue #6): known engine/sidecar jargon maps to a
  // short friendly line; the raw text stays as a suffix in parentheses.
  function friendlyError(e: unknown): string {
    const raw = (e instanceof Error ? e.message : String(e)).replace(/^Error:\s*/, "");
    const lower = raw.toLowerCase();
    let plain: string | null = null;
    if (lower.includes("enoent") || lower.includes("no such file")) plain = "That file wasn't found";
    else if (lower.includes("eacces") || lower.includes("permission")) plain = "Fotoflow can't open that file";
    else if (lower.includes("cannot decode") || lower.includes("unreadable")) plain = "That photo couldn't be read";
    else if (lower.includes("sidecar") || lower.includes("did not answer")) plain = "The photo engine isn't responding";
    else if (lower.includes("not mocked")) plain = "That needs the desktop app";
    else if (lower.includes("vector")) plain = "Similarity search needs a one-time setup first";
    return plain ? `${plain} (${raw})` : raw;
  }

  // Partial-failure lists (issue #6): up to 3 failed names, never bare counts.
  function partialNames(failed: string[]): string {
    return failed.slice(0, 3).join(", ") + (failed.length > 3 ? ", …" : "");
  }

  async function refresh() {
    resetThumbs();
    const f = currentFilters();
    assets = await libraryList({ ...f, limit: pageSize() });
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

  let loadingMore = $state(false);

  async function loadMore() {
    if (loadingMore) return;
    if (assets.length >= filteredTotal) return;
    loadingMore = true;
    try {
      const next = await libraryList({
        ...currentFilters(),
        limit: pageSize(),
        offset: assets.length
      });
      if (next.length === 0) return;
      // Offset windows go stale while the library mutates underneath the
      // grid: imports prepend rows and enrich backfills capture dates, so a
      // later page can repeat ids from an earlier one. Appending those would
      // duplicate keys in the grid's keyed each block, and Svelte aborts the
      // update with each_key_duplicate — a frozen grid that looks hung.
      const seen = new Set(assets.map((a) => a.id));
      const fresh = next.filter((a) => !seen.has(a.id));
      if (fresh.length === 0) {
        // Everything overlapped: the total moved under us, so resync the
        // gate and let the next scroll settle instead of spinning.
        try {
          filteredTotal = await libraryCount(currentFilters());
        } catch {
          /* keep the stale total; a later page may still advance */
        }
        return;
      }
      assets = [...assets, ...fresh];
      void loadThumbs(fresh);
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      loadingMore = false;
    }
    // Viewport fill: if the sentinel is still visible after appending (short
    // pages, tall viewports), keep paging without waiting for another scroll
    // transition — a stationary sentinel otherwise stalls the grid until the
    // user scrolls away and back. Only reached when this load appended rows,
    // so fully-overlapped pages (resync above) stop here instead of spinning.
    if (typeof document !== "undefined") {
      const el = document.getElementById("grid-sentinel");
      const r = el?.getBoundingClientRect();
      if (el && r && r.top < window.innerHeight && r.bottom > 0 && assets.length < filteredTotal) {
        void loadMore();
      }
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

  // Infinite scroll: the #grid-sentinel div at the grid end reuses the
  // thumb-observer pattern — on intersect it pages in the next chunk via
  // the existing loadMore() (which no-ops while loading or fully loaded).
  let gridObserver: IntersectionObserver | null = null;

  function ensureGridObserver(): IntersectionObserver | null {
    if (gridObserver) return gridObserver;
    if (typeof IntersectionObserver === "undefined") return null;
    gridObserver = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) void loadMore();
        }
      },
      // No prefetch margin: the sentinel must actually enter the viewport,
      // so a short library never pages past its first chunk on its own.
      { rootMargin: "0px" }
    );
    return gridObserver;
  }

  function observeGridEnd(node: HTMLElement) {
    const obs = ensureGridObserver();
    if (obs) obs.observe(node);
    return {
      destroy() {
        gridObserver?.unobserve(node);
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
    for (const { asset } of sectionAssets) {
      if (groupKeyOf(asset) === key) out.push(asset.id);
    }
    return out;
  }

  // One-go group selection: selects every loaded photo in the section
  // (or clears just the section when all of it is already selected).
  // Section = the full loaded set, not just the highlights reel.
  // Additive across groups; only reachable in select mode (the header
  // checkbox is gated on selectMode like the per-photo boxes), so no
  // mode flip here — the library never looks selected by default.
  function toggleGroup(key: string) {
    const ids = groupIds(key);
    const next = new Set(selected);
    const all = ids.length > 0 && ids.every((id) => next.has(id));
    if (all) for (const id of ids) next.delete(id);
    else for (const id of ids) next.add(id);
    selected = next;
  }

  // Library grid keyboard support: roving focus over the visual grid plus
  // select-mode shortcuts. Reuses the existing `selected` set, `confirm`
  // flow (via batchTrash) and toggleFavorite — no parallel selection model.
  let gridFocusId = $state<number | null>(null);
  let rangeAnchorId = $state<number | null>(null);

  function gridColumnCount(): number {
    try {
      const figs = Array.from(document.querySelectorAll(".grid figure"));
      if (figs.length < 2) return 1;
      const top = (figs[0] as HTMLElement).offsetTop;
      let cols = 0;
      for (const f of figs) {
        if ((f as HTMLElement).offsetTop === top) cols++;
        else break;
      }
      return Math.max(1, cols);
    } catch {
      return 1;
    }
  }

  function gridFocusPos(): number {
    const i = displayAssets.findIndex((x) => x.asset.id === gridFocusId);
    return i < 0 ? 0 : i;
  }

  function focusGridAt(pos: number, extend = false) {
    const items = displayAssets;
    if (items.length === 0) return;
    const at = Math.max(0, Math.min(pos, items.length - 1));
    const prev = gridFocusId;
    const entry = items[at];
    gridFocusId = entry.asset.id;
    try {
      (document.querySelector(`[data-grid-id="${entry.asset.id}"]`) as HTMLElement | null)?.focus();
    } catch {
      /* focus stays where it was */
    }
    if (!selectMode) return;
    if (extend) {
      const anchor = rangeAnchorId ?? prev ?? entry.asset.id;
      if (rangeAnchorId === null) rangeAnchorId = anchor;
      const ai = items.findIndex((x) => x.asset.id === anchor);
      const a = ai < 0 ? at : ai;
      const next = new Set(selected);
      for (let i = Math.min(a, at); i <= Math.max(a, at); i++) next.add(items[i].asset.id);
      selected = next;
    } else {
      rangeAnchorId = entry.asset.id;
    }
  }

  // Typing targets stay out of the way: text inputs, search boxes, selects,
  // textareas and contenteditables never trigger grid shortcuts. Grid
  // checkboxes/radios are controls, not text entry, so keys pass through.
  function isTypingTarget(t: EventTarget | null): boolean {
    const el = t as HTMLElement | null;
    if (!el || !("tagName" in el)) return false;
    if (el.isContentEditable) return true;
    const tag = el.tagName;
    if (tag === "TEXTAREA" || tag === "SELECT") return true;
    if (tag === "INPUT") {
      const type = (el as HTMLInputElement).type;
      if (type === "checkbox" || type === "radio") return false;
      return true;
    }
    return false;
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
    // Honor the user's Trash-vs-Flag mode; selection clears only after the
    // action is confirmed (Cancel in the dialog keeps the selection).
    confirm = { ids: [...selected], label: `${selected.size} selected assets` };
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
        place: null, people: [], cameraMake: null, cameraModel: null, lens: null,
        latitude: null, longitude: null, iso: null, fNumber: null,
        exposureTime: null, focalLength: null, focalLength35: null
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
    resetShowZoom();
    const at = Math.min(Math.max(index, 0), items.length - 1);
    show = { items, index: at, playing: true, src: null, error: null };
    void loadShow(items[at]);
    showTimer = setInterval(() => stepShow(1), SLIDE_MS);
  }

  function stepShow(delta: number) {
    if (!show || show.items.length === 0) return;
    const next = (show.index + delta + show.items.length) % show.items.length;
    show.index = next;
    resetShowZoom();
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

  // Slideshow zoom: per-slide scale + pan (reset on every slide change).
  // Buttons, +/−/0 keys and wheel zoom; drag pans once zoomed past 1x.
  let showZoom = $state(1);
  let showPan = $state({ x: 0, y: 0 });

  function resetShowZoom() {
    showZoom = 1;
    showPan = { x: 0, y: 0 };
  }

  function zoomShow(dir: 1 | -1) {
    if (!show) return;
    const next = dir > 0 ? showZoom * 1.25 : showZoom / 1.25;
    showZoom = Math.min(5, Math.max(1, Math.abs(next - 1) < 0.01 ? 1 : next));
    if (showZoom === 1) showPan = { x: 0, y: 0 };
  }

  function showMediaStyle(): string {
    // translate() is outermost so pan is already in screen px: 1px of drag
    // moves the image 1px on screen at any zoom (do NOT divide by zoom —
    // that makes the image lag the cursor).
    const z = showZoom;
    return `transform: translate(${showPan.x}px, ${showPan.y}px) scale(${z}); transform-origin: center;` +
      (z > 1 ? ` cursor: ${panDrag ? "grabbing" : "grab"}; touch-action: none;` : "");
  }

  // Non-passive wheel listener so pinch/scroll-to-zoom never scrolls the page.
  function zoomWheel(node: HTMLElement) {
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomShow(e.deltaY > 0 ? -1 : 1);
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return { destroy() { node.removeEventListener("wheel", onWheel); } };
  }

  let panDrag = $state<{ startX: number; startY: number; baseX: number; baseY: number } | null>(null);

  function panStart(e: PointerEvent) {
    if (showZoom <= 1 || e.button !== 0) return;
    // Kill native image drag (it would steal the gesture after a few px)
    // and track on window: Svelte delegates pointer handlers, so capture on
    // currentTarget would retarget events away from the image mid-drag.
    e.preventDefault();
    panDrag = { startX: e.clientX, startY: e.clientY, baseX: showPan.x, baseY: showPan.y };
    window.addEventListener("pointermove", panMove);
    window.addEventListener("pointerup", panEnd, { once: true });
    window.addEventListener("pointercancel", panEnd, { once: true });
  }

  function panMove(e: PointerEvent) {
    if (!panDrag) return;
    showPan = {
      x: panDrag.baseX + (e.clientX - panDrag.startX),
      y: panDrag.baseY + (e.clientY - panDrag.startY)
    };
  }

  function panEnd(e: PointerEvent) {
    // Snap to the release point: a dropped/coalesced final move must not
    // leave the image short of where the pointer lifted.
    if (panDrag) {
      showPan = {
        x: panDrag.baseX + (e.clientX - panDrag.startX),
        y: panDrag.baseY + (e.clientY - panDrag.startY)
      };
    }
    panDrag = null;
    window.removeEventListener("pointermove", panMove);
  }

  // Slideshow is fullscreen: lock page scroll behind the theater while open
  // (otherwise the body's scrollbar stays visible at the right edge).
  $effect(() => {
    if (!show) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = prev;
    };
  });

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

  // Blurred review items need viewer-quality sources like compare panes.
  $effect(() => {
    if (!review || reviewItems.length === 0) return;
    const item = reviewItems[Math.min(review.pos, reviewItems.length - 1)];
    if (item?.kind !== "blur") return;
    const a = blurred.find((x) => x.id === item.assetId);
    if (a) void ensureCompareSrc(blurAsMember(a));
  });

  function viewerKey(e: KeyboardEvent) {
    // Cleanup review mode owns keys on its tab (no dialogs, not typing).
    if (tab === "cleanup" && review && !confirm && !similar && !picker && !isTypingTarget(e.target)) {
      reviewKey(e);
      return;
    }
    if (show) {
      if (e.key === "Escape") closeShow();
      else if (e.key === "ArrowRight") { e.preventDefault(); stepShow(1); }
      else if (e.key === "ArrowLeft") { e.preventDefault(); stepShow(-1); }
      else if (e.key === " ") { e.preventDefault(); toggleShowPlay(); }
      else if (e.key === "+" || e.key === "=") { e.preventDefault(); zoomShow(1); }
      else if (e.key === "-" || e.key === "_") { e.preventDefault(); zoomShow(-1); }
      else if (e.key === "0") { e.preventDefault(); resetShowZoom(); }
      else if ((e.key === "i" || e.key === "I") && !isTypingTarget(e.target)) {
        e.preventDefault();
        infoOpen = !infoOpen;
      }
      return;
    }
    if (!viewer) {
      if (tab !== "library") return;
      if (confirm || picker || similar || nameDialog || deleteAlbumTarget || removeRootTarget) return;
      if (isTypingTarget(e.target)) return;
      // Timeline zoom: Ctrl/Cmd+= in, Ctrl/Cmd+− out (library only, no
      // dialogs/viewer/theater open, never inside text entry).
      if (
        (e.ctrlKey || e.metaKey) && !e.altKey &&
        (e.key === "=" || e.key === "+" || e.key === "-" || e.key === "_")
      ) {
        e.preventDefault();
        zoomLib(e.key === "=" || e.key === "+" ? 1 : -1);
        return;
      }
      const items = displayAssets;
      if (items.length === 0) return;
      if ((e.ctrlKey || e.metaKey) && (e.key === "a" || e.key === "A")) {
        if (!selectMode) return;
        e.preventDefault();
        selected = new Set(items.map((x) => x.asset.id));
        return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const pos = gridFocusPos();
      if (e.key === "ArrowRight") { e.preventDefault(); focusGridAt(gridFocusId === null ? 0 : pos + 1, e.shiftKey); }
      else if (e.key === "ArrowLeft") { e.preventDefault(); focusGridAt(gridFocusId === null ? 0 : pos - 1, e.shiftKey); }
      else if (e.key === "ArrowDown") { e.preventDefault(); focusGridAt(gridFocusId === null ? 0 : pos + gridColumnCount(), e.shiftKey); }
      else if (e.key === "ArrowUp") { e.preventDefault(); focusGridAt(gridFocusId === null ? 0 : pos - gridColumnCount(), e.shiftKey); }
      else if (e.key === "Home") { e.preventDefault(); focusGridAt(0, e.shiftKey); }
      else if (e.key === "End") { e.preventDefault(); focusGridAt(items.length - 1, e.shiftKey); }
      else if (e.key === "Enter") {
        const cur = items[gridFocusId === null ? 0 : pos];
        if (!cur) return;
        e.preventDefault();
        gridFocusId = cur.asset.id;
        if (selectMode) { toggleSelect(cur.asset.id); rangeAnchorId = cur.asset.id; }
        else openViewer(cur.index);
      } else if (e.key === " ") {
        if (!selectMode) return;
        const cur = items[gridFocusId === null ? 0 : pos];
        if (!cur) return;
        e.preventDefault();
        gridFocusId = cur.asset.id;
        toggleSelect(cur.asset.id);
        rangeAnchorId = cur.asset.id;
      } else if (e.key === "Delete" || e.key === "Backspace") {
        if (!selectMode || selected.size === 0) return;
        e.preventDefault();
        batchTrash();
      } else if (e.key === "s" || e.key === "S") {
        if (selectMode && selected.size > 0) { e.preventDefault(); void batchFavorite(); }
        else {
          const cur = items[gridFocusId === null ? 0 : pos];
          if (!cur) return;
          e.preventDefault();
          gridFocusId = cur.asset.id;
          void toggleFavorite(cur.asset);
        }
      }
      return;
    }
    if (e.key === "Escape") closeViewer();
    else if (e.key === "ArrowRight") { e.preventDefault(); stepViewer(1); }
    else if (e.key === "ArrowLeft") { e.preventDefault(); stepViewer(-1); }
    else if ((e.key === "s" || e.key === "S") && !isTypingTarget(e.target)) {
      const rows = viewerRows();
      const cur = rows[viewer.index];
      if (cur && cur.id >= 0) { e.preventDefault(); void toggleFavorite(cur); }
    } else if ((e.key === "i" || e.key === "I") && !isTypingTarget(e.target)) {
      e.preventDefault();
      infoOpen = !infoOpen;
    }
  }

  // A11y (issue #4): roving-tabindex tablist with automatic activation.
  const TAB_ORDER = ["library", "cleanup", "people", "events", "albums", "map", "settings"] as const;
  type TabId = (typeof TAB_ORDER)[number];

  function activateTab(next: TabId) {
    if (tab === next) return;
    tab = next;
    if (next === "library") void refresh();
    else if (next === "cleanup") void refreshCleanup();
    else if (next === "people") { void loadPeople(); void loadPets(); }
    else if (next === "events") void loadEvents();
    else if (next === "albums") void loadAlbums();
    else if (next === "map") void loadMap();
  }

  // Pinned shortcuts (Apple Photos-style): one click jumps to a tab with a
  // clean filter preset. Only existing capabilities — no new queries beyond
  // the sortAdded ordering. Recently Deleted is deliberately absent: there
  // is no trash history to show yet.
  function resetLibraryFilters() {
    filter = "all";
    favFilter = false;
    petFilter = false;
    monthFilter = null;
    locFilter = "all";
    personFilter = null;
    sceneFilter = null;
    sortAdded = false;
    if (searchTimer) clearTimeout(searchTimer);
    searchText = "";
    searchQuery = null;
  }

  async function pinLibrary(preset: () => void) {
    tab = "library";
    resetLibraryFilters();
    preset();
    await refresh();
  }

  function pinFavorites() {
    return pinLibrary(() => {
      favFilter = true;
    });
  }

  function pinRecent() {
    return pinLibrary(() => {
      sortAdded = true;
    });
  }

  function pinVideos() {
    return pinLibrary(() => {
      filter = "video";
    });
  }

  function pinScreenshots() {
    tab = "library";
    resetLibraryFilters();
    return applySearch("screenshot");
  }

  function pinMap() {
    if (tab === "map") return;
    tab = "map";
    void loadMap();
  }

  function pinPeople() {
    if (tab === "people") return;
    tab = "people";
    void loadPeople();
    void loadPets();
  }

  function tabKey(e: KeyboardEvent) {
    const el = e.target as HTMLElement | null;
    if (!el || el.getAttribute("role") !== "tab") return;
    const i = TAB_ORDER.indexOf(tab);
    let next: TabId | null = null;
    if (e.key === "ArrowRight") next = TAB_ORDER[(i + 1) % TAB_ORDER.length];
    else if (e.key === "ArrowLeft") next = TAB_ORDER[(i - 1 + TAB_ORDER.length) % TAB_ORDER.length];
    else if (e.key === "Home") next = TAB_ORDER[0];
    else if (e.key === "End") next = TAB_ORDER[TAB_ORDER.length - 1];
    else return;
    e.preventDefault();
    const target = next;
    activateTab(target);
    void tick().then(() => document.getElementById(`tab-${target}`)?.focus());
  }

  // A11y (issue #4): modal dialogs — initial focus, focus trap, Esc cancels.
  function trapTabKey(e: KeyboardEvent, root: HTMLElement) {
    if (e.key !== "Tab") return;
    const items = [...root.querySelectorAll<HTMLElement>(
      "button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex='-1'])"
    )].filter((n) => n.offsetParent !== null);
    if (items.length === 0) { e.preventDefault(); return; }
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }

  function dialogFocus(node: HTMLElement, onClose: () => void) {
    const prev = document.activeElement as HTMLElement | null;
    const first = node.querySelector<HTMLElement>("input:not(:disabled), button:not(:disabled)");
    (first ?? node).focus();
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); return; }
      trapTabKey(e, node);
    }
    node.addEventListener("keydown", onKey);
    return {
      destroy() {
        node.removeEventListener("keydown", onKey);
        if (prev && document.contains(prev)) prev.focus();
      }
    };
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
      error = friendlyError(e);
    } finally {
      importing = false;
    }
  }

  // SSD ingest: copy a camera card / device folder to fast storage first,
  // then index the copy. Progress streams on ingest-progress; the result
  // offers one-click indexing of the destination.
  let ingest = $state<{
    src: string;
    dest: string;
    running: boolean;
    progress: IngestProgress | null;
    result: IngestSummary | null;
  } | null>(null);

  function openIngest() {
    ingest = { src: "", dest: "", running: false, progress: null, result: null };
  }

  async function pickIngestDir(which: "src" | "dest") {
    const picked = await open({
      directory: true,
      multiple: false,
      title: which === "src" ? "Copy photos from…" : "Copy photos to…"
    });
    if (typeof picked === "string" && picked !== "" && ingest) ingest[which] = picked;
  }

  async function runIngest() {
    if (!ingest || ingest.running) return;
    const src = ingest.src.trim();
    const dest = ingest.dest.trim();
    if (src === "" || dest === "") {
      error = "Pick a source folder and a destination first";
      return;
    }
    ingest.running = true;
    ingest.progress = null;
    ingest.result = null;
    try {
      const summary = await ingestCopy(src, dest);
      if (ingest) ingest.result = summary;
    } catch (e) {
      error = friendlyError(e);
    } finally {
      if (ingest) ingest.running = false;
    }
  }

  // Best-of-place pick: balanced coverage curation for one event.
  // Loads members on demand like addEventToAlbum (no expand-first needed).
  async function addEventPicks(eventId: number) {
    const albumId = eventAlbumChoice[eventId] ?? albumOptions[0]?.id;
    if (albumId == null || eventAlbumBusy !== null) return;
    let members = eventMembers[eventId];
    if (members === undefined) {
      try {
        members = await eventAssets(eventId);
        eventMembers = { ...eventMembers, [eventId]: members };
      } catch (e) {
        error = String(e).replace(/^Error: /, "");
        return;
      }
    }
    if (members.length === 0) {
      toast = "No readable photos in this event";
      return;
    }
    eventAlbumBusy = eventId;
    try {
      const added = await addEventPicksToAlbum(eventId, albumId, 12);
      const target = albumOptions.find((a) => a.id === albumId);
      await loadAlbums();
      toast =
        target != null
          ? `Added ${added} top pick${added === 1 ? "" : "s"} to “${target.name}”`
          : `Added ${added} top pick${added === 1 ? "" : "s"} to the album`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventAlbumBusy = null;
    }
  }

  // Space saver: duplicate extras + blurred, recoded small into one folder.
  // Near-group lookalikes stay out: those deserve a review decision first.
  let spaceSaverIds = $derived.by(() => {
    const ids = new Set<number>();
    for (const g of groups) {
      for (const m of g.members) {
        if (m.id !== g.keptId) ids.add(m.id);
      }
    }
    for (const a of blurred) ids.add(a.id);
    return [...ids];
  });

  async function runSpaceSaver() {
    const ids = spaceSaverIds;
    if (ids.length === 0 || exportBusy !== null) return;
    const dest = await open({ directory: true, multiple: false, title: "Compressed copies go to…" });
    if (typeof dest !== "string" || dest === "") return;
    exportBusy = "space-saver";
    exportResult = null;
    try {
      exportResult = await exportBundle({ assetIds: ids }, "space-saver", "compact", dest);
      exportKey = "space-saver";
      toast = `Compressed ${exportResult.exported} photo${exportResult.exported === 1 ? "" : "s"} (${fmtBytes(exportResult.bytes)}) to ${exportResult.dir}`;
    } catch (e) {
      error = friendlyError(e);
    } finally {
      exportBusy = null;
    }
  }

  // Recently Deleted: trashed assets beyond the session-undo window.
  // Restoring reuses the same ~/.Trash lookup as session undo; entries whose
  // files left the macOS Trash report the plain-language reason instead.
  let trashOpen = $state(false);
  let trashRows = $state<TrashEntry[]>([]);
  let trashLoading = $state(false);

  async function openTrash() {
    trashOpen = true;
    trashLoading = true;
    try {
      trashRows = await trashList();
    } catch (e) {
      error = friendlyError(e);
    } finally {
      trashLoading = false;
    }
  }

  async function restoreTrashRow(entry: TrashEntry) {
    try {
      const r = await restoreAssets([entry.assetId]);
      if (r.restored > 0) {
        trashRows = trashRows.filter((t) => t.assetId !== entry.assetId);
        await refresh();
        toast = `Restored ${entry.name}`;
      } else {
        error = r.failed[0] ?? `Couldn't restore ${entry.name}`;
      }
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function restoreTrashAll() {
    const ids = trashRows.map((t) => t.assetId);
    if (ids.length === 0) return;
    try {
      const r = await restoreAssets(ids);
      trashRows = await trashList();
      await refresh();
      toast =
        r.failed.length === 0
          ? `Restored ${r.restored} photo${r.restored === 1 ? "" : "s"}`
          : `Restored ${r.restored}, ${r.failed.length} couldn't come back`;
      if (r.failed.length > 0) error = r.failed.slice(0, 3).join("; ");
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function clearTrash() {
    try {
      const n = await clearTrashLog();
      trashRows = [];
      toast = n > 0 ? `Cleared ${n} ${n === 1 ? "entry" : "entries"} from Recently Deleted` : "Recently Deleted is already empty";
    } catch (e) {
      error = friendlyError(e);
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
        `${summary.near} near-dupe groups`,
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
      error = friendlyError(e);
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
    for (const g of groups) primeCompare(g, `dup${g.id}`);
    for (const g of nearGroups) primeCompare(g, `near${g.id}`);
    pruneVerdicts();
  }

  let highlightBusy = $state<string | null>(null);
  let highlightResult = $state<HighlightResult | null>(null);
  let highlightKey = $state<string | null>(null);
  // Per-run export job keys echoed back on every "export-progress" event;
  // exportProgress only ever shows the active run's phase.
  let highlightJob = $state<string | null>(null);
  let shareJob = $state<string | null>(null);
  let exportProgress = $state<ExportProgress | null>(null);

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
      error = friendlyError(e);
    } finally {
      exportBusy = null;
    }
  }

  async function makeHighlight(src: { eventId?: number; albumId?: number }, name: string) {
    const key = sourceKey(src);
    highlightBusy = key;
    highlightResult = null;
    highlightKey = key;
    highlightJob = `highlight:${key}`;
    try {
      highlightResult = await exportHighlight(src, name.replace(/\s+/g, "-").toLowerCase(), highlightJob);
      toast = "Highlight video exported";
    } catch (e) {
      error = friendlyError(e);
    } finally {
      highlightBusy = null;
      if (exportProgress?.job === highlightJob) exportProgress = null;
    }
  }

  // Watch the exported highlight in the viewer: a synthetic video row over
  // the exports file (same pattern as cluster review). Starring is disabled
  // for it, like other non-library rows.
  function watchHighlight() {
    if (!highlightResult) return;
    const path = highlightResult.output;
    watchPath(path, path.split("/").pop() ?? path, highlightResult.bytes, highlightResult.seconds);
  }

  // Finished-export affordances: show the pack folder in Finder, or open its
  // offline gallery page in the default browser. No-ops outside the app.
  async function revealExportDir(dir: string) {
    try {
      await revealInDir(dir);
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function openPackGallery(dir: string) {
    try {
      await openExternal(`${dir}/index.html`);
    } catch (e) {
      error = friendlyError(e);
    }
  }

  function watchPath(path: string, name: string, sizeBytes: number, duration: number | null) {
    viewerList = [
      {
        id: -1, path, name, ext: "mp4", kind: "video" as const,
        mime: "video/mp4", sizeBytes, modified: 0,
        capturedAt: null, width: 1920, height: 1080,
        duration, favorite: 0, missing: 0,
        sha256: null, place: null, people: [], cameraMake: null,
        cameraModel: null, lens: null, latitude: null, longitude: null,
        iso: null, fNumber: null, exposureTime: null, focalLength: null,
        focalLength35: null
      }
    ];
    reviewCluster = null;
    viewer = { index: 0 };
    viewerSrc = null;
    viewerError = null;
    viewerTags = [];
    void loadViewer(viewerList[0]);
  }

  // Share pack: memory video + Day-organized best + offline gallery for
  // family. Compact (chat-size, metadata-free) is the default; Drive keeps
  // full quality with an optional GPS strip.
  type ShareTarget =
    | { kind: "trip"; trip: TripRow }
    | { kind: "event"; event: EventRow }
    | { kind: "album"; album: AlbumRow };
  let sharePack = $state<ShareTarget | null>(null);
  let shareMode = $state<"drive" | "compact">("compact");
  let shareStrip = $state(true);
  let shareDest = $state("");
  let shareBusy = $state(false);
  let shareResult = $state<SharePackSummary | null>(null);

  function shareTitle(): string {
    if (!sharePack) return "";
    return sharePack.kind === "trip"
      ? sharePack.trip.name
      : sharePack.kind === "event"
        ? sharePack.event.name
        : sharePack.album.name;
  }

  function openSharePack(target: ShareTarget) {
    sharePack = target;
    shareMode = "compact";
    shareStrip = true;
    shareDest = "";
    shareResult = null;
  }

  async function pickShareDest() {
    try {
      const dest = await open({ directory: true, multiple: false, title: "Share pack goes to…" });
      if (typeof dest === "string" && dest !== "") shareDest = dest;
    } catch (e) {
      error = `Folder picker unavailable: ${String(e).replace(/^Error: /, "")}`;
    }
  }

  async function runSharePack() {
    const target = sharePack;
    const dest = shareDest.trim();
    if (!target || shareBusy || dest === "") return;
    shareBusy = true;
    shareResult = null;
    shareJob =
      target.kind === "album"
        ? `share:album:${target.album.id}`
        : target.kind === "trip"
          ? `share:trip:${target.trip.id}`
          : `share:event:${target.event.id}`;
    try {
      const source =
        target.kind === "album"
          ? { albumId: target.album.id }
          : target.kind === "trip"
            ? { eventIds: target.trip.eventIds }
            : { eventIds: [target.event.id] };
      shareResult = await exportSharePack(source, shareTitle(), shareMode, shareStrip, dest, shareJob);
      const r = shareResult;
      toast = `Packed ${r.exported} photo${r.exported === 1 ? "" : "s"}${r.skipped > 0 ? ` · ${r.skipped} skipped` : ""} to ${r.dir}`;
    } catch (e) {
      error = friendlyError(e);
    } finally {
      shareBusy = false;
      if (exportProgress?.job === shareJob) exportProgress = null;
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
        r.missing === 0 && r.restored === 0 && r.unreadable === 0
          ? "All indexed files are present"
          : `${r.missing} missing · ${r.restored} restored` +
            (r.unreadable > 0
              ? ` · ${r.unreadable} unreadable (${partialNames(r.unreadablePaths)}) — reconnect the volume and retry Check files`
              : "");
    } catch (e) {
      error = friendlyError(e);
    }
  }

  async function loadPeople() {
    resetThumbs();
    peopleLoading = true;
    people = [];
    try {
      people = await clustersList();
      void loadClusterFaces();
      void loadPersonOptions();
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
      // Same-name auto-merge: tagging two clusters identically unifies them
      // into one person (lowest id survives, the manual-merge convention).
      // Matching is trimmed + case-insensitive; the merge reuses the undoable
      // path below so a mistaken tag splits back apart with Undo.
      const norm = name.trim().toLowerCase();
      const dups =
        norm === ""
          ? []
          : people.filter(
              (c) => c.id !== clusterId && (c.name ?? "").trim().toLowerCase() === norm
            );
      if (dups.length > 0) {
        const ids = [clusterId, ...dups.map((c) => c.id)].sort((a, b) => a - b);
        const into = await mergeIdSet(ids);
        toast = `Merged ${ids.length} clusters named “${name.trim()}” (${people.find((c) => c.id === into)?.faceCount ?? 0} faces) — undo splits them back`;
      }
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
    void loadPersonOptions();
    void refresh();
  }

  // Merge helper: ids sorted ascending, ids[0] survives. Snapshots every
  // absorbed cluster BEFORE merging so Undo can split the merge back apart.
  // Shared by manual tick-merge and same-name auto-merge.
  async function mergeIdSet(ids: number[]): Promise<number> {
    const into = ids[0];
    const snapshots: { id: number; name: string | null; faceIds: number[] }[] = [];
    for (let k = 1; k < ids.length; k++) {
      const cluster = people.find((c) => c.id === ids[k]);
      let faces: FaceRow[] = [];
      try {
        faces = await facesInCluster(ids[k]);
      } catch {
        faces = [];
      }
      snapshots.push({ id: ids[k], name: cluster?.name ?? null, faceIds: faces.map((f) => f.id) });
    }
    for (let k = 1; k < ids.length; k++) await mergeClusters(ids[k], into);
    mergeSelect = new Set();
    people = await clustersList();
    void loadClusterFaces();
    void loadPersonOptions();
    lastAction = { kind: "merge", ids: ids.slice(1), label: `merge into #${into}`, extra: { into, snapshots } };
    return into;
  }

  async function runMerge() {
    if (mergeSelect.size < 2) return;
    const ids = [...mergeSelect].sort((a, b) => a - b);
    try {
      const into = await mergeIdSet(ids);
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
      lastAction = { kind: "unassign", ids: [faceId], label: "face", extra: { faceId, clusterId } };
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
  // Desktop tiles arrive as asset-protocol file URLs (browser-cached); done()
  // fires on img load so tiles fade in instead of popping, and zoom-end-only
  // loading keeps zoom animations free of tile storms.
  class CachedTiles extends L.GridLayer {
    createTile(coords: Coords, done: DoneCallback): HTMLImageElement {
      const img = document.createElement("img");
      img.alt = "";
      img.onload = () => done(undefined, img);
      img.onerror = () => done(new Error(`tile ${coords.z}/${coords.x}/${coords.y} failed to load`), img);
      mapTile(coords.z, coords.x, coords.y, tileSource).then(
        (url) => {
          img.src = url;
        },
        (err) => done(err instanceof Error ? err : new Error(String(err)), img)
      );
      return img;
    }
  }

  let mapEl = $state<HTMLDivElement | null>(null);
  let mapObj: LeafletMap | null = null;
  let pinLayer: LayerGroup | null = null;
  let tileLayer: L.GridLayer | null = null;
  let mapPinsRows = $state<MapPin[]>([]);
  let mapLoading = $state(false);
  let mapError = $state<string | null>(null);

  const TILE_SOURCE_KEY = "fotoflow-map-source";
  const TILE_MAX_ZOOM = { osm: 19, topo: 17 } as const;

  // Grid clustering: below this zoom nearby pins merge into count badges
  // (one DOM marker per cluster instead of hundreds of canvas circles);
  // at street level every pin renders individually. Cell ≈ 64 screen px.
  const MAP_CLUSTER_BELOW = 12;
  const CLUSTER_PX = 64;

  function clusterCellDeg(zoom: number): number {
    return ((CLUSTER_PX / 256) * 360) / Math.pow(2, zoom);
  }

  type PinCluster = { lat: number; lng: number; pins: MapPin[] };

  function clusterPins(pins: MapPin[], zoom: number): PinCluster[] {
    const cell = clusterCellDeg(zoom);
    const groups = new Map<string, MapPin[]>();
    for (const p of pins) {
      const key = `${Math.floor(p.latitude / cell)}:${Math.floor(p.longitude / cell)}`;
      const g = groups.get(key);
      if (g) g.push(p);
      else groups.set(key, [p]);
    }
    return [...groups.values()].map((group) => ({
      lat: group.reduce((s, p) => s + p.latitude, 0) / group.length,
      lng: group.reduce((s, p) => s + p.longitude, 0) / group.length,
      pins: group
    }));
  }

  function loadTileSource(): "osm" | "topo" {
    try {
      if (typeof localStorage === "undefined") return "osm";
      return localStorage.getItem(TILE_SOURCE_KEY) === "topo" ? "topo" : "osm";
    } catch {
      return "osm";
    }
  }

  let tileSource = $state<"osm" | "topo">(loadTileSource());

  function setTileSource(next: "osm" | "topo") {
    if (tileSource === next) return;
    tileSource = next;
    try {
      localStorage.setItem(TILE_SOURCE_KEY, next);
    } catch {
      /* storage unavailable: layer choice just doesn't persist */
    }
    if (tileLayer && mapObj) {
      // Terrain caps at z17: pull back before reloading so tiles exist.
      if (mapObj.getZoom() > TILE_MAX_ZOOM[next]) mapObj.setZoom(TILE_MAX_ZOOM[next]);
      (tileLayer.options as L.GridLayerOptions).maxZoom = TILE_MAX_ZOOM[next];
      tileLayer.redraw();
    }
  }

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
      tileLayer = null;
    }
    if (!mapObj) {
      // preferCanvas: pins redraw per pan/zoom frame — canvas stays smooth
      // where hundreds of SVG circleMarkers jank. minZoom 2 avoids
      // whole-world tile loads on deep zoom-out. attributionControl is off:
      // the single required tile credit lives in the hint line below the map.
      const opts: MapOptions = { attributionControl: false, preferCanvas: true, minZoom: 2 };
      mapObj = L.map(mapEl, opts).setView([20, 0], 2);
      // Test affordance: canvas pins aren't DOM-queryable, so e2e drives a
      // real click through these instead of counting SVG paths.
      (window as unknown as { __ffMap?: LeafletMap }).__ffMap = mapObj;
      tileLayer = new CachedTiles({
        tileSize: 256,
        maxZoom: TILE_MAX_ZOOM[tileSource],
        updateWhenZooming: false, // load once per finished zoom, not mid-animation
        keepBuffer: 2, // prefetch margin; larger values over-request whole worlds at low z
        updateInterval: 200 // fewer mid-pan bursts contending for the 2 download slots
      }).addTo(mapObj);
      // Re-cluster on every finished pan/zoom; renderPins never fits bounds
      // itself, so this can't loop (fitBounds is a one-shot on load).
      mapObj.on("moveend", () => renderPins());
    }
    renderPins(true);
    requestAnimationFrame(() => mapObj?.invalidateSize());
  }

  function addPinMarker(p: MapPin) {
    const m = L.circleMarker([p.latitude, p.longitude], {
      radius: 7,
      color: "#2f6bff",
      weight: 2,
      fillColor: "#2f6bff",
      fillOpacity: 0.65
    });
    m.bindPopup(() => pinPopupEl(p));
    pinLayer?.addLayer(m);
  }

  function renderPins(fit = false) {
    if (!mapObj) return;
    pinLayer?.remove();
    pinLayer = L.layerGroup().addTo(mapObj);
    const zoom = mapObj.getZoom();
    if (zoom < MAP_CLUSTER_BELOW) {
      for (const c of clusterPins(mapPinsRows, zoom)) {
        if (c.pins.length === 1) {
          addPinMarker(c.pins[0]);
        } else {
          const m = L.marker([c.lat, c.lng], {
            title: `${c.pins.length} photos`,
            icon: L.divIcon({
              className: "map-cluster-wrap",
              html: `<div class="map-cluster" data-count="${c.pins.length}">${c.pins.length}</div>`,
              iconSize: [36, 36]
            })
          });
          // One click dissolves toward street level (never past maxZoom).
          m.on("click", () =>
            mapObj?.setView([c.lat, c.lng], Math.min(zoom + 2, mapObj?.getMaxZoom() ?? 18))
          );
          pinLayer.addLayer(m);
        }
      }
    } else {
      for (const p of mapPinsRows) addPinMarker(p);
    }
    if (fit && mapPinsRows.length > 0) {
      mapObj.fitBounds(
        L.latLngBounds(mapPinsRows.map((p) => [p.latitude, p.longitude] as [number, number])),
        { padding: [30, 30] }
      );
    }
    (window as unknown as { __ffMapPins?: [number, number][] }).__ffMapPins = mapPinsRows.map(
      (p) => [p.latitude, p.longitude] as [number, number]
    );
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
    if (albumId == null || eventAlbumBusy !== null) return;
    // Collapsed cards haven't loaded members yet — fetch on demand so ＋
    // Album works without expanding Show photos first.
    let members = eventMembers[eventId];
    if (members === undefined) {
      try {
        members = await eventAssets(eventId);
        eventMembers = { ...eventMembers, [eventId]: members };
      } catch (e) {
        error = String(e).replace(/^Error: /, "");
        return;
      }
    }
    if (members.length === 0) {
      toast = "No readable photos in this event";
      return;
    }
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
      toast = `Removed ${target.root} — ${s.removedAssets} asset${s.removedAssets === 1 ? "" : "s"} removed from the index`;
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
      void loadPersonOptions();
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
      await loadTrips();
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
      await loadTrips();
      toast = `Grouped into ${events.length} event${events.length === 1 ? "" : "s"}`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      eventsLoading = false;
    }
  }

  async function loadTrips() {
    try {
      trips = await tripsList();
      // Default each trip's album target like the per-event dropdowns.
      if (albumOptions.length > 0) {
        const next: Record<string, number | null> = { ...tripAlbumChoice };
        for (const t of trips) {
          if (next[t.id] == null || !albumOptions.some((a) => a.id === next[t.id])) {
            next[t.id] = albumOptions[0].id;
          }
        }
        tripAlbumChoice = next;
      }
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  async function addTripToAlbum(trip: TripRow) {
    const albumId = tripAlbumChoice[trip.id] ?? albumOptions[0]?.id;
    if (albumId == null || tripAlbumBusy !== null) return;
    tripAlbumBusy = trip.id;
    try {
      let total = 0;
      for (const eid of trip.eventIds) {
        let members = eventMembers[eid];
        if (members === undefined) {
          members = await eventAssets(eid);
          eventMembers = { ...eventMembers, [eid]: members };
        }
        if (members.length === 0) continue;
        const r = await albumAdd(albumId, members.map((m) => m.id));
        total += r.added;
      }
      const target = albumOptions.find((a) => a.id === albumId);
      await loadAlbums();
      toast =
        target != null
          ? `Added ${total} photo${total === 1 ? "" : "s"} from “${trip.name}” to “${target.name}”`
          : `Added ${total} photo${total === 1 ? "" : "s"} to the album`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      tripAlbumBusy = null;
    }
  }

  // Trip-scale balanced curation: 24 best across the trip's stops (every
  // day and stop covered, dupes/blur excluded, videos mixed in).
  async function addTripPicks(trip: TripRow) {
    const albumId = tripAlbumChoice[trip.id] ?? albumOptions[0]?.id;
    if (albumId == null || tripAlbumBusy !== null) return;
    tripAlbumBusy = trip.id;
    try {
      const added = await curatePicksToAlbum(trip.eventIds, albumId, 24);
      const target = albumOptions.find((a) => a.id === albumId);
      await loadAlbums();
      toast =
        target != null
          ? `Added ${added} trip pick${added === 1 ? "" : "s"} from “${trip.name}” to “${target.name}”`
          : `Added ${added} trip pick${added === 1 ? "" : "s"} to the album`;
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    } finally {
      tripAlbumBusy = null;
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
      await loadTrips();
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

  // Compare panels (issue #7): per duplicate/similar group, two large panes
  // (keeper left, selected candidate right). Selection + viewer-quality
  // sources live here; keeper swaps reuse chooseKeeper/chooseNearKeeper.
  type CompareGroup = { id: number; keptId: number | null; members: DuplicateMember[] };
  let compareSel = $state<Record<string, number | null>>({});
  let compareBig = $state<Record<string, string>>({});
  let compareErr = $state<Record<string, string>>({});
  let compareBusy = $state<Record<string, boolean>>({});

  function keeperMember(g: CompareGroup): DuplicateMember | undefined {
    return g.members.find((m) => m.id === g.keptId) ?? g.members[0];
  }

  // Keeper score breakdown, mirroring the backend order (favourite, then
  // quality, then earliest capture). Plain span — never role=status (shared
  // with toasts; export assertions scope on it).
  function keeperWhy(g: CompareGroup, keeper: DuplicateMember): string {
    const bits: string[] = [];
    if (keeper.favorite) bits.push("★ favourite");
    const bestQ = Math.max(...g.members.map((m) => m.qualityScore ?? -1));
    if ((keeper.qualityScore ?? -1) >= bestQ && bestQ > 0)
      bits.push(`sharpest (${Math.round(bestQ)})`);
    const earliest = Math.min(
      ...g.members.map((m) => m.capturedAt ?? Number.POSITIVE_INFINITY)
    );
    if ((keeper.capturedAt ?? Number.POSITIVE_INFINITY) <= earliest)
      bits.push("earliest");
    return bits.length > 0 ? `why keeper: ${bits.join(" · ")}` : keeper.name;
  }

  function pickCandidate(g: CompareGroup, key: string): DuplicateMember | undefined {
    const sel = compareSel[key];
    // An explicit pick (even the keeper) wins; otherwise the first
    // non-keeper so the right pane never mirrors the left by default.
    return (
      g.members.find((m) => m.id === sel) ??
      g.members.find((m) => m.id !== g.keptId) ??
      g.members[0]
    );
  }

  function selectCandidate(key: string, id: number) {
    compareSel = { ...compareSel, [key]: id };
  }

  function memberKind(m: Pick<DuplicateMember, "id">): "image" | "video" {
    return assets.find((a) => a.id === m.id)?.kind ?? "image";
  }

  function comparePaneSrc(m: DuplicateMember): string | null {
    // Mock mode (?web=1): engineThumb resolves for any maxPx so the
    // viewer-quality src below normally wins; while it is in flight (or if
    // a viewer-quality fetch ever fails) fall back to the pooled thumbnail
    // so panes still render in tests.
    return compareBig[m.path] ?? thumbs[m.path] ?? null;
  }

  // Viewer-quality source for a compare pane: same pipeline as loadViewer
  // (engineThumb at 1600px for photos, assetUrl for video incl. its
  // loading/error states) — no new engine commands. Narrow param so
  // blurred-asset review items (which are AssetRows, not members) reuse it.
  async function ensureCompareSrc(m: Pick<DuplicateMember, "id" | "path">) {
    if (compareBig[m.path] || compareErr[m.path] || compareBusy[m.path]) return;
    compareBusy[m.path] = true;
    try {
      if (memberKind(m) === "video") {
        const url = assetUrl(m.path);
        // Empty in web mode (bridge.assetUrl): the pane falls back to the
        // thumbnail poster, same as the viewer markup.
        if (url) compareBig[m.path] = url;
        else if (thumbs[m.path]) compareBig[m.path] = thumbs[m.path];
        else throw new Error("no playable file in the web preview");
      } else {
        const { image } = await engineThumb(m.path, 1600);
        if (!image) throw new Error("engine returned no image");
        compareBig[m.path] = thumbSrc(image);
      }
    } catch (e) {
      compareErr[m.path] = String(e).replace(/^Error: /, "");
    } finally {
      compareBusy[m.path] = false;
    }
  }

  function primeCompare(g: CompareGroup, key: string) {
    const k = keeperMember(g);
    if (k) void ensureCompareSrc(k);
    const c = pickCandidate(g, key);
    if (c && c.path !== k?.path) void ensureCompareSrc(c);
  }

  async function swapDuplicateKeeper(groupId: number, keepId: number) {
    await chooseKeeper(groupId, keepId);
    const g = groups.find((x) => x.id === groupId);
    if (g) primeCompare(g, `dup${groupId}`);
  }

  async function swapNearKeeper(groupId: number, keepId: number) {
    await chooseNearKeeper(groupId, keepId);
    const g = nearGroups.find((x) => x.id === groupId);
    if (g) primeCompare(g, `near${groupId}`);
  }

  // Candidate strip keyboard: ←/→ cycles the candidate, guarded to the
  // compare panel that owns the focused strip button so the global
  // viewerKey handler on <svelte:window> keeps every other context.
  function cycleCandidate(g: CompareGroup, key: string, dir: 1 | -1) {
    const ids = g.members.filter((m) => m.id !== g.keptId).map((m) => m.id);
    if (ids.length === 0) return;
    const cur = pickCandidate(g, key)?.id ?? ids[0];
    const at = ids.indexOf(cur);
    const next = ids[(((at < 0 ? 0 : at) + dir) % ids.length + ids.length) % ids.length];
    selectCandidate(key, next);
    const m = g.members.find((x) => x.id === next);
    if (m) void ensureCompareSrc(m);
  }

  function compareStripKey(e: KeyboardEvent, g: CompareGroup, key: string) {
    const panel = (e.currentTarget as HTMLElement | null)?.closest?.("[data-compare]");
    const t = e.target as HTMLElement | null;
    if (!panel || !t || !panel.contains(t)) return;
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    if (t.dataset.stripId === undefined) return;
    e.preventDefault();
    e.stopPropagation();
    const before = pickCandidate(g, key)?.id;
    cycleCandidate(g, key, e.key === "ArrowRight" ? 1 : -1);
    const next = pickCandidate(g, key)?.id;
    if (next === undefined || next === before) return;
    requestAnimationFrame(() => {
      panel.querySelector<HTMLElement>(`[data-strip-id="${next}"]`)?.focus();
    });
  }

  async function applyDeletion() {
    if (!confirm) return;
    const { ids, label } = confirm;
    confirm = null;
    try {
      if (deleteMode === "trash") {
        const result = await trashAssets(ids);
        if (result.failed.length > 0) {
          error = `${result.trashed} moved to Trash, ${result.failed.length} couldn't be moved (${partialNames(result.failed)}) — check the files and try again`;
        }
        lastAction = { kind: "trash", ids, label };
      } else {
        // Flag only: nothing on disk changes, the choice stays reversible.
        for (const id of ids) flagged.add(id);
        flagged = flagged;
        saveFlagged(flagged);
        lastAction = { kind: "flag", ids, label };
      }
      await refreshCleanup();
      // Drop only the acted-on ids so unrelated Library selection survives
      // Cleanup-initiated deletes (which never touch `selected`).
      selected = new Set([...selected].filter((id) => !ids.includes(id)));
      toast = `${label} · ${deleteMode === "trash" ? "moved to Trash" : "flagged only"}`;
      // A bulk review apply shrinks the queue behind it.
      pruneVerdicts();
      if (review && buildReviewItems().length === 0) {
        review = null;
        toast = "Review complete — every decision applied";
      }
    } catch (e) {
      error = String(e).replace(/^Error: /, "");
    }
  }

  // Review mode (triage queue): one cleanup item on screen — duplicates,
  // then similar, then blurred — with staged (not applied) verdicts and a
  // single bulk apply. Keyboard-first so there's no scrolling between
  // decisions; every key has a matching button for mouse users.
  type ReviewItem = { kind: "dup" | "near"; groupId: number } | { kind: "blur"; assetId: number };

  let review = $state<{ pos: number } | null>(null);
  let verdicts = $state<Record<string, { remove: number[] }>>({});

  function buildReviewItems(): ReviewItem[] {
    const items: ReviewItem[] = [];
    for (const g of groups) items.push({ kind: "dup", groupId: g.id });
    for (const g of nearGroups) items.push({ kind: "near", groupId: g.id });
    for (const a of blurred) items.push({ kind: "blur", assetId: a.id });
    return items;
  }

  let reviewItems = $derived(review ? buildReviewItems() : []);

  function reviewKeyOf(item: ReviewItem): string {
    return item.kind === "blur" ? `blur${item.assetId}` : `${item.kind}${item.groupId}`;
  }

  function reviewGroupOf(item: ReviewItem): { g: DuplicateGroup | NearGroup; key: string; kind: "dup" | "near" } | null {
    if (item.kind === "blur") return null;
    const list = item.kind === "dup" ? groups : nearGroups;
    const g = list.find((x) => x.id === item.groupId);
    return g ? { g, key: reviewKeyOf(item), kind: item.kind } : null;
  }

  function currentReviewItem(): ReviewItem | undefined {
    if (!review || reviewItems.length === 0) return undefined;
    return reviewItems[Math.min(review.pos, reviewItems.length - 1)];
  }

  // Non-keeper member ids for a group item, from live data.
  function reviewExtras(item: ReviewItem): number[] {
    const found = reviewGroupOf(item);
    if (!found) return [];
    return found.g.members.filter((m) => m.id !== found.g.keptId).map((m) => m.id);
  }

  function startReview() {
    pruneVerdicts();
    if (buildReviewItems().length === 0) {
      toast = "Nothing to review — run the finders above first";
      return;
    }
    review = { pos: 0 };
  }

  function exitReview() {
    review = null;
  }

  function advanceReview() {
    if (!review) return;
    review.pos = Math.min(review.pos + 1, Math.max(0, reviewItems.length - 1));
  }

  function stageCurrent() {
    const item = currentReviewItem();
    if (!item) return;
    const key = reviewKeyOf(item);
    const remove = item.kind === "blur" ? [item.assetId] : reviewExtras(item);
    if (remove.length === 0) {
      toast = "Nothing to stage here — every copy is the keeper";
      return;
    }
    verdicts = { ...verdicts, [key]: { remove } };
    advanceReview();
  }

  function skipCurrent() {
    const item = currentReviewItem();
    if (!item) return;
    const key = reviewKeyOf(item);
    if (verdicts[key] !== undefined) {
      const next = { ...verdicts };
      delete next[key];
      verdicts = next;
    }
    advanceReview();
  }

  function backReview() {
    if (!review) return;
    review.pos = Math.max(0, review.pos - 1);
  }

  function reviewKeepCandidate() {
    const item = currentReviewItem();
    const found = item ? reviewGroupOf(item) : null;
    if (!found) return;
    const cand = pickCandidate(found.g, found.key);
    if (!cand || cand.id === found.g.keptId) return;
    void (found.kind === "dup"
      ? swapDuplicateKeeper(found.g.id, cand.id)
      : swapNearKeeper(found.g.id, cand.id));
    // Keeper changed: the old staged set is stale, re-decide.
    const key = found.key;
    if (verdicts[key] !== undefined) {
      const next = { ...verdicts };
      delete next[key];
      verdicts = next;
    }
  }

  function cycleInCurrent(dir: 1 | -1) {
    const item = currentReviewItem();
    const found = item ? reviewGroupOf(item) : null;
    if (found) cycleCandidate(found.g, found.key, dir);
  }

  // Drop verdicts whose items vanished; intersect group verdicts with live
  // non-keeper members. Assigns only on change so callers can run it freely.
  function pruneVerdicts() {
    const keep: Record<string, { remove: number[] }> = {};
    for (const [key, v] of Object.entries(verdicts)) {
      if (key.startsWith("blur")) {
        const id = Number(key.slice(4));
        if (blurred.some((a) => a.id === id) && v.remove.includes(id)) keep[key] = { remove: [id] };
        continue;
      }
      const m = key.match(/^(dup|near)(\d+)$/);
      if (!m) continue;
      const list = m[1] === "dup" ? groups : nearGroups;
      const g = list.find((x) => x.id === Number(m[2]));
      if (!g) continue;
      const live = g.members.filter((x) => x.id !== g.keptId).map((x) => x.id);
      const inter = v.remove.filter((id) => live.includes(id));
      if (inter.length > 0) keep[key] = { remove: inter };
    }
    if (JSON.stringify(keep) !== JSON.stringify(verdicts)) verdicts = keep;
    if (review) {
      const len = buildReviewItems().length;
      if (len > 0 && review.pos >= len) review.pos = len - 1;
    }
  }

  function stagedStats(): { decisions: number; photos: number } {
    const vals = Object.values(verdicts);
    return {
      decisions: vals.length,
      photos: vals.reduce((s, v) => s + v.remove.length, 0)
    };
  }

  function clearVerdicts() {
    verdicts = {};
  }

  async function applyStaged() {
    pruneVerdicts();
    const entries = Object.entries(verdicts);
    const ids = [...new Set(entries.flatMap(([, v]) => v.remove))];
    if (ids.length === 0) {
      toast = "Nothing staged — press D on items to stage them for removal";
      return;
    }
    const n = entries.length;
    confirm = {
      ids,
      label: `${ids.length} photo${ids.length === 1 ? "" : "s"} in ${n} decision${n === 1 ? "" : "s"}`
    };
  }

  function reviewKey(e: KeyboardEvent) {
    const item = currentReviewItem();
    if (!item) return;
    // Enter/Space on a focused button keeps its native behavior.
    if ((e.key === "Enter" || e.key === " ") && (e.target as HTMLElement | null)?.tagName === "BUTTON") return;
    const k = e.key;
    if (k === "d" || k === "D" || k === "Enter") {
      e.preventDefault();
      stageCurrent();
    } else if (k === "s" || k === "S") {
      e.preventDefault();
      skipCurrent();
    } else if (k === "b" || k === "B" || k === "Backspace") {
      e.preventDefault();
      backReview();
    } else if (k === "ArrowRight") {
      e.preventDefault();
      cycleInCurrent(1);
    } else if (k === "ArrowLeft") {
      e.preventDefault();
      cycleInCurrent(-1);
    } else if (k === "k" || k === "K") {
      e.preventDefault();
      reviewKeepCandidate();
    } else if (k === "a" || k === "A") {
      e.preventDefault();
      void applyStaged();
    } else if (k === "Escape") {
      e.preventDefault();
      exitReview();
    }
  }

  function reviewTitle(item: ReviewItem | undefined, pos: number, total: number): string {
    if (!item) return "Review complete";
    if (item.kind === "blur") return `Blurred photo ${pos + 1} of ${total}`;
    const found = reviewGroupOf(item);
    const n = found?.g.members.length ?? 0;
    const what = item.kind === "dup" ? "Duplicate" : "Similar";
    return `${what} group ${pos + 1} of ${total} · ${n} photo${n === 1 ? "" : "s"}`;
  }

  function blurAsMember(a: AssetRow): DuplicateMember {
    return {
      id: a.id,
      path: a.path,
      name: a.name,
      isDuplicate: false,
      capturedAt: a.capturedAt,
      qualityScore: null,
      favorite: a.favorite !== 0
    };
  }

  async function undoLast() {
    const action = lastAction;
    if (!action) return;
    lastAction = null;
    try {
      if (action.kind === "flag") {
        for (const id of action.ids) flagged.delete(id);
        flagged = flagged;
        saveFlagged(flagged);
        await refreshCleanup();
        toast = `Undid flag on ${action.label}`;
      } else if (action.kind === "trash") {
        const result = await restoreAssets(action.ids);
        if (result.failed.length > 0) {
          error = `${result.restored} restored, ${result.failed.length} couldn't be restored (${partialNames(result.failed)}) — try again`;
        }
        await refresh();
        await refreshCleanup();
        toast = `Restored ${action.label} from Trash`;
      } else if (action.kind === "unassign") {
        const faceId = action.extra?.faceId;
        const clusterId = action.extra?.clusterId;
        if (faceId == null || clusterId == null) {
          error = "Nothing to undo";
          return;
        }
        await assignFace(faceId, clusterId);
        people = await clustersList();
        const all = await facesInCluster(clusterId);
        const shown = openClusterId === clusterId ? all : all.slice(0, 8);
        peopleFaces = { ...peopleFaces, [clusterId]: shown };
        void loadPersonOptions();
        toast = "Reattached that face — undo complete";
      } else {
        for (const snap of action.extra?.snapshots ?? []) {
          await restoreCluster(snap.name, snap.faceIds);
        }
        people = await clustersList();
        void loadClusterFaces();
        void loadPersonOptions();
        toast = "Split the merge back apart — undo complete";
      }
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
      onIngestProgress((p) => {
        if (ingest) ingest.progress = p;
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
      }),
      onExportProgress((p) => {
        if (p.job === highlightJob || p.job === shareJob) {
          exportProgress = p.phase === "done" ? null : p;
        }
      })
    ];
    void refresh();
    void loadPersonOptions();
    return () => {
      for (const off of offs) void off.then((f) => f());
      thumbObserver?.disconnect();
      thumbObserver = null;
      gridObserver?.disconnect();
      gridObserver = null;
    };
  });
</script>

<svelte:window onkeydown={viewerKey} />
  <main class="container">
  <header>
    <h1>fotoflow</h1>
    <p class="tagline">on-device library management for photo and video</p>
  </header>

  <div class="shell" style="--sidebar-width: {sidebarWidth}px">
    {#if !sidebarHidden}
    <aside class="sidebar" aria-label="Library navigation">
      <div class="sidebar-top">
        <button
          type="button"
          class="ghost sidebar-hide"
          aria-label="Hide sidebar"
          title="Hide sidebar for full-width photos"
          onclick={toggleSidebar}
        >«</button>
      </div>
      <!-- svelte-ignore a11y_interactive_supports_focus: roving-tabindex tablist, only the active tab is a tab stop -->
      <div class="tabs" role="tablist" aria-label="Library sections" onkeydown={tabKey}>
    <button id="tab-library" class="tab" class:on={tab === "library"} role="tab" aria-selected={tab === "library"} tabindex={tab === "library" ? 0 : -1} onclick={() => { tab = "library"; void refresh(); }}>Library</button>
    <button id="tab-cleanup" class="tab" class:on={tab === "cleanup"} role="tab" aria-selected={tab === "cleanup"} tabindex={tab === "cleanup" ? 0 : -1} onclick={() => { tab = "cleanup"; void refreshCleanup(); }}>
      Cleanup{#if cleanup && cleanup.duplicateGroups > 0}&nbsp;{cleanup.duplicateGroups}{/if}
    </button>
    <button id="tab-people" class="tab" class:on={tab === "people"} role="tab" aria-selected={tab === "people"} tabindex={tab === "people" ? 0 : -1} onclick={() => { tab = "people"; void loadPeople(); void loadPets(); }}>
      People{#if people.length > 0}&nbsp;{people.length}{/if}
    </button>
    <button id="tab-events" class="tab" class:on={tab === "events"} role="tab" aria-selected={tab === "events"} tabindex={tab === "events" ? 0 : -1} onclick={() => { tab = "events"; void loadEvents(); }}>
      Events{#if events.length > 0}&nbsp;{events.length}{/if}
    </button>
    <button id="tab-albums" class="tab" class:on={tab === "albums"} role="tab" aria-selected={tab === "albums"} tabindex={tab === "albums" ? 0 : -1} onclick={() => { tab = "albums"; void loadAlbums(); }}>
      Albums{#if albums.length > 0}&nbsp;{albums.length}{/if}
    </button>
    <button id="tab-map" class="tab" class:on={tab === "map"} role="tab" aria-selected={tab === "map"} tabindex={tab === "map" ? 0 : -1} onclick={() => { tab = "map"; void loadMap(); }}>
      Map
    </button>
    <button id="tab-settings" class="tab" class:on={tab === "settings"} role="tab" aria-selected={tab === "settings"} tabindex={tab === "settings" ? 0 : -1} onclick={() => { tab = "settings"; }}>
      Settings
    </button>
      </div>
      <div class="pinned" role="group" aria-label="Pinned">
        <h3 class="sub">Pinned</h3>
        <button type="button" class="tab" onclick={() => void pinFavorites()}>Favorites</button>
        <button type="button" class="tab" onclick={() => void pinRecent()}>Recently Saved</button>
        <button type="button" class="tab" onclick={pinMap}>Map</button>
        <button type="button" class="tab" onclick={() => void pinVideos()}>Videos</button>
        <button type="button" class="tab" onclick={() => void pinScreenshots()}>Screenshots</button>
        <button type="button" class="tab" onclick={pinPeople}>People</button>
        <button type="button" class="tab" onclick={() => void openTrash()}>Recently Deleted</button>
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions: pointer-only drag handle; width persists once set -->
      <div
        class="resize-handle"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize sidebar"
        title="Drag to resize"
        onpointerdown={onResizeStart}
      ></div>
    </aside>
    {/if}
    <div class="content">
      {#if sidebarHidden}
        <button
          type="button"
          class="ghost sidebar-show"
          aria-label="Show sidebar"
          title="Show sidebar"
          onclick={toggleSidebar}
        >» Show sidebar</button>
      {/if}

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
        <button
          onclick={startReview}
          disabled={groups.length + nearGroups.length + blurred.length === 0}
          title="Step through every group one by one, keyboard-first"
        >
          Review all ({groups.length + nearGroups.length + blurred.length})
        </button>
        <button
          onclick={() => void runSpaceSaver()}
          disabled={spaceSaverIds.length === 0 || exportBusy !== null}
          title="Recode duplicate extras + blurred small (1280px) into one folder — originals stay put"
        >
          {exportBusy === "space-saver" ? "Compressing…" : `Compress extras (${spaceSaverIds.length})`}
        </button>
      </div>
      {#if exportResult && exportKey === "space-saver"}
        <p class="hint">
          Compressed {exportResult.exported} to {exportResult.dir} ({fmtBytes(exportResult.bytes)})
          {#if exportResult.skipped > 0}· {exportResult.skipped} skipped{/if}
        </p>
      {/if}
      <p class="hint">Exact duplicates share the same file hash · Similar photos just look alike · Blurred means a low sharpness score — “keeper” is the copy worth keeping.</p>

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

      {#snippet comparePane(m: DuplicateMember)}
        {#if memberKind(m) === "video" && compareBig[m.path]}
          {#key compareBig[m.path]}
            <!-- svelte-ignore a11y_media_has_caption: library files ship no caption tracks -->
            <video src={compareBig[m.path]} controls playsinline></video>
          {/key}
        {:else if memberKind(m) === "video"}
          {#if comparePaneSrc(m)}
            <img src={comparePaneSrc(m)!} alt={m.name} />
          {/if}
          <p class="hint">Video playback needs the desktop app — the web preview has no file to play.</p>
        {:else if comparePaneSrc(m)}
          <img src={comparePaneSrc(m)!} alt={m.name} />
        {:else if compareErr[m.path]}
          <p class="error">{compareErr[m.path]}</p>
        {:else}
          <p class="hint">Loading full size…</p>
        {/if}
      {/snippet}

      {#snippet groupArticle(g: DuplicateGroup | NearGroup, key: string, kind: "dup" | "near")}
        {@const keeper = keeperMember(g)}
        {@const cand = pickCandidate(g, key)}
        {@const extras = g.members.filter((m) => m.id !== g.keptId)}
        <article class="group">
          <header>
            {#if kind === "dup" && "sizeBytes" in g}
              <span>{g.members.length} copies · {fmtBytes(g.sizeBytes)} each</span>
              <span class="waste">keep 1, free {fmtBytes(g.wasteBytes)}</span>
            {:else}
              <span>{g.members.length} similar · keep 1</span>
              <span class="waste">free {fmtBytes(g.wasteBytes)}</span>
            {/if}
          </header>
          {#if keeper && cand}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions: keydown delegates to this panel's strip buttons only; compareStripKey guards to [data-strip-id] focus -->
            <section
              class="compare"
              data-compare={key}
              aria-label={kind === "dup" ? `Compare duplicates in group ${g.id}` : `Compare similar photos in group ${g.id}`}
              onkeydown={(e) => compareStripKey(e, g, key)}
            >
              <div class="compare-panes">
                <figure class="compare-pane" data-pane="keeper">
                  <span class="badge keep">Keeper</span>
                  <span class="keeper-why">{keeperWhy(g, keeper)}</span>
                  <div class="compare-img">
                    {@render comparePane(keeper)}
                  </div>
                </figure>
                <figure class="compare-pane" data-pane="candidate">
                  <span class="candidate-name">{cand.name}</span>
                  <div class="compare-img">
                    {@render comparePane(cand)}
                  </div>
                  <figcaption><span class="name" title={cand.path}>{cand.name}</span></figcaption>
                </figure>
              </div>
              {#if cand.id !== g.keptId}
                {#if kind === "dup"}
                  <button class="tiny" onclick={() => void swapDuplicateKeeper(g.id, cand.id)}>
                    Keep this one instead
                  </button>
                {:else}
                  <button class="tiny" onclick={() => void swapNearKeeper(g.id, cand.id)}>
                    Keep this one instead
                  </button>
                {/if}
              {/if}
            </section>
          {/if}
          <div class="members">
            {#each g.members as member (member.id)}
              <figure class:keeper={member.id === g.keptId}>
                <button
                  type="button"
                  class="thumb small cleanup-preview strip"
                  data-strip-id={member.id}
                  aria-pressed={cand != null && member.id === cand.id}
                  aria-label={`Show ${member.name} in compare`}
                  title={`Show ${member.name} in compare`}
                  onclick={() => {
                    selectCandidate(key, member.id);
                    void ensureCompareSrc(member);
                  }}
                >
                  {#if thumbs[member.path]}
                    <img src={thumbs[member.path]} alt="" loading="lazy" />
                  {:else}
                    <span class="placeholder">IMG</span>
                  {/if}
                </button>
                <figcaption>
                  <span class="name" title={member.path}>{member.name}</span>
                  {#if member.id === g.keptId}
                    <span class="badge keep">keeper</span>
                  {:else if kind === "dup"}
                    <div class="row-actions">
                      <button class="tiny" onclick={() => void chooseKeeper(g.id, member.id)}>
                        Keep this
                      </button>
                      <button class="tiny danger" onclick={() => (confirm = { ids: [member.id], label: member.name })}>
                        Remove
                      </button>
                    </div>
                  {:else}
                    <span class="row-actions">
                      <button class="tiny" onclick={() => void chooseNearKeeper(g.id, member.id)}>
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
          {#if extras.length > 0}
            <button
              class="tiny danger"
              onclick={() => (confirm = { ids: extras.map((m) => m.id), label: `${extras.length} ${kind === "dup" ? "duplicates" : "similar photos"}` })}
            >
              Move {extras.length} extra {extras.length === 1 ? "copy" : "copies"} to Trash
            </button>
          {/if}
        </article>
      {/snippet}

      {#if review}
        {@const items = reviewItems}
        {@const rpos = items.length === 0 ? 0 : Math.min(review.pos, items.length - 1)}
        {@const item = items[rpos]}
        {@const staged = stagedStats()}
        {@const verdict = item ? verdicts[reviewKeyOf(item)] : undefined}
        {@const found = item && item.kind !== "blur" ? reviewGroupOf(item) : null}
        {@const blurAsset = item && item.kind === "blur" ? blurred.find((a) => a.id === item.assetId) : undefined}
        {@const extrasCount = !item ? 0 : item.kind === "blur" ? 1 : reviewExtras(item).length}
        <div class="review" role="region" aria-label="Cleanup review">
          <div class="review-head">
            <span>{reviewTitle(item, rpos, items.length)}</span>
            {#if verdict}
              <span class="chip on">Staged · {verdict.remove.length} photo{verdict.remove.length === 1 ? "" : "s"}</span>
            {/if}
            <button type="button" class="ghost" onclick={exitReview}>Exit review</button>
          </div>
          {#if item?.kind === "blur"}
            {#if blurAsset}
              <article class="group">
                <header>
                  <span>{blurAsset.name}</span>
                  <span class="waste">{blurAsset.width && blurAsset.height ? `${blurAsset.width}×${blurAsset.height} · ` : ""}{fmtBytes(blurAsset.sizeBytes)}</span>
                </header>
                <div class="compare-img blur-review">
                  {@render comparePane(blurAsMember(blurAsset))}
                </div>
              </article>
            {:else}
              <p class="hint">That photo is already decided — press S to move on.</p>
            {/if}
          {:else if found}
            {@render groupArticle(found.g, found.key, found.kind)}
          {:else if item}
            <p class="hint">This group is already decided — press S to move on.</p>
          {:else}
            <p class="hint">Review complete.</p>
          {/if}
          {#if item}
            <div class="review-controls">
              {#if item.kind === "blur"}
                <button type="button" onclick={() => stageCurrent()}>Stage for removal (D)</button>
                <button type="button" class="ghost" onclick={() => skipCurrent()}>Keep · skip (S)</button>
              {:else}
                <button type="button" onclick={() => stageCurrent()}>Stage {extrasCount} extra{extrasCount === 1 ? "" : "s"} (D)</button>
                <button type="button" class="ghost" onclick={() => skipCurrent()}>Skip (S)</button>
                <button type="button" class="ghost" onclick={() => reviewKeepCandidate()}>Keep candidate (K)</button>
              {/if}
              <button type="button" class="ghost" onclick={() => backReview()} disabled={rpos === 0}>Back (B)</button>
            </div>
            <p class="hint">D stage · S skip · ←/→ candidate · K keep candidate · B back · A apply · Esc exit</p>
          {/if}
          {#if staged.decisions > 0}
            <div class="batch-bar" aria-label="Staged cleanup decisions">
              <span>{staged.photos} photo{staged.photos === 1 ? "" : "s"} staged in {staged.decisions} decision{staged.decisions === 1 ? "" : "s"}</span>
              <button type="button" class="ghost" onclick={clearVerdicts}>Clear staged</button>
              <button type="button" class="tiny danger" onclick={() => void applyStaged()}>Apply all (A)</button>
            </div>
          {/if}
        </div>
      {:else}
      {#if groups.length}
        <h3 class="sub">Duplicate groups</h3>
        {#each groups as group (group.id)}
          {@const dupKey = `dup${group.id}`}
          {@render groupArticle(group, dupKey, "dup")}
        {/each}
      {/if}

      {#if nearGroups.length}
        <h3 class="sub">Similar photos</h3>
        <p class="hint">Perceptual matches (resaves, crops, resizes) — review before trashing.</p>
        {#each nearGroups as group (group.id)}
          {@const nearKey = `near${group.id}`}
          {@render groupArticle(group, nearKey, "near")}
        {/each}
      {/if}

      {#if blurred.length}
        <h3 class="sub">Likely blurred</h3>
        <div class="grid">
          {#each blurred as asset (asset.id)}
            <figure>
              <div class="thumb cleanup-preview">
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
      {/if}
    </section>
  {:else if tab === "library"}
  <section class="card" aria-labelledby="library-heading">
    <div class="card-head toolbar">
      <h2 id="library-heading">Library</h2>
      <div class="zoom-toggle seg" role="group" aria-label="Timeline zoom">
        {#each LIB_ZOOMS as z (z)}
          <button
            type="button"
            class="ghost"
            class:on={libZoom === z}
            aria-pressed={libZoom === z}
            aria-label={`Zoom timeline: ${LIB_ZOOM_LABELS[z]}`}
            title={`Zoom timeline: ${LIB_ZOOM_LABELS[z]}`}
            onclick={() => setLibZoom(z)}
          >{LIB_ZOOM_LABELS[z]}</button>
        {/each}
      </div>
      <div class="density-slider" role="group" aria-label="Thumbnail size">
        <span class="hint" aria-hidden="true">Size</span>
        <input
          id="thumb-size"
          type="range"
          min={GRID_MIN_MIN}
          max={GRID_MIN_MAX}
          step="0.5"
          aria-label="Thumbnail size in rem"
          title={`Thumbnail size (${gridMin} rem)`}
          bind:value={gridMin}
          onchange={() => {
            gridMin = clampGridMin(gridMin);
            saveGridMin(gridMin);
          }}
        />
        <button
          type="button"
          class="ghost"
          class:on={gridDetails}
          aria-pressed={gridDetails}
          aria-label="Toggle photo details"
          title="Names, stars and info under photos"
          onclick={() => { gridDetails = !gridDetails; saveGridDetails(gridDetails); }}
        >Details</button>
      </div>
      {#if selectMode}
        <button class="ghost" onclick={() => { selectMode = false; selected = new Set(); }}>Done</button>
      {:else}
        <button class="ghost" onclick={() => void enterSelectMode()}>Select</button>
      {/if}
      <button onclick={chooseFolder} disabled={importing}>
        {importing ? "Importing…" : "Add folder"}
      </button>
      <button
        type="button"
        class="ghost"
        title="Copy a camera card to fast storage, then index the copy"
        onclick={openIngest}
      >Copy to SSD…</button>
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
      {#if activeFilterParts.length > 0}
        <p class="hint">
          Filters: {activeFilterParts.join(" · ")}
          <button class="tiny" onclick={clearAllFilters}>Clear all filters</button>
        </p>
      {/if}

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
        <button class="tiny danger" disabled={selected.size === 0} onclick={batchTrash}>{deleteMode === "trash" ? "Trash" : "Flag"}</button>
        <button class="ghost" disabled={selected.size === 0} onclick={() => (selected = new Set())}>Clear</button>
      </div>
    {/if}

    {#if assets.length}
      {#if zoomDrill}
        <div class="drill-banner" role="status">
          <span>{drillLabel()} · {drilledAssets.length} {drilledAssets.length === 1 ? "photo" : "photos"}</span>
          <button
            type="button"
            class="ghost"
            onclick={drillBack}
          >Back to {zoomDrill.from === "month" ? "months" : "years"}</button>
        </div>
      {/if}
      <div class="grid" data-details={gridDetails} style="--grid-min: {gridMin}rem" use:gridZoomWheel title="Ctrl+scroll to zoom the timeline">
        {#each displayAssets as { asset, index }, di (asset.id)}
          {#if effGroup !== "none" && (di === 0 || groupKeyOf(displayAssets[di - 1].asset) !== groupKeyOf(asset))}
            {@const gkey = groupKeyOf(asset)}
            {@const gids = groupIds(gkey)}
            <h4 class="group-head">
              {#if selectMode}
                <input
                  type="checkbox"
                  class="selbox-inline"
                  aria-label={`Select ${groupLabelOf(asset)}`}
                  checked={gids.length > 0 && gids.every((id) => selected.has(id))}
                  onchange={() => toggleGroup(gkey)}
                />
              {/if}
              {groupLabelOf(asset)} ({groupCounts.get(gkey) ?? 0})
              {#if highlightsMode}
                <button
                  type="button"
                  class="tiny show-all"
                  aria-label={`Show all ${groupLabelOf(asset)} (${groupCounts.get(gkey) ?? 0})`}
                  title="Open this period"
                  onclick={() => drillInto(gkey)}
                >Show all</button>
              {/if}
            </h4>
          {/if}
          <figure class:selected={selected.has(asset.id)}>
            {#if selectMode}
              <input
                type="checkbox"
                class="selbox"
                checked={selected.has(asset.id)}
                aria-label={`Select ${asset.name}`}
                onchange={() => { toggleSelect(asset.id); rangeAnchorId = asset.id; gridFocusId = asset.id; }}
              />
            {/if}
            <button
              type="button"
              class="thumb clickable"
              use:observeThumb={asset.path}
              data-grid-id={asset.id}
              tabindex={gridFocusId === null ? (di === 0 ? 0 : -1) : gridFocusId === asset.id ? 0 : -1}
              onfocus={() => { gridFocusId = asset.id; }}
              title={selectMode ? `Select ${asset.name}` : `Open ${asset.name}`}
              onclick={() => { if (selectMode) toggleSelect(asset.id); else openViewer(index); }}
            >
              {#if thumbs[asset.path]}
                <img src={thumbs[asset.path]} alt={asset.name} loading="lazy" />
              {:else}
                <span class="placeholder">{asset.ext.toUpperCase()}</span>
              {/if}
            </button>
            {#if gridDetails}
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
            {/if}
          </figure>
        {/each}
      </div>
      <p class="hint">← → ↑ ↓ move · Enter opens · S stars{#if selectMode} · Shift+←/→ extends selection · Ctrl/⌘+A selects all · Del removes{/if}</p>
      <div id="grid-sentinel" use:observeGridEnd aria-hidden="true">
        {#if loadingMore}<span class="hint">Loading more…</span>{/if}
      </div>
    {:else if !importing}
      <p class="hint">No assets yet — add a folder to build the index.</p>
    {/if}

    {#if stats && assets.length > 0}
      <div class="enrich">
        <span class="hint">Showing {(zoomDrill ? drilledAssets.length : assets.length)} of {libraryTotal()}</span>
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
      {#if trips.length}
        <h3 class="sub">Trips · {trips.length}</h3>
        {#each trips as trip (trip.id)}
          <article class="group event-card">
            <header>
              <span title={`${trip.stopCount} stops`}>{trip.name}</span>
              <span class="waste">
                {new Date(trip.startedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                – {new Date(trip.endedAt * 1000).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
              </span>
            </header>
            <p class="meta">
              {trip.stopCount} stop{trip.stopCount === 1 ? "" : "s"} ·
              {trip.assetCount} asset{trip.assetCount === 1 ? "" : "s"}{#if trip.place} · {trip.place}{/if}
            </p>
            <p class="story">{trip.summary}</p>
            <p class="meta trip-stats">
              <span class="chip static">{trip.kind === "routine" ? "Routine" : "Away"}</span>
              {#if trip.topPeople.length}
                {trip.topPeople.map((p) => `${p.name} ×${p.faces}`).join(" · ")}
              {/if}
            </p>
            {#if albumOptions.length > 0}
              <div class="row-actions">
                <select
                  aria-label={`Add ${trip.name} to album`}
                  onchange={(e) => {
                    tripAlbumChoice = {
                      ...tripAlbumChoice,
                      [trip.id]: e.currentTarget.value === "" ? null : Number(e.currentTarget.value)
                    };
                  }}
                >
                  {#each albumOptions as a (a.id)}
                    <option
                      value={a.id}
                      selected={(tripAlbumChoice[trip.id] ?? albumOptions[0]?.id) === a.id}
                    >{a.name}</option>
                  {/each}
                </select>
                <button
                  class="tiny"
                  disabled={trip.assetCount === 0 || (tripAlbumChoice[trip.id] ?? albumOptions[0]?.id) == null || tripAlbumBusy !== null}
                  aria-label={`Add trip ${trip.name} to album`}
                  title={`Add trip ${trip.name} to album`}
                  onclick={() => void addTripToAlbum(trip)}
                >
                  {tripAlbumBusy === trip.id ? "Adding…" : "＋ Album"}
                </button>
                <button
                  class="tiny"
                  disabled={trip.assetCount === 0 || (tripAlbumChoice[trip.id] ?? albumOptions[0]?.id) == null || tripAlbumBusy !== null}
                  aria-label={`Curate best 24 from trip ${trip.name}`}
                  title="Balanced best across every day and stop — favorites, named people, videos mixed in"
                  onclick={() => void addTripPicks(trip)}
                >
                  {tripAlbumBusy === trip.id ? "Adding…" : "＋ Best 24"}
                </button>
              </div>
            {/if}
            <div class="row-actions">
              <button
                class="tiny"
                disabled={trip.assetCount === 0}
                aria-label={`Share pack for trip ${trip.name}`}
                title="Memory video + Day-organized best + offline gallery for family"
                onclick={() => openSharePack({ kind: "trip", trip })}
              >＋ Share pack…</button>
            </div>
          </article>
        {/each}
      {/if}
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
                  disabled={event.assetCount === 0 || (eventAlbumChoice[event.id] ?? albumOptions[0]?.id) == null || eventAlbumBusy !== null}
                  onclick={() => void addEventToAlbum(event.id)}
                >
                  {eventAlbumBusy === event.id ? "Adding…" : "＋ Album"}
                </button>
                <button
                  class="tiny"
                  disabled={event.assetCount === 0 || (eventAlbumChoice[event.id] ?? albumOptions[0]?.id) == null || eventAlbumBusy !== null}
                  title="Balanced best across the event — favorites, named people, videos mixed in"
                  onclick={() => void addEventPicks(event.id)}
                >
                  {eventAlbumBusy === event.id ? "Adding…" : "＋ Best 12"}
                </button>
              {/if}
            </div>
            <div class="row-actions">
              <button
                class="tiny"
                disabled={event.assetCount === 0}
                aria-label={`Share pack for ${event.name}`}
                title="Memory video + Day-organized best + offline gallery for family"
                onclick={() => openSharePack({ kind: "event", event })}
              >Share pack…</button>
            </div>
            {#if highlightBusy === `e${event.id}` && exportProgress?.job === highlightJob}
              <p class="hint" role="status">
                {#if (exportProgress?.total ?? 0) > 0}
                  <progress
                    max={exportProgress.total}
                    value={exportProgress.done}
                    aria-label={exportProgress.label}
                  ></progress>
                {/if}
                {exportProgress.label || "Exporting…"}
              </p>
            {/if}
            {#if highlightResult && highlightKey === `e${event.id}`}
              <p class="hint">
                Highlight: {highlightResult.seconds.toFixed(1)}s · {fmtBytes(highlightResult.bytes)}
                · {highlightResult.photos} photo{highlightResult.photos === 1 ? "" : "s"}{#if highlightResult.videosIncluded > 0} · {highlightResult.videosIncluded} video clip{highlightResult.videosIncluded === 1 ? "" : "s"}{/if}
                {#if highlightResult.videosSkipped > 0}· {highlightResult.videosSkipped} unreadable skipped{/if}
                <br />{highlightResult.output}
              </p>
              <div class="row-actions">
                <button
                  type="button"
                  class="tiny"
                  title="Watch the highlight video"
                  aria-label="Watch highlight video"
                  onclick={watchHighlight}
                >▶ Watch</button>
              </div>
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
              <button
                class="tiny"
                disabled={album.assetCount === 0}
                aria-label={`Share pack for album ${album.name}`}
                title="Memory video + Day-organized best + offline gallery for family"
                onclick={() => openSharePack({ kind: "album", album })}
              >Share pack…</button>
              <button class="tiny danger" onclick={() => void deleteAlbum(album)}>Delete album</button>
            </div>
            {#if highlightBusy === `a${album.id}` && exportProgress?.job === highlightJob}
              <p class="hint" role="status">
                {#if (exportProgress?.total ?? 0) > 0}
                  <progress
                    max={exportProgress.total}
                    value={exportProgress.done}
                    aria-label={exportProgress.label}
                  ></progress>
                {/if}
                {exportProgress.label || "Exporting…"}
              </p>
            {/if}
            {#if highlightResult && highlightKey === `a${album.id}`}
              <p class="hint">
                Highlight: {highlightResult.seconds.toFixed(1)}s · {fmtBytes(highlightResult.bytes)}
                · {highlightResult.photos} photo{highlightResult.photos === 1 ? "" : "s"}{#if highlightResult.videosIncluded > 0} · {highlightResult.videosIncluded} video clip{highlightResult.videosIncluded === 1 ? "" : "s"}{/if}
                {#if highlightResult.videosSkipped > 0}· {highlightResult.videosSkipped} unreadable skipped{/if}
                <br />{highlightResult.output}
              </p>
              <div class="row-actions">
                <button
                  type="button"
                  class="tiny"
                  title="Watch the highlight video"
                  aria-label="Watch highlight video"
                  onclick={watchHighlight}
                >▶ Watch</button>
              </div>
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
      <div class="seg" role="group" aria-label="Map layer">
        <button
          class="ghost"
          class:on={tileSource === "osm"}
          aria-pressed={tileSource === "osm"}
          onclick={() => setTileSource("osm")}
        >Streets</button>
        <button
          class="ghost"
          class:on={tileSource === "topo"}
          aria-pressed={tileSource === "topo"}
          onclick={() => setTileSource("topo")}
        >Terrain</button>
      </div>
    </div>
    {#if mapLoading}
      <p class="hint">Loading pins…</p>
    {:else if mapError}
      <p class="error">{mapError}</p>
    {:else if mapPinsRows.length === 0}
      <p class="hint">No geotagged photos yet — photos with GPS land here after Refresh metadata.</p>
    {/if}
    <div class="map-wrap" bind:this={mapEl} role="application" aria-label="Photo map"></div>
    <p class="hint">{tileSource === "topo" ? "Topo tiles © OpenStreetMap contributors, SRTM · style © OpenTopoMap (CC-BY-SA)" : "Tiles © OpenStreetMap contributors"} · cached on this device · select a pin, then “Show in Library”.</p>
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
    </div><!-- /.content -->
  </div><!-- /.shell -->

  {#if trashOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scrim" onclick={(e) => { if (e.target === e.currentTarget) trashOpen = false; }}>
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="trash-heading" tabindex={-1} use:dialogFocus={() => (trashOpen = false)}>
        <h3 id="trash-heading">Recently Deleted</h3>
        <p class="hint">Photos trashed from anywhere in the app. Restoring puts the file back where it was — unless the macOS Trash was emptied since.</p>
        {#if trashLoading}
          <p class="hint">Loading…</p>
        {:else if trashRows.length === 0}
          <p class="hint">Nothing here — trashed photos wait here until you clear the list.</p>
        {:else}
          <ul class="trash-list">
            {#each trashRows as t (t.assetId)}
              <li>
                <span class="name" title={t.path}>{t.name}</span>
                <span class="hint">{fmtDate(t.trashedAt)}</span>
                <button type="button" class="tiny" onclick={() => void restoreTrashRow(t)}>Restore</button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="dialog-actions">
          <button class="ghost" onclick={() => (trashOpen = false)}>Close</button>
          {#if trashRows.length > 0}
            <button class="ghost" onclick={() => void clearTrash()}>Clear list</button>
            <button disabled={trashLoading} onclick={() => void restoreTrashAll()}>Restore all</button>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if ingest}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scrim" onclick={(e) => { if (e.target === e.currentTarget && !ingest?.running) ingest = null; }}>
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="ingest-heading" tabindex={-1} use:dialogFocus={() => { if (!ingest?.running) ingest = null; }}>
        <h3 id="ingest-heading">Copy photos to fast storage</h3>
        <p class="hint">Mirrors a camera card or device folder to an SSD, skipping files that are already there. Index the copy afterwards with one click.</p>
        <div class="path-entry">
          <input
            type="text"
            placeholder="Copy from… (camera card)"
            aria-label="Copy photos from"
            bind:value={ingest.src}
            disabled={ingest.running}
          />
          <button type="button" class="ghost" disabled={ingest.running} onclick={() => void pickIngestDir("src")}>Browse…</button>
        </div>
        <div class="path-entry">
          <input
            type="text"
            placeholder="Copy to… (SSD folder)"
            aria-label="Copy photos to"
            bind:value={ingest.dest}
            disabled={ingest.running}
          />
          <button type="button" class="ghost" disabled={ingest.running} onclick={() => void pickIngestDir("dest")}>Browse…</button>
        </div>
        {#if ingest.progress}
          {@const p = ingest.progress}
          <progress max={Math.max(1, p.total)} value={p.copied + p.skipped}></progress>
          <p class="hint">
            {p.copied + p.skipped} of {p.total} · {fmtBytes(p.bytes)} of {fmtBytes(p.totalBytes)}{#if p.failed.length > 0} · {p.failed.length} failed{/if}
          </p>
        {/if}
        {#if ingest.result}
          {@const r = ingest.result}
          <p class="hint">
            Copied {r.copied} · skipped {r.skipped} identical · {fmtBytes(r.bytes)}{#if r.failed.length > 0} · {r.failed.length} failed: {r.failed.slice(0, 3).join("; ")}{/if}
          </p>
        {/if}
        <div class="dialog-actions">
          {#if ingest.result}
            <button class="ghost" onclick={() => (ingest = null)}>Close</button>
            <button onclick={() => { const d = ingest?.result?.dest; ingest = null; if (d) void runImport(d); }}>Index this folder</button>
          {:else}
            <button class="ghost" disabled={ingest.running} onclick={() => (ingest = null)}>Cancel</button>
            <button disabled={ingest.running} onclick={() => void runIngest()}>
              {ingest.running || ingest.progress ? "Copying…" : "Start copy"}
            </button>
          {/if}
        </div>
      </div>
    </div>
  {/if}

  {#if sharePack}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scrim" onclick={(e) => { if (e.target === e.currentTarget && !shareBusy) sharePack = null; }}>
      <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="share-heading" tabindex={-1} use:dialogFocus={() => { if (!shareBusy) sharePack = null; }}>
        <h3 id="share-heading">Share “{shareTitle()}” with family</h3>
        {#if shareResult}
          {@const r = shareResult}
          <p class="hint">
            Packed {r.exported} photo{r.exported === 1 ? "" : "s"}{#if r.skipped > 0} · {r.skipped} skipped{/if}
            ({fmtBytes(r.bytes)}) to {r.dir} — memory video plus Day folders plus an offline gallery page.
          </p>
          <div class="dialog-actions">
            <button class="ghost" onclick={() => (sharePack = null)}>Close</button>
            <button
              class="ghost"
              title="Show the pack folder in Finder"
              onclick={() => void revealExportDir(r.dir)}
            >Reveal in Finder</button>
            <button
              class="ghost"
              title="Open the offline gallery page in a browser"
              onclick={() => void openPackGallery(r.dir)}
            >Open gallery</button>
            <button
              onclick={() => {
                const v = r.video;
                sharePack = null;
                watchPath(v, v.split("/").pop() ?? v, r.bytes, null);
              }}
            >▶ Watch pack video</button>
          </div>
        {:else}
          <p class="hint">Memory video + Day-organized best picks + an offline gallery page friends can open in any browser.</p>
          <div class="seg" role="group" aria-label="Pack quality">
            <button
              type="button"
              class="ghost"
              class:on={shareMode === "compact"}
              aria-pressed={shareMode === "compact"}
              onclick={() => (shareMode = "compact")}
            >Chat-size</button>
            <button
              type="button"
              class="ghost"
              class:on={shareMode === "drive"}
              aria-pressed={shareMode === "drive"}
              onclick={() => (shareMode = "drive")}
            >Full quality</button>
          </div>
          <label class="mode">
            <input type="checkbox" bind:checked={shareStrip} />
            Strip GPS location{#if shareMode === "compact"} (chat-size copies carry no metadata anyway){/if}
          </label>
          <div class="path-entry">
            <input
              type="text"
              placeholder="Save pack to… (folder)"
              aria-label="Share pack destination"
              bind:value={shareDest}
              disabled={shareBusy}
            />
            <button type="button" class="ghost" disabled={shareBusy} onclick={() => void pickShareDest()}>Browse…</button>
          </div>
          <div class="dialog-actions">
            <button class="ghost" disabled={shareBusy} onclick={() => (sharePack = null)}>Cancel</button>
            <button disabled={shareBusy || shareDest.trim() === ""} onclick={() => void runSharePack()}>
              {shareBusy ? "Packing…" : "Build share pack"}
            </button>
          </div>
          {#if shareBusy && exportProgress?.job === shareJob}
            <p class="hint" role="status">
              {#if (exportProgress?.total ?? 0) > 0}
                <progress
                  max={exportProgress.total}
                  value={exportProgress.done}
                  aria-label={exportProgress.label}
                ></progress>
              {/if}
              {exportProgress.label || "Packing…"}
            </p>
          {/if}
        {/if}
      </div>
    </div>
  {/if}

  {#if confirm}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="scrim" onclick={(e) => { if (e.target === e.currentTarget) confirm = null; }}>
      <div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-heading" tabindex={-1} use:dialogFocus={() => (confirm = null)}>
        <h3 id="confirm-heading">{deleteMode === "trash" ? `Move ${confirm.label} to Trash?` : `Flag ${confirm.label}?`}</h3>
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
    <!-- svelte-ignore a11y_click_events_have_key_events: scrim click is a pointer affordance; Esc cancels via dialogFocus -->
    <div class="scrim" role="dialog" aria-modal="true" aria-labelledby="picker-heading" tabindex={-1} onclick={(e) => { if (e.target === e.currentTarget) { picker = null; resetThumbs(); } }}>
      <div class="dialog wide" use:dialogFocus={() => { picker = null; resetThumbs(); }}>
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

  {#snippet infoPanel(a: AssetRow)}
    {@const cam = cameraLabel(a)}
    {@const expo = exposureParts(a)}
    {@const loc = [a.place, gpsLabel(a)].filter((x): x is string => !!x).join(" · ")}
    <dl class="info" aria-label="Photo info">
      {#if cam}<div><dt>Camera</dt><dd>{cam}</dd></div>{/if}
      {#if a.lens}<div><dt>Lens</dt><dd>{a.lens}</dd></div>{/if}
      {#if expo.length > 0}<div><dt>Exposure</dt><dd>{expo.join(" · ")}</dd></div>{/if}
      {#if loc}<div><dt>Location</dt><dd>{loc}</dd></div>{/if}
      {#if a.width && a.height}
        <div><dt>Dimensions</dt><dd>{a.width}×{a.height}{a.kind === "video" && a.duration ? ` · ${trimNum(a.duration)} s` : ""}</dd></div>
      {/if}
      <div><dt>Size</dt><dd>{fmtBytes(a.sizeBytes)}</dd></div>
      <div><dt>Format</dt><dd>{a.ext.toUpperCase()}{a.mime ? ` · ${a.mime}` : ""}</dd></div>
      {#if a.capturedAt}<div><dt>Taken</dt><dd>{fmtDate(a.capturedAt)}</dd></div>{/if}
    </dl>
  {/snippet}

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
            <button
              type="button"
              class="tiny"
              aria-pressed={infoOpen}
              aria-label="Photo info"
              title="Camera, location and file info (I)"
              onclick={() => (infoOpen = !infoOpen)}
            >ⓘ</button>
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
          {#if infoOpen}
            {@render infoPanel(current)}
          {/if}
          <p class="hint">← → move · esc closes · S stars · I shows info</p>
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
          <button class="tiny" onclick={() => zoomShow(-1)} disabled={showZoom <= 1} aria-label="Zoom out">−</button>
          <button class="tiny" onclick={resetShowZoom} disabled={showZoom === 1} aria-label="Reset zoom">{Math.round(showZoom * 100)}%</button>
          <button class="tiny" onclick={() => zoomShow(1)} disabled={showZoom >= 5} aria-label="Zoom in">+</button>
          <button
            class="tiny"
            aria-pressed={infoOpen}
            aria-label="Photo info"
            title="Camera, location and file info (I)"
            onclick={() => (infoOpen = !infoOpen)}
          >ⓘ</button>
          <button class="tiny" onclick={closeShow} aria-label="Close slideshow">✕</button>
        </div>
      </div>
      <div class="theater-stage" use:zoomWheel>
        {#if sc.kind === "video" && show.src}
          {#key show.src}
            <!-- svelte-ignore a11y_media_has_caption: library files ship no caption tracks -->
            <video
              src={show.src}
              controls
              autoplay
              playsinline
              style={showMediaStyle()}
              onpointerdown={panStart}
            ></video>
          {/key}
        {:else if sc.kind === "video"}
          {#if thumbs[sc.path]}
            <img src={thumbs[sc.path]} alt={sc.name} />
          {/if}
          <p class="hint">Video playback needs the desktop app — the web preview has no file to play.</p>
        {:else if show.src}
          {#key show.src}
            <img
              src={show.src}
              alt={sc.name}
              draggable="false"
              style={showMediaStyle()}
              onpointerdown={panStart}
            />
          {/key}
        {:else if show.error}
          <p class="error">{show.error}</p>
        {:else}
          <p class="hint">Loading full size…</p>
        {/if}
      </div>
      {#if infoOpen}
        <div class="theater-info">
          {@render infoPanel(sc)}
        </div>
      {/if}
      <p class="hint theater-hint">‹ › move · +/− zoom · 0 reset · space plays/pauses · esc closes</p>
    </div>
  {/if}

  {#if similar}
    <!-- svelte-ignore a11y_click_events_have_key_events: scrim click is a pointer affordance; Esc cancels via dialogFocus -->
    <div
      class="scrim"
      role="dialog"
      aria-modal="true"
      aria-label={`Similar to ${similar.name}`}
      tabindex={-1}
      onclick={(e) => { if (e.target === e.currentTarget) similar = null; }}
    >
      <div class="dialog wide" use:dialogFocus={() => (similar = null)}>
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
    <!-- svelte-ignore a11y_click_events_have_key_events: scrim click is a pointer affordance; Esc cancels via dialogFocus -->
    <div class="scrim" role="dialog" aria-modal="true" aria-label={nameDialog.title} tabindex={-1} onclick={(e) => { if (e.target === e.currentTarget) nameDialog = null; }}>
      <form
        class="dialog"
        use:dialogFocus={() => (nameDialog = null)}
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
    <!-- svelte-ignore a11y_click_events_have_key_events: scrim click is a pointer affordance; Esc cancels via dialogFocus -->
    <div class="scrim" role="dialog" aria-modal="true" aria-label="Delete album" tabindex={-1} onclick={(e) => { if (e.target === e.currentTarget) deleteAlbumTarget = null; }}>
      <div class="dialog" use:dialogFocus={() => (deleteAlbumTarget = null)}>
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
    <!-- svelte-ignore a11y_click_events_have_key_events: scrim click is a pointer affordance; Esc cancels via dialogFocus -->
    <div class="scrim" role="dialog" aria-modal="true" aria-label="Remove indexed folder" tabindex={-1} onclick={(e) => { if (e.target === e.currentTarget) removeRootTarget = null; }}>
      <div class="dialog" use:dialogFocus={() => (removeRootTarget = null)}>
        <h3>Remove “{removeRootTarget.root}”?</h3>
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
    <p class="toast" role="status">
      {toast}
      {#if lastAction}
        <button class="ghost" onclick={() => void undoLast()}>Undo</button>
      {/if}
    </p>
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
    padding: 1rem clamp(0.5rem, 2vw, 1.5rem) 2rem;
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
    padding: 0.75rem;
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
    z-index: 40;
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
    /* Viewport-filling and full-bleed inside the card: the toolbar and
       credit keep the card frame, the tiles take everything else. */
    height: calc(100dvh - 16rem);
    min-height: 22rem;
    margin: 0 -0.75rem -0.75rem;
    border: 0;
    border-top: 1px solid #232733;
    border-radius: 0 0 12px 12px;
    overflow: hidden;
    background: #e5e3df;
  }
  .map-wrap :global(.leaflet-container) {
    height: 100%;
    background: #e5e3df;
    font: inherit;
  }
  /* Cluster badges: plain divIcon wrapper (no white square), count bubble. */
  .map-wrap :global(.map-cluster-wrap) {
    background: transparent;
    border: 0;
  }
  .map-wrap :global(.map-cluster) {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    background: rgba(47, 107, 255, 0.88);
    border: 2px solid #fff;
    color: #fff;
    font-size: 0.8rem;
    font-weight: 700;
    display: grid;
    place-items: center;
    cursor: pointer;
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
    overflow: hidden;
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
  .thumb.clickable:focus-visible {
    outline: 2px solid #2f6bff;
    outline-offset: 2px;
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
    z-index: 60;
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
    grid-template-columns: repeat(auto-fit, minmax(min(var(--grid-min, 8.5rem), 100%), 1fr));
    gap: 3px;
    margin-top: 0.6rem;
  }
  /* Infinite-scroll sentinel: a zero-height div never re-intersects once it
     rests inside the viewport, silently stalling paging (needs two opens to
     notice). A real hit area plus a loading hint keeps it transitioning and
     visible while the next chunk is in flight. */
  #grid-sentinel {
    min-height: 3rem;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .grid .thumb {
    border-radius: 5px;
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
    margin: 0.6rem 0 0.25rem;
  }
  .group-head:first-child {
    margin-top: 0;
  }
  .group-head .show-all {
    margin-left: auto;
  }
  /* Review mode: one item on screen, progress head, decision controls,
     staged bulk bar. Reuses .group/.compare/.batch-bar wholesale. */
  .review {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .review-head {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-size: 0.9rem;
    font-weight: 600;
    color: #cfd5e1;
  }
  .review-head button {
    margin-left: auto;
  }
  .review-controls {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.5rem;
  }
  .compare-img.blur-review {
    max-width: 42rem;
  }
  /* Drill-down banner: period label + count left, Back pushed right. */
  .drill-banner {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.85rem;
    font-weight: 600;
    color: #cfd5e1;
    margin: 0.2rem 0 0.4rem;
  }
  .drill-banner button {
    margin-left: auto;
  }
  /* Photo info panel: label/value rows; rows without data are omitted. */
  .info {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    margin: 0.5rem 0 0;
    padding: 0.6rem 0.8rem;
    background: #0d0f14;
    border: 1px solid #232733;
    border-radius: 8px;
    font-size: 0.78rem;
  }
  .info > div {
    display: flex;
    gap: 0.8rem;
  }
  .info dt {
    flex: 0 0 5.5rem;
    color: #8a93a5;
  }
  .info dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  /* Theater info floats centered under the stage. */
  .theater-info {
    align-self: center;
    width: min(28rem, 92%);
  }
  /* Recently Deleted rows: name + date left, Restore right. */
  .trash-list {
    list-style: none;
    margin: 0.5rem 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    max-height: 16rem;
    overflow: auto;
  }
  .trash-list li {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .trash-list .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  /* Frozen layout shell: sidebar nav rail + fluid content center. */
  .shell {
    display: flex;
    gap: 1rem;
    align-items: flex-start;
  }
  .sidebar {
    position: sticky;
    top: 1rem;
    width: var(--sidebar-width, 248px);
    flex: 0 0 auto;
    background: #151820;
    border: 1px solid #232733;
    border-radius: 12px;
    padding: 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    max-height: calc(100vh - 2rem);
    overflow: auto;
  }
  .sidebar .tabs {
    flex-direction: column;
    align-items: stretch;
    margin-bottom: 0;
  }
  .sidebar .tab {
    width: 100%;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Sidebar top row: hide toggle pushed right; show toggle sits atop
     the content when the sidebar is hidden. */
  .sidebar-top {
    display: flex;
    justify-content: flex-end;
  }
  .sidebar-hide {
    padding: 0.2rem 0.5rem;
  }
  .sidebar-show {
    margin-bottom: 0.6rem;
  }
  /* Pinned shortcuts: same full-width buttons as tabs, tighter stack. */
  .sidebar .pinned {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.15rem;
  }
  .sidebar .pinned .sub {
    margin: 0.3rem 0.2rem 0.15rem;
  }
  .resize-handle {
    position: absolute;
    top: 0;
    right: -5px;
    width: 10px;
    height: 100%;
    cursor: col-resize;
    touch-action: none;
  }
  .sidebar {
    position: sticky;
  }
  .content {
    flex: 1;
    min-width: 0;
  }
  /* Sticky 48–56px toolbar per tab: existing card-head blocks. */
  .card-head {
    position: sticky;
    top: 0;
    z-index: 5;
    background: #151820;
    min-height: 48px;
    align-items: center;
    padding-top: 0.4rem;
    padding-bottom: 0.4rem;
  }
  .card-head h2 {
    margin-right: auto;
  }
  .density-slider, .zoom-toggle {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }
  .density-slider input[type="range"] {
    width: 6rem;
    accent-color: #2f6bff;
  }
  /* Segmented two-option group (map layer toggle): ghost buttons with a
     visible active state, mirroring .chip.on. */
  .seg {
    display: flex;
    gap: 0.3rem;
  }
  .seg button.ghost.on {
    background: #2f6bff;
    border-color: #2f6bff;
    color: #fff;
  }
  /* Sticky filter rail beneath the toolbar. */
  .filters {
    position: sticky;
    top: 56px;
    z-index: 4;
    background: #151820;
    padding-top: 0.3rem;
    padding-bottom: 0.3rem;
  }
  /* Single sticky bottom batch bar: N selected left, ghost middle,
     Trash/Flag + Clear pushed far right. */
  .batch-bar {
    position: sticky;
    bottom: 0;
    z-index: 6;
    background: #10131a;
    border: 1px solid #232733;
    border-radius: 10px;
    padding: 0.5rem 0.6rem;
  }
  .batch-bar .tiny.danger {
    margin-left: auto;
  }
  /* Cleanup group footer spans the full card width. */
  .group > button.tiny.danger {
    width: 100%;
    margin-top: 0.4rem;
  }
  /* Issue #7: one shared preview size for every cleanup figure. Duplicates
     and similar render in .members (6rem tiles) while blurred renders in
     .grid (8.5rem tiles); the fixed clamp height normalizes the oversized
     blurred preview down/up to match, with a responsive minimum. */
  .thumb.cleanup-preview {
    width: 100%;
    height: clamp(7rem, 18vw, 10rem);
    min-height: 7rem;
    aspect-ratio: auto;
  }
  .thumb.cleanup-preview img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  button.strip {
    cursor: pointer;
    border: 1px solid transparent;
  }
  button.strip[aria-pressed="true"] {
    outline: 2px solid #2f6bff;
    outline-offset: 1px;
  }
  button.strip:focus-visible {
    outline: 2px solid #2f6bff;
    outline-offset: 2px;
  }
  /* Issue #7 compare panel: two large panes (keeper left, candidate
     right) plus the member strip of small thumbs that picks the candidate. */
  .compare {
    border: 1px solid #232733;
    border-radius: 10px;
    padding: 0.6rem;
    margin-bottom: 0.6rem;
    background: #10131a;
  }
  .compare-panes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.6rem;
    margin-bottom: 0.5rem;
  }
  @media (max-width: 560px) {
    .compare-panes {
      grid-template-columns: 1fr;
    }
  }
  .compare-pane {
    margin: 0;
    min-width: 0;
  }
  .compare-pane .badge {
    margin-bottom: 0.3rem;
  }
  .candidate-name {
    display: inline-block;
    font-size: 0.75rem;
    font-weight: 600;
    color: #e7eaf0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
    margin-bottom: 0.3rem;
  }
  .compare-img {
    height: clamp(10rem, 28vw, 18rem);
    min-height: 10rem;
    background: #000;
    border-radius: 8px;
    overflow: hidden;
    display: grid;
    place-items: center;
  }
  .compare-img img,
  .compare-img video {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .compare-img .hint,
  .compare-img .error {
    margin: 0.5rem;
    text-align: center;
  }
  .compare > button.tiny {
    margin-bottom: 0.5rem;
  }
</style>
