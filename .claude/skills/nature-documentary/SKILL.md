---
name: nature-documentary
description: Use when producing nature/wildlife documentaries from the fotoflow library — building chaptered mini-docs from bird, cat, wildlife, flower or tree footage (Vivo X200 Pro library), editing the documentary engine/Rust/frontend pipeline, verifying documentary renders, or debugging make_documentary/CLIP subject tagging.
---

# Nature documentary pipeline

Chaptered mini-docs (60–600s) per event/album: favorites seed the cut, CLIP
subject buckets rotate for variety, chapter cards open at day boundaries and
subject changes, stats end card closes. Camera audio rides at native level;
music only fills silent stretches. Deliberately silent (no TTS narration).

## When something must be proven, never reasoned

Media claims get probes, not documentation reading. The verified probe is
`scripts/doc_probe.swift` in this skill folder: synthetic stills + a
video-only clip + a WAV, muxed via an AVAssetExportSession composition, fed
to the real engine binary over JSON-lines stdin, asserted by Goertzel band
power and Vision OCR. Run from the repo root:

```bash
rm -rf /tmp/docprobe && swift .claude/skills/nature-documentary/scripts/doc_probe.swift
```

Expect 15 PASS. Reuse its patterns for any new render claim, with two
limits: the probe proves presence, layout, and audio bands — not exact
visual geometry. For font sizes, colors, or crop centering, decode the card
frame (`AVAssetImageGenerator` at the card midpoint, like the probe's
`ocrFrame`) and assert frame metrics (glyph extents, crop-band means) or
read the frame yourself. For real-footage questions (bird framing, Vivo
telephoto crops), skip synthetic media: feed the library file straight to
`make_documentary` over engine stdin with the same JSON-lines protocol —
no app session needed.

Two macOS 26 wedges this probe survived (do not re-fight them):

- An AVAssetWriter with an *in-writer* audio input wedges its video input
  even in a synthetic generator, across feed orders. Generate video-only
  and mux the camera-audio WAV in with an `AVMutableComposition` +
  `AVAssetExportSession` export. (`AVAssetWriter` status codes: completed=2;
  `AVAssetExportSession` completed=3.)
- `Process.standardInput` set to a `Pipe` reads back as the `Pipe`, not a
  `FileHandle` — keep your own `Pipe` references and use
  `fileHandleForWriting`/`fileHandleForReading`.

## Pipeline map (all paths verified 2026-10-25)

- Engine: `make_documentary` case → `makeHighlight(inputs, output,
  maxSeconds, musicPath, portrait:, naturalFirst:, endCard:)` — one builder,
  documentary is flags on it (sidecar/engine/Sources/engine/main.swift).
  Reply carries `aspect`, `naturalFirst`, `endCard` in addition to the
  highlight reply fields.
- Rust: `export_documentary` (src-tauri/src/lib.rs) → `documentary_clips`
  (favorites seed → CLIP round-robin → quality fill) →
  `db::documentary_plan` (pure, unit-tested: chapters at day/subject
  changes) → `render_documentary` (engine timeout 1800s — doc renders run
  to 10 min; the story timeout of 600s is too short). Fallback when CLIP
  models/vectors are missing: plain story curation, doc still exports.
- Frontend: "Documentary" button on event + album cards, dialog
  (`aria-label="Documentary video"`), `doc:*` job key, mock stub in
  mock.ts so e2e exercises the real subscription path.
- Subject tagging: `DOC_SUBJECT_PROMPTS` (bird/cat/flower/tree/wildlife,
  ~6 prompts) × member `clip_vectors`/`video_keyframes`, threshold
  `DOC_SUBJECT_MIN_COSINE = 0.22`. Labels stay generic ("Bird") until the
  species-classifier phase; chapter text is `{date} · {Subject}` or
  `{date} · {place}`.

## Recurring traps (each cost a real failure once)

- After `pnpm build:engine`, restart the dev app — the sidecar loads at
  spawn; a running session keeps the old binary.
- A new export job key is dead UI until it is added to the
  `onExportProgress` filter in `src/routes/+page.svelte` (the
  `p.job === highlightJob || p.job === docJob || …` guard). Progress events
  for unlisted jobs are silently dropped — the export "succeeds" with no
  progress, and the busy-card e2e test times out.
- Svelte dialog/markup blocks: when mirroring the highlight blocks, anchor
  edits on indentation-exact context — an ambiguous anchor silently lands
  the album block inside the event card (references an undefined variable
  and svelte-check catches it only afterwards).
- e2e timing: the mock documentary render is deliberately slow
  (200 ms/clip) so the busy-card progress assertion can catch it. Don't
  speed it up; don't assert wall time.

## Gates for any behavior change

`pnpm check` (0 errors 0 warnings) + `cargo test --lib` (from src-tauri/)
+ `pnpm test:e2e`. Report the counts. `test:e2e:api` only when the
localhost server surface changes (documentary is Tauri-write-only).

## Deferred phases (do not silently start)

- Species classifier (real "Kingfisher" chapter names): needs model
  research + download flow within the 2GB budget.
- Subject-tracked 9:16 reframe (current vertical is center-crop).
- Trip-level documentaries (needs multi-event selection; trip ids are
  positional and unstable — db.rs TRIP_GAP_HOURS comment).
- Ducking/level automation: music-under-camera is structurally zero in
  natural-first mode; only revisit if music-under-voice is wanted.
