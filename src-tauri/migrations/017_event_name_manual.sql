-- 017_event_name_manual: provenance flag for event names.
--
-- cluster_events regenerates default names from local calendar dates on every
-- rebuild. rename_event sets this flag so manual names survive rebuilds and
-- the local-date backfill: rows with name_manual = 1 keep their stored name,
-- all others are regenerated. Pre-flag manual renames are detected at cluster
-- time by comparing the stored name against the legacy UTC auto-name
-- (see format_event_range_utc in db.rs); anything else-shaped keeps its name
-- and gets flagged.

ALTER TABLE events ADD COLUMN name_manual INTEGER NOT NULL DEFAULT 0;
