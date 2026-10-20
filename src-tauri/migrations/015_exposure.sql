-- 015_exposure: persist exposure settings from engine stat.
--
-- The engine has emitted iso / fNumber / exposureTime since the stat
-- enrichment landed, and focalLength / focalLength35 with this release —
-- but apply_stat dropped them on the floor. These columns keep them.
-- Nullable so pre-migration rows stay NULL; resetting stat_at re-enqueues
-- those rows for the next incremental enrich pass (stat is metadata-only,
-- no image decode, so the backfill is cheap).

ALTER TABLE assets ADD COLUMN iso INTEGER;
ALTER TABLE assets ADD COLUMN f_number REAL;
ALTER TABLE assets ADD COLUMN exposure_time REAL;
ALTER TABLE assets ADD COLUMN focal_length REAL;
ALTER TABLE assets ADD COLUMN focal_length_35 INTEGER;

UPDATE assets SET stat_at = NULL WHERE stat_at IS NOT NULL;

-- Recently Saved orders by insert recency; the covering index keeps it
-- a pure index walk on large libraries.
CREATE INDEX idx_assets_first_seen ON assets (first_seen_at DESC);
