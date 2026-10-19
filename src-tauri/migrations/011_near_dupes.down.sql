-- Down migration for 011_near_dupes: drop the perceptual-hash columns and
-- the near-duplicate groups. Exact-duplicate data is untouched; re-running
-- the near-dupe pass rebuilds everything from the images on disk.

DROP TABLE IF EXISTS near_groups;
ALTER TABLE assets DROP COLUMN is_near_duplicate;
ALTER TABLE assets DROP COLUMN near_group_id;
ALTER TABLE assets DROP COLUMN dhash_error;
ALTER TABLE assets DROP COLUMN dhash_attempts;
ALTER TABLE assets DROP COLUMN dhash_at;
ALTER TABLE assets DROP COLUMN dhash;
