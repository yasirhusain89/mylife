# Fotoflow

Tauri v2 + SvelteKit photo manager (Rust backend `src-tauri/`, Swift engine
sidecar `sidecar/engine/`, SQLite). On-device: import, cleanup, people,
events/trips, albums, share packs, localhost API. Library never leaves the
machine — no cloud sync.

## Requirements

Functional:
- Folder import with metadata enrichment (EXIF/GPS/date), thumbnail catalog,
  pagination, month/location/person filters, missing-file verification.
- Cleanup: exact-duplicate groups, blur scoring, Trash-based deletion, review UI.
- People: Vision face detection + feature-print clustering, named people,
  dog/cat pet detection + sightings.
- Events/trips: time + GPS clustering (48h gap / 50km radius), naming,
  routine vs trip distinction.
- Albums: user-curated, covers, membership management.
- Exports/highlights: per-event or per-album memory video (mixed photo/video,
  title cards, Ken Burns), Drive full-quality / WhatsApp compact recode,
  share packs (Day folders + offline gallery).
- Localhost HTTP API for read operations; writes only via Tauri commands
  (favorites, albums, tags).

Non-functional:
- On-device only. Model downloads ≤ ~2GB total. Network = reverse geocode +
  OSM/OpenTopoMap tiles (no other outbound).
- Face embeddings are biometric (GDPR Art. 9) → explicit consent UX.
- Story videos are silent for now; camera-audio mixing needs off-AVAssetReader
  extraction (any audio-track interaction poisons in-process video decode on
  macOS 26).
- Deletion = macOS Trash only; never bypass system dialog.
- Progress bars count finished units, never wall time — heavy units hold, then
  jump; the filename/clip label proves liveness.

## Environment

- M4 Mac mini Pro, macOS 26.6.2, CLT 26.6 (`swiftc` 6.3.3 direct; **SwiftPM
  is broken**, Xcode install is a user task).
- Dev session runs in background (`OPENCODE_ALLOW_BG=1`,
  `pnpm tauri dev > /tmp/fotoflow-dev*.log`); live app
  `target/debug/fotoflow`, `http://127.0.0.1:1420/`.
- `/Volumes/YH-SanP2TB` must be mounted for live-disk probes.
- Never launch GUI apps/URLs; one foreground check per bash call
  (`sleep` ≤120s); kill only by exact PID, never watchdog loops.

## Commands

```bash
pnpm check          # svelte-check — must be 0 errors, 0 warnings
pnpm test:e2e       # Playwright, web-mock mode (?web=1)
pnpm test:e2e:api   # HTTP API suite (spawns its own server + vite)
cargo test --lib    # Rust suite (run from src-tauri/)
pnpm build:engine   # rebuild Swift sidecar -> src-tauri/binaries/ (gitignored)
pnpm index:semantic # rebuild semantic index after big moves
```

Gate for any behavior change: `check` + `cargo test --lib` + `test:e2e`
(+ `:api` when the server surface changes). Report the counts.

## Architecture

- Engine protocol v1: JSON-lines stdio `{id,cmd,params}` → final reply;
  interim `chunk`/`progress` events, keyed by id. Rust `engine.rs`
  (`call_blocking`, `call_with_events`); Swift `main.swift` (`handle`,
  `background`, global `requestID` for progress).
- Long ops emit Tauri events (`import-progress`, `export-progress {job,
  phase, done, total, label}`, …) → `src/lib/library.ts onX` →
  `bridge.ts pick` → `mock.ts` stub. Frontend filters progress on its
  active job key; `total: 0` = indeterminate phase label.
- Bridge modes: Tauri app (real IPC), `?web=1` (in-memory mock for e2e),
  `?api=` (read-only HTTP). Mock exports play scripted progress so e2e
  exercises the real subscription path.
- DB: `~/Library/Application Support/com.yasihusain.fotoflow/fotoflow.db`
  (`user_version` 16, migrations 001–016, `foreign_keys=ON`).
  Thumbs `{app_data}/thumbs/` (FNV-1a, 512/256MB), map tiles
  `{app_data}/maptiles/` (128/64MB). Videos via `assetProtocol`
  (`convertFileSrc`); photos base64.

## Capability phases & open gaps (from README plan)

- **P1 Import & Library — shipped, gaps.** Open: drag-and-drop import,
  osxphotos import from Apple Photos, LQIP placeholders.
- **P2 Cleanup — partial.** Shipped: exact-dup groups, dHash near-dup
  groups with burst-split (600s gap cap — a 50-lookalike chain spanning
  22.7h proved pure union-find unusable), keeper score breakdown
  (`why keeper: ★ favourite · sharpest (N) · earliest`). Open:
  utility-shot detection, low-quality scoring, batch confirm.
- **P3 Intelligence — partial.** Open: MobileCLIP2 embeddings + semantic search,
  VLM captions on demand.
- **P4 Events & Summaries — partial.** Open: routine-vs-trip distinction,
  LLM text summaries, event stats (top people, days, places).
- **P5 Highlight videos — partial.** Open: camera-audio mixing (deferred,
  see quirks), diversity-picked shots, Ken Burns pans, transitions, narration,
  export presets.
- **P6 Audience distribution — partial.** Open: redaction policies, watermarks,
  faces allowlists, selection budgets, Mail.app attach.

## Competitive comparison (bar to meet; depth in README "Original research")

