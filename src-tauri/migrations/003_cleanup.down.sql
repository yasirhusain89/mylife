-- Rollback for 003_cleanup — archive, don't destroy.
--
-- duplicate_groups is renamed (not dropped) so the detected grouping survives a
-- rollback for inspection. The added asset columns cannot be dropped without a
-- full table rebuild (SQLite forbids dropping columns an index depends on), so
-- assets is archived and recreated without them.

ALTER TABLE duplicate_groups RENAME TO duplicate_groups_003_bak;

ALTER TABLE assets RENAME TO assets_003_bak;

CREATE TABLE assets (
    id            INTEGER PRIMARY KEY,
    path          TEXT    NOT NULL UNIQUE,
    name          TEXT    NOT NULL,
    ext           TEXT    NOT NULL DEFAULT '',
    kind          TEXT    NOT NULL CHECK (kind IN ('image', 'video')),
    mime          TEXT,
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    modified      REAL    NOT NULL DEFAULT 0,
    captured_at   REAL,
    width         INTEGER,
    height        INTEGER,
    duration      REAL,
    camera_make   TEXT,
    camera_model  TEXT,
    lens          TEXT,
    latitude      REAL,
    longitude     REAL,
    sha256        TEXT,
    favorite      INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    missing       INTEGER NOT NULL DEFAULT 0 CHECK (missing IN (0, 1)),
    scan_id       INTEGER REFERENCES scans(id) ON DELETE SET NULL,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    stat_at       INTEGER,
    stat_attempts INTEGER NOT NULL DEFAULT 0,
    stat_error    TEXT
);

INSERT INTO assets SELECT
    id, path, name, ext, kind, mime, size_bytes, modified, captured_at, width,
    height, duration, camera_make, camera_model, lens, latitude, longitude,
    sha256, favorite, missing, scan_id, first_seen_at, last_seen_at, stat_at,
    stat_attempts, stat_error
FROM assets_003_bak;

CREATE INDEX idx_assets_kind_captured ON assets (kind, captured_at DESC);
CREATE INDEX idx_assets_captured       ON assets (captured_at DESC);
CREATE INDEX idx_assets_sha256         ON assets (sha256) WHERE sha256 IS NOT NULL;
CREATE INDEX idx_assets_missing        ON assets (missing) WHERE missing = 1;
CREATE INDEX idx_assets_scan           ON assets (scan_id);
CREATE INDEX idx_assets_stat_pending   ON assets (stat_at) WHERE stat_at IS NULL;
