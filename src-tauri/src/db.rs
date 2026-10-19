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
    (5, include_str!("../migrations/005_events.sql")),
    (6, include_str!("../migrations/006_albums.sql")),
    (7, include_str!("../migrations/007_geocode.sql")),
    (8, include_str!("../migrations/008_person_names.sql")),
    (9, include_str!("../migrations/009_scene_tags.sql")),
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanRoot {
    pub root: String,
    pub scans: i64,
    pub last_started_at: i64,
    pub assets: i64,
    pub missing: i64,
}

/// Escape for LIKE patterns: roots containing `%`/`_` must match literally.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Distinct indexed source folders (latest scan first), each with its live
/// asset counts. No migration: derived from the existing scans/assets tables.
pub fn scan_roots(conn: &Connection) -> Result<Vec<ScanRoot>, String> {
    let mut stmt = conn
        .prepare("SELECT root, COUNT(*), MAX(started_at) FROM scans GROUP BY root ORDER BY MAX(started_at) DESC")
        .map_err(|e| format!("prepare scan roots: {e}"))?;
    let roots = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })
        .map_err(|e| format!("query scan roots: {e}"))?;
    let mut out = Vec::new();
    for row in roots {
        let (root, scans, last_started_at) = row.map_err(|e| format!("scan root row: {e}"))?;
        // Equality covers a file directly AT the root; the LIKE arm needs the
        // slash so `/photos2` never matches root `/photos`. Both slash
        // variants of the root are accepted (`/photos` and `/photos/`).
        let base = root.trim_end_matches('/');
        let pattern = format!("{}/%", escape_like(base));
        let (assets, missing): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(missing), 0) FROM assets
                 WHERE path = ?1 OR path = ?2 OR path LIKE ?3 ESCAPE '\\'",
                params![root, base, pattern],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("scan root counts: {e}"))?;
        out.push(ScanRoot { root, scans, last_started_at, assets, missing });
    }
    Ok(out)
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
    pub name: Option<String>,
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
                    (SELECT path FROM faces WHERE cluster_id = fc.id ORDER BY confidence DESC LIMIT 1),
                    fc.name
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
                name: r.get(3)?,
            })
        })
        .map_err(|e| format!("query clusters: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("cluster row: {e}"))?);
    }
    Ok(out)
}

pub fn rename_cluster(conn: &Connection, cluster_id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("person name cannot be empty".into());
    }
    let changed = conn
        .execute("UPDATE face_clusters SET name = ?2 WHERE id = ?1", params![cluster_id, name])
        .map_err(|e| format!("rename cluster: {e}"))?;
    if changed == 0 {
        return Err(format!("cluster {cluster_id} not found"));
    }
    Ok(())
}

/// Moves every face from `from_id` into `into_id`, keeps the target's name
/// unless it is unnamed (then the source name carries over), and drops the
/// emptied cluster. Returns the merged face count.
pub fn merge_clusters(conn: &Connection, from_id: i64, into_id: i64) -> Result<i64, String> {
    if from_id == into_id {
        return Err("cannot merge a cluster into itself".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| format!("merge tx: {e}"))?;
    let exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM face_clusters WHERE id = ?1 OR id = ?2",
            params![from_id, into_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("merge lookup: {e}"))?;
    if exists != 2 {
        return Err("one of the clusters does not exist".into());
    }
    tx.execute(
        "UPDATE faces SET cluster_id = ?2 WHERE cluster_id = ?1",
        params![from_id, into_id],
    )
    .map_err(|e| format!("merge faces: {e}"))?;
    tx.execute(
        "UPDATE face_clusters SET name = COALESCE(name, (SELECT name FROM face_clusters WHERE id = ?1))
         WHERE id = ?2",
        params![from_id, into_id],
    )
    .map_err(|e| format!("merge name: {e}"))?;
    tx.execute("DELETE FROM face_clusters WHERE id = ?1", params![from_id])
        .map_err(|e| format!("drop merged cluster: {e}"))?;
    let count: i64 = tx
        .query_row("SELECT COUNT(*) FROM faces WHERE cluster_id = ?1", params![into_id], |r| {
            r.get(0)
        })
        .map_err(|e| format!("merge count: {e}"))?;
    tx.execute(
        "UPDATE face_clusters SET face_count = ?2 WHERE id = ?1",
        params![into_id, count],
    )
    .map_err(|e| format!("merge recount: {e}"))?;
    tx.commit().map_err(|e| format!("merge commit: {e}"))?;
    Ok(count)
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



/// Existence check: flags rows whose file is gone (ejected volume, deleted
/// file) and un-flags rows that came back. Never deletes rows — remounting the
/// volume and re-running this restores everything.
pub fn verify_missing(conn: &Connection) -> Result<(i64, i64), String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("verify tx: {e}"))?;
    // Row-by-row existence check (stat per row); batched in one transaction.
    let paths: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, path FROM assets WHERE missing = 0")
            .map_err(|e| format!("verify select: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("verify query: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("verify row: {e}"))?);
        }
        v
    };
    let mut missing = 0i64;
    for (id, path) in &paths {
        if !std::path::Path::new(path).exists() {
            tx.execute("UPDATE assets SET missing = 1 WHERE id = ?1", params![id])
                .map_err(|e| format!("verify update: {e}"))?;
            missing += 1;
        }
    }
    // Files that returned (e.g. volume remounted) clear the flag.
    let back: Vec<(i64, String)> = {
        let mut stmt = tx
            .prepare("SELECT id, path FROM assets WHERE missing = 1")
            .map_err(|e| format!("verify select2: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("verify query2: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("verify row2: {e}"))?);
        }
        v
    };
    let mut restored = 0i64;
    for (id, path) in &back {
        if std::path::Path::new(path).exists() {
            tx.execute("UPDATE assets SET missing = 0 WHERE id = ?1", params![id])
                .map_err(|e| format!("verify restore: {e}"))?;
            restored += 1;
        }
    }
    tx.commit().map_err(|e| format!("verify commit: {e}"))?;
    Ok((missing, restored))
}

