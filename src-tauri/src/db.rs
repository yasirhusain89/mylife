// SQLite library index. Schema lives in ../migrations/*.sql and is embedded at
// compile time so bundled builds need no external files.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_init.sql")),
    (2, include_str!("../migrations/002_stat_attempts.sql")),
    (3, include_str!("../migrations/003_cleanup.sql")),
    (4, include_str!("../migrations/004_faces.sql")),
];

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
    // WAL keeps reads fast while an import writes.
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.pragma_update(None, "foreign_keys", "ON").ok();
    conn.pragma_update(None, "synchronous", "NORMAL").ok();
    Ok(conn)
}

/// Applies every migration above the stored `user_version`. Returns the versions applied.
pub fn migrate(conn: &Connection) -> Result<Vec<i64>, String> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
    let mut applied = Vec::new();
    for (version, sql) in MIGRATIONS {
        if *version <= current {
            continue;
        }
        conn.execute_batch(sql).map_err(|e| format!("migration {version}: {e}"))?;
        conn.pragma_update(None, "user_version", *version)
            .map_err(|e| format!("migration {version} version bump: {e}"))?;
        applied.push(*version);
    }
    Ok(applied)
}

// MARK: - Row types

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub ext: String,
    pub kind: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub modified: f64,
    pub captured_at: Option<f64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<f64>,
    pub favorite: i64,
    pub missing: i64,
    pub sha256: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub photos: i64,
    pub videos: i64,
    pub bytes: i64,
    pub missing: i64,
    pub hashed: i64,
}

#[derive(Default)]
pub struct ScanCounters {
    pub scanned: i64,
    pub imported: i64,
    pub unchanged: i64,
    pub bytes: i64,
}

// MARK: - Scans

pub fn begin_scan(conn: &Connection, root: &str, recursive: bool) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO scans (root, recursive, started_at) VALUES (?1, ?2, ?3)",
        params![root, i64::from(recursive), now()],
    )
    .map_err(|e| format!("begin scan: {e}"))?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_scan(
    conn: &Connection,
    scan_id: i64,
    c: &ScanCounters,
    error: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE scans SET finished_at = ?2, scanned = ?3, imported = ?4, unchanged = ?5, bytes = ?6, error = ?7
         WHERE id = ?1",
        params![scan_id, now(), c.scanned, c.imported, c.unchanged, c.bytes, error],
    )
    .map_err(|e| format!("finish scan: {e}"))?;
    Ok(())
}

// MARK: - Assets

/// Upserts one scanned asset. Returns true when the row was created or changed.
pub fn upsert_asset(conn: &Connection, a: &Value2, scan_id: i64) -> Result<bool, String> {
    let existing: Option<(i64, i64, f64)> = conn
        .query_row(
            "SELECT id, size_bytes, modified FROM assets WHERE path = ?1",
            params![a.path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("lookup asset: {e}"))?;

    let ts = now();
    if let Some((id, size, modified)) = existing {
        // Same bytes at the same mtime: only bump the sighting, keep metadata.
        if size == a.size_bytes && (modified - a.modified).abs() < 0.001 {
            conn.execute(
                "UPDATE assets SET last_seen_at = ?2, missing = 0 WHERE id = ?1",
                params![id, ts],
            )
            .map_err(|e| format!("touch asset: {e}"))?;
            return Ok(false);
        }
        conn.execute(
            "UPDATE assets SET name = ?2, ext = ?3, kind = ?4, mime = ?5, size_bytes = ?6,
                    modified = ?7, missing = 0, last_seen_at = ?8, scan_id = ?9
             WHERE id = ?1",
            params![id, a.name, a.ext, a.kind, a.mime, a.size_bytes, a.modified, ts, scan_id],
        )
        .map_err(|e| format!("update asset: {e}"))?;
        return Ok(true);
    }

    conn.execute(
        "INSERT INTO assets (path, name, ext, kind, mime, size_bytes, modified,
                             scan_id, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![a.path, a.name, a.ext, a.kind, a.mime, a.size_bytes, a.modified, scan_id, ts],
    )
    .map_err(|e| format!("insert asset: {e}"))?;
    Ok(true)
}

/// Owned copy of a scanned asset, decoupled from serde_json lifetimes.
pub struct Value2 {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub kind: String,
    pub mime: Option<String>,
    pub size_bytes: i64,
    pub modified: f64,
}

pub fn asset_from_json(v: &serde_json::Value) -> Result<Value2, String> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let path = s("path");
    if path.is_empty() {
        return Err("scanned asset missing path".into());
    }
    let name = {
        let n = s("name");
        if n.is_empty() {
            path.rsplit('/').next().unwrap_or(&path).to_string()
        } else {
            n
        }
    };
    Ok(Value2 {
        ext: s("ext").to_lowercase(),
        kind: s("kind"),
        mime: v.get("mime").and_then(|x| x.as_str()).map(str::to_string),
        size_bytes: v.get("sizeBytes").and_then(|x| x.as_i64()).unwrap_or(0),
        modified: v.get("modified").and_then(|x| x.as_f64()).unwrap_or(0.0),
        path,
        name,
    })
}


