# fotoflow

On-device library management for photo and video. Tauri v2 + SvelteKit frontend, Swift engine sidecar (Vision, CoreML, AVFoundation), SQLite index. No cloud sync — the library never leaves the machine.

Current capabilities: folder import with metadata enrichment, cleanup (exact-duplicate detection, blur scoring, Trash-based deletion), people clustering (on-device face detection + feature-print clustering), event clustering (time + GPS), user-curated albums, per-event/per-album exports for Google Drive (full quality) and WhatsApp (1280px recode), and highlight-video slideshows.

## Develop

- `pnpm tauri dev` — run the app (dev window + engine sidecar)
- `pnpm build:engine` — rebuild the Swift sidecar (`scripts/build-engine.sh`)
- `cargo test` (in `src-tauri/`) — Rust suite; `pnpm check` — svelte-check

## Original research

What the tier-1 consumer apps actually ship (the bar to meet), the on-device model landscape, and the pipeline blueprints this project builds on.

### 1. Requirements & comparison of capabilities

#### Apple Photos (iOS 26) — the strongest on-device-only reference

- Natural-language semantic search over photos and videos, fully on-device — including search results that jump to the matching frame inside a video ("Relevant video thumbnails in search results", apple.com/os/pdf/All_New_Features_iOS_26_Sept_2025.pdf). Understands compositional queries: "Maya skateboarding in a tie-dye shirt", "Orange cat", "Chicken roasting in oven" (support.apple.com/guide/iphone/use-apple-intelligence-in-photos-iphf7de217f0).
- Prompted memory movies: type a description, on-device intelligence picks the best media, "crafts a storyline with unique chapters and a narrative arc, and sets it to music" — this is the current leading edge of aggregate highlight videos from photo libraries, on-device.
- People & pets: automatic face grouping with user-assigned names, extendable to named pets (Find and name people and pets in Photos).
- Clean Up (on-device object removal/inpainting via tap/brush/circle).
- Spatial scenes: monocular 2D→3D depth, generated on the Neural Engine, works even on decades-old photos.
- Near-duplicate / burst-aware dedup (dblabsapps.com/blog/ios-26-photos-features/).
- Foundation Models framework: any app gets free, offline access to Apple's on-device LLM with guided generation, tool calling, and image understanding (developer.apple.com/foundation-models-framework). Gen 3 (June 2026): AFM 3 Core (~3B dense) + AFM 3 Core Advanced — a 20B sparse/MoE model with weights on flash, expert-swapped to DRAM at runtime, 1–4B active params, natively multimodal, plus its expressive on-device voices (TTS MOS 4.15 vs 3.87 baseline) (machinelearning.apple.com/research/introducing-third-generation-of-apple-foundation-models).

#### Google Photos — the strongest curation & narration reference (though Gemini parts are cloud)

- Auto-organization: Trips, Best-of-Month, Memories carousel auto-built from time + location + content clustering.
- Recap: year-in-review video with stats (top people, photo counts, hobbies), personalization (hide people/photos and regenerate), shareable "souvenirs" (short clips/collages), and export to CapCut with exclusive templates (blog.google/products-and-platforms/products/photos/google-photos-2025-recap/).
- Ask Photos: conversational search with follow-ups ("which of these were in Croatia?", "find photos that feel like spring"), auto-edit by description, transcription — cloud Gemini, but defines the UX users now expect.
- Gemini tasks: multi-step agentic workflows ("find the 15 best trip photos, enhance, make a shared album, draft email") with recurring scheduled background organization — the frontier UX pattern for auto-albums (support.google.com/photos/answer/18116629).

#### Samsung Gallery (Galaxy AI) — the strongest video reference (mostly on-device)

