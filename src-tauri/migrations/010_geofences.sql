-- 010_geofences: bounding boxes on the place cache turn cached rows into
-- fences. A later lookup first tries its exact 100m cell, then containment
-- in any cached box (smallest wins), and only then hits the network — so
-- photos near an already-resolved spot never re-pull.
--
-- Nullable because rows cached before this migration have no box; they
-- keep working as exact-cell entries.

ALTER TABLE geocode_cache ADD COLUMN min_lat REAL;
ALTER TABLE geocode_cache ADD COLUMN max_lat REAL;
ALTER TABLE geocode_cache ADD COLUMN min_lon REAL;
ALTER TABLE geocode_cache ADD COLUMN max_lon REAL;

CREATE INDEX idx_geocode_cache_bbox ON geocode_cache (min_lat, max_lat, min_lon, max_lon);