// MARK: - Faces & clusters

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRow {
    pub id: i64,
    pub path: String,
    pub cluster_id: Option<i64>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub confidence: f64,
    pub detected_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceClusterRow {
    pub id: i64,
    pub face_count: i64,
    pub representative_path: Option<String>,
}

pub fn save_faces(conn: &Connection, path: &str, faces: &[serde_json::Value]) -> Result<Vec<i64>, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("faces tx: {e}"))?;
    let mut ids = Vec::new();
    for (i, f) in faces.iter().enumerate() {
        tx.execute(
                "INSERT INTO faces (path, cluster_id, x, y, width, height, confidence, detected_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    path,
                    f.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    now(),
                ],
            )
            .map_err(|e| format!("insert face: {e}"))?;
        let id: i64 = tx.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .map_err(|e| format!("last row id: {e}"))?;
        ids.push(id);
    }
    tx.commit().map_err(|e| format!("faces commit: {e}"))?;
    Ok(ids)
}

pub fn save_cluster(
    conn: &Connection,
    member_ids: &[i64],
) -> Result<i64, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("cluster tx: {e}"))?;
    let cluster_id = tx
        .execute(
            "INSERT INTO face_clusters (face_count, created_at) VALUES (?1, ?2)",
            params![member_ids.len() as i64, now()],
        )
        .map_err(|e| format!("insert cluster: {e}"))?;
    let cluster_id = tx.last_insert_rowid();
    for id in member_ids {
        tx.execute(
            "UPDATE faces SET cluster_id = ?2 WHERE id = ?1",
            params![id, cluster_id],
        )
        .map_err(|e| format!("link face: {e}"))?;
    }
    tx.commit().map_err(|e| format!("cluster commit: {e}"))?;
    Ok(cluster_id)
}

pub fn clusters_list(conn: &Connection) -> Result<Vec<FaceClusterRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT fc.id, fc.face_count,
                    (SELECT path FROM faces WHERE cluster_id = fc.id ORDER BY confidence DESC LIMIT 1)
             FROM face_clusters fc
             ORDER BY fc.face_count DESC, fc.id",
        )
        .map_err(|e| format!("prepare clusters: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FaceClusterRow {
                id: r.get(0)?,
                face_count: r.get(1)?,
                representative_path: r.get(2)?,
            })
        })
        .map_err(|e| format!("query clusters: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("cluster row: {e}"))?);
    }
    Ok(out)
}

pub fn faces_in_cluster(conn: &Connection, cluster_id: i64) -> Result<Vec<FaceRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, cluster_id, x, y, width, height, confidence, detected_at
             FROM faces WHERE cluster_id = ?1 ORDER BY confidence DESC",
        )
        .map_err(|e| format!("prepare faces: {e}"))?;
    let rows = stmt
        .query_map(params![cluster_id], |r| {
            Ok(FaceRow {
                id: r.get(0)?,
                path: r.get(1)?,
                cluster_id: r.get(2)?,
                x: r.get(3)?,
                y: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                confidence: r.get(7)?,
                detected_at: r.get(8)?,
            })
        })
        .map_err(|e| format!("query faces: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("face row: {e}"))?);
    }
    Ok(out)
}

pub fn faces_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM faces", [], |r| r.get(0))
        .map_err(|e| format!("faces_count: {e}"))
}

// MARK: - Queries

// MARK: - Metadata enrichment

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichSummary {
    pub processed: i64,
    pub enriched: i64,
    pub failed: i64,
    pub pending: i64,
    pub elapsed: f64,
}