- Auto Trim: on-device extraction of key moments from up to 90 min of footage (60 clips / 3 hours total) into an instant highlight reel with short/long/custom output length, reorder + music/stickers (samsung.com/us/galaxy-ai/; news.samsung.com interview describes the on-device Samsung Research video analytics optimization). This is the commercial SOTA for event/action highlights from video on-device.
- My FanCam: track a single subject, auto-crop to chosen aspect ratio (auto-reframe).
- Best Face / Audio Eraser (speaker/wind/noise filtering).
- One UI 8.5 instant search: real-time search results as you type, and a video result opens at the timestamp where "sea" or "sunset" occurs (sammobile.com/news/one-ui-8-5-gallery-instant-search-results-more/).
- Photo Assist generative edit with text prompts; Object Eraser and Auto Trim work offline, Generative Edit is (their one exception) hybrid.

#### Takeaways to copy

1. Semantic search that indexes inside videos (frame-level)
2. Memory movies with a generated narrative arc and chapters
3. Adjustable-length auto-trim
4. Subject-tracked auto-reframe
5. Recap/souvenir sharing pipeline
6. Background incremental indexing with instant-as-you-type latency

### 2. On-device model landscape (buildable stack)

#### Semantic search

| Component | State of the art | Numbers |
| --- | --- | --- |
| Embedding model | Apple MobileCLIP2 (FastViT, MIT/sample-code) | S0: 43 MB vision, ~5 ms/img, 71.5% zero-shot; S2: 143 MB, ~7 ms, 77.2%; B: 330 MB, ~14 ms, 79.4% (plhery/mobileclip2-onnx, eyenode README) |
| Embedding model (alt) | SigLIP 2 (Apache-2.0, 768-d, stronger multilingual) | 6.94 ms p50 on Snapdragon Hexagon NPU (Galaxy S26, LiteRT JIT-compiled QNN), 13.7 ms GPU; ~60 ms FP16 GPU on Pixel 8a (huggingface.co/litert-community/SigLIP2-base-patch16-224; runanywhere/siglip2_base_HNPU) |
| Runtimes | LiteRT (TFLite successor) w/ per-device JIT to NPU/GPU; CoreML+ANE on iOS; ExecuTorch (react-native-executorch); QNN context binaries | QHexRT: 4.5 ms text / 7.4 ms vision per encoder call |
| Vector store | vDSP/dot-product brute force is instant to tens of thousands of photos (~2 KB/photo) (github.com/john-rocky/coreai-kit PhotoSearch — 4 ms/photo Neural Engine indexing, 290 MB bundle); HNSW ANN (react-native-rag, FAISS-class) beyond ~30–50k |  |
| Proven app pattern | Incremental background indexing, int8-dynamic quantization, no-INTERNET-permission privacy test (github.com/KrunalWankhade9021/ImageSense) — embedding fidelity int8 vs fp32 ≥0.89 cosine |  |
| Frame-level video search | Sample keyframes → same embedding index; Apple precedent (video frame thumbnails in results + Samsung timestamp-jump) |  |

#### People & pets

- Face detection: Vision framework (iOS) / ML Kit or MediaPipe BlazeFace (Android); ONNX options SCRFD/YuNet.
- Face embedding: MobileFaceNet — 1–3 MB, <1M params, 99.48% LFW, 128-d, ArcFace loss (aihub.qualcomm.com/models/mobile_facenet). FaceNet/ArcFace at 512-d buy accuracy at 10–30+ MB (truetech.dev face-grouping pipeline writeup).
- Clustering: DBSCAN (eps ≈ 0.4–0.5 cosine, minPoints 2) → for 20k+ faces, ANN-based instead of O(n²).
- Privacy note: face embeddings are biometric data → GDPR Art. 9 special category; on-device-only processing is your compliance story, but still needs explicit consent UX (InspireFace/InsightFace SDK exists if you want a licensed production SDK with anti-spoofing).
- Pets: YOLOv11n pet detection + breed ID; dedicated TFLite stacks even do 46-point dog-face landmarks (github.com/hugocornellier/dog_detection) — pet face-clustering is a real differentiator few OSS stacks ship.

#### Quality / coloring / curation models

