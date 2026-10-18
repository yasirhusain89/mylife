-- 004_faces: person clustering from Vision feature prints.
--
-- One row per detected face; faces are grouped into clusters (people) by the
-- engine's feature-print distance. Groups are derived data: they can be
-- recomputed by re-running cluster_faces.

CREATE TABLE face_clusters (
    id            INTEGER PRIMARY KEY,
    face_count    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL
);

CREATE TABLE faces (
    id            INTEGER PRIMARY KEY,
    path          TEXT    NOT NULL,
    cluster_id    INTEGER REFERENCES face_clusters(id) ON DELETE SET NULL,
    x             REAL    NOT NULL,
    y             REAL    NOT NULL,
    width         REAL    NOT NULL,
    height        REAL    NOT NULL,
    confidence    REAL    NOT NULL,
    detected_at   INTEGER NOT NULL
);

CREATE INDEX idx_faces_cluster ON faces (cluster_id);
CREATE INDEX idx_faces_path    ON faces (path);

