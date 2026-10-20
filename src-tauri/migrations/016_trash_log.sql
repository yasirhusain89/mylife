-- 016_trash_log: Recently Deleted history for trashed assets.
--
-- trash_assets moves files to the macOS Trash and flags rows missing; this
-- log remembers what went when so the UI can list and restore beyond the
-- session-undo window. One row per asset (re-trash refreshes the stamp).
-- Emptying the macOS Trash leaves stale rows: restore then fails with the
-- existing plain-language reason, and Clear list forgets them.

CREATE TABLE trash_log (
    asset_id   INTEGER PRIMARY KEY,
    path       TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    trashed_at INTEGER NOT NULL
);
