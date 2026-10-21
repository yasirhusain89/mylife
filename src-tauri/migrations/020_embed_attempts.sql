-- 020_embed_attempts: bound retries for semantic-search embedding.
--
-- A file that cannot be decoded writes no vector row, so the candidate query
-- (NOT EXISTS vector) offers it again on every pass. Without a bound, one
-- corrupt video makes the backfill loop forever. Rows are deleted on success,
-- so this table only ever holds currently-failing assets. Kinds are namespaced
-- ('clip-image' / 'clip-video') so a reindex can clear them without touching
-- other encoders' state.

CREATE TABLE embed_attempts (
    asset_id   INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    kind       TEXT    NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    updated_at TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (asset_id, kind)
);