// MARK: - Events

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRow {
    pub id: i64,
    pub name: String,
    pub started_at: f64,
    pub ended_at: f64,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub place: Option<String>,
    pub asset_count: i64,
    pub face_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAssetRow {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: String,
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * (a.sqrt() * (1.0 - a).max(0.0).sqrt()).asin()
}

#[derive(Clone)]
struct EventCandidate {
    id: i64,
    path: String,
    name: String,
    kind: String,
    taken_at: f64,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

/// Sequential clustering: ordered by capture time; a new event starts when
/// the gap exceeds gap_hours, or when consecutive photos are far apart.
pub fn cluster_events(
    conn: &Connection,
    gap_hours: f64,
    max_distance_m: f64,
) -> Result<(i64, i64), String> {
    let mut candidates: Vec<EventCandidate> = {
        let mut stmt = conn.prepare(
            "SELECT id, path, name, kind, COALESCE(captured_at, modified), latitude, longitude
             FROM assets WHERE missing = 0 ORDER BY COALESCE(captured_at, modified), id",
        )
        .map_err(|e| format!("prepare candidates: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(EventCandidate {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    name: r.get(2)?,
                    kind: r.get(3)?,
                    taken_at: r.get(4)?,
                    latitude: r.get(5)?,
                    longitude: r.get(6)?,
                })
            })
            .map_err(|e| format!("query candidates: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("candidate row: {e}"))?);
        }
        v
    };

    let tx = conn.unchecked_transaction().map_err(|e| format!("events tx: {e}"))?;
    tx.execute("DELETE FROM event_assets", [])
        .map_err(|e| format!("clear event_assets: {e}"))?;
    tx.execute("DELETE FROM events", [])
        .map_err(|e| format!("clear events: {e}"))?;

    let gap_seconds = gap_hours * 3600.0;
    let mut event_count = 0i64;
    let mut current: Option<i64> = None;
    let mut last_time: Option<f64> = None;
    let mut last_lat: Option<f64> = None;
    let mut last_lon: Option<f64> = None;

    for c in &candidates {
        let same_event = match last_time {
            Some(t) => {
                let time_ok = (c.taken_at - t) <= gap_seconds;
                let dist_ok = match (last_lat, last_lon, c.latitude, c.longitude) {
                    (Some(a), Some(b), Some(x), Some(y)) => haversine_m(a, b, x, y) <= max_distance_m,
                    _ => true,
                };
                time_ok && dist_ok
            }
            None => true,
        };
        if !same_event {
            current = None;
        }
        if current.is_none() {
            let stamp = format_event_time(c.taken_at);
            tx.execute(
                "INSERT INTO events (name, started_at, ended_at, latitude, longitude, asset_count, face_count, created_at)
                 VALUES (?1, ?2, ?2, ?3, ?4, 0, 0, ?5)",
                params![stamp, c.taken_at, c.latitude, c.longitude, now()],
            )
            .map_err(|e| format!("insert event: {e}"))?;
            current = Some(tx.last_insert_rowid());
            event_count += 1;
        }
        let eid = current.unwrap();
        tx.execute(
            "INSERT INTO event_assets (event_id, asset_id) VALUES (?1, ?2)",
            params![eid, c.id],
        )
        .map_err(|e| format!("insert event_asset: {e}"))?;
        last_time = Some(c.taken_at);
        last_lat = c.latitude;
        last_lon = c.longitude;
    }

    let rows: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare("SELECT event_id, COUNT(*) FROM event_assets GROUP BY event_id")
            .map_err(|e| format!("prepare counts: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("query counts: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("count row: {e}"))?);
        }
        v
    };
    for (eid, n) in &rows {
        tx.execute(
            "UPDATE events SET asset_count = ?2 WHERE id = ?1",
            params![eid, n],
        )
        .map_err(|e| format!("update count: {e}"))?;
    }
    // ended_at tracks the latest member: inserts only stamp the first asset.
    tx.execute(
        "UPDATE events SET ended_at =
            (SELECT MAX(COALESCE(a.captured_at, a.modified)) FROM assets a
             JOIN event_assets ea ON ea.asset_id = a.id WHERE ea.event_id = events.id)",
        [],
    )
    .map_err(|e| format!("update event spans: {e}"))?;
    for (eid, _) in &rows {
        let fc: i64 = tx.query_row(
            "SELECT COUNT(*) FROM faces f
             WHERE f.path IN (SELECT a.path FROM assets a JOIN event_assets ea ON ea.asset_id = a.id WHERE ea.event_id = ?1)",
            params![eid], |r| r.get(0),
        ).unwrap_or(0);
        tx.execute(
            "UPDATE events SET face_count = ?2 WHERE id = ?1",
            params![eid, fc],
        )
        .map_err(|e| format!("update face count: {e}"))?;
    }

    tx.commit().map_err(|e| format!("events commit: {e}"))?;

    // Default names are set after the spans are final: date or date range,
    // plus a cached place suffix when one is already known. Cache-only —
    // clustering never hits the network; `resolve_places` fills the gaps.
    let spans: Vec<(i64, f64, f64, Option<f64>, Option<f64>)> = {
        let mut stmt = conn
            .prepare("SELECT id, started_at, ended_at, latitude, longitude FROM events")
            .map_err(|e| format!("prepare event spans: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .map_err(|e| format!("query event spans: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("event span row: {e}"))?);
        }
        v
    };
    for (eid, start, end, lat, lon) in &spans {
        let mut name = format_event_range(*start, *end);
        if let (Some(la), Some(lo)) = (lat, lon) {
            if let Ok(Some(place)) =
                geocode_cache_lookup(conn, round_coord(*la), round_coord(*lo))
            {
                name = format!("{name} · {place}");
            }
        }
        conn.execute("UPDATE events SET name = ?2 WHERE id = ?1", params![eid, name])
            .map_err(|e| format!("name event: {e}"))?;
    }

    Ok((event_count, candidates.len() as i64))
}

