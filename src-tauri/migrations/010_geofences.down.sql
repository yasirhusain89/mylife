-- Down migration for 010_geofences: drop the bounding-box columns and
-- index. Exact-cell caching keeps working; containment reuse stops until
-- the columns come back.

DROP INDEX IF EXISTS idx_geocode_cache_bbox;
ALTER TABLE geocode_cache DROP COLUMN max_lon;
ALTER TABLE geocode_cache DROP COLUMN min_lon;
ALTER TABLE geocode_cache DROP COLUMN max_lat;
ALTER TABLE geocode_cache DROP COLUMN min_lat;
