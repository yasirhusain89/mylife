-- Down migration for 018_review_decisions — archive, don't destroy.
-- Dismissed suggestions may resurface in the queue; the archive keeps the
-- "already said separate" answers recoverable.

ALTER TABLE review_decisions RENAME TO review_decisions_018_bak;
