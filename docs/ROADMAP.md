# fotoflow roadmap

## How to read this

- Statuses mirror the README "Plan — capabilities by phase" section (P1–P6, shipped / partial / open).
- Update both files when a phase moves; this file orders the queue, the README owns the phase definitions.
- Anything marked "verify" was not confirmed in README, `git log --oneline -30`, or `src/routes/+page.svelte` — check before building on it.

## Now — shipped

- Folder import with SQLite catalog, EXIF/GPS enrichment, and post-import auto-organize — shipped.
- Library grid with viewport-gated pooled thumbnails, pagination (60/page), lightbox viewer, favorites, multi-select — shipped.
- Filename search over names, tags, people, events, and places with relevance ranking (name > tag > event > person) plus recent queries — shipped (keyword+metadata only, no vector index).
- Missing-file verification for ejected volumes with restore on reconnect — shipped.
- Cleanup: exact-duplicate groups (size-bucketed hashing) and blur scoring with Trash-based deletion / flag-only review — shipped.
- People: on-device face detection + feature-print clustering with rename and merge — shipped.
- Events: time + GPS clustering with kept manual names, gap control, offline state/country fences plus cached OpenStreetMap place names (the only network call) — shipped.
- Albums: user-curated lists as export sources with add / remove / rename / delete — shipped.
- Distribution: per-event/per-album Drive (full quality) and WhatsApp (1280px recode, video included) bundles — shipped.
- Highlight videos: per-event mixed photo/video montage with original camera audio, capped at 60s — shipped (partial).
- Engine health check (ping + OS/arch/memory readout) in Settings — shipped.

## Next — queued, ordered by user value per effort

- CI workflow (GitHub Actions: engine build, cargo test, svelte-check, Playwright) — nothing runs automatically today (no `.github/` in repo); stops silent regressions — S.
- Disk-health verify (read-probe file headers, flag bit-rot vs missing) — current verify only checks presence; user's USB disk already has unreadable files — S/M.
- Sidecar watchdog (restart the Swift engine on call timeout) — one engine process over stdio; one bad file stalls all engine traffic: a timeout drops the reply but the wedged process keeps queuing — S/M.
- Thumb disk cache (persist thumbnails across restarts) — thumbs today are in-memory with pooled viewport loading, so every grid rebuild re-renders — M.
- Background scheduling (startup resume of stranded enrich / hash / tag passes) — post-import pipeline exists but only missing-file verify runs at boot; passes don't resume — M.
- Near-duplicate detection (CNN/CLIP embedding cosine) — exact-hash exists; README P2 lists near-dup clustering as open — M/L.
- True vector semantic search via on-device CLIP model (MobileCLIP2 S0 ~43MB, inside the 2GB model-download budget) over current keyword+metadata search — unlocks natural-language queries incl. video frames — M/L.
- Narrated recaps + redaction policies — plan-level, honestly open per README P5 (narration) and P6 (redaction, watermarks, allowlists); needs LLM script + TTS + EXIF/sensitive-content policy — L.

## Later / research

- Memory-movie style chapters with narrative arc and recap/souvenir sharing pipeline (Apple/Google pattern in README appendix).
- Conversational search with follow-ups and query-time re-ranking over the embedding index (Ask Photos pattern; needs the vector index above first).
- Video depth: adjustable-length Auto Trim fit-to-length plus subject-tracked reframe; pet identity tracking and NIMA aesthetic scoring as curation upgrades (all appendix themes, none committed).
