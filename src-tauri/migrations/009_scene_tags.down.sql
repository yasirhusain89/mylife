-- Down migration for 009_scene_tags: drop the scene-label cache.
-- Re-running `tag_scenes` rebuilds it from the images on disk.

DROP TABLE IF EXISTS asset_tags;
