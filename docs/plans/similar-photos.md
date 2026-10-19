# Similar-photos search (visual similarity v1)

Scope decision: **image-query similarity first** ("more like this photo").
Natural-language text query ("beach at sunset") is explicitly deferred to v2 —
it needs a joint text+image model, while v1 ships value with an image-only encoder.

## Why this shape

- Existing near-dupe detection (`dhash`, migration 011) catches exact/near
  copies only. Users also want *semantic* neighbours (same dog, same place,
  same scene, different shot).
- Libraries here are ≤ ~50k assets: brute-force cosine over compact vectors
  is fast enough (<100 ms) with zero index dependency. No HNSW in v1.

## Model choice (needs user sign-off)

| Option | Dims | Download | Quality |
|---|---|---|---|
| A. `VNGenerateImageFeaturePrint` (built-in Vision) | 2048 f32 | **0 MB** | moderate; free, no licensing/download risk |
| B. MobileCLIP-S0 image encoder, CoreML | 512 f32 | ~150–300 MB | clearly better semantics |

Recommendation: **ship A first** (zero download keeps the ≤2 GB / offline
promise trivially true), design the schema for B (`model` column), add B as
an opt-in upgrade. If A proves too weak in practice, skip to B.

## Changes

1. **Engine** (`sidecar/engine`): `embed {path}` → `{vector: base64 f32}`,
   `embed_batch {paths}` with progress chunks (mirrors `dhashBatch`).
2. **Migration 013** `asset_vectors(asset_id PK, model TEXT, dim INT,
   vec BLOB)` + `similar_cache`? No cache table in v1 — brute force live.
   Validate with `validate_migration.py` (`foreign_keys=ON`).
3. **Rust** (`db.rs`/`lib.rs`): `save_vector`, `similar_to(asset_id,
   model, limit)` cosine over BLOBs (int8-quantize at write if A/2048 f32
   proves heavy: 50k × 2048 × 4 B = 400 MB — likely needs quant; B/512 f32
   = 100 MB, acceptable); `embed_library` backfill + hook into
   `run_post_import` after `tag_scenes` with `embed-progress` events.
4. **UI**: "Similar" button in viewer + Library grid item actions → results
   grid reusing the event member-grid pattern. Cleanup keeps dHash section
   (exact dupes); semantic neighbours live under Library, not Cleanup.
5. **Tests**: determinism (same file → cosine 1.0), crop/resize still close,
   unrelated photo far; engine roundtrip; mock `embed` + e2e similar-flow
   specs.

## Explicitly out

Text query, cross-modal (needs text encoder + tokenizer), HNSW/ANN, GPU
batching, face-identity vectors (faces keep the existing cluster pipeline).

## Risks

- A-quality: feature-print similarity can surprise (texture > subject).
  Mitigate with a small golden set (10–15 labelled pairs) asserted in tests.
- Thumb-cache-style disk growth: vectors dwarf thumbs — measure and quantize.
