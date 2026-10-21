import * as realLibrary from "./library";
import * as realEngine from "./engine";
import { open as tauriOpen } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { convertFileSrc } from "@tauri-apps/api/core";
import * as mock from "./mock";
import * as api from "./api";
import { isApiMode } from "./api";

export { isApiMode };
export const isApi = isApiMode();

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
const API = isApiMode();

type RealLibrary = typeof realLibrary;
type MockLibrary = typeof mock;
type ApiLibrary = typeof api;

function pick<K extends keyof RealLibrary & keyof MockLibrary>(key: K): RealLibrary[K] {
  return (WEB ? mock[key] : realLibrary[key]) as RealLibrary[K];
}

function pickRead<K extends keyof RealLibrary & keyof MockLibrary & keyof ApiLibrary>(
  key: K
): RealLibrary[K] {
  if (API) return api[key] as unknown as RealLibrary[K];
  return pick(key);
}

/** Write counterpart of pickRead: API client in api mode, else the Tauri/mock pick. */
function pickWrite<K extends keyof RealLibrary & keyof MockLibrary & keyof ApiLibrary>(
  key: K
): RealLibrary[K] {
  if (API) return api[key] as unknown as RealLibrary[K];
  return pick(key);
}

export const importFolder = pickWrite("importFolder");
export const libraryList = pickRead("libraryList");
export const libraryCount = pickRead("libraryCount");
export const libraryMonths = pickRead("libraryMonths");
export const libraryStats = pickRead("libraryStats");
export const scanRoots = pickRead("scanRoots");
export const assetsInRoot = pickRead("assetsInRoot");
export const removeRoot = pickWrite("removeRoot");
export const mapPins = pickRead("mapPins");
export const mapTile = pickRead("mapTile");
export const setFavorite = pickWrite("setFavorite");
export const tagScenes = pickWrite("tagScenes");
export const onTagProgress = pick("onTagProgress");
export const sceneTags = pickRead("sceneTags");
export const assetTags = pick("assetTags");
export const runPostImport = pick("runPostImport");
export const onPipelineProgress = pick("onPipelineProgress");
export const onImportProgress = pick("onImportProgress");
export const onImportError = pick("onImportError");
export const enrichAssets = pick("enrichAssets");
export const onEnrichProgress = pick("onEnrichProgress");
export const findDuplicates = pick("findDuplicates");
export const scoreQuality = pick("scoreQuality");
export const findSimilar = pick("findSimilar");
export const nearGroupsList = pick("nearGroupsList");
export const keepNearCopy = pick("keepNearCopy");
export const onNearProgress = pick("onNearProgress");
export const similarTo = pick("similarTo");
export const embedLibrary = pick("embedLibrary");
export const onEmbedProgress = pick("onEmbedProgress");
export const duplicatesList = pick("duplicatesList");
export const keepCopy = pick("keepCopy");
export const cleanupStats = pick("cleanupStats");
export const blurredAssets = pick("blurredAssets");
export const trashAssets = pick("trashAssets");
export const restoreAssets = pick("restoreAssets");
export const faceThumb = pick("faceThumb");
export const runFaceClustering = pick("runFaceClustering");
export const verifyLibrary = pick("verifyLibrary");
export const onClusterProgress = pick("onClusterProgress");
export const clustersList = pickRead("clustersList");
export const renameCluster = pick("renameCluster");
export const mergeClusters = pick("mergeClusters");
export const restoreCluster = pick("restoreCluster");
export const unassignFace = pick("unassignFace");
export const assignFace = pick("assignFace");
export const detectPets = pick("detectPets");
export const onPetProgress = pick("onPetProgress");
export const onPetError = pick("onPetError");
export const sightingsList = pick("sightingsList");
export const facesInCluster = pick("facesInCluster");
export const facesCount = pick("facesCount");
export const reviewCandidates = pick("reviewCandidates");
export const dismissReviewCandidate = pick("dismissReviewCandidate");
export const clipModelsPresent = pick("clipModelsPresent");
export const downloadClipModels = pick("downloadClipModels");
export const onClipDownloadProgress = pick("onClipDownloadProgress");
export const embedClipBackfill = pick("embedClipBackfill");
export const reindexClip = pick("reindexClip");
export const cancelClipBackfill = pick("cancelClipBackfill");
export const removeClipModels = pick("removeClipModels");
export const searchText = pickRead("searchText");
export const clusterEvents = pick("clusterEvents");
export const eventsList = pickRead("eventsList");
export const eventAssets = pick("eventAssets");
export const tripsList = pickRead("tripsList");
export const curatePicksToAlbum = pick("curatePicksToAlbum");
export const renameEvent = pick("renameEvent");
export const resolvePlaces = pick("resolvePlaces");
export const onGeocodeProgress = pick("onGeocodeProgress");
export const onGeocodeError = pick("onGeocodeError");
export const albumsList = pickRead("albumsList");
export const albumCreate = pickWrite("albumCreate");
export const albumRename = pick("albumRename");
export const albumDelete = pick("albumDelete");
export const albumAssets = pick("albumAssets");
export const albumAdd = pickWrite("albumAdd");
export const albumRemove = pick("albumRemove");
export const exportBundle = pick("exportBundle");
export const exportSharePack = pick("exportSharePack");
export const exportHighlight = pick("exportHighlight");
export const onExportProgress = pick("onExportProgress");
export const ingestCopy = pick("ingestCopy");
export const onIngestProgress = pick("onIngestProgress");
export const addEventPicksToAlbum = pick("addEventPicksToAlbum");
export const trashList = pick("trashList");
export const clearTrashLog = pick("clearTrashLog");
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

/**
 * Reveal a finished export in Finder / open its gallery page. No-ops in web
 * and API modes (no real files behind the mock, no opener bridge over HTTP).
 */
export async function revealInDir(path: string): Promise<void> {
  if (WEB || API) return;
  await revealItemInDir(path);
}

export async function openExternal(path: string): Promise<void> {
  if (WEB || API) return;
  await openPath(path);
}

/**
 * Playable/src-able URL for a library file. Empty in web mode (no real
 * files behind the mock); the viewer falls back to the thumbnail poster.
 */
export function assetUrl(path: string): string {
  if (WEB) return "";
  return convertFileSrc(path);
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
  EmbedProgress,
  EmbedSummary,
  EnrichProgress,
  NearDupeSummary,
  NearGroup,
  EnrichSummary,
  EventAssetRow,
  EventRow,
  ExportProgress,
  ExportSummary,
  FaceClusterRow,
  FaceRow,
  GeocodeProgress,
  HighlightResult,
  ImportProgress,
  ImportSummary,
  IngestProgress,
  IngestSummary,
  LibraryFilters,
  LibraryMonth,
  LibraryStats,
  MapPin,
  PetProgress,
  PetSighting,
  DetectPetsSummary,
  PipelineProgress,
  PipelineSummary,
  ResolvePlacesSummary,
  TrashEntry,
  TripRow,
  ScanRoot,
  SceneTag,
  SceneTagCount,
  SharePackSummary,
  RemoveRootSummary,
  RestoreSummary,
  ReviewCandidate,
  ReviewClusterRef,
  ReviewFaceRef,
  SearchHit,
  ClipModelsStatus,
  ClipDownloadProgress,
  SimilarHit,
  TagProgress,
  TagScenesSummary
} from "./library";
export type { SysInfo } from "./engine";
