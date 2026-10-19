-- Down migration for 008_person_names: drop the cluster name column.
-- Face rows and cluster membership are untouched; re-running
-- `cluster_faces` recreates unnamed clusters.

ALTER TABLE face_clusters DROP COLUMN name;
