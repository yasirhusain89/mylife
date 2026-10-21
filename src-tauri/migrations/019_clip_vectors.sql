-- 019_clip_vectors: semantic-search embeddings (OpenCLIP ViT-B-32, CoreML).
--
-- Separate tables from asset_vectors on purpose: that table keys one vector
-- per asset (the VN feature print for visual similarity), while semantic
-- search needs its own model space plus many rows per video (one per
-- keyframe). Re-embedding refills both tables from the files on disk.

CREATE TABLE clip_vectors (
    asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    dim      INTEGER NOT NULL,
    vec      BLOB    NOT NULL
);

CREATE TABLE video_keyframes (
    asset_id   INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    t_seconds  REAL    NOT NULL,
    dim        INTEGER NOT NULL,
    vec        BLOB    NOT NULL,
    PRIMARY KEY (asset_id, t_seconds)
);

CREATE INDEX idx_keyframes_asset ON video_keyframes (asset_id);