/// Formats a unix timestamp as "Mon DD" without any external crate.
fn format_event_time(secs: f64) -> String {
    let days = (secs / 86400.0).floor() as i64;
    let mut d = days;
    let mut y = 1970;
    if d >= 0 {
        while d >= 365 + leap(y) as i64 { d -= 365 + leap(y) as i64; y += 1; }
    } else {
        while d < 0 { y -= 1; d += 365 + leap(y) as i64; }
    }
    let mut m = 0;
    for dim in month_days(y) {
        if d < dim { break; }
        d -= dim; m += 1;
    }
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
               "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][m];
    format!("{} {}", mon, d + 1)
}

/// Default event name: single date ("Oct 9") or span ("Oct 9–11").
/// Rename stays manual; this only sets the initial label.
fn format_event_range(started_at: f64, ended_at: f64) -> String {
    let same_day = (started_at / 86400.0).floor() == (ended_at / 86400.0).floor();
    if same_day {
        format_event_time(started_at)
    } else {
        format!("{}–{}", format_event_time(started_at), format_event_time(ended_at))
    }
}

fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_days(y: i64) -> [i64; 12] {
    [
        31,
        if leap(y) { 29i64 } else { 28i64 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ]
}

// MARK: - Events queries
pub fn events_list(conn: &Connection) -> Result<Vec<EventRow>, String> {
    let mut stmt = conn.prepare(
        "SELECT id, name, started_at, ended_at, latitude, longitude, place, asset_count, face_count
         FROM events ORDER BY started_at DESC",
    ).map_err(|e| format!("prepare events: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(EventRow {
                id: r.get(0)?,
                name: r.get(1)?,
                started_at: r.get(2)?,
                ended_at: r.get(3)?,
                latitude: r.get(4)?,
                longitude: r.get(5)?,
                place: r.get(6)?,
                asset_count: r.get(7)?,
                face_count: r.get(8)?,
            })
        })
        .map_err(|e| format!("query events: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("event row: {e}"))?);
    }
    Ok(out)
}

pub fn event_assets(conn: &Connection, event_id: i64) -> Result<Vec<EventAssetRow>, String> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.path, a.name, a.kind FROM assets a
         JOIN event_assets ea ON ea.asset_id = a.id
         WHERE ea.event_id = ?1 ORDER BY a.path",
    ).map_err(|e| format!("prepare event assets: {e}"))?;
    let rows = stmt
        .query_map(params![event_id], |r| {
            Ok(EventAssetRow {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
            })
        })
        .map_err(|e| format!("query event assets: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("event asset row: {e}"))?);
    }
    Ok(out)
}

pub fn rename_event(conn: &Connection, event_id: i64, name: &str) -> Result<(), String> {
    let changed = conn
        .execute("UPDATE events SET name = ?2 WHERE id = ?1", params![event_id, name])
        .map_err(|e| format!("rename event: {e}"))?;
    if changed == 0 {
        return Err(format!("event {event_id} not found"));
    }
    Ok(())
}

// MARK: - Place names (reverse-geocode cache; the one network exception)

/// Cache cell: 3-decimal rounding (~100m), so nearby centroids share a lookup.
pub fn round_coord(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub struct PlaceTarget {
    pub event_id: i64,
    pub latitude: f64,
    pub longitude: f64,
}

/// Events that have coordinates but no resolved place name yet.
pub fn events_missing_places(conn: &Connection) -> Result<Vec<PlaceTarget>, String> {
    let mut stmt = conn
        .prepare("SELECT id, latitude, longitude FROM events WHERE place IS NULL AND latitude IS NOT NULL AND longitude IS NOT NULL ORDER BY id")
        .map_err(|e| format!("prepare missing places: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PlaceTarget {
                event_id: r.get(0)?,
                latitude: r.get(1)?,
                longitude: r.get(2)?,
            })
        })
        .map_err(|e| format!("query missing places: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("missing place row: {e}"))?);
    }
    Ok(out)
}

pub fn geocode_cache_lookup(
    conn: &Connection,
    lat_key: f64,
    lon_key: f64,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT place FROM geocode_cache WHERE lat_key = ?1 AND lon_key = ?2",
        params![lat_key, lon_key],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| format!("geocode lookup: {e}"))
}

pub fn geocode_cache_store(
    conn: &Connection,
    lat_key: f64,
    lon_key: f64,
    place: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO geocode_cache (lat_key, lon_key, place, fetched_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (lat_key, lon_key) DO UPDATE SET place = excluded.place, fetched_at = excluded.fetched_at",
        params![lat_key, lon_key, place, now()],
    )
    .map_err(|e| format!("geocode store: {e}"))?;
    Ok(())
}

pub fn set_event_place(conn: &Connection, event_id: i64, place: &str) -> Result<(), String> {
    conn.execute("UPDATE events SET place = ?2 WHERE id = ?1", params![event_id, place])
        .map_err(|e| format!("set event place: {e}"))?;
    Ok(())
}