/// Assets that have never been successfully enriched. Rows that exhausted
/// `max_attempts` are skipped so a resumable pass cannot spin on them.
pub fn pending_stat(conn: &Connection, limit: i64, max_attempts: i64) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM assets
             WHERE stat_at IS NULL AND missing = 0 AND stat_attempts < ?1
             ORDER BY (captured_at IS NULL), captured_at DESC, id
             LIMIT ?2",
        )
        .map_err(|e| format!("prepare pending_stat: {e}"))?;
    let rows = stmt
        .query_map(params![max_attempts, limit], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query pending_stat: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("pending_stat row: {e}"))?);
    }
    Ok(out)
}

pub fn apply_stat(conn: &Connection, id: i64, stat: &serde_json::Value) -> Result<(), String> {
    let n = |k: &str| stat.get(k).and_then(|v| v.as_i64());
    let f = |k: &str| stat.get(k).and_then(|v| v.as_f64());
    let s = |k: &str| stat.get(k).and_then(|v| v.as_str());

    conn.execute(
        "UPDATE assets SET
            captured_at  = COALESCE(?2, captured_at),
            width        = COALESCE(?3, width),
            height       = COALESCE(?4, height),
            duration     = COALESCE(?5, duration),
            camera_make  = COALESCE(?6, camera_make),
            camera_model = COALESCE(?7, camera_model),
            lens         = COALESCE(?8, lens),
            latitude     = COALESCE(?9, latitude),
            longitude    = COALESCE(?10, longitude),
            stat_at      = ?11,
            stat_error   = NULL,
            stat_attempts = stat_attempts + 1
         WHERE id = ?1",
        params![
            id,
            f("capturedAt"),
            n("width"),
            n("height"),
            f("duration"),
            s("cameraMake"),
            s("cameraModel"),
            s("lens"),
            f("latitude"),
            f("longitude"),
            now(),
        ],
    )
    .map_err(|e| format!("apply_stat: {e}"))?;
    Ok(())
}

/// Records a failed attempt. The row keeps `stat_at IS NULL` so it stays
/// eligible for a later retry until `max_attempts` is reached.
pub fn apply_stat_failure(conn: &Connection, id: i64, error: &str) -> Result<(), String> {
    let message: String = error.chars().take(200).collect();
    conn.execute(
        "UPDATE assets SET stat_error = ?2, stat_attempts = stat_attempts + 1 WHERE id = ?1",
        params![id, message],
    )
    .map_err(|e| format!("apply_stat_failure: {e}"))?;
    Ok(())
}

pub fn enrich_counts(conn: &Connection, max_attempts: i64) -> Result<(i64, i64), String> {
    let row = conn
        .query_row(
            "SELECT
                COALESCE(SUM(stat_at IS NULL AND missing = 0 AND stat_attempts < ?1), 0),
                COALESCE(SUM(stat_at IS NULL AND stat_attempts >= ?1), 0)
             FROM assets",
            params![max_attempts],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| format!("enrich_counts: {e}"))?;
    Ok(row)
}


// MARK: - Deduplication

/// Size buckets: a duplicate must have identical size, so only assets sharing a
/// size with someone else can possibly be a duplicate. Hashing is the expensive
/// step, so it runs only on assets in a populated bucket (rounded to 4 KiB so
/// near-identical exports still land together).
pub fn size_buckets(conn: &Connection) -> Result<Vec<i64>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT (size_bytes / 4096) * 4096 AS bucket
             FROM assets
             WHERE missing = 0 AND size_bytes > 0 AND sha256 IS NULL
             GROUP BY bucket
             HAVING COUNT(*) > 1
             ORDER BY bucket",
        )
        .map_err(|e| format!("prepare size_buckets: {e}"))?;
    let rows = stmt
        .query_map([], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("query size_buckets: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("size_buckets row: {e}"))?);
    }
    Ok(out)
}

/// Unhashed assets whose size falls in a candidate bucket.
pub fn hash_candidates(conn: &Connection, buckets: &[i64], limit: i64) -> Result<Vec<(i64, String)>, String> {
    if buckets.is_empty() {
        return Ok(Vec::new());
    }
    let list = buckets.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT id, path FROM assets
         WHERE sha256 IS NULL AND missing = 0 AND hash_attempts < 3
           AND (size_bytes / 4096) * 4096 IN ({list})
         ORDER BY size_bytes, id
         LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare hash_candidates: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query hash_candidates: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("hash_candidates row: {e}"))?);
    }
    Ok(out)
}