| Capability | Leader | Gap to copy |
|---|---|---|
| Semantic search incl. video frames | Apple iOS 26 / Samsung 8.5 | keyframe→embedding index, timestamp jump |
| Auto-albums / trips / hierarchy | Google Photos | routine-location trip model + bg tasks |
| People & pets | Apple | named groups incl. pets |
| Dedup + best-shot | Apple / bppicker-class | burst-aware near-dup + composite selection |
| Aesthetic scoring | Google NIMA | 10-bin distribution, ~11ms |
| Narrated memory movie | Apple memory movies | chapters + arc + music from text prompt |
| Video highlight reel | Samsung Auto Trim | 90-min on-device analysis, fit-to-length |
| Subject-aware auto-reframe | Samsung My FanCam | on-device tracking reframe |
| Spoken on-device recaps | Nobody ships | **open differentiator** — AFM 3 voices / system TTS now feasible |

Key product takeaways: semantic search inside videos; memory movies with
narrative arc + chapters; adjustable-length auto-trim; subject-tracked
auto-reframe; recap/souvenir sharing pipeline; background indexing with
instant-as-you-type latency.

## macOS 26 quirks (measured, not lore)

- Still images expose zero `AVAssetTrack`s → photo frames are synthesized
  (3s @30fps Ken Burns), never composed.
- `VNGenerateFaceCaptureRequest` is removed (`FeaturePrintObservation`).
- Sync `tracks(withMediaType:)` **deadlocks main** → AV work in
  `background<T>` with main-runloop pump. `session.error` deprecated (kept).
- **Any `AVAssetReader` audio interaction wedges in-process video decode**
  (bare `.audio` query stalled DJI footage 90/90 → 42–43; an audio-output
  reader hung a probe past 115s). **But `ExtAudioFile` (AudioToolbox) is
  safe**: full PCM read (125M frames) → video decode still 120/120 on the
  same file (probe `/tmp/audio_probe.swift`, 2026-10-20). Camera-audio can
  proceed via `ExtAudioFile` → audio `AVAssetWriterInput`; never attach an
  audio output to `AVAssetReader`. (Bare `.audio` query did NOT reproduce
  on H264 lecture footage — wedge may be codec-specific.)
- Bitmap draws land row 0 = frame top; title-card text wants `flipped:false`
  (frame OCR caught `flipped:true` printing upside down).
- `compressVideo` uses the HQ `1280x720` export preset (orientation
  preserving: portrait → 720×1280); transcodes are **media-engine-bound**
  (concurrency measured ~no gain — serial is optimal, don't re-try).
- Highlight stills decode at cover × 1.10 headroom (`coverThumbnail`);
  per-op costs: photo recode ~0.12s, 4K-min video ~6s.

## Product decisions (locked)

- Deletion = macOS Trash only. Model downloads ≤ ~2GB. Network = reverse
  geocode + OSM/OpenTopoMap tiles only.
- API writes = favorites + albums + tags. Imports are path-based.
  Text-query needs a license-clean encoder + full re-embed (blocked).
- Trips: 48h gap / 50km radius. Curation: balanced (favorites, faces,
  sharpness, dHash veto, 20% video quota). Share packs capped at 60 files.

## Workflow

- Surgical diffs; verify before declaring done; Conventional-Commit style
  messages; push to `main` when green (`gh` authed as `yasirhusain89`).
- Parallel agents: isolated `git worktree` + branch each, ports 1422–1428
  (`pw.*.config.ts`, deleted after), region ownership, merge → full gate.
- e2e selectors stay byte-identical (`tablist`/`tab`/`alertdialog`/
  `role=status`); toast shares `role=status` — scope export assertions to
  the busy card/dialog.
- Prove media behavior with throwaway `/tmp` probes (synthetic media →
  engine stdin → frame-band/OCR assertions), never coordinate reasoning.
- Leave other tooling's artifacts alone: `.gitignore` semantic hunks,
  `scripts/semantic_index.py`, `.idea/`, `docs/plans/text-query.md`,
  `e2e/probe.spec.ts`, `docs/capability-map.html`.

## Reusable scripts & recurring actions

### Build engine (recurring after Swift changes)
`pnpm build:engine` → `src-tauri/binaries/engine-aarch64-apple-darwin`.
Restart the app after rebuilding — the sidecar loads at spawn, running
sessions stay on the old binary.

### Media probe (recurring before any AV/CoreGraphics claim)
Synthetic media in `/tmp` → feed engine via JSON-lines stdin → assert
frame-band means / OCR text / exit codes. Never reason about pixel output
from documentation; measure it.

### Live-DB probe (WAL!)
The app holds the DB open: **never file-copy `fotoflow.db`** (the `-wal`
holds recent commits — a bare copy reads stale). Snapshot with
`sqlite3 <live> ".backup main /tmp/x.db"` and probe the copy.

### Semantic index (recurring after big structural moves)
`pnpm index:semantic` — needs `ollama serve` + `nomic-embed-text`.
Falls back to native Grep if ollama isn't running.

### Commit / push gate (recurring before any PR or branch handoff)
`pnpm check` + `cargo test --lib` + `pnpm test:e2e` + `pnpm test:e2e:api`
all green → commit → push → restart `pnpm tauri dev`.

## Next tracks (unstarted)

- Camera-audio mixing: `ExtAudioFile` path verified — next is prototype
  `makeHighlight` with audio `AVAssetWriterInput` fed from `ExtAudioFile` PCM.
- People review queue: 83 unassigned faces (re-run clustering), 4 cross-name
  near pairs + 19 near-miss singletons to adjudicate. Threshold stays 0.55
  (0.60 would chain-fuse Amol/Y1/T2 — measured 2026-10-20).
- Map pan/zoom smoothness check (needs human frame review).

<!-- semantic fallback: `pnpm semantic:q "concept"` covers src/, src-tauri/, scripts/, e2e/. -->
<!-- Needs `ollama serve` + nomic-embed-text. Rebuild with `pnpm index:semantic`. -->
