-- Down migration for 015_exposure: drop the exposure columns.
-- Already re-statted rows keep their other stat fields; exposure data is
-- re-derivable from the files on disk via the next enrich pass.

ALTER TABLE assets DROP COLUMN focal_length_35;
ALTER TABLE assets DROP COLUMN focal_length;
ALTER TABLE assets DROP COLUMN exposure_time;
ALTER TABLE assets DROP COLUMN f_number;
ALTER TABLE assets DROP COLUMN iso;
