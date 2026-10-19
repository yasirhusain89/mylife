-- Down migration for 007_geocode: drop the place-name cache.
-- Event rows keep their already-resolved `place` text; only the lookup
-- cache is removed, so re-running `resolve_places` re-fetches over the
-- network instead of reading cached rows.

DROP TABLE IF EXISTS geocode_cache;
