-- 009_scene_tags: Vision scene-classifier labels per asset.
--
-- One row per (asset, tag): mountains, flowers, birds and the ~1000 other
-- everyday categories from VNClassifyImageRequest. Derived data like faces:
-- re-running `tag_scenes` refills it. Cascade keeps deletes consistent.

CREATE TABLE asset_tags (
    asset_id   INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    tag        TEXT    NOT NULL,
    confidence REAL    NOT NULL,
    PRIMARY KEY (asset_id, tag)
);

CREATE INDEX idx_asset_tags_tag ON asset_tags (tag);
