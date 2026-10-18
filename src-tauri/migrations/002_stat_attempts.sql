-- 002_stat_attempts: track metadata-enrichment attempts per asset.
--
-- A failed `stat` is usually temporary (an iCloud placeholder that has not been
-- downloaded yet, or a file still being written). Without attempt bookkeeping a
-- resumable enrichment pass would spin forever on those rows, and a permanently
-- unreadable file would look identical to a healthy one.

ALTER TABLE assets ADD COLUMN stat_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE assets ADD COLUMN stat_error TEXT;

-- Keep the pending-statement index aligned with the new bookkeeping.
CREATE INDEX idx_assets_stat_pending ON assets (stat_at) WHERE stat_at IS NULL;