pub fn apply_hash(conn: &Connection, id: i64, sha256: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE assets SET sha256 = ?2, hash_at = ?3, hash_error = NULL,
                hash_attempts = hash_attempts + 1
         WHERE id = ?1",
        params![id, sha256, now()],
    )
    .map_err(|e| format!("apply_hash: {e}"))?;
    Ok(())
}

pub fn apply_hash_failure(conn: &Connection, id: i64, error: &str) -> Result<(), String> {
    let message: String = error.chars().take(200).collect();
    conn.execute(
        "UPDATE assets SET hash_error = ?2, hash_attempts = hash_attempts + 1 WHERE id = ?1",
        params![id, message],
    )
    .map_err(|e| format!("apply_hash_failure: {e}"))?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub waste_bytes: i64,
    pub kept_id: Option<i64>,
    pub members: Vec<DuplicateMember>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateMember {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub is_duplicate: bool,
    pub captured_at: Option<f64>,
    pub quality_score: Option<f64>,
}

/// Rebuilds duplicate groups from hashed assets.
///
/// The keeper is chosen deterministically: a favourite wins, then the best
/// quality score, then the earliest capture date, then the lowest id. Nothing is
/// deleted here — grouping only records which copies are redundant.
pub fn rebuild_duplicate_groups(conn: &Connection) -> Result<(i64, i64), String> {
    let groups: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare(
                "SELECT sha256, size_bytes FROM assets
                 WHERE sha256 IS NOT NULL AND missing = 0
                 GROUP BY sha256, size_bytes HAVING COUNT(*) > 1",
            )
            .map_err(|e| format!("prepare dup groups: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| format!("query dup groups: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("dup group row: {e}"))?);
        }
        v
    };

    let tx = conn.unchecked_transaction().map_err(|e| format!("dup tx: {e}"))?;
    // Groups are derived state: clear flags and rows together so repeated runs
    // stay idempotent (the unique index on sha256+size_bytes would otherwise
    // reject a second rebuild).
    tx.execute("UPDATE assets SET duplicate_group_id = NULL, is_duplicate = 0", [])
        .map_err(|e| format!("reset duplicate flags: {e}"))?;
    tx.execute("DELETE FROM duplicate_groups", [])
        .map_err(|e| format!("clear duplicate groups: {e}"))?;

    let mut group_count = 0i64;
    let mut waste_total = 0i64;

    for (sha, size) in groups {
        let members: Vec<i64> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id FROM assets WHERE sha256 = ?1 AND size_bytes = ?2 AND missing = 0
                     ORDER BY favorite DESC, COALESCE(quality_score, -1.0) DESC,
                              captured_at ASC, id ASC",
                )
                .map_err(|e| format!("prepare members: {e}"))?;
            let rows = stmt
                .query_map(params![sha, size], |r| r.get::<_, i64>(0))
                .map_err(|e| format!("query members: {e}"))?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(|e| format!("member row: {e}"))?);
            }
            v
        };
        if members.len() < 2 {
            continue;
        }

        let kept = members[0];
        let waste = size * (members.len() as i64 - 1);
        tx.execute(
            "INSERT INTO duplicate_groups (sha256, size_bytes, kept_id, waste_bytes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![sha, size, kept, waste, now()],
        )
        .map_err(|e| format!("insert dup group: {e}"))?;
        let group_id = tx.last_insert_rowid();

        for id in &members {
            let is_dup = i64::from(*id != kept);
            tx.execute(
                "UPDATE assets SET duplicate_group_id = ?2, is_duplicate = ?3 WHERE id = ?1",
                params![id, group_id, is_dup],
            )
            .map_err(|e| format!("mark member: {e}"))?;
        }

        group_count += 1;
        waste_total += waste;
    }

    tx.commit().map_err(|e| format!("dup commit: {e}"))?;
    Ok((group_count, waste_total))
}

