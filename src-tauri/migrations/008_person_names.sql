-- 008_person_names: human names for face clusters (people).
--
-- Clusters stay append-only (re-running cluster_faces adds rows, never
-- wipes), so names survive rescans. Merging moves faces between clusters.

ALTER TABLE face_clusters ADD COLUMN name TEXT;
