-- 011_near_dupes: perceptual-hash near-duplicate detection.
--
-- Exact dupes key on sha256; near-dupes (re-saves, crops, resizes) key on a
-- 64-bit dHash with Hamming distance <= 8. Groups are derived like
-- duplicate_groups: re-running the pass rebuilds them. Membership mirrors
-- the exact system (asset columns) so the review UI stays symmetric.

ALTER TABLE assets ADD COLUMN dhash          TEXT;
ALTER TABLE assets ADD COLUMN dhash_at       INTEGER;
ALTER TABLE assets ADD COLUMN dhash_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE assets ADD COLUMN dhash_error    TEXT;

CREATE TABLE near_groups (
    id          INTEGER PRIMARY KEY,
    kept_id     INTEGER REFERENCES assets(id) ON DELETE CASCADE,
    waste_bytes INTEGER NOT NULL DEFAULT 0,
    resolved    INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
    created_at  INTEGER NOT NULL
);

ALTER TABLE assets ADD COLUMN near_group_id     INTEGER REFERENCES near_groups(id) ON DELETE SET NULL;
ALTER TABLE assets ADD COLUMN is_near_duplicate INTEGER NOT NULL DEFAULT 0 CHECK (is_near_duplicate IN (0, 1));

CREATE INDEX idx_assets_near_group ON assets (near_group_id) WHERE near_group_id IS NOT NULL;
CREATE INDEX idx_assets_dhash_pending ON assets (id) WHERE dhash IS NULL AND kind = 'image' AND missing = 0;
