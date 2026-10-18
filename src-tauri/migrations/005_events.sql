-- 005_events: event clustering (time + location proximity).
--
-- Events are derived: re-clustering is always possible from the asset table.
-- name is user-settable; until renamed it is generated ("<date> at <place>").

CREATE TABLE events (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL,
    started_at  REAL    NOT NULL,
    ended_at    REAL    NOT NULL,
    latitude    REAL,
    longitude   REAL,
    place       TEXT,
    asset_count INTEGER NOT NULL DEFAULT 0,
    face_count  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);

CREATE TABLE event_assets (
    event_id  INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
    asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    PRIMARY KEY (event_id, asset_id)
);

CREATE INDEX idx_event_assets_asset ON event_assets (asset_id);
CREATE INDEX idx_events_started_at  ON events (started_at);

