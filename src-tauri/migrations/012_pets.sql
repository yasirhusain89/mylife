-- 012_pets: cat/dog sightings from VNRecognizeAnimalsRequest.
--
-- One row per detected animal (bounding box + label), mirroring faces but
-- deliberately NOT clustered: pet identity tracking is a follow-up, this
-- pass only records "a dog/cat is in this photo". Derived data like faces:
-- re-running `detect_pets` refills it.

CREATE TABLE animal_sightings (
    id          INTEGER PRIMARY KEY,
    path        TEXT    NOT NULL,
    label       TEXT    NOT NULL,
    x           REAL    NOT NULL,
    y           REAL    NOT NULL,
    width       REAL    NOT NULL,
    height      REAL    NOT NULL,
    confidence  REAL    NOT NULL,
    detected_at INTEGER NOT NULL
);

CREATE INDEX idx_sightings_path  ON animal_sightings (path);
CREATE INDEX idx_sightings_label ON animal_sightings (label);