/// Short "City, Region" label from a Nominatim `address` object. Pure and
/// unit-tested; the network layer only feeds it JSON.
pub fn format_place_nominatim(address: &serde_json::Value) -> Option<String> {
    let str_field = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            address
                .get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    };
    let locality = str_field(&["city", "town", "village", "hamlet", "municipality", "county"]);
    let region = str_field(&["state", "country"]);
    match (locality, region) {
        (Some(l), Some(r)) if l != r => Some(format!("{l}, {r}")),
        (Some(l), _) => Some(l),
        (None, Some(r)) => Some(r),
        (None, None) => None,
    }
}

// MARK: - Albums (user-curated lists, the "audience groups" for export)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumRow {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub asset_count: i64,
    // Newest-added image, so the UI can show a cover thumbnail.
    pub cover_path: Option<String>,
}

pub fn album_create(conn: &Connection, name: &str) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("album name cannot be empty".into());
    }
    conn.execute(
        "INSERT INTO albums (name, created_at) VALUES (?1, ?2)",
        params![name, now()],
    )
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            format!("an album named \u{201c}{name}\u{201d} already exists")
        } else {
            format!("create album: {e}")
        }
    })?;
    Ok(conn.last_insert_rowid())
}

pub fn album_rename(conn: &Connection, album_id: i64, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("album name cannot be empty".into());
    }
    let n = conn
        .execute("UPDATE albums SET name = ?1 WHERE id = ?2", params![name, album_id])
        .map_err(|e| format!("rename album: {e}"))?;
    if n == 0 {
        return Err("no such album".into());
    }
    Ok(())
}

/// Deletes the album and its membership rows (assets themselves are untouched).
/// Returns the number of membership links removed.
pub fn album_delete(conn: &Connection, album_id: i64) -> Result<i64, String> {
    let removed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM album_assets WHERE album_id = ?1",
            params![album_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("album count: {e}"))?;
    conn.execute("DELETE FROM albums WHERE id = ?1", params![album_id])
        .map_err(|e| format!("delete album: {e}"))?;
    Ok(removed)
}

pub fn albums_list(conn: &Connection) -> Result<Vec<AlbumRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT al.id, al.name, al.created_at,
                    COUNT(aa.asset_id),
                    (SELECT a.path FROM album_assets aa2 JOIN assets a ON a.id = aa2.asset_id
                     WHERE aa2.album_id = al.id AND a.kind = 'image' AND a.missing = 0
                     ORDER BY aa2.added_at DESC LIMIT 1)
             FROM albums al
             LEFT JOIN album_assets aa ON aa.album_id = al.id
             GROUP BY al.id ORDER BY al.name",
        )
        .map_err(|e| format!("prepare albums: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(AlbumRow {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                asset_count: r.get(3)?,
                cover_path: r.get(4)?,
            })
        })
        .map_err(|e| format!("query albums: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("album row: {e}"))?);
    }
    Ok(out)
}

/// Adds assets to an album; existing memberships are left alone.
/// Returns how many rows were actually added.
pub fn album_add_assets(
    conn: &Connection,
    album_id: i64,
    asset_ids: &[i64],
) -> Result<i64, String> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM albums WHERE id = ?1",
            params![album_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("album lookup: {e}"))?;
    if exists == 0 {
        return Err("no such album".into());
    }
    let mut added = 0i64;
    let mut stmt = conn
        .prepare(
            "INSERT OR IGNORE INTO album_assets (album_id, asset_id, added_at) VALUES (?1, ?2, ?3)",
        )
        .map_err(|e| format!("prepare album add: {e}"))?;
    for id in asset_ids {
        added += stmt
            .execute(params![album_id, id, now()])
            .map_err(|e| format!("album add: {e}"))? as i64;
    }
    Ok(added)
}

/// Removes one asset from an album (but never from the library).
pub fn album_remove_asset(
    conn: &Connection,
    album_id: i64,
    asset_id: i64,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM album_assets WHERE album_id = ?1 AND asset_id = ?2",
        params![album_id, asset_id],
    )
    .map_err(|e| format!("album remove: {e}"))?;
    Ok(())
}

pub fn album_assets(conn: &Connection, album_id: i64) -> Result<Vec<EventAssetRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.path, a.name, a.kind FROM assets a
             JOIN album_assets aa ON aa.asset_id = a.id
             WHERE aa.album_id = ?1
             ORDER BY aa.added_at DESC, a.name",
        )
        .map_err(|e| format!("prepare album assets: {e}"))?;
    let rows = stmt
        .query_map(params![album_id], |r| {
            Ok(EventAssetRow {
                id: r.get(0)?,
                path: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
            })
        })
        .map_err(|e| format!("query album assets: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("album asset row: {e}"))?);
    }
    Ok(out)
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

#[derive(Default)]
pub struct LibraryQuery<'a> {
    pub kind: Option<&'a str>,
    pub blurred_only: bool,
    /// Only assets that have a detected face in this person cluster.
    pub person_cluster: Option<i64>,
    /// Some(true) = geotagged only, Some(false) = untagged only.
    pub has_location: Option<bool>,
    /// Capture-time window (epoch seconds, half-open [min, max)).
    pub captured_min: Option<f64>,
    pub captured_max: Option<f64>,
    /// Some(true) = starred only.
    pub favorite_only: Option<bool>,
    /// Only assets carrying this scene tag (from `tag_scenes`).
    pub scene_tag: Option<&'a str>,
    pub offset: i64,
    pub limit: i64,
}

/// The WHERE fragment shared by library() and library_count() so the
/// "Showing X of N" total always matches the filtered grid.
const LIBRARY_WHERE: &str = "WHERE (?1 IS NULL OR kind = ?1) AND missing = 0
  AND (?2 = 0 OR blurred = 1)
  AND (?3 IS NULL OR EXISTS (SELECT 1 FROM faces f WHERE f.path = a.path AND f.cluster_id = ?3))
  AND (?4 IS NULL OR ((?4 = 1) = (latitude IS NOT NULL)))
  AND (?5 IS NULL OR COALESCE(captured_at, modified) >= ?5)
  AND (?6 IS NULL OR COALESCE(captured_at, modified) < ?6)
  AND (?7 IS NULL OR favorite = ?7)
  AND (?8 IS NULL OR EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag = ?8))";

