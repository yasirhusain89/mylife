-- User-curated albums: named, hand-picked photo lists usable as export
-- sources (audience groups for Drive / WhatsApp), independent of the
-- auto-clustered events.
CREATE TABLE albums (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

-- Membership is a plain ordered list: an asset can live in several albums
-- at once, and album order is insertion order (rowid ordering of the PK).
-- Cascades keep membership in sync with album and asset deletions.
CREATE TABLE album_assets (
    album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (album_id, asset_id)
);

CREATE INDEX idx_album_assets_asset ON album_assets(asset_id);
