import type {
  AlbumRow,
  AssetRow,
  EventRow,
  FaceClusterRow,
  LibraryFilters,
  LibraryMonth,
  LibraryStats,
  ScanRoot,
  SceneTagCount,
  TagScenesSummary
} from "./library";

/**
 * Localhost read API client: same function surface as the Tauri-backed
 * reads in library.ts, backed by the Rust HTTP server (see
 * src-tauri/src/server.rs `router()`). Selected by bridge.ts when
 * `isApiMode()` is true.
 *
 * Base URL comes from `VITE_FOTOFLOW_API` or the `?api=` query param
 * (query wins when both are set); null when neither is set.
 */

export function apiBase(): string | null {
  if (typeof window !== "undefined") {
    try {
      const q = new URLSearchParams(window.location.search).get("api")?.trim();
      if (q) return q.replace(/\/+$/, "");
    } catch {
      /* URL parsing unavailable — fall through to the env check */
    }
  }
  try {
    const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env;
    const fromEnv = env?.["VITE_FOTOFLOW_API"]?.trim();
    if (fromEnv) return fromEnv.replace(/\/+$/, "");
  } catch {
    /* import.meta unavailable — no API base */
  }
  return null;
}

export function isApiMode(): boolean {
  return apiBase() !== null;
}

function base(): string {
  const b = apiBase();
  if (!b) throw new Error("API base is not configured (set ?api= or VITE_FOTOFLOW_API)");
  return b;
}

async function getJson<T>(path: string, params?: Record<string, string | number | boolean>): Promise<T> {
  const url = new URL(`${base()}${path}`);
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      url.searchParams.set(k, String(v));
    }
  }
  const res = await fetch(url.toString());
  if (!res.ok) {
    let detail: string | null = null;
    try {
      const body = (await res.json()) as { error?: unknown };
      if (typeof body?.error === "string" && body.error.trim() !== "") detail = body.error;
    } catch {
      /* non-JSON error body — fall back to status text */
    }
    throw new Error(detail ?? `GET ${path} failed: ${res.status} ${res.statusText}`);
  }
  return (await res.json()) as T;
}

function libraryParams(opts: LibraryFilters): Record<string, string | number | boolean> {
  const out: Record<string, string | number | boolean> = {};
  if (opts.kind !== undefined) out["kind"] = opts.kind;
  if (opts.offset !== undefined) out["offset"] = opts.offset;
  if (opts.limit !== undefined) out["limit"] = opts.limit;
  if (opts.personCluster !== undefined) out["personCluster"] = opts.personCluster;
  if (opts.hasLocation !== undefined) out["hasLocation"] = opts.hasLocation;
  if (opts.capturedMin !== undefined) out["capturedMin"] = opts.capturedMin;
  if (opts.capturedMax !== undefined) out["capturedMax"] = opts.capturedMax;
  if (opts.favoriteOnly !== undefined) out["favoriteOnly"] = opts.favoriteOnly;
  if (opts.sceneTag !== undefined) out["sceneTag"] = opts.sceneTag;
  if (opts.hasPet !== undefined) out["hasPet"] = opts.hasPet;
  const q = opts.query?.trim();
  if (q) out["query"] = q;
  return out;
}

export function libraryList(opts: LibraryFilters = {}): Promise<AssetRow[]> {
  return getJson<AssetRow[]>("/api/library", libraryParams(opts));
}

export async function libraryCount(
  opts: Omit<LibraryFilters, "offset" | "limit"> = {}
): Promise<number> {
  const { count } = await getJson<{ count: number }>("/api/library/count", libraryParams(opts));
  return count;
}

export function libraryMonths(): Promise<LibraryMonth[]> {
  return getJson<LibraryMonth[]>("/api/library/months");
}

export function libraryStats(): Promise<LibraryStats> {
  return getJson<LibraryStats>("/api/stats");
}

export function scanRoots(): Promise<ScanRoot[]> {
  return getJson<ScanRoot[]>("/api/roots");
}

export function assetsInRoot(root: string): Promise<number[]> {
  return getJson<number[]>(`/api/root-assets?root=${encodeURIComponent(root)}`);
}

export function clustersList(): Promise<FaceClusterRow[]> {
  return getJson<FaceClusterRow[]>("/api/clusters");
}

export function eventsList(): Promise<EventRow[]> {
  return getJson<EventRow[]>("/api/events");
}

export function albumsList(): Promise<AlbumRow[]> {
  return getJson<AlbumRow[]>("/api/albums");
}

export function sceneTags(): Promise<SceneTagCount[]> {
  return getJson<SceneTagCount[]>("/api/tags");
}

/**
 * Write slice: same names/signatures as the Tauri-backed writes in
 * library.ts, served by the Rust HTTP server (see src-tauri/src/server.rs).
 * The server's tag run is synchronous with no progress stream — it
 * resolves with the final summary directly.
 */

async function postJson<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${base()}${path}`, {
    method: "POST",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body)
  });
  if (!res.ok) {
    let detail: string | null = null;
    try {
      const parsed = (await res.json()) as { error?: unknown };
      if (typeof parsed?.error === "string" && parsed.error.trim() !== "") detail = parsed.error;
    } catch {
      /* non-JSON error body — fall back to status text */
    }
    throw new Error(detail ?? `POST ${path} failed: ${res.status} ${res.statusText}`);
  }
  return (await res.json()) as T;
}

export function setFavorite(assetId: number, favorite: boolean): Promise<void> {
  return postJson<void>("/api/favorites", { id: assetId, favorite });
}

export async function albumCreate(name: string): Promise<number> {
  const { id } = await postJson<{ id: number }>("/api/albums", { name });
  return id;
}

export function albumAdd(albumId: number, assetIds: number[]): Promise<{ added: number }> {
  return postJson<{ added: number }>(`/api/albums/${albumId}/add`, { assetIds });
}

export function tagScenes(): Promise<TagScenesSummary> {
  return postJson<TagScenesSummary>("/api/tags/run");
}