pub fn library(conn: &Connection, q: &LibraryQuery) -> Result<Vec<AssetRow>, String> {
    let sql = format!(
        "SELECT id, path, name, ext, kind, mime, size_bytes, modified, captured_at,
                       width, height, duration, favorite, missing, sha256
                FROM assets a
                {LIBRARY_WHERE}
                ORDER BY COALESCE(captured_at, modified) DESC, id DESC
                LIMIT ?9 OFFSET ?10"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare library: {e}"))?;
    let rows = stmt
        .query_map(
            params![
                q.kind,
                i64::from(q.blurred_only),
                q.person_cluster,
                q.has_location.map(i64::from),
                q.captured_min,
                q.captured_max,
                q.favorite_only.map(i64::from),
                q.scene_tag,
                q.limit,
                q.offset
            ],
            |r| {
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

/// Filtered total for the "Showing X of N" label; same predicates as library().
pub fn library_count(conn: &Connection, q: &LibraryQuery) -> Result<i64, String> {
    let sql = format!("SELECT COUNT(*) FROM assets a {LIBRARY_WHERE}");
    conn.query_row(
        &sql,
        params![
            q.kind,
            i64::from(q.blurred_only),
            q.person_cluster,
            q.has_location.map(i64::from),
            q.captured_min,
            q.captured_max,
            q.favorite_only.map(i64::from),
            q.scene_tag
        ],
        |r| r.get(0),
    )
    .map_err(|e| format!("library count: {e}"))
}

/// Stars or unstars one asset. Returns the new value.
pub fn set_favorite(conn: &Connection, asset_id: i64, favorite: bool) -> Result<(), String> {
    let changed = conn
        .execute(
            "UPDATE assets SET favorite = ?2 WHERE id = ?1",
            params![asset_id, i64::from(favorite)],
        )
        .map_err(|e| format!("set favorite: {e}"))?;
    if changed == 0 {
        return Err(format!("asset {asset_id} not found"));
    }
    Ok(())
}

// MARK: - Scene tags (Vision classifier labels, derived like faces)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTag {
    pub tag: String,
    pub confidence: f64,
}

/// Readable images with no tags yet (videos are skipped: the classifier is
/// stills-only, and a 0.5s poster frame mislabels motion).
pub fn untagged_images(conn: &Connection) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM assets
             WHERE kind = 'image' AND missing = 0
             AND NOT EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = assets.id)
             ORDER BY id",
        )
        .map_err(|e| format!("prepare untagged: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query untagged: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("untagged row: {e}"))?);
    }
    Ok(out)
}

pub fn store_tags(conn: &Connection, asset_id: i64, tags: &[(String, f64)]) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("tags tx: {e}"))?;
    for (tag, confidence) in tags {
        tx.execute(
            "INSERT INTO asset_tags (asset_id, tag, confidence) VALUES (?1, ?2, ?3)
             ON CONFLICT (asset_id, tag) DO UPDATE SET confidence = excluded.confidence",
            params![asset_id, tag, confidence],
        )
        .map_err(|e| format!("store tag: {e}"))?;
    }
    tx.commit().map_err(|e| format!("tags commit: {e}"))?;
    Ok(())
}

/// Distinct tags in the library with asset counts, most common first.
pub fn distinct_tags(conn: &Connection) -> Result<Vec<(String, i64)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT tag, COUNT(DISTINCT asset_id) FROM asset_tags
             GROUP BY tag ORDER BY COUNT(DISTINCT asset_id) DESC, tag",
        )
        .map_err(|e| format!("prepare tags: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query tags: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("tag row: {e}"))?);
    }
    Ok(out)
}

