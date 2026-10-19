-- Down migration for 013_vectors: drop the embeddings. Re-running
-- `embed_library` rebuilds them from the images on disk.

DROP TABLE IF EXISTS asset_vectors;
