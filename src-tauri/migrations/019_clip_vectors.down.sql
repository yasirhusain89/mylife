-- Down migration for 019_clip_vectors — archive, don't destroy.
-- Semantic vectors and keyframes are re-derivable from the files on disk
-- (embed backfill), so dropping the tables only forgets cached work; the
-- archive keeps it recoverable.

ALTER TABLE clip_vectors RENAME TO clip_vectors_019_bak;
ALTER TABLE video_keyframes RENAME TO video_keyframes_019_bak;