/// Tags for one asset, best first (viewer display).
pub fn asset_tags(conn: &Connection, asset_id: i64) -> Result<Vec<SceneTag>, String> {
    let mut stmt = conn
        .prepare("SELECT tag, confidence FROM asset_tags WHERE asset_id = ?1 ORDER BY confidence DESC")
        .map_err(|e| format!("prepare asset tags: {e}"))?;
    let rows = stmt
        .query_map(params![asset_id], |r| Ok(SceneTag { tag: r.get(0)?, confidence: r.get(1)? }))
        .map_err(|e| format!("query asset tags: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("asset tag row: {e}"))?);
    }
    Ok(out)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryMonth {
    pub month: String,
    pub count: i64,
}

/// Months present in the readable library, newest first, for the date picker.
pub fn library_months(conn: &Connection) -> Result<Vec<LibraryMonth>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT strftime('%Y-%m', COALESCE(captured_at, modified), 'unixepoch') AS m,
                    COUNT(*)
             FROM assets
             WHERE missing = 0 AND COALESCE(captured_at, modified) IS NOT NULL
             GROUP BY m ORDER BY m DESC",
        )
        .map_err(|e| format!("prepare library months: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(LibraryMonth {
                month: r.get(0)?,
                count: r.get(1)?,
            })
        })
        .map_err(|e| format!("query library months: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("library month row: {e}"))?);
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
        assert_eq!(migrate(&conn).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert!(migrate(&conn).unwrap().is_empty(), "second call is a no-op");
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 9, "user_version tracks the highest applied migration");

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

        let all = library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].path, "/old.jpg", "newest capture first");

        let paged = library(&conn, &LibraryQuery { offset: 1, limit: 10, ..Default::default() }).unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].path, "/new.jpg", "offset skips the newest");

        let videos = library(&conn, &LibraryQuery { kind: Some("video"), limit: 10, ..Default::default() }).unwrap();
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

        let blurred_rows = library(&conn, &LibraryQuery { blurred_only: true, limit: 10, ..Default::default() }).unwrap();
        assert_eq!(blurred_rows.len(), 1, "only the flagged photo");
        assert_eq!(blurred_rows[0].path, "/soft.jpg");

        let all_rows = library(&conn, &LibraryQuery { kind: Some("image"), limit: 10, ..Default::default() }).unwrap();
        assert_eq!(all_rows.len(), 2, "kind + blurred filters compose");

        let page = library(&conn, &LibraryQuery { blurred_only: true, limit: 1, ..Default::default() }).unwrap();
        assert_eq!(page.len(), 1, "limit still applies alongside the filter");
        assert!(library(&conn, &LibraryQuery { kind: Some("video"), blurred_only: true, limit: 10, ..Default::default() }).unwrap().is_empty());
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
    fn cluster_names_and_merges() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mk = |path: &str| {
            save_faces(
                &conn,
                path,
                &[serde_json::json!({"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
            )
            .unwrap()
        };
        let a = save_cluster(&conn, &mk("/a.jpg")).unwrap();
        let b = save_cluster(&conn, &mk("/b.jpg")).unwrap();

        rename_cluster(&conn, a, "Maya").unwrap();
        assert!(rename_cluster(&conn, a, "  ").is_err(), "blank names rejected");
        assert!(rename_cluster(&conn, 9999, "Ghost").is_err(), "unknown cluster errors");

        // Unnamed target inherits the source name; counts merge; source gone.
        let merged = merge_clusters(&conn, a, b).unwrap();
        assert_eq!(merged, 2);
        let clusters = clusters_list(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].id, b);
        assert_eq!(clusters[0].name.as_deref(), Some("Maya"));
        assert_eq!(faces_in_cluster(&conn, b).unwrap().len(), 2);

        assert!(merge_clusters(&conn, b, b).is_err(), "self-merge rejected");
        assert!(merge_clusters(&conn, b, 9999).is_err(), "missing target errors");
    }

    #[test]
    fn library_filters_by_month_location_and_person() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // a: Oct, geotagged; b: Oct, untagged; c: Nov, geotagged.
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/c.jpg", 1, 3.0), scan).unwrap();
        // 2026-10-03, 2026-10-04, 2026-11-02.
        conn.execute("UPDATE assets SET captured_at = 1791000000.0 WHERE path = '/a.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET captured_at = 1791100000.0 WHERE path = '/b.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET captured_at = 1793600000.0 WHERE path = '/c.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET latitude = 1.0, longitude = 2.0 WHERE path IN ('/a.jpg', '/c.jpg')", [])
            .unwrap();
        // One face on /a.jpg, clustered as person 1.
        let face_ids = save_faces(
            &conn,
            "/a.jpg",
            &[serde_json::json!({"x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2, "confidence": 0.9})],
        )
        .unwrap();
        let cluster = save_cluster(&conn, &face_ids).unwrap();

        let base = LibraryQuery { limit: 10, ..Default::default() };
        assert_eq!(library(&conn, &base).unwrap().len(), 3);

        // Month window for October 2026: [Oct 1, Nov 1) UTC.
        let oct = LibraryQuery { captured_min: Some(1_790_812_800.0), captured_max: Some(1_793_491_200.0), limit: 10, ..Default::default() };
        assert_eq!(library(&conn, &oct).unwrap().len(), 2);

        // Location presence.
        let geo = LibraryQuery { has_location: Some(true), limit: 10, ..Default::default() };
        assert_eq!(library(&conn, &geo).unwrap().len(), 2);
        let nongeo = LibraryQuery { has_location: Some(false), limit: 10, ..Default::default() };
        let rows = library(&conn, &nongeo).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/b.jpg");

        // Person cluster.
        let person = LibraryQuery { person_cluster: Some(cluster), limit: 10, ..Default::default() };
        let rows = library(&conn, &person).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/a.jpg");
        assert_eq!(library_count(&conn, &person).unwrap(), 1);

        // Months list covers both months, newest first.
        let months = library_months(&conn).unwrap();
        assert_eq!(months.len(), 2);
        assert_eq!(months[0].month, "2026-11");
        assert_eq!(months[1].month, "2026-10");
    }

    #[test]
    fn favorites_toggle_and_filter() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        let aid: i64 = conn
            .query_row("SELECT id FROM assets WHERE path = '/a.jpg'", [], |r| r.get(0))
            .unwrap();

        let starred = LibraryQuery { favorite_only: Some(true), limit: 10, ..Default::default() };
        assert!(library(&conn, &starred).unwrap().is_empty());
        set_favorite(&conn, aid, true).unwrap();
        let rows = library(&conn, &starred).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/a.jpg");
        assert_eq!(library_count(&conn, &starred).unwrap(), 1);
        set_favorite(&conn, aid, false).unwrap();
        assert!(library(&conn, &starred).unwrap().is_empty());
        assert!(set_favorite(&conn, 9999, true).is_err(), "unknown asset errors");
    }

    #[test]
    fn scene_tags_store_and_filter() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        let aid: i64 = conn
            .query_row("SELECT id FROM assets WHERE path = '/a.jpg'", [], |r| r.get(0))
            .unwrap();

        assert!(untagged_images(&conn).unwrap().len() == 2);
        store_tags(&conn, aid, &[("mountain".to_string(), 0.9), ("lake".to_string(), 0.4)]).unwrap();
        // Re-storing updates confidence instead of duplicating.
        store_tags(&conn, aid, &[("mountain".to_string(), 0.95)]).unwrap();

        let remaining = untagged_images(&conn).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].1, "/b.jpg");

        let tags = distinct_tags(&conn).unwrap();
        assert_eq!(tags.len(), 2);
        let mine = asset_tags(&conn, aid).unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].tag, "mountain");

        let q = LibraryQuery { scene_tag: Some("mountain"), limit: 10, ..Default::default() };
        let rows = library(&conn, &q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/a.jpg");
        assert_eq!(library_count(&conn, &q).unwrap(), 1);
        let none = LibraryQuery { scene_tag: Some("bird"), limit: 10, ..Default::default() };
        assert!(library(&conn, &none).unwrap().is_empty());
    }

    #[test]
    fn album_crud_and_membership() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        let a = asset("/a.jpg", 1, 1.0);
        upsert_asset(&conn, &a, scan).unwrap();
        let b = asset("/b.jpg", 1, 2.0);
        upsert_asset(&conn, &b, scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let aid = id_for("/a.jpg");
        let bid = id_for("/b.jpg");

        let id = album_create(&conn, "For Drive").unwrap();
        assert!(album_create(&conn, "For Drive").is_err(), "names are unique");

        // Add both assets; re-adding the first must not duplicate it.
        assert_eq!(album_add_assets(&conn, id, &[aid, bid, aid]).unwrap(), 2);
        let list = albums_list(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "For Drive");
        assert_eq!(list[0].asset_count, 2);

        let members = album_assets(&conn, id).unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().all(|m| m.id == aid || m.id == bid));

        // Remove one; album count drops, library untouched.
        album_remove_asset(&conn, id, bid).unwrap();
        assert_eq!(albums_list(&conn).unwrap()[0].asset_count, 1);

        // Renaming keeps the row.
        album_rename(&conn, id, "Family").unwrap();
        assert_eq!(albums_list(&conn).unwrap()[0].name, "Family");

        // Deleting drops membership rows only.
        let removed = album_delete(&conn, id).unwrap();
        assert_eq!(removed, 1);
        assert!(albums_list(&conn).unwrap().is_empty());
        let still_there: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still_there, 2, "album delete never touches library rows");
    }

    #[test]
    fn cluster_events_groups_by_time_and_distance() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let now_ts = 1_700_000_000.0;
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // same event: close in time
        upsert_asset(&conn, &asset("/a.jpg", 1, now_ts), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, now_ts + 1800.0), scan).unwrap();
        // new event: 10 hours later (> 4h gap)
        upsert_asset(&conn, &asset("/c.jpg", 1, now_ts + 40_000.0), scan).unwrap();

        let (events, assets) = cluster_events(&conn, 4.0, 500.0).unwrap();
        assert_eq!(events, 2, "two events by time gap");
        assert_eq!(assets, 3, "all three assets assigned");

        let rows = events_list(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].asset_count, 1, "second event (later) listed first");
        assert_eq!(rows[1].asset_count, 2, "first event has two assets");

        // rename sticks
        rename_event(&conn, rows[1].id, "Birthday brunch").unwrap();
        let rows = events_list(&conn).unwrap();
        assert_eq!(rows.iter().find(|r| r.id == rows[1].id).unwrap().name, "Birthday brunch");

        let ea = event_assets(&conn, rows[1].id).unwrap();
        assert_eq!(ea.len(), 2);
    }

    #[test]
    fn cluster_events_groups_by_location_proximity() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // same place, 50m apart, 1 hour apart -> one event
        upsert_asset(&conn, &asset("/near1.jpg", 1, 1_700_000_000.0), scan).unwrap();
        upsert_asset(&conn, &asset("/near2.jpg", 1, 1_700_000_000.0 + 3600.0), scan).unwrap();
        // set lat/lon: near1 ~ SF, near2 ~ 50m away
        conn.execute("UPDATE assets SET latitude=37.7749, longitude=-122.4194 WHERE path='/near1.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET latitude=37.77495, longitude=-122.4194 WHERE path='/near2.jpg'", []).unwrap();

        let (events, _) = cluster_events(&conn, 4.0, 500.0).unwrap();
        assert_eq!(events, 1, "nearby photos stay in one event");

        // 5 km apart -> separate events
        conn.execute("UPDATE assets SET latitude=37.8249 WHERE path='/near2.jpg'", []).unwrap();
        let (events, _) = cluster_events(&conn, 4.0, 500.0).unwrap();
        assert_eq!(events, 2, "distant photos split events");
    }

    #[test]
    fn cluster_events_name_spans_and_cached_places() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // Two-day trip: 30h apart needs a wide gap to stay one event.
        upsert_asset(&conn, &asset("/d1.jpg", 1, 1_700_000_000.0), scan).unwrap();
        upsert_asset(&conn, &asset("/d2.jpg", 1, 1_700_000_000.0 + 108_000.0), scan).unwrap();
        conn.execute(
            "UPDATE assets SET latitude = 37.7749, longitude = -122.4194",
            [],
        )
        .unwrap();
        geocode_cache_store(&conn, round_coord(37.7749), round_coord(-122.4194), "San Francisco, California")
            .unwrap();

        let (events, _) = cluster_events(&conn, 48.0, 500.0).unwrap();
        assert_eq!(events, 1, "continuous days group with a wide gap");
        let rows = events_list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].name.contains('–'),
            "multi-day default name is a span, got: {}",
            rows[0].name
        );
        assert!(
            rows[0].name.contains("San Francisco"),
            "cached place suffixes the name, got: {}",
            rows[0].name
        );

        // Same-day events keep a bare date name.
        let (events, _) = cluster_events(&conn, 4.0, 500.0).unwrap();
        assert_eq!(events, 2, "narrow gap splits the days again");
        let rows = events_list(&conn).unwrap();
        assert!(rows.iter().all(|r| !r.name.contains('–')), "single-day names have no span");
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

    #[test]
    fn scan_roots_groups_scans_with_live_counts() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let s1 = begin_scan(&conn, "/photos", true).unwrap();
        upsert_asset(&conn, &asset("/photos/a.jpg", 1, 1.0), s1).unwrap();
        upsert_asset(&conn, &asset("/photos/b.jpg", 1, 1.0), s1).unwrap();
        // Rescan of the same root groups into one row.
        let s2 = begin_scan(&conn, "/photos", true).unwrap();
        upsert_asset(&conn, &asset("/photos/c.jpg", 1, 1.0), s2).unwrap();
        let s3 = begin_scan(&conn, "/photos2", true).unwrap();
        upsert_asset(&conn, &asset("/photos2/d.jpg", 1, 1.0), s3).unwrap();
        // Special LIKE chars in a root must match literally.
        let s4 = begin_scan(&conn, "/my%photos", true).unwrap();
        upsert_asset(&conn, &asset("/my%photos/e.jpg", 1, 1.0), s4).unwrap();
        conn.execute("UPDATE assets SET missing = 1 WHERE path = '/photos/b.jpg'", []).unwrap();

        let roots = scan_roots(&conn).unwrap();
        assert_eq!(roots.len(), 3);
        let main = roots.iter().find(|r| r.root == "/photos").unwrap();
        assert_eq!(main.scans, 2, "rescans group under one root");
        assert_eq!(main.assets, 3, "/photos2 must not leak into /photos");
        assert_eq!(main.missing, 1);
        let plus = roots.iter().find(|r| r.root == "/photos2").unwrap();
        assert_eq!((plus.assets, plus.missing), (1, 0));
        let pct = roots.iter().find(|r| r.root == "/my%photos").unwrap();
        assert_eq!(pct.assets, 1, "% in root matches literally");
    }

    #[test]
    fn geocode_cache_feeds_missing_places_without_network() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO events (name, started_at, ended_at, latitude, longitude, asset_count, face_count, created_at)
             VALUES ('Oct 9', 1.0, 2.0, 37.7749, -122.4194, 3, 0, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events (name, started_at, ended_at, latitude, longitude, asset_count, face_count, created_at)
             VALUES ('Oct 10', 3.0, 4.0, NULL, NULL, 1, 0, 1)",
            [],
        )
        .unwrap();

        let missing = events_missing_places(&conn).unwrap();
        assert_eq!(missing.len(), 1, "only the geotagged event needs a place");
        assert_eq!(missing[0].latitude, 37.7749);

        // Rounding buckets nearby centroids into one cache cell.
        assert_eq!(round_coord(37.77494), round_coord(37.7749));
        assert!((round_coord(-122.41944) - -122.419).abs() < 1e-9);

        assert!(geocode_cache_lookup(&conn, 37.775, -122.419).unwrap().is_none());
        geocode_cache_store(&conn, 37.775, -122.419, "San Francisco, California").unwrap();
        assert_eq!(
            geocode_cache_lookup(&conn, 37.775, -122.419).unwrap().as_deref(),
            Some("San Francisco, California")
        );

        set_event_place(&conn, missing[0].event_id, "San Francisco, California").unwrap();
        assert!(events_missing_places(&conn).unwrap().is_empty());
        let rows = events_list(&conn).unwrap();
        let placed = rows.iter().find(|r| r.id == missing[0].event_id).unwrap();
        assert_eq!(placed.place.as_deref(), Some("San Francisco, California"));
    }

    #[test]
    fn format_place_prefers_city_region() {
        let addr = serde_json::json!({"city": "San Francisco", "state": "California", "country": "United States"});
        assert_eq!(format_place_nominatim(&addr).as_deref(), Some("San Francisco, California"));
        let addr = serde_json::json!({"village": "Mawlynnong", "country": "India"});
        assert_eq!(format_place_nominatim(&addr).as_deref(), Some("Mawlynnong, India"));
        let addr = serde_json::json!({"country": "Iceland"});
        assert_eq!(format_place_nominatim(&addr).as_deref(), Some("Iceland"));
        let addr = serde_json::json!({"city": "Singapore", "country": "Singapore"});
        assert_eq!(format_place_nominatim(&addr).as_deref(), Some("Singapore"), "no doubled locality");
        let addr = serde_json::json!({"road": "Some Street"});
        assert!(format_place_nominatim(&addr).is_none(), "street alone is not a place label");
    }
}