- NIMA aesthetics + technical quality (Google/idealo MobileNet, Apache-2.0): 1–10 score from a 10-bin distribution, ~6.4 MB each, ~11 ms each on-device GPU (huggingface.co/litert-community/NIMA-LiteRT). EMD loss predicts score distributions, giving a confidence signal too.
- Simple signal bundle that OSS curators converge on: sharpness (Laplacian variance) + exposure + face-presence + rule-of-thirds composition, weighted combination with real-time sliders (bppicker).
- Near-duplicate: exact → dHash/phash (Hamming ≤ 10); near → CNN/CLIP embedding cosine 0.5–0.9. idealo's benchmark shows plain hashes fail badly on near-dups while CNN embeddings hit ~0.999 precision (idealo.github.io/imagededup benchmarks). Apple-class products additionally cluster bursts within ~15 s windows (bppicker defaults) and pick a "best shot" per group — SharpSift/Apple duplicate detection precedent.
- Best-photo / cover selection: argmax composite score + greedy selection with per-day caps and monthly coverage for temporal diversity (bppicker pipeline), descending from IBM's event-clustering + quality-screening albuming work (Loui & Savakis, IEEE TMM 2003).

#### Event clustering / auto-albums (the research lineage)

State of the art is an ensemble over cheap metadata + content signals, hierarchically:

- Temporal clustering between consecutive photos (time-gap based, MMC/agglomerative).
- Spatial: GPS → reverse geocode → density clustering (~30 km) → routine vs non-routine locations; a Trip = everything from leaving a routine location until returning to one (Tankoyeu et al., "Indexing media by personal events" — F ≈ 0.8 on PicasaWeb-scale validation).
- Content: color-histogram / ordinal / CLIP-embedding similarity to split events into scenes ("Temporal event clustering", ACM TOMM 2005; ST-OCP agglomerative clustering using EXIF camera parameters to split indoor/outdoor/night sub-events).
- Hierarchical layers: day → part-of-day → location → environment → similarity, with a BIQA (blind image quality) sub-layer filtering junk between layers (MEMORIA, ACM LSC 2024 — directly maps to your 4 smart-album scopes).
- Semantic labels from CLIP zero-shot concepts + OCR (Vision framework / LiteRT OCR) for screenshots/documents/receipt-type albums.
- User-behavior learning (routine detection) outperforms static clustering — the "personal events" parading innovation.

#### On-device LLM + voice (narration / storytelling)

- iOS: Foundation Models framework — on-device LLM with guided generation (typed structured output), tool calling, and image understanding, free and offline → ideal for captioning, album titles, promo/recap scripts, full Ask-Photos-style chat over your local index.
- Android: Gemma 3n E2B/E4B via LiteRT-LM (Google AI Edge) — multimodal (text+image+audio), ~3.4 GB int4 E2B on Metal/QNN, proven shipping pattern (AccessEye narrator runs Gemma 3n fully offline with AVSpeechSynthesizer, github.com/orestislef/AccessEye); Gemini Nano/AICore as the built-in path; AI Edge function-calling library.
- TTS: Apple's on-device expressive voices (AFM 3 Core Advanced, MOS 4.15) or AVSpeechSynthesizer; Android system TTS / Chirp-class on-device voices. A narrated recap (LLM script → TTS → ducked music bed) is now fully achievable on-device and remains rare in consumer apps → differentiator.

### 3. Highlight video creation — pipeline blueprint

The OSS state of the art (desktop pipelines, but each stage maps to on-device primitives):

**Stage 1 — Shot/scene detection:** PySceneDetect (Content/Adaptive/Threshold detectors, v0.7.2) is the reference desktop tool; the on-device pattern is frame-difference histogram sampling over decoded thumbnails + a lightweight TransNetV2-style boundary model, sampling only keyframes.

**Stage 2 — Highlight/moment scoring.** Best practice is multi-axis scoring (as in ai-montage-agent):

1. motion (optical flow)
2. audio RMS energy + onset peaks (librosa-class analysis; on-device equivalents exist in audio DSP libs)
3. face count/size/smiles
4. camera-motion stability
5. NIMA aesthetic score at sampled frames
6. semantic affinity to the requested theme via CLIP embeddings ("birthday", "day at the lake")

Samsung Auto Trim is the commercial benchmark for on-device speed over 90-min footage with the added trick of adjustable output duration (fit-to-length scheduling).

**Stage 3 — Timeline / narrative planning:**

