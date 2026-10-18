-- Rollback for 002_stat_attempts — archive, don't destroy.
--
-- SQLite cannot drop a column that an index or constraint depends on without
-- rebuilding the table, so the enriched table is archived whole and a clean
-- copy is recreated from the pre-002 columns. Every row is re-derivable by
-- rescanning, so archiving loses no source data.

ALTER TABLE assets RENAME TO assets_002_bak;

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
    stat_at       INTEGER
);

INSERT INTO assets SELECT * FROM assets_002_bak;

CREATE INDEX idx_assets_kind_captured ON assets (kind, captured_at DESC);
CREATE INDEX idx_assets_captured       ON assets (captured_at DESC);
CREATE INDEX idx_assets_sha256         ON assets (sha256) WHERE sha256 IS NOT NULL;
CREATE INDEX idx_assets_missing        ON assets (missing) WHERE missing = 1;
CREATE INDEX idx_assets_scan           ON assets (scan_id);
