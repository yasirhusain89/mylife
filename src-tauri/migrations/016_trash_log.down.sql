-- Down migration for 016_trash_log: drop the Recently Deleted history.
-- Library rows are untouched; only the restore list is forgotten.

DROP TABLE trash_log;
