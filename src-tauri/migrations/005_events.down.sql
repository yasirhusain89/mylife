-- Rollback for 005_events — archive, don't destroy.
ALTER TABLE event_assets RENAME TO event_assets_005_bak;
ALTER TABLE events RENAME TO events_005_bak;
