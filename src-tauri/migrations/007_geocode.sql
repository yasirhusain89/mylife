-- 007_geocode: on-disk cache for reverse-geocoded place names.
--
-- One row per ~100m coordinate cell (3-decimal rounding): event centroids
-- that fall in the same cell share one lookup, so re-clustering and repeat
-- runs never hit the network twice for the same area. The network itself
-- (Nominatim reverse-geocode, the single allowed exception to the
-- on-device-only rule) happens in the `resolve_places` command.

CREATE TABLE geocode_cache (
    lat_key    REAL    NOT NULL,
    lon_key    REAL    NOT NULL,
    place      TEXT    NOT NULL,
    fetched_at INTEGER NOT NULL,
    PRIMARY KEY (lat_key, lon_key)
);
