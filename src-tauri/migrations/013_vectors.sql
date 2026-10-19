-- 013_vectors: visual-similarity embeddings (similar-photos v1).
--
-- One row per embedded asset per model. v1 ships Apple's built-in
-- VNGenerateImageFeaturePrint (zero download; measured 768 f32 on this SDK,
-- stored per-row in `dim` so encoder upgrades never re-key the table). The
-- `model` column leaves room for an opt-in upgrade (e.g. MobileCLIP).
-- Vectors are brute-force cosine-compared at query time (no ANN in v1);
-- re-running `embed_library` refills them from the images on disk.

CREATE TABLE asset_vectors (
    asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    model    TEXT    NOT NULL DEFAULT 'vn-featureprint',
    dim      INTEGER NOT NULL,
    vec      BLOB    NOT NULL
);

CREATE INDEX idx_vectors_model ON asset_vectors (model);