- Music beat map: tempo, beats, energy curve, peaks/valleys → cut density parameter (cut every beat vs every Nth beat) with anti-repetition, source spacing, black-frame filtering (beat-synced-edit repo is a clean reference implementation).
- Agenda/story fit: high-energy clips on energy peaks, calm on valleys; hook (10 s "strobe") vs long-form (90 s "epic") mode presets (beatsync-engine shows config-driven mode design).
- Memory-movie style: chapters + narrative arc; LLM (on-device, guided generation) can generate the chapter plan + captions + narration script from the media summaries.
- Event recaps: quality-screened top shots + temporal diversity caps + stats overlays (Google Recap pattern) + souvenirs (3–6 s sub-clips + collages for social).

**Stage 4 — Beat-locked polishing:** dialogue ducking (BGM ducks under speech), color harmonization/brightness matching across clips, auto-reframe with subject tracking (facam/My FanCam analog), transitions matched to energy (white-flash montage, kick punches).

**Stage 5 — On-device rendering (the real engineering constraint — no ffmpeg on mobile):**

- iOS: AVFoundation compositing (AVMutableComposition + AVVideoComposition with Core Image transitions, hardware-encoded AVAssetExportSession).
- Android: Media3 Transformer (Transformer effect composition + hardware muxer).

This is the piece the OSS montage repos don't solve for mobile; every shipping app (Apple/Samsung) renders via platform hardware pipelines.

User-driven montage falls out of the same pipeline: user picks clips → skip Stage-2 selection, run scoring only for auto-reframe + beat sync. AI storytelling = Stage 3 planner + on-device LLM narration (differentiator; only Apple's memory-movie prompt approach currently ships anything like this, and theirs is generated-free chapters, not spoken narration).

### 4. Scorecard: where the leading edge actually is

| Capability | Leader | Why it's the bar |
| --- | --- | --- |
| Semantic search incl. video frames | Apple iOS 26 + Samsung 8.5 | never-JIT search + timestamp jump into video |
| Auto-albums / trips / hierarchy | Google Photos | routine-location trip model + recurring background tasks |
| People & pets | Apple (+ Samsung pet Portrait) | named groups incl. pets; pet faces via MediaPipe-class stacks in OSS |
| Dedup + best-shot | Apple / bppicker-class | burst-aware near-dup + composite quality selection |
| Aesthetic scoring | Google NIMA lineage | 10-bin distributions, 11 ms, deployed at Google scale |
| Photo→movie narrated storytelling | Apple memory movies | generated chapters + narrative arc + music from a text prompt |
| Video highlight reel | Samsung Auto Trim | 90-min on-device analysis, fit-to-length |
| Subject-aware auto-reframe | Samsung My FanCam | on-device tracking reframe |
| Narration (voice) | Nobody ships spoken on-device recaps → open gap | AFM 3 voices / system TTS now make it feasible |

### 5. Suggested reference architecture (on-device, incremental)

```text
Ingest (new/changed assets)
  → per-asset pass: faces+pets detect → embeddings | CLIP embed (full + 5 keyframes/video)
                    NIMA aesthetic+technical | dHash | OCR | GPS/time
  → SQLite metadata + flat float32 vector index (add HNSW >30k assets)
  → background org pass: temporal → spatial routine/non-routine → content/semantic → event tree
  → curation pass: dedup groups → composite best-shot → diversity-capped smart albums + covers
Query: text→CLIP-text embed + metadata filters → topic-merged; optional F-M on-device LLM chat layer
Creators:
  - recap/memory movie: stage-2 scoring → diversity selection → narrative plan → chapters
  - montage/highlight: scene detect → multi-axis score → beat map → refined shot list
  - narration: LLM script (guided generation) → TTS → ducked music →
  - render: AVFoundation / Media3 Transformer (hw encoders)
```

> One caveat on the OSS pile: the beat-sync/montage repos (beat-synced-edit, ai-montage-agent, clipforge, CutAI/MIT) are excellent blueprints for the logic, but they're Python/ffmpeg desktop stacks — port the decision layer (EDL + scoring) to Swift/Kotlin and render natively.
