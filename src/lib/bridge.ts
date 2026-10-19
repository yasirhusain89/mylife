import * as realLibrary from "./library";
import * as realEngine from "./engine";
import { open as tauriOpen } from "@tauri-apps/plugin-dialog";
import * as mock from "./mock";

/**
 * Web-mode selector: `?web=1` ( Playwright) or VITE_FOTOFLOW_WEB=1 serves
 * the in-memory mock; the Tauri app window keeps the real IPC bridge.
 * Default stays Tauri so plain `vite dev` tabs behave exactly as before.
 */
export function isWebMode(): boolean {
  try {
    const env = (import.meta as unknown as { env?: Record<string, string | undefined> }).env;
    if (env?.["VITE_FOTOFLOW_WEB"] === "1") return true;
  } catch {
    /* import.meta unavailable — fall through to the query check */
  }
  if (typeof window !== "undefined") {
    try {
      if (new URLSearchParams(window.location.search).has("web")) return true;
    } catch {
      /* URL parsing unavailable — stay on the real bridge */
    }
  }
  return false;
}

const WEB = isWebMode();

type RealLibrary = typeof realLibrary;
type MockLibrary = typeof mock;

function pick<K extends keyof RealLibrary & keyof MockLibrary>(key: K): RealLibrary[K] {
  return (WEB ? mock[key] : realLibrary[key]) as RealLibrary[K];
}

export const importFolder = pick("importFolder");
export const libraryList = pick("libraryList");
export const libraryCount = pick("libraryCount");
export const libraryMonths = pick("libraryMonths");
export const libraryStats = pick("libraryStats");
export const onImportProgress = pick("onImportProgress");
export const onImportError = pick("onImportError");
export const enrichAssets = pick("enrichAssets");
export const onEnrichProgress = pick("onEnrichProgress");
export const findDuplicates = pick("findDuplicates");
export const scoreQuality = pick("scoreQuality");
export const duplicatesList = pick("duplicatesList");
export const keepCopy = pick("keepCopy");
export const cleanupStats = pick("cleanupStats");
export const blurredAssets = pick("blurredAssets");
export const trashAssets = pick("trashAssets");
export const faceThumb = pick("faceThumb");
export const runFaceClustering = pick("runFaceClustering");
export const verifyLibrary = pick("verifyLibrary");
export const onClusterProgress = pick("onClusterProgress");
export const clustersList = pick("clustersList");
export const facesInCluster = pick("facesInCluster");
export const facesCount = pick("facesCount");
export const clusterEvents = pick("clusterEvents");
export const eventsList = pick("eventsList");
export const eventAssets = pick("eventAssets");
export const renameEvent = pick("renameEvent");
export const resolvePlaces = pick("resolvePlaces");
export const onGeocodeProgress = pick("onGeocodeProgress");
export const onGeocodeError = pick("onGeocodeError");
export const albumsList = pick("albumsList");
export const albumCreate = pick("albumCreate");
export const albumRename = pick("albumRename");
export const albumDelete = pick("albumDelete");
export const albumAssets = pick("albumAssets");
export const albumAdd = pick("albumAdd");
export const albumRemove = pick("albumRemove");
export const exportBundle = pick("exportBundle");
export const exportHighlight = pick("exportHighlight");
export const onHashProgress = pick("onHashProgress");
export const onQualityProgress = pick("onQualityProgress");
export const engineThumb = pick("engineThumb");

export const engineCall: typeof realEngine.engineCall = WEB
  ? mock.engineCall
  : realEngine.engineCall;
export const engineSysInfo: typeof realEngine.engineSysInfo = WEB
  ? mock.engineSysInfo
  : realEngine.engineSysInfo;

/** Folder picker: mock path in web mode, Tauri dialog in the app. */
export function open(opts?: { title?: string; directory?: boolean; multiple?: boolean }): Promise<string | null> {
  if (WEB) return mock.open(opts);
  return tauriOpen(opts ?? {}) as Promise<string | null>;
}

export type {
  AlbumRow,
  AssetRow,
  CleanupProgress,
  CleanupStats,
  ClusterEventsSummary,
  ClusterProgress,
  DuplicateGroup,
  DuplicateMember,
  EnrichProgress,
  EnrichSummary,
  EventAssetRow,
  EventRow,
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
  ResolvePlacesSummary
} from "./library";
export type { SysInfo } from "./engine";
