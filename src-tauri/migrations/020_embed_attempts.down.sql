-- Down migration for 020_embed_attempts — archive, don't destroy.
-- The table is live state (retry counters), not derived content, but the
-- archive keeps the down path reversible without a data-loss surprise.

ALTER TABLE embed_attempts RENAME TO embed_attempts_020_bak;
