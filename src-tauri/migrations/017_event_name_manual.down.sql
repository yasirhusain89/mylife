-- Down migration for 017_event_name_manual: drop the name provenance flag.
-- Event names themselves are untouched; only the manual/auto distinction is
-- forgotten. The flag is re-derivable: the next cluster pass re-marks manual
-- names via the legacy-UTC auto-name comparison (see db.rs).

ALTER TABLE events DROP COLUMN name_manual;
