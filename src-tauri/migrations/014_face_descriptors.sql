-- 014_face_descriptors: persist Vision feature-print descriptors for global regrouping.
--
-- One BLOB per detected face: little-endian f32 bytes (base64 on the wire,
-- same encoding as asset_vectors.vec). Nullable so pre-migration rows stay
-- NULL; the next cluster_faces run re-embeds those images incrementally.
-- Groups remain derived data: replace_clusters() rebuilds them from these
-- descriptors, preserving user names via mutual-majority overlap.

ALTER TABLE faces ADD COLUMN descriptor BLOB;
