-- Down migration for 012_pets: drop the sightings. Re-running
-- `detect_pets` rebuilds them from the images on disk.

DROP TABLE IF EXISTS animal_sightings;
