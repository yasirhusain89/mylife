-- Down migration for 014_face_descriptors: drop the descriptor column.
-- Face rows and cluster membership are untouched; re-running
-- `cluster_faces` re-embeds descriptors from the images on disk.

ALTER TABLE faces DROP COLUMN descriptor;