pub fn duplicate_groups(conn: &Connection) -> Result<Vec<DuplicateGroup>, String> {
    let groups: Vec<DuplicateGroup> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, sha256, size_bytes, waste_bytes, kept_id FROM duplicate_groups
                 ORDER BY waste_bytes DESC, id",
            )
            .map_err(|e| format!("prepare list dup groups: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(DuplicateGroup {
                    id: r.get(0)?,
                    sha256: r.get(1)?,
                    size_bytes: r.get(2)?,
                    waste_bytes: r.get(3)?,
                    kept_id: r.get(4)?,
                    members: Vec::new(),
                })
            })
            .map_err(|e| format!("list dup groups: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("dup group row: {e}"))?);
        }
        v
    };

    let mut out = Vec::new();
    for mut g in groups {
        let members: Vec<DuplicateMember> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, path, name, is_duplicate, captured_at, quality_score FROM assets
                     WHERE duplicate_group_id = ?1 ORDER BY is_duplicate ASC, captured_at ASC, id",
                )
                .map_err(|e| format!("prepare members: {e}"))?;
            let rows = stmt
                .query_map(params![g.id], |r| {
                    Ok(DuplicateMember {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        name: r.get(2)?,
                        is_duplicate: r.get::<_, i64>(3)? == 1,
                        captured_at: r.get(4)?,
                        quality_score: r.get(5)?,
                    })
                })
                .map_err(|e| format!("query members: {e}"))?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(|e| format!("member row: {e}"))?);
            }
            v
        };
        g.members = members;
        out.push(g);
    }
    Ok(out)
}

// MARK: - Quality

pub fn quality_candidates(conn: &Connection, limit: i64) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM assets
             WHERE kind = 'image' AND missing = 0 AND quality_at IS NULL
               AND width IS NOT NULL AND height IS NOT NULL
             ORDER BY id LIMIT ?1",
        )
        .map_err(|e| format!("prepare quality_candidates: {e}"))?;
    let rows = stmt
        .query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query quality_candidates: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("quality row: {e}"))?);
    }
    Ok(out)
}

pub fn apply_quality(conn: &Connection, id: i64, sharpness: f64, blurred: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE assets SET quality_score = ?2, blurred = ?3, quality_at = ?4 WHERE id = ?1",
        params![id, sharpness, i64::from(blurred), now()],
    )
    .map_err(|e| format!("apply_quality: {e}"))?;
    Ok(())
}

pub fn cleanup_stats(conn: &Connection) -> Result<CleanupStats, String> {
    let row = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM duplicate_groups),
                (SELECT COALESCE(SUM(waste_bytes), 0) FROM duplicate_groups),
                (SELECT COUNT(*) FROM assets WHERE blurred = 1)",
            [],
            |r| {
                Ok(CleanupStats {
                    duplicate_groups: r.get(0)?,
                    waste_bytes: r.get(1)?,
                    blurred: r.get(2)?,
                })
            },
        )
        .map_err(|e| format!("cleanup_stats: {e}"))?;
    Ok(row)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupStats {
    pub duplicate_groups: i64,
    pub waste_bytes: i64,
    pub blurred: i64,
}

// MARK: - Queries

pub fn library(
    conn: &Connection,
    kind: Option<&str>,
    blurred_only: bool,
    offset: i64,
    limit: i64,
) -> Result<Vec<AssetRow>, String> {
    let sql = "SELECT id, path, name, ext, kind, mime, size_bytes, modified, captured_at,
                      width, height, duration, favorite, missing, sha256
               FROM assets
               WHERE (?1 IS NULL OR kind = ?1) AND missing = 0
                 AND (?2 = 0 OR blurred = 1)
               ORDER BY COALESCE(captured_at, modified) DESC, id DESC
               LIMIT ?3 OFFSET ?4";
    let mut stmt = conn.prepare(sql).map_err(|e| format!("prepare library: {e}"))?;
    let rows = stmt
        .query_map(params![kind, i64::from(blurred_only), limit, offset], |r| {
            Ok(AssetRow {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                ext: r.get(3)?,
                kind: r.get(4)?,
                mime: r.get(5)?,
                size_bytes: r.get(6)?,
                modified: r.get(7)?,
                captured_at: r.get(8)?,
                width: r.get(9)?,
                height: r.get(10)?,
                duration: r.get(11)?,
                favorite: r.get(12)?,
                missing: r.get(13)?,
                sha256: r.get(14)?,
            })
        })
        .map_err(|e| format!("query library: {e}"))?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("row: {e}"))?);
    }
    Ok(out)
}

