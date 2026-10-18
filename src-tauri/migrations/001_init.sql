-- 001_init: photo/video library index.
--
-- One row per media file on disk. `path` is the natural key (a file lives at one
-- place); `sha256` is the dedupe key and stays NULL until the hash pass runs,
-- so partial unique indexes let many un-hashed rows coexist.
--
-- migrate:down — see 001_init.down.sql (renames tables to *_001_bak).

CREATE TABLE scans (
    id          INTEGER PRIMARY KEY,
    root        TEXT    NOT NULL,
    recursive   INTEGER NOT NULL DEFAULT 1,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    scanned     INTEGER NOT NULL DEFAULT 0,
    imported    INTEGER NOT NULL DEFAULT 0,
    unchanged   INTEGER NOT NULL DEFAULT 0,
    bytes       INTEGER NOT NULL DEFAULT 0,
    error       TEXT
);

CREATE TABLE assets (
    id           INTEGER PRIMARY KEY,
    path         TEXT    NOT NULL UNIQUE,
    name         TEXT    NOT NULL,
    ext          TEXT    NOT NULL DEFAULT '',
    kind         TEXT    NOT NULL CHECK (kind IN ('image', 'video')),
    mime         TEXT,
    size_bytes   INTEGER NOT NULL DEFAULT 0,
    modified     REAL    NOT NULL DEFAULT 0,
    captured_at  REAL,
    width        INTEGER,
    height       INTEGER,
    duration     REAL,
    camera_make  TEXT,
    camera_model TEXT,
    lens         TEXT,
    latitude     REAL,
    longitude    REAL,
    sha256       TEXT,
    favorite     INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    missing      INTEGER NOT NULL DEFAULT 0 CHECK (missing IN (0, 1)),
    scan_id      INTEGER REFERENCES scans(id) ON DELETE SET NULL,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    stat_at       INTEGER
);

CREATE INDEX idx_assets_kind_captured ON assets (kind, captured_at DESC);
CREATE INDEX idx_assets_captured       ON assets (captured_at DESC);
CREATE INDEX idx_assets_sha256         ON assets (sha256) WHERE sha256 IS NOT NULL;
CREATE INDEX idx_assets_missing        ON assets (missing) WHERE missing = 1;
CREATE INDEX idx_assets_scan           ON assets (scan_id);
