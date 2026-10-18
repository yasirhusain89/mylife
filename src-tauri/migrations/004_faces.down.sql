-- Rollback for 004_faces — archive, don't destroy.
ALTER TABLE faces RENAME TO faces_004_bak;
ALTER TABLE face_clusters RENAME TO face_clusters_004_bak;
