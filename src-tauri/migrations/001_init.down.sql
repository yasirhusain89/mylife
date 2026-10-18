-- Rollback for 001_init — archive, don't destroy.
--
-- Tables are renamed to *_001_bak rather than dropped: rollback keeps every row
-- recoverable, and re-applying 001_init recreates empty tables under the
-- original names (the _bak copies can be dropped by hand once the rescan that
-- follows a rollback has been verified).

ALTER TABLE assets RENAME TO assets_001_bak;
ALTER TABLE scans RENAME TO scans_001_bak;