pub fn stats(conn: &Connection) -> Result<LibraryStats, String> {
    let row = conn
        .query_row(
            "SELECT
                COALESCE(SUM(kind = 'image'), 0),
                COALESCE(SUM(kind = 'video'), 0),
                COALESCE(SUM(size_bytes), 0),
                COALESCE(SUM(missing), 0),
                COALESCE(SUM(sha256 IS NOT NULL), 0)
             FROM assets",
            [],
            |r| {
                Ok(LibraryStats {
                    photos: r.get(0)?,
                    videos: r.get(1)?,
                    bytes: r.get(2)?,
                    missing: r.get(3)?,
                    hashed: r.get(4)?,
                })
            },
        )
        .map_err(|e| format!("stats: {e}"))?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str, size: i64, modified: f64) -> Value2 {
        Value2 {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            ext: "jpg".into(),
            kind: "image".into(),
            mime: Some("image/jpeg".into()),
            size_bytes: size,
            modified,
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate(&conn).unwrap(), vec![1, 2, 3, 4]);
        assert!(migrate(&conn).unwrap().is_empty(), "second call is a no-op");
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 4, "user_version tracks the highest applied migration");

        // A database already at the latest version stays untouched.
        let before: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets WHERE stat_attempts = 0", [], |r| r.get(0))
            .unwrap();
        migrate(&conn).unwrap();
        let after: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets WHERE stat_attempts = 0", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn upsert_inserts_then_reports_unchanged() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();

        assert!(upsert_asset(&conn, &asset("/a.jpg", 100, 10.0), scan).unwrap());
        // Same size + mtime is a no-op sighting.
        assert!(!upsert_asset(&conn, &asset("/a.jpg", 100, 10.0), scan).unwrap());
        // Changed mtime counts as an update.
        assert!(upsert_asset(&conn, &asset("/a.jpg", 120, 11.0), scan).unwrap());

        let (n, size): (i64, i64) =
            conn.query_row("SELECT COUNT(*), SUM(size_bytes) FROM assets", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(n, 1, "path is the natural key");
        assert_eq!(size, 120);
    }

    #[test]
    fn library_orders_by_capture_time_and_filters_kind() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/new.jpg", 1, 100.0), scan).unwrap();
        upsert_asset(&conn, &asset("/old.jpg", 1, 1.0), scan).unwrap();
        conn.execute("UPDATE assets SET captured_at = 50.0 WHERE path = '/new.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET captured_at = 900.0 WHERE path = '/old.jpg'", []).unwrap();

        let all = library(&conn, None, false, 0, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].path, "/old.jpg", "newest capture first");

        let paged = library(&conn, None, false, 1, 10).unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].path, "/new.jpg", "offset skips the newest");

        let videos = library(&conn, Some("video"), false, 0, 10).unwrap();
        assert!(videos.is_empty(), "kind filter excludes images");

        let s = stats(&conn).unwrap();
        assert_eq!(s.photos, 2);
        assert_eq!(s.videos, 0);
        assert_eq!(s.bytes, 2);
    }

    #[test]
    fn library_blurred_filter_binds_the_right_parameters() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/soft.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/sharp.jpg", 1, 2.0), scan).unwrap();
        conn.execute(
            "UPDATE assets SET quality_score = 3.0, blurred = 1 WHERE path = '/soft.jpg'",
            [],
        )
        .unwrap();

        let blurred_rows = library(&conn, None, true, 0, 10).unwrap();
        assert_eq!(blurred_rows.len(), 1, "only the flagged photo");
        assert_eq!(blurred_rows[0].path, "/soft.jpg");

        let all_rows = library(&conn, Some("image"), false, 0, 10).unwrap();
        assert_eq!(all_rows.len(), 2, "kind + blurred filters compose");

        let page = library(&conn, None, true, 0, 1).unwrap();
        assert_eq!(page.len(), 1, "limit still applies alongside the filter");
        assert!(library(&conn, Some("video"), true, 0, 10).unwrap().is_empty());
    }

    #[test]
    fn faces_and_clusters_round_trip() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();

        let face_ids = save_faces(
            &conn,
            "/a.jpg",
            &[serde_json::json!({
                "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.95
            })],
        )
        .unwrap();
        assert_eq!(face_ids.len(), 1);

        let cluster_id = save_cluster(&conn, &face_ids).unwrap();
        let cs = cleanup_stats(&conn).unwrap(); // sanity: no cleanup tables conflict
        let _ = cs.duplicate_groups;

        let clusters = clusters_list(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].face_count, 1);

        let members = faces_in_cluster(&conn, cluster_id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].path, "/a.jpg");
        assert!((members[0].confidence - 0.95).abs() < 0.001);

        let stats = faces_count(&conn).unwrap();
        assert_eq!(stats, 1);
    }

    #[test]
    fn unknown_kind_is_rejected_by_check_constraint() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        let mut bad = asset("/x.jpg", 1, 1.0);
        bad.kind = "other".into();
        assert!(upsert_asset(&conn, &bad, scan).is_err(), "kind must be image or video");
    }
}
