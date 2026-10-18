-- 003_cleanup: exact-duplicate detection and image-quality flagging.
--
-- Duplicate detection keys on (sha256, size_bytes): identical bytes are the only
-- reliable "this is literally the same file" signal, and size_bucketed() keeps
-- the candidate set small so only plausible duplicates are ever hashed.
--
-- quality_score is a Laplacian-variance sharpness score (higher is sharper);
-- blurred is the derived flag written alongside it.

CREATE TABLE duplicate_groups (
    id            INTEGER PRIMARY KEY,
    sha256        TEXT    NOT NULL,
    size_bytes    INTEGER NOT NULL,
    kept_id       INTEGER REFERENCES assets(id) ON DELETE CASCADE,
    waste_bytes   INTEGER NOT NULL DEFAULT 0,
    resolved      INTEGER NOT NULL DEFAULT 0 CHECK (resolved IN (0, 1)),
    created_at    INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_dup_groups_sha ON duplicate_groups (sha256, size_bytes);

ALTER TABLE assets ADD COLUMN duplicate_group_id INTEGER REFERENCES duplicate_groups(id) ON DELETE SET NULL;
ALTER TABLE assets ADD COLUMN is_duplicate    INTEGER NOT NULL DEFAULT 0 CHECK (is_duplicate IN (0, 1));
-- Hash bookkeeping mirrors stat_*: a file that cannot be read must not be
-- re-hashed on every pass.
ALTER TABLE assets ADD COLUMN hash_at        INTEGER;
ALTER TABLE assets ADD COLUMN hash_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE assets ADD COLUMN hash_error    TEXT;

ALTER TABLE assets ADD COLUMN quality_score   REAL;
ALTER TABLE assets ADD COLUMN blurred         INTEGER NOT NULL DEFAULT 0 CHECK (blurred IN (0, 1));
ALTER TABLE assets ADD COLUMN quality_at      INTEGER;

CREATE INDEX idx_assets_dup_group ON assets (duplicate_group_id) WHERE duplicate_group_id IS NOT NULL;
CREATE INDEX idx_assets_blurred    ON assets (blurred) WHERE blurred = 1;
CREATE INDEX idx_assets_sha_size   ON assets (sha256, size_bytes) WHERE sha256 IS NOT NULL;
CREATE INDEX idx_assets_hash_pending ON assets (size_bytes) WHERE sha256 IS NULL;