#[cfg(test)]
mod verify_tests {
    use super::*;

    #[test]
    fn verify_flags_missing_files_and_restores_them() {
        let dir = std::env::temp_dir().join("ff-verify-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, dir.to_str().unwrap(), true).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            upsert_asset(
                &conn,
                &Value2 {
                    path: dir.join(name).to_string_lossy().to_string(),
                    name: name.to_string(),
                    ext: "jpg".into(),
                    kind: "image".into(),
                    mime: None,
                    size_bytes: 1,
                    modified: 1.0,
                },
                scan,
            )
            .unwrap();
        }

        // Everything present: nothing changes.
        assert_eq!(verify_missing(&conn).unwrap(), (0, 0));

        // "Eject" one file: flagged missing and hidden from library queries.
        std::fs::remove_file(dir.join("b.jpg")).unwrap();
        assert_eq!(verify_missing(&conn).unwrap(), (1, 0));
        assert_eq!(library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap().len(), 2);
        let still_indexed: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(still_indexed, 3, "missing files are flagged, never deleted");

        // "Remount": the flag clears and the file reappears.
        std::fs::write(dir.join("b.jpg"), b"x").unwrap();
        assert_eq!(verify_missing(&conn).unwrap(), (0, 1));
        assert_eq!(library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap().len(), 3);
    }
}
