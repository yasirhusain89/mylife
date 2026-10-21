-- 018_review_decisions: persistent dismissals for the People review queue.
--
-- The queue proposes near-duplicate groups (cluster pairs whose closest faces
-- sit just above the union-find threshold) and lone faces near a named person.
-- Accepting merges/assigns via the existing commands; rejecting records the
-- bridge face pair here so the same suggestion never resurfaces. Face ids are
-- stable across re-clustering (only cluster ids churn), so pairs stay valid;
-- orphaned rows from deleted faces are harmless and never match.

CREATE TABLE review_decisions (
    face_a     INTEGER NOT NULL,
    face_b     INTEGER NOT NULL,
    decision   TEXT    NOT NULL CHECK (decision IN ('separate')),
    decided_at INTEGER NOT NULL,
    PRIMARY KEY (face_a, face_b)
);
