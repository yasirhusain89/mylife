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
    (10, include_str!("../migrations/010_geofences.sql")),
    (11, include_str!("../migrations/011_near_dupes.sql")),
    (12, include_str!("../migrations/012_pets.sql")),
    (13, include_str!("../migrations/013_vectors.sql")),
    (14, include_str!("../migrations/014_face_descriptors.sql")),
    (15, include_str!("../migrations/015_exposure.sql")),
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
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub iso: Option<i64>,
    pub f_number: Option<f64>,
    pub exposure_time: Option<f64>,
    pub focal_length: Option<f64>,
    pub focal_length_35: Option<i64>,
    /// First non-null event place for this asset (for Group-by-Location).
    pub place: Option<String>,
    /// Sorted, de-duplicated named people on this photo (for Group-by-Person).
    pub people: Vec<String>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapPin {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

/// Map marker cap: thousands of circle markers stay interactive; beyond this
/// the map needs clustering or filters (follow-up).
pub const MAP_PINS_LIMIT: i64 = 5000;

/// Every geotagged, present asset for the map view. No filters in v1: the map
/// shows the whole library, ordered by id.
pub fn map_pins(conn: &Connection) -> Result<Vec<MapPin>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, name, latitude, longitude FROM assets
             WHERE missing = 0 AND latitude IS NOT NULL AND longitude IS NOT NULL
             ORDER BY id LIMIT ?1",
        )
        .map_err(|e| format!("prepare map pins: {e}"))?;
    let rows = stmt
        .query_map(params![MAP_PINS_LIMIT], |r| {
            Ok(MapPin { id: r.get(0)?, path: r.get(1)?, name: r.get(2)?, latitude: r.get(3)?, longitude: r.get(4)? })
        })
        .map_err(|e| format!("query map pins: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("map pin row: {e}"))?);
    }
    Ok(out)
}

#[derive(Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RemoveRootSummary {
    pub root: String,
    pub removed_assets: i64,
    pub removed_faces: i64,
    pub removed_sightings: i64,
    pub pruned_clusters: i64,
    pub removed_scans: i64,
}

/// Forgets one indexed root: its scan records, every asset under it (files on
/// disk are untouched), plus the path-keyed derived rows (faces, animal
/// sightings) those assets owned. Clusters left with no faces are pruned;
/// everything else (albums, events, tags, vectors) follows the assets via
/// ON DELETE CASCADE. Same match rule as `scan_roots` counts.
///
/// Symlinked roots (macOS /tmp → /private/tmp) match twice: the import walk
/// stores canonicalized asset paths while the scan record keeps the literal
/// root, so both spellings are matched. Unresolvable roots (deleted or
/// ejected volumes) fall back to the literal spelling.
pub fn remove_root(conn: &Connection, root: &str) -> Result<RemoveRootSummary, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("remove root tx: {e}"))?;
    let mut spellings = vec![root.to_string()];
    if let Ok(canonical) = std::fs::canonicalize(root) {
        let cs = canonical.to_string_lossy().into_owned();
        if cs != root {
            spellings.push(cs);
        }
    }
    // One (path = ? OR path = ? OR path LIKE ? ESCAPE '\') arm per spelling.
    let mut asset_where = Vec::new();
    let mut params: Vec<String> = Vec::new();
    for spelling in &spellings {
        let base = spelling.trim_end_matches('/');
        asset_where.push("(path = ? OR path = ? OR path LIKE ? ESCAPE '\\')".to_string());
        params.push(spelling.clone());
        params.push(base.to_string());
        params.push(format!("{}/%", escape_like(base)));
    }
    let asset_where = asset_where.join(" OR ");
    let param_refs: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|s| s as &dyn rusqlite::ToSql).collect();

    let removed_assets: i64 = tx
        .query_row(&format!("SELECT COUNT(*) FROM assets WHERE {asset_where}"), param_refs.as_slice(), |r| {
            r.get(0)
        })
        .map_err(|e| format!("count root assets: {e}"))?;
    let removed_faces: i64 = tx
        .query_row(&format!("SELECT COUNT(*) FROM faces WHERE {asset_where}"), param_refs.as_slice(), |r| {
            r.get(0)
        })
        .map_err(|e| format!("count root faces: {e}"))?;
    let removed_sightings: i64 = tx
        .query_row(
            &format!("SELECT COUNT(*) FROM animal_sightings WHERE {asset_where}"),
            param_refs.as_slice(),
            |r| r.get(0),
        )
        .map_err(|e| format!("count root sightings: {e}"))?;

    tx.execute(&format!("DELETE FROM faces WHERE {asset_where}"), param_refs.as_slice())
        .map_err(|e| format!("delete root faces: {e}"))?;
    tx.execute(&format!("DELETE FROM animal_sightings WHERE {asset_where}"), param_refs.as_slice())
        .map_err(|e| format!("delete root sightings: {e}"))?;
    tx.execute(&format!("DELETE FROM assets WHERE {asset_where}"), param_refs.as_slice())
        .map_err(|e| format!("delete root assets: {e}"))?;
    let scan_where = spellings.iter().map(|_| "root = ?").collect::<Vec<_>>().join(" OR ");
    let scan_refs: Vec<&dyn rusqlite::ToSql> =
        spellings.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let removed_scans = tx
        .execute(&format!("DELETE FROM scans WHERE {scan_where}"), scan_refs.as_slice())
        .map_err(|e| format!("delete root scans: {e}"))? as i64;

    // Face counts go stale with the face deletes; refresh, then drop empties.
    tx.execute(
        "UPDATE face_clusters SET face_count = (SELECT COUNT(*) FROM faces WHERE cluster_id = face_clusters.id)",
        [],
    )
    .map_err(|e| format!("refresh cluster counts: {e}"))?;
    let pruned_clusters = tx
        .execute("DELETE FROM face_clusters WHERE face_count = 0", [])
        .map_err(|e| format!("prune empty clusters: {e}"))? as i64;

    tx.commit().map_err(|e| format!("remove root commit: {e}"))?;
    Ok(RemoveRootSummary {
        root: root.to_string(),
        removed_assets,
        removed_faces,
        removed_sightings,
        pruned_clusters,
        removed_scans,
    })
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

/// Every asset id under a root (same match rule as `scan_roots` counts),
/// for one-click "add entire folder to album". Capped: folders bigger than
/// this need the filtered Library + batch bar instead.
pub fn asset_ids_in_root(conn: &Connection, root: &str) -> Result<Vec<i64>, String> {
    let base = root.trim_end_matches('/');
    let pattern = format!("{}/%", escape_like(base));
    let mut stmt = conn
        .prepare(
            "SELECT id FROM assets
             WHERE path = ?1 OR path = ?2 OR path LIKE ?3 ESCAPE '\\'
             ORDER BY id LIMIT 20000",
        )
        .map_err(|e| format!("prepare root assets: {e}"))?;
    let ids = stmt
        .query_map(params![root, base, pattern], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("query root assets: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("root asset row: {e}"))?;
    Ok(ids)
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
    for f in faces.iter() {
        let descriptor: Option<Vec<u8>> = match f.get("descriptor") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => {
                let bytes = b64_decode(s).map_err(|e| format!("invalid descriptor: {e}"))?;
                // Must decode to whole f32s; reject truncated blobs early so
                // face_descriptors() never sees them.
                if bytes.is_empty() || bytes_to_floats(&bytes).is_none() {
                    return Err("invalid descriptor blob".into());
                }
                Some(bytes)
            }
            _ => None,
        };
        tx.execute(
                "INSERT INTO faces (path, cluster_id, x, y, width, height, confidence, detected_at, descriptor)
                 VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    path,
                    f.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    f.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    now(),
                    descriptor,
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

/// Standard base64 decode for face descriptors (no new dependency).
fn b64_value_face(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("invalid base64".to_string()),
    }
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("malformed base64".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return Err("malformed base64".to_string());
        }
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            if c == b'=' {
                if i < 4 - pad {
                    return Err("malformed base64".to_string());
                }
                n <<= 6;
            } else {
                n = (n << 6) | b64_value_face(c)? as u32;
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// All stored face descriptors for global regrouping: (face_id, f32 vector).
/// Skips rows with NULL or empty descriptors; errors when the surviving
/// vectors disagree on dimension (corrupt embed, must not silently cluster).
pub fn face_descriptors(conn: &Connection) -> Result<Vec<(i64, Vec<f32>)>, String> {
    let mut stmt = conn
        .prepare("SELECT id, descriptor FROM faces WHERE descriptor IS NOT NULL ORDER BY id")
        .map_err(|e| format!("prepare face descriptors: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<Vec<u8>>>(1)?)))
        .map_err(|e| format!("query face descriptors: {e}"))?;
    let mut out: Vec<(i64, Vec<f32>)> = Vec::new();
    for row in rows {
        let (id, blob) = row.map_err(|e| format!("face descriptor row: {e}"))?;
        let Some(bytes) = blob else { continue };
        if bytes.is_empty() {
            continue;
        }
        let Some(vec) = bytes_to_floats(&bytes) else { continue };
        if vec.is_empty() {
            continue;
        }
        out.push((id, vec));
    }
    if let Some(first_len) = out.first().map(|(_, v)| v.len()) {
        for (id, v) in &out {
            if v.len() != first_len {
                return Err(format!(
                    "face descriptors have inconsistent dimensions for face {id}: got {}, want {first_len}",
                    v.len()
                ));
            }
        }
    }
    Ok(out)
}

/// Rebuilds person clusters from explicit face-id groups (derived data).
/// Runs in one transaction: snapshots existing membership + names, clears
/// `face_clusters`, re-inserts one row per non-empty group with a live
/// recount, and carries each old name to the new group holding the
/// MUTUAL-majority of that cluster's faces (mirrors cluster_events:
/// overlap * 2 >= both sides, best overlap wins, each old name used once).
/// Ties go to the lowest old cluster id; groups with no majority keep NULL.
pub fn replace_clusters(
    conn: &Connection,
    groups: Vec<Vec<i64>>,
) -> Result<Vec<i64>, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("replace clusters tx: {e}"))?;

    // Snapshot existing clusters (ordered for deterministic tie-breaks).
    let old_clusters: Vec<(i64, Option<String>)> = {
        let mut stmt = tx
            .prepare("SELECT id, name FROM face_clusters ORDER BY id")
            .map_err(|e| format!("prepare old clusters: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))
            .map_err(|e| format!("query old clusters: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("old cluster row: {e}"))?);
        }
        v
    };
    let mut old_members: std::collections::HashMap<i64, std::collections::HashSet<i64>> =
        std::collections::HashMap::new();
    {
        let mut stmt = tx
            .prepare("SELECT cluster_id, id FROM faces WHERE cluster_id IS NOT NULL")
            .map_err(|e| format!("prepare old members: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| format!("query old members: {e}"))?;
        for row in rows {
            let (cid, fid) = row.map_err(|e| format!("old member row: {e}"))?;
            old_members.entry(cid).or_default().insert(fid);
        }
    }

    // Clear derived state. Explicitly null faces first so the rebuild works
    // even when foreign_keys is off; with FK on this is a no-op extra.
    tx.execute("UPDATE faces SET cluster_id = NULL", [])
        .map_err(|e| format!("clear face links: {e}"))?;
    tx.execute("DELETE FROM face_clusters", [])
        .map_err(|e| format!("clear clusters: {e}"))?;

    // Re-insert one row per non-empty group with a live recount.
    let mut new_ids: Vec<i64> = Vec::new();
    let mut new_sets: Vec<(i64, std::collections::HashSet<i64>)> = Vec::new();
    for group in &groups {
        if group.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO face_clusters (face_count, created_at) VALUES (0, ?1)",
            params![now()],
        )
        .map_err(|e| format!("insert cluster: {e}"))?;
        let nid = tx.last_insert_rowid();
        // Link members that actually exist; placeholders keep the SQL static.
        for fid in group {
            tx.execute("UPDATE faces SET cluster_id = ?2 WHERE id = ?1", params![fid, nid])
                .map_err(|e| format!("link face: {e}"))?;
        }
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM faces WHERE cluster_id = ?1",
                params![nid],
                |r| r.get(0),
            )
            .map_err(|e| format!("recount cluster: {e}"))?;
        tx.execute(
            "UPDATE face_clusters SET face_count = ?2 WHERE id = ?1",
            params![nid, count],
        )
        .map_err(|e| format!("recount update: {e}"))?;
        let linked: std::collections::HashSet<i64> = {
            let mut stmt = tx
                .prepare("SELECT id FROM faces WHERE cluster_id = ?1")
                .map_err(|e| format!("linked members: {e}"))?;
            let rows = stmt
                .query_map(params![nid], |r| r.get::<_, i64>(0))
                .map_err(|e| format!("linked members: {e}"))?;
            let mut set = std::collections::HashSet::new();
            for row in rows {
                set.insert(row.map_err(|e| format!("linked member row: {e}"))?);
            }
            set
        };
        // Fall back to the requested ids when the concat trick yields nothing
        // (e.g. empty group already skipped); keeps overlap logic total.
        let set = if linked.is_empty() {
            group.iter().cloned().collect()
        } else {
            linked
        };
        new_ids.push(nid);
        new_sets.push((nid, set));
    }

    // Carry names by mutual majority (events rule). Each old name used once;
    // lowest old cluster id wins ties via ordered scan + strict best.
    let mut used_old = vec![false; old_clusters.len()];
    for (nid, members) in &new_sets {
        if members.is_empty() {
            continue;
        }
        let mut best: Option<usize> = None;
        let mut best_score = 0i64;
        for (k, (oid, oname)) in old_clusters.iter().enumerate() {
            if used_old[k] {
                continue;
            }
            let Some(name) = oname else { continue };
            if name.trim().is_empty() {
                continue;
            }
            let Some(old_set) = old_members.get(oid) else { continue };
            if old_set.is_empty() {
                continue;
            }
            let inter = members.intersection(old_set).count() as i64;
            if inter * 2 >= members.len() as i64
                && inter * 2 >= old_set.len() as i64
                && inter > best_score
            {
                best_score = inter;
                best = Some(k);
            }
        }
        if let Some(k) = best {
            used_old[k] = true;
            let name = old_clusters[k].1.clone().unwrap_or_default();
            tx.execute("UPDATE face_clusters SET name = ?2 WHERE id = ?1", params![nid, name])
                .map_err(|e| format!("carry cluster name: {e}"))?;
        }
    }

    tx.commit().map_err(|e| format!("replace clusters commit: {e}"))?;
    Ok(new_ids)
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
    let tx = conn.unchecked_transaction().map_err(|e| format!("rename tx: {e}"))?;
    let changed = tx
        .execute("UPDATE face_clusters SET name = ?2 WHERE id = ?1", params![cluster_id, name])
        .map_err(|e| format!("rename cluster: {e}"))?;
    if changed == 0 {
        return Err(format!("cluster {cluster_id} not found"));
    }
    sync_person_tags(&tx, cluster_id)?;
    tx.commit().map_err(|e| format!("rename commit: {e}"))?;
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
    // The source row is gone: re-stamp the survivor so moved photos carry
    // its name and the source's old stamp is dropped where unjustified.
    sync_person_tags(&tx, into_id)?;
    tx.commit().map_err(|e| format!("merge commit: {e}"))?;
    Ok(count)
}

/// Detaches one face from its cluster (the photo is untouched; the face
/// becomes unassigned). Recounts the cluster. Returns the cluster id so
/// callers can refresh. Errors when the face is unknown or already free.
pub fn unassign_face(conn: &Connection, face_id: i64) -> Result<i64, String> {
    let cluster: Option<Option<i64>> = conn
        .query_row("SELECT cluster_id FROM faces WHERE id = ?1", params![face_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("face lookup: {e}"))?;
    let cluster_id = match cluster {
        None => return Err(format!("face {face_id} not found")),
        Some(None) => return Err(format!("face {face_id} is not assigned to any cluster")),
        Some(Some(id)) => id,
    };
    let tx = conn.unchecked_transaction().map_err(|e| format!("unassign tx: {e}"))?;
    tx.execute("UPDATE faces SET cluster_id = NULL WHERE id = ?1", params![face_id])
        .map_err(|e| format!("unassign face: {e}"))?;
    tx.execute(
        "UPDATE face_clusters SET face_count = (SELECT COUNT(*) FROM faces WHERE cluster_id = ?1)
         WHERE id = ?1",
        params![cluster_id],
    )
    .map_err(|e| format!("unassign recount: {e}"))?;
    // The photo loses its stamp only when this was its last link to the
    // named cluster; sync decides per asset, not per face.
    sync_person_tags(&tx, cluster_id)?;
    tx.commit().map_err(|e| format!("unassign commit: {e}"))?;
    Ok(cluster_id)
}

/// (id, path) lookup for undo-restore. The ~/.Trash move itself lives in
/// the command layer (main-thread NSFileManager), so this DB half stays
/// unit-testable and the filesystem hop is mock-covered.
pub fn asset_paths(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, String)>, String> {
    let json = serde_json::to_string(ids).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, path FROM assets WHERE id IN (SELECT value FROM json_each(?1))")
        .map_err(|e| format!("restore lookup: {e}"))?;
    let rows = stmt
        .query_map(params![json], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("restore lookup: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("restore row: {e}"))?);
    }
    Ok(out)
}

/// Clears the missing flag for rows moved back from the Trash.
pub fn clear_missing(conn: &Connection, ids: &[i64]) -> Result<usize, String> {
    if ids.is_empty() {
        return Ok(0);
    }
    let json = serde_json::to_string(ids).map_err(|e| e.to_string())?;
    let changed = conn
        .execute(
            "UPDATE assets SET missing = 0 WHERE id IN (SELECT value FROM json_each(?1))",
            params![json],
        )
        .map_err(|e| format!("restore flag: {e}"))?;
    Ok(changed)
}

/// Reattaches one face to a cluster (undo for unassign_face). Recounts the
/// target (and the previous cluster when the face moves between two live
/// clusters). Errors when the face or the cluster is unknown.
pub fn assign_face(conn: &Connection, face_id: i64, cluster_id: i64) -> Result<(), String> {
    let face: Option<Option<i64>> = conn
        .query_row("SELECT cluster_id FROM faces WHERE id = ?1", params![face_id], |r| r.get(0))
        .optional()
        .map_err(|e| format!("face lookup: {e}"))?;
    let prev = match face {
        None => return Err(format!("face {face_id} not found")),
        Some(v) => v,
    };
    let cluster_exists: i64 = conn
        .query_row("SELECT COUNT(*) FROM face_clusters WHERE id = ?1", params![cluster_id], |r| {
            r.get(0)
        })
        .map_err(|e| format!("cluster lookup: {e}"))?;
    if cluster_exists == 0 {
        return Err(format!("cluster {cluster_id} not found"));
    }
    if prev == Some(cluster_id) {
        return Ok(());
    }
    let tx = conn.unchecked_transaction().map_err(|e| format!("assign tx: {e}"))?;
    tx.execute("UPDATE faces SET cluster_id = ?2 WHERE id = ?1", params![face_id, cluster_id])
        .map_err(|e| format!("assign face: {e}"))?;
    tx.execute(
        "UPDATE face_clusters SET face_count = (SELECT COUNT(*) FROM faces WHERE cluster_id = ?1)
         WHERE id = ?1",
        params![cluster_id],
    )
    .map_err(|e| format!("assign recount: {e}"))?;
    if let Some(old) = prev {
        tx.execute(
            "UPDATE face_clusters SET face_count = (SELECT COUNT(*) FROM faces WHERE cluster_id = ?1)
             WHERE id = ?1",
            params![old],
        )
        .map_err(|e| format!("assign recount: {e}"))?;
    }
    sync_person_tags(&tx, cluster_id)?;
    if let Some(old) = prev {
        sync_person_tags(&tx, old)?;
    }
    tx.commit().map_err(|e| format!("assign commit: {e}"))?;
    Ok(())
}

/// Reinserts one face_clusters row (undo for merge_clusters) and moves the
/// given faces into it. The name is the snapshot from before the merge;
/// blank snapshots restore as NULL so the card renders "Cluster <id>".
/// Returns the new cluster id (a fresh rowid, not the dropped one).
pub fn restore_cluster(
    conn: &Connection,
    name: Option<&str>,
    face_ids: &[i64],
) -> Result<i64, String> {
    if face_ids.is_empty() {
        return Err("no faces to restore".into());
    }
    let clean = name.map(str::trim).filter(|n| !n.is_empty());
    let json = serde_json::to_string(face_ids).map_err(|e| e.to_string())?;
    let found: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM faces WHERE id IN (SELECT value FROM json_each(?1))",
            params![json],
            |r| r.get(0),
        )
        .map_err(|e| format!("restore faces lookup: {e}"))?;
    if found != face_ids.len() as i64 {
        return Err("one of the faces does not exist".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| format!("restore tx: {e}"))?;
    // Survivors losing members need a recount + tag re-sync afterwards.
    let mut prev: Vec<i64> = {
        let mut stmt = tx
            .prepare(
                "SELECT DISTINCT cluster_id FROM faces
                 WHERE id IN (SELECT value FROM json_each(?1)) AND cluster_id IS NOT NULL",
            )
            .map_err(|e| format!("restore survivors: {e}"))?;
        let rows = stmt
            .query_map(params![json], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("restore survivors: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("restore survivor row: {e}"))?);
        }
        v
    };
    prev.sort_unstable();
    prev.dedup();
    tx.execute(
        "INSERT INTO face_clusters (face_count, created_at, name) VALUES (0, ?1, ?2)",
        params![now(), clean],
    )
    .map_err(|e| format!("restore cluster: {e}"))?;
    let nid = tx.last_insert_rowid();
    for fid in face_ids {
        tx.execute("UPDATE faces SET cluster_id = ?2 WHERE id = ?1", params![fid, nid])
            .map_err(|e| format!("restore link: {e}"))?;
    }
    let count: i64 = tx
        .query_row("SELECT COUNT(*) FROM faces WHERE cluster_id = ?1", params![nid], |r| {
            r.get(0)
        })
        .map_err(|e| format!("restore count: {e}"))?;
    tx.execute(
        "UPDATE face_clusters SET face_count = ?2 WHERE id = ?1",
        params![nid, count],
    )
    .map_err(|e| format!("restore recount: {e}"))?;
    for pid in &prev {
        if *pid == nid {
            continue;
        }
        tx.execute(
            "UPDATE face_clusters SET face_count = (SELECT COUNT(*) FROM faces WHERE cluster_id = ?1)
             WHERE id = ?1",
            params![pid],
        )
        .map_err(|e| format!("restore recount: {e}"))?;
    }
    sync_person_tags(&tx, nid)?;
    for pid in &prev {
        if *pid == nid {
            continue;
        }
        sync_person_tags(&tx, *pid)?;
    }
    tx.commit().map_err(|e| format!("restore commit: {e}"))?;
    Ok(nid)
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

// MARK: - Pet sightings (dogs/cats; no identity tracking yet)

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetSighting {
    pub id: i64,
    pub path: String,
    pub label: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub confidence: f64,
}

pub fn save_sightings(
    conn: &Connection,
    path: &str,
    sightings: &[serde_json::Value],
) -> Result<Vec<i64>, String> {
    let tx = conn.unchecked_transaction().map_err(|e| format!("sightings tx: {e}"))?;
    tx.execute("DELETE FROM animal_sightings WHERE path = ?1", params![path])
        .map_err(|e| format!("clear sightings: {e}"))?;
    let mut ids = Vec::new();
    for s in sightings {
        let label = s.get("label").and_then(|v| v.as_str()).unwrap_or("animal");
        tx.execute(
            "INSERT INTO animal_sightings (path, label, x, y, width, height, confidence, detected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                path,
                label,
                s.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0),
                s.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0),
                s.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0),
                s.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0),
                s.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                now(),
            ],
        )
        .map_err(|e| format!("insert sighting: {e}"))?;
        let id: i64 = tx.query_row("SELECT last_insert_rowid()", [], |r| r.get(0))
            .map_err(|e| format!("last row id: {e}"))?;
        ids.push(id);
    }
    tx.commit().map_err(|e| format!("sightings commit: {e}"))?;
    Ok(ids)
}

pub fn sightings_list(conn: &Connection) -> Result<Vec<PetSighting>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, label, x, y, width, height, confidence FROM animal_sightings
             ORDER BY confidence DESC, id",
        )
        .map_err(|e| format!("prepare sightings: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PetSighting {
                id: r.get(0)?,
                path: r.get(1)?,
                label: r.get(2)?,
                x: r.get(3)?,
                y: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                confidence: r.get(7)?,
            })
        })
        .map_err(|e| format!("query sightings: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("sighting row: {e}"))?);
    }
    Ok(out)
}

pub fn sightings_count(conn: &Connection) -> Result<i64, String> {
    conn.query_row("SELECT COUNT(*) FROM animal_sightings", [], |r| r.get(0))
        .map_err(|e| format!("sightings_count: {e}"))
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

    // Snapshot manual names/places before the rebuild wipes them: new
    // events that overlap an old one by member majority inherit them.
    let old_events: Vec<(String, Option<String>, std::collections::HashSet<i64>)> = {
        let mut stmt = conn
            .prepare(
                "SELECT e.name, e.place, ea.asset_id FROM events e
                 LEFT JOIN event_assets ea ON ea.event_id = e.id",
            )
            .map_err(|e| format!("prepare old events: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?)))
            .map_err(|e| format!("query old events: {e}"))?;
        let mut by_name: std::collections::HashMap<(String, Option<String>), std::collections::HashSet<i64>> =
            std::collections::HashMap::new();
        for row in rows {
            let (name, place, asset) = row.map_err(|e| format!("old event row: {e}"))?;
            let entry = by_name.entry((name, place)).or_default();
            if let Some(id) = asset {
                entry.insert(id);
            }
        }
        by_name.into_iter().map(|((name, place), members)| (name, place, members)).collect()
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

    // Carry manual names/places across rebuilds: a new event whose members
    // overlap an old one by majority on both sides inherits them. Each old
    // event matches at most once; leftovers keep their fresh default names.
    if !old_events.is_empty() {
        let mut new_sets: Vec<(i64, std::collections::HashSet<i64>)> = Vec::new();
        {
            let mut stmt = conn
                .prepare("SELECT event_id, asset_id FROM event_assets")
                .map_err(|e| format!("prepare new members: {e}"))?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .map_err(|e| format!("query new members: {e}"))?;
            let mut by_event: std::collections::HashMap<i64, std::collections::HashSet<i64>> =
                std::collections::HashMap::new();
            for row in rows {
                let (eid, aid) = row.map_err(|e| format!("new member row: {e}"))?;
                by_event.entry(eid).or_default().insert(aid);
            }
            new_sets = by_event.into_iter().collect();
            new_sets.sort_by_key(|(eid, _)| *eid);
        }
        let mut used_old = vec![false; old_events.len()];
        for (eid, members) in &new_sets {
            if members.is_empty() {
                continue;
            }
            let mut best: Option<usize> = None;
            let mut best_score = 0i64;
            for (k, (_, _, old_members)) in old_events.iter().enumerate() {
                if used_old[k] || old_members.is_empty() {
                    continue;
                }
                let inter: i64 = members.intersection(old_members).count() as i64;
                // Majority on both sides, then best overlap wins.
                if inter * 2 >= members.len() as i64
                    && inter * 2 >= old_members.len() as i64
                    && inter > best_score
                {
                    best_score = inter;
                    best = Some(k);
                }
            }
            if let Some(k) = best {
                used_old[k] = true;
                let (name, place, _) = &old_events[k];
                conn.execute(
                    "UPDATE events SET name = ?2, place = ?3 WHERE id = ?1",
                    params![eid, name, place],
                )
                .map_err(|e| format!("carry event name: {e}"))?;
            }
        }
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
/// Coarse `≈`-marked labels count as missing: they are provisional and a
/// network resolve refines them.
pub fn events_missing_places(conn: &Connection) -> Result<Vec<PlaceTarget>, String> {
    let mut stmt = conn
        .prepare("SELECT id, latitude, longitude FROM events WHERE (place IS NULL OR place LIKE '≈ %') AND latitude IS NOT NULL AND longitude IS NOT NULL ORDER BY id")
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
    bbox: Option<(f64, f64, f64, f64)>,
) -> Result<(), String> {
    let (min_lat, max_lat, min_lon, max_lon) = match bbox {
        Some((a, b, c, d)) => (Some(a), Some(b), Some(c), Some(d)),
        None => (None, None, None, None),
    };
    conn.execute(
        "INSERT INTO geocode_cache (lat_key, lon_key, place, fetched_at, min_lat, max_lat, min_lon, max_lon)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (lat_key, lon_key) DO UPDATE SET place = excluded.place, fetched_at = excluded.fetched_at,
             min_lat = excluded.min_lat, max_lat = excluded.max_lat,
             min_lon = excluded.min_lon, max_lon = excluded.max_lon",
        params![lat_key, lon_key, place, now(), min_lat, max_lat, min_lon, max_lon],
    )
    .map_err(|e| format!("geocode store: {e}"))?;
    Ok(())
}

/// Geofence reuse: the cached place whose bounding box contains the point,
/// smallest box wins (a neighbourhood beats its city). None when no cached
/// box covers the point — the caller then hits the network.
pub fn geocode_covering(conn: &Connection, lat: f64, lon: f64) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT place FROM geocode_cache
         WHERE min_lat IS NOT NULL AND ?1 BETWEEN min_lat AND max_lat
           AND ?2 BETWEEN min_lon AND max_lon
         ORDER BY (max_lat - min_lat) * (max_lon - min_lon) ASC, fetched_at DESC
         LIMIT 1",
        params![lat, lon],
        |r| r.get(0),
    )
    .optional()
    .map_err(|e| format!("geocode covering: {e}"))
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
            iso          = COALESCE(?12, iso),
            f_number     = COALESCE(?13, f_number),
            exposure_time = COALESCE(?14, exposure_time),
            focal_length = COALESCE(?15, focal_length),
            focal_length_35 = COALESCE(?16, focal_length_35),
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
            n("iso"),
            f("fNumber"),
            f("exposureTime"),
            f("focalLength"),
            n("focalLength35"),
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

// MARK: - Near duplicates (perceptual dHash grouping)

/// Hamming distance between two 16-hex-char dHashes; None on malformed input.
pub fn dhash_distance(a: &str, b: &str) -> Option<u32> {
    if a.len() != 16 || b.len() != 16 {
        return None;
    }
    let (x, y) = (u64::from_str_radix(a, 16).ok()?, u64::from_str_radix(b, 16).ok()?);
    Some((x ^ y).count_ones())
}

/// Pairs at or under this distance read as "visually the same shot".
pub const NEAR_DUPE_THRESHOLD: u32 = 8;
/// Pairwise grouping is O(n^2); above this many hashed images the pass
/// refuses rather than wedging the app.
pub const NEAR_DUPE_MAX_IMAGES: i64 = 20_000;

pub fn dhash_candidates(conn: &Connection, limit: i64) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path FROM assets
             WHERE kind = 'image' AND missing = 0 AND dhash IS NULL
             ORDER BY id LIMIT ?1",
        )
        .map_err(|e| format!("prepare dhash candidates: {e}"))?;
    let rows = stmt
        .query_map(params![limit], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query dhash candidates: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("dhash candidate row: {e}"))?);
    }
    Ok(out)
}

pub fn apply_dhash(conn: &Connection, id: i64, dhash: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE assets SET dhash = ?2, dhash_at = ?3, dhash_error = NULL,
                dhash_attempts = dhash_attempts + 1
         WHERE id = ?1",
        params![id, dhash, now()],
    )
    .map_err(|e| format!("apply_dhash: {e}"))?;
    Ok(())
}

pub fn apply_dhash_failure(conn: &Connection, id: i64, error: &str) -> Result<(), String> {
    let message: String = error.chars().take(200).collect();
    conn.execute(
        "UPDATE assets SET dhash_error = ?2, dhash_attempts = dhash_attempts + 1 WHERE id = ?1",
        params![id, message],
    )
    .map_err(|e| format!("apply_dhash_failure: {e}"))?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearGroup {
    pub id: i64,
    pub waste_bytes: i64,
    pub kept_id: Option<i64>,
    pub members: Vec<DuplicateMember>,
}

/// Rebuilds near-duplicate groups from dHashes with union-find over all
/// pairs at or under NEAR_DUPE_THRESHOLD. Keeper order mirrors the exact
/// system: favourite, then sharpness, then earliest capture, then lowest id.
pub fn rebuild_near_groups(conn: &Connection) -> Result<(i64, i64), String> {
    // (id, path, name, size, favorite, quality, taken_at, dhash)
    let assets: Vec<(i64, String, String, i64, i64, Option<f64>, Option<f64>, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT id, path, name, size_bytes, favorite, quality_score, COALESCE(captured_at, modified), dhash
                 FROM assets WHERE kind = 'image' AND missing = 0 AND dhash IS NOT NULL",
            )
            .map_err(|e| format!("prepare near assets: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Option<f64>>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, String>(7)?,
                ))
            })
            .map_err(|e| format!("query near assets: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("near asset row: {e}"))?);
        }
        v
    };
    if assets.len() as i64 > NEAR_DUPE_MAX_IMAGES {
        return Err(format!(
            "near-dupe pass refuses {} images (cap {}) — narrow the library first",
            assets.len(),
            NEAR_DUPE_MAX_IMAGES
        ));
    }

    // Union-find over qualifying pairs.
    let mut parent: Vec<usize> = (0..assets.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for i in 0..assets.len() {
        for j in (i + 1)..assets.len() {
            let close = dhash_distance(&assets[i].7, &assets[j].7)
                .map_or(false, |d| d <= NEAR_DUPE_THRESHOLD);
            if close {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut components: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..assets.len() {
        let r = find(&mut parent, i);
        components.entry(r).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = components
        .into_values()
        .filter(|c| c.len() > 1)
        .collect();
    // Deterministic order: biggest groups first.
    groups.sort_by_key(|c| std::cmp::Reverse(c.len()));

    let tx = conn.unchecked_transaction().map_err(|e| format!("near tx: {e}"))?;
    tx.execute("UPDATE assets SET near_group_id = NULL, is_near_duplicate = 0", [])
        .map_err(|e| format!("reset near flags: {e}"))?;
    tx.execute("DELETE FROM near_groups", [])
        .map_err(|e| format!("clear near groups: {e}"))?;

    let mut group_count = 0i64;
    let mut waste_total = 0i64;
    for comp in &groups {
        // Keeper order: favourite, sharpness, earliest capture, lowest id.
        let mut members: Vec<usize> = comp.clone();
        members.sort_by(|&a, &b| {
            assets[b]
                .4
                .cmp(&assets[a].4)
                .then(assets[b].5.partial_cmp(&assets[a].5).unwrap_or(std::cmp::Ordering::Equal))
                .then(assets[a].6.partial_cmp(&assets[b].6).unwrap_or(std::cmp::Ordering::Equal))
                .then(assets[a].0.cmp(&assets[b].0))
        });
        let kept = assets[members[0]].0;
        let waste: i64 = members[1..].iter().map(|&m| assets[m].3).sum();
        tx.execute(
            "INSERT INTO near_groups (kept_id, waste_bytes, created_at) VALUES (?1, ?2, ?3)",
            params![kept, waste, now()],
        )
        .map_err(|e| format!("insert near group: {e}"))?;
        let group_id = tx.last_insert_rowid();
        for &m in &members {
            let is_dup = i64::from(assets[m].0 != kept);
            tx.execute(
                "UPDATE assets SET near_group_id = ?2, is_near_duplicate = ?3 WHERE id = ?1",
                params![assets[m].0, group_id, is_dup],
            )
            .map_err(|e| format!("mark near member: {e}"))?;
        }
        group_count += 1;
        waste_total += waste;
    }

    tx.commit().map_err(|e| format!("near commit: {e}"))?;
    Ok((group_count, waste_total))
}

pub fn near_groups(conn: &Connection) -> Result<Vec<NearGroup>, String> {
    let groups: Vec<NearGroup> = {
        let mut stmt = conn
            .prepare("SELECT id, waste_bytes, kept_id FROM near_groups ORDER BY waste_bytes DESC, id")
            .map_err(|e| format!("prepare list near groups: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok(NearGroup { id: r.get(0)?, waste_bytes: r.get(1)?, kept_id: r.get(2)?, members: Vec::new() })
            })
            .map_err(|e| format!("list near groups: {e}"))?;
        let mut v = Vec::new();
        for row in rows {
            v.push(row.map_err(|e| format!("near group row: {e}"))?);
        }
        v
    };

    let mut out = Vec::new();
    for mut g in groups {
        let members: Vec<DuplicateMember> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, path, name, is_near_duplicate, captured_at, quality_score FROM assets
                     WHERE near_group_id = ?1 ORDER BY is_near_duplicate ASC, captured_at ASC, id",
                )
                .map_err(|e| format!("prepare near members: {e}"))?;
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
                .map_err(|e| format!("query near members: {e}"))?;
            let mut v = Vec::new();
            for row in rows {
                v.push(row.map_err(|e| format!("near member row: {e}"))?);
            }
            v
        };
        g.members = members;
        out.push(g);
    }
    Ok(out)
}

pub fn near_keep(conn: &Connection, group_id: i64, keep_id: i64) -> Result<(), String> {
    let belongs: bool = conn
        .query_row(
            "SELECT 1 FROM assets WHERE id = ?1 AND near_group_id = ?2",
            params![keep_id, group_id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !belongs {
        return Err(format!("asset {keep_id} is not part of near group {group_id}"));
    }
    conn.execute(
        "UPDATE near_groups SET kept_id = ?2, resolved = 1 WHERE id = ?1",
        params![group_id, keep_id],
    )
    .map_err(|e| format!("near keep: {e}"))?;
    conn.execute(
        "UPDATE assets SET is_near_duplicate = (id != ?2) WHERE near_group_id = ?1",
        params![group_id, keep_id],
    )
    .map_err(|e| format!("near keep flags: {e}"))?;
    Ok(())
}

// MARK: - Visual similarity vectors (similar-photos v1)

/// Encoder model shipped in v1: Apple's built-in Vision feature print
/// (zero download). Stored per row so an opt-in upgrade never re-keys.
pub const VECTOR_MODEL: &str = "vn-featureprint";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarHit {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub kind: String,
    pub score: f64,
}

/// Cosine similarity in f64; 0.0 when either side is degenerate so a broken
/// vector sorts last instead of poisoning the ranking with NaN.
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for i in 0..a.len() {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Little-endian f32 bytes, the on-disk form of `vec`.
pub fn floats_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Inverse of `floats_to_bytes`; None on a truncated blob.
pub fn bytes_to_floats(b: &[u8]) -> Option<Vec<f32>> {
    if b.len() % 4 != 0 {
        return None;
    }
    Some(
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Inserts or replaces one asset's vector for `model`. Re-embedding the same
/// file is idempotent: the row is overwritten, never duplicated.
pub fn save_vector(
    conn: &Connection,
    asset_id: i64,
    model: &str,
    dim: i64,
    vec: &[f32],
) -> Result<(), String> {
    if vec.len() as i64 != dim {
        return Err(format!(
            "vector dim mismatch for asset {asset_id}: got {}, want {dim}",
            vec.len()
        ));
    }
    conn.execute(
        "INSERT INTO asset_vectors (asset_id, model, dim, vec) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (asset_id) DO UPDATE SET model = excluded.model, dim = excluded.dim, vec = excluded.vec",
        params![asset_id, model, dim, floats_to_bytes(vec)],
    )
    .map_err(|e| format!("save vector: {e}"))?;
    Ok(())
}

/// Readable images with no vector for `model` yet, for the embed backfill.
/// Failures leave no row, so they are retried on the next pass.
pub fn embed_candidates(conn: &Connection, model: &str, limit: i64) -> Result<Vec<(i64, String)>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.path FROM assets a
             WHERE a.kind = 'image' AND a.missing = 0
               AND NOT EXISTS (SELECT 1 FROM asset_vectors v WHERE v.asset_id = a.id AND v.model = ?1)
             ORDER BY a.id LIMIT ?2",
        )
        .map_err(|e| format!("prepare embed candidates: {e}"))?;
    let rows = stmt
        .query_map(params![model, limit], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| format!("query embed candidates: {e}"))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| format!("embed candidate row: {e}"))?);
    }
    Ok(out)
}

/// Brute-force cosine over the stored BLOBs for `model`, best first, query
/// excluded. Libraries here are ≤ ~50k assets: fast enough with no index.
pub fn similar_to(conn: &Connection, asset_id: i64, model: &str, limit: i64) -> Result<Vec<SimilarHit>, String> {
    let query: Vec<f32> = conn
        .query_row(
            "SELECT vec FROM asset_vectors WHERE asset_id = ?1 AND model = ?2",
            params![asset_id, model],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .optional()
        .map_err(|e| format!("similar query vector: {e}"))?
        .and_then(|b| bytes_to_floats(&b))
        .ok_or_else(|| format!("asset {asset_id} has no '{model}' vector yet"))?;
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.path, a.name, a.kind, v.vec FROM asset_vectors v
             JOIN assets a ON a.id = v.asset_id
             WHERE v.model = ?1 AND v.asset_id != ?2 AND a.missing = 0",
        )
        .map_err(|e| format!("prepare similar: {e}"))?;
    let rows = stmt
        .query_map(params![model, asset_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Vec<u8>>(4)?,
            ))
        })
        .map_err(|e| format!("query similar: {e}"))?;
    let mut hits = Vec::new();
    for row in rows {
        let (id, path, name, kind, blob) = row.map_err(|e| format!("similar row: {e}"))?;
        let Some(other) = bytes_to_floats(&blob) else { continue };
        hits.push(SimilarHit { id, path, name, kind, score: cosine(&query, &other) });
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    hits.truncate(limit.max(0) as usize);
    Ok(hits)
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
    /// Some(true) = photos with a detected pet only.
    pub has_pet: Option<bool>,
    /// Substring match on file name or path (search box). The LIKE pattern
    /// (with `%`/`_` escaped) is built inside library()/library_count().
    pub query: Option<&'a str>,
    /// Newest-first by first_seen_at (the Pinned "Recently Saved" shortcut).
    pub sort_added: bool,
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
  AND (?8 IS NULL OR EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag = ?8))
  AND (?9 IS NULL OR a.name LIKE ?9 ESCAPE '\\' OR a.path LIKE ?9 ESCAPE '\\'
    OR EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag LIKE ?9 ESCAPE '\\')
    OR EXISTS (SELECT 1 FROM event_assets ea JOIN events e ON e.id = ea.event_id
               WHERE ea.asset_id = a.id AND (e.name LIKE ?9 ESCAPE '\\' OR e.place LIKE ?9 ESCAPE '\\'))
    OR EXISTS (SELECT 1 FROM faces f JOIN face_clusters fc ON fc.id = f.cluster_id
               WHERE f.path = a.path AND fc.name LIKE ?9 ESCAPE '\\')
    OR EXISTS (SELECT 1 FROM animal_sightings s
               WHERE s.path = a.path AND s.label LIKE ?9 ESCAPE '\\'))
  AND (?10 IS NULL OR ((?10 = 1) = EXISTS (SELECT 1 FROM animal_sightings s WHERE s.path = a.path)))";

/// Relevance tier for search results; inert (all zero) without a query.
/// Name/path hits outrank tag, event, and person hits; recency breaks ties.
const LIBRARY_RANK: &str = "CASE WHEN ?9 IS NULL THEN 0
  WHEN a.name LIKE ?9 ESCAPE '\\' OR a.path LIKE ?9 ESCAPE '\\' THEN 0
  WHEN EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id = a.id AND t.tag LIKE ?9 ESCAPE '\\') THEN 1
  WHEN EXISTS (SELECT 1 FROM animal_sightings s WHERE s.path = a.path AND s.label LIKE ?9 ESCAPE '\\') THEN 1
  WHEN EXISTS (SELECT 1 FROM event_assets ea JOIN events e ON e.id = ea.event_id
               WHERE ea.asset_id = a.id AND (e.name LIKE ?9 ESCAPE '\\' OR e.place LIKE ?9 ESCAPE '\\')) THEN 2
  WHEN EXISTS (SELECT 1 FROM faces f JOIN face_clusters fc ON fc.id = f.cluster_id
               WHERE f.path = a.path AND fc.name LIKE ?9 ESCAPE '\\') THEN 3
  ELSE 4 END";

pub fn library(conn: &Connection, q: &LibraryQuery) -> Result<Vec<AssetRow>, String> {
    let like = q.query.map(|s| format!("%{}%", escape_like(s)));
    // Recently Saved skips relevance ranking: pure insert-recency order.
    let order = if q.sort_added {
        "first_seen_at DESC, id DESC".to_string()
    } else {
        format!("{LIBRARY_RANK}, COALESCE(captured_at, modified) DESC, id DESC")
    };
    let sql = format!(
        "SELECT id, path, name, ext, kind, mime, size_bytes, modified, captured_at,
                        width, height, duration, favorite, missing, sha256,
                 (SELECT e.place FROM event_assets ea JOIN events e ON e.id = ea.event_id
                   WHERE ea.asset_id = a.id AND e.place IS NOT NULL AND e.place != ''
                   ORDER BY e.id LIMIT 1),
                 (SELECT GROUP_CONCAT(fc.name, CHAR(31)) FROM faces f
                   JOIN face_clusters fc ON fc.id = f.cluster_id
                   WHERE f.path = a.path AND fc.name IS NOT NULL AND fc.name != ''),
                 camera_make, camera_model, lens, latitude, longitude,
                 iso, f_number, exposure_time, focal_length, focal_length_35
                 FROM assets a
                 {LIBRARY_WHERE}
                 ORDER BY {order}
                 LIMIT ?11 OFFSET ?12"
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
                like.as_deref(),
                q.has_pet.map(i64::from),
                q.limit,
                q.offset
            ],
            |r| {
            let raw_people: Option<String> = r.get(16)?;
            let mut people: Vec<String> = raw_people
                .map(|s| s.split('\x1f').filter(|p| !p.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
            people.sort();
            people.dedup();
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
                place: r.get(15)?,
                people,
                camera_make: r.get(17)?,
                camera_model: r.get(18)?,
                lens: r.get(19)?,
                latitude: r.get(20)?,
                longitude: r.get(21)?,
                iso: r.get(22)?,
                f_number: r.get(23)?,
                exposure_time: r.get(24)?,
                focal_length: r.get(25)?,
                focal_length_35: r.get(26)?,
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
    let like = q.query.map(|s| format!("%{}%", escape_like(s)));
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
            q.scene_tag,
            like.as_deref(),
            q.has_pet.map(i64::from)
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

// MARK: - Person tags (named-cluster stamps on photos)

/// Canonical asset tag for a named person cluster: `person:<name>`.
/// None for blank names so unnamed clusters stamp nothing.
pub fn person_tag(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(format!("person:{trimmed}"))
    }
}

/// Full reconcile for one cluster: every asset holding a face in the cluster
/// carries the cluster's current `person:<name>` tag (confidence 1.0), and
/// any `person:*` tag on an affected asset that no linked named cluster
/// justifies is removed. Affected = current members plus holders of the
/// desired tag, so unassign-last and merge-away clean up without knowing
/// old names, and renames replace instead of stacking. Scene tags and other
/// `person:*` stamps from sibling clusters sharing the photo are untouched.
pub fn sync_person_tags(conn: &Connection, cluster_id: i64) -> Result<(), String> {
    let name: Option<String> = conn
        .query_row("SELECT name FROM face_clusters WHERE id = ?1", params![cluster_id], |r| {
            r.get(0)
        })
        .map_err(|e| format!("person sync lookup: {e}"))?;
    let desired = name.as_deref().and_then(person_tag);

    let members: Vec<(i64, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT a.id, a.path FROM assets a
                 WHERE EXISTS (SELECT 1 FROM faces f WHERE f.cluster_id = ?1 AND f.path = a.path)",
            )
            .map_err(|e| format!("person sync members: {e}"))?;
        let rows = stmt
            .query_map(params![cluster_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| format!("person sync members: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("person sync member row: {e}"))?);
        }
        out
    };

    let mut affected: std::collections::HashMap<i64, String> =
        members.iter().cloned().collect();
    if let Some(tag) = desired.as_deref() {
        let mut stmt = conn
            .prepare("SELECT asset_id FROM asset_tags WHERE tag = ?1")
            .map_err(|e| format!("person sync holders: {e}"))?;
        let rows = stmt
            .query_map(params![tag], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("person sync holders: {e}"))?;
        for row in rows {
            let aid = row.map_err(|e| format!("person sync holder row: {e}"))?;
            if !affected.contains_key(&aid) {
                let path: Option<String> = conn
                    .query_row("SELECT path FROM assets WHERE id = ?1", params![aid], |r| {
                        r.get(0)
                    })
                    .optional()
                    .map_err(|e| format!("person sync holder path: {e}"))?;
                if let Some(p) = path {
                    affected.insert(aid, p);
                }
            }
        }
    }

    for (aid, path) in &affected {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT fc.name FROM faces f
                 JOIN face_clusters fc ON fc.id = f.cluster_id
                 WHERE f.path = ?1 AND fc.name IS NOT NULL AND TRIM(fc.name) <> ''",
            )
            .map_err(|e| format!("person sync linked: {e}"))?;
        let linked: Vec<String> = stmt
            .query_map(params![path], |r| r.get(0))
            .map_err(|e| format!("person sync linked: {e}"))?
            .filter_map(|r| r.map_err(|e| format!("person sync linked row: {e}")).transpose())
            .collect::<Result<Vec<String>, String>>()?;
        let justified: std::collections::HashSet<String> =
            linked.iter().filter_map(|n| person_tag(n)).collect();

        let mut stmt = conn
            .prepare("SELECT tag FROM asset_tags WHERE asset_id = ?1 AND tag LIKE 'person:%'")
            .map_err(|e| format!("person sync current: {e}"))?;
        let current: Vec<String> = stmt
            .query_map(params![aid], |r| r.get(0))
            .map_err(|e| format!("person sync current: {e}"))?
            .filter_map(|r| r.map_err(|e| format!("person sync current row: {e}")).transpose())
            .collect::<Result<Vec<String>, String>>()?;
        for tag in &current {
            if !justified.contains(tag) {
                conn.execute(
                    "DELETE FROM asset_tags WHERE asset_id = ?1 AND tag = ?2",
                    params![aid, tag],
                )
                .map_err(|e| format!("person sync remove: {e}"))?;
            }
        }
        for tag in &justified {
            conn.execute(
                "INSERT INTO asset_tags (asset_id, tag, confidence) VALUES (?1, ?2, 1.0)
                 ON CONFLICT (asset_id, tag) DO UPDATE SET confidence = excluded.confidence",
                params![aid, tag],
            )
            .map_err(|e| format!("person sync stamp: {e}"))?;
        }
    }
    Ok(())
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
        assert_eq!(migrate(&conn).unwrap(), vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
        assert!(migrate(&conn).unwrap().is_empty(), "second call is a no-op");
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 15, "user_version tracks the highest applied migration");

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
    fn library_sort_added_orders_by_first_seen() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/old.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/new.jpg", 1, 100.0), scan).unwrap();
        // Flip insert recency: /old.jpg was (re)seen much later.
        conn.execute("UPDATE assets SET first_seen_at = first_seen_at + 100000 WHERE path = '/old.jpg'", []).unwrap();

        let rows = library(&conn, &LibraryQuery { sort_added: true, limit: 10, ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "/old.jpg", "recently saved first");
        assert_eq!(rows[1].path, "/new.jpg");
    }

    #[test]
    fn apply_stat_persists_exposure_fields() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 100, 10.0), scan).unwrap();
        let id: i64 = conn
            .query_row("SELECT id FROM assets WHERE path = '/a.jpg'", [], |r| r.get(0))
            .unwrap();
        let stat = serde_json::json!({
            "capturedAt": 50.0, "width": 4032, "height": 3024,
            "cameraMake": "TestCam", "cameraModel": "TC-1", "lens": "TC 26mm",
            "latitude": 37.78, "longitude": -122.41,
            "iso": 100, "fNumber": 2.8, "exposureTime": 0.004,
            "focalLength": 6.8, "focalLength35": 26
        });
        apply_stat(&conn, id, &stat).unwrap();

        let rows = library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 1);
        let a = &rows[0];
        assert_eq!(a.iso, Some(100));
        assert_eq!(a.camera_make.as_deref(), Some("TestCam"));
        assert_eq!(a.camera_model.as_deref(), Some("TC-1"));
        assert_eq!(a.lens.as_deref(), Some("TC 26mm"));
        assert_eq!(a.latitude, Some(37.78));
        assert_eq!(a.longitude, Some(-122.41));
        assert!((a.f_number.unwrap() - 2.8).abs() < 1e-9);
        assert!((a.exposure_time.unwrap() - 0.004).abs() < 1e-12);
        assert!((a.focal_length.unwrap() - 6.8).abs() < 1e-9);
        assert_eq!(a.focal_length_35, Some(26));
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
    fn near_dupes_group_by_hamming_distance() {
        assert_eq!(dhash_distance("0000000000000000", "0000000000000000"), Some(0));
        assert_eq!(dhash_distance("0000000000000000", "ffffffffffffffff"), Some(64));
        assert_eq!(dhash_distance("0000000000000000", "00000000000000ff"), Some(8));
        assert!(dhash_distance("zz", "0000000000000000").is_none(), "malformed rejected");
        assert!(dhash_distance("short", "0000000000000000").is_none(), "short rejected");

        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // a-b differ by 5 bits, b-c by 5, a-c by 10: one transitive group.
        upsert_asset(&conn, &asset("/a.jpg", 100, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 200, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/c.jpg", 400, 3.0), scan).unwrap();
        upsert_asset(&conn, &asset("/far.jpg", 800, 4.0), scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        apply_dhash(&conn, id_for("/a.jpg"), "0000000000000000").unwrap();
        apply_dhash(&conn, id_for("/b.jpg"), "000000000000001f").unwrap();
        apply_dhash(&conn, id_for("/c.jpg"), "00000000000003ff").unwrap();
        apply_dhash(&conn, id_for("/far.jpg"), "ffffffffffffffff").unwrap();
        set_favorite(&conn, id_for("/c.jpg"), true).unwrap();

        let (groups, waste) = rebuild_near_groups(&conn).unwrap();
        assert_eq!(groups, 1, "transitive chain forms one group");
        assert_eq!(waste, 300, "non-keeper sizes summed");
        let list = near_groups(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].members.len(), 3);
        assert_eq!(list[0].kept_id, Some(id_for("/c.jpg")), "favourite wins keeper");

        // Rebuild is idempotent; keeper can move.
        let (groups2, _) = rebuild_near_groups(&conn).unwrap();
        assert_eq!(groups2, 1);
        near_keep(&conn, list[0].id, id_for("/a.jpg")).unwrap();
        assert_eq!(near_groups(&conn).unwrap()[0].kept_id, Some(id_for("/a.jpg")));
        assert!(near_keep(&conn, list[0].id, 9999).is_err(), "outsider keeper rejected");
    }

    #[test]
    fn vectors_cascade_with_assets() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        let aid: i64 =
            conn.query_row("SELECT id FROM assets WHERE path = '/a.jpg'", [], |r| r.get(0)).unwrap();

        // Unknown asset ids are rejected; the vector itself round-trips.
        assert!(save_vector(&conn, 9999, VECTOR_MODEL, 2, &[1.0, 0.0]).is_err());
        save_vector(&conn, aid, VECTOR_MODEL, 2, &[1.0, 0.0]).unwrap();
        assert!(save_vector(&conn, aid, VECTOR_MODEL, 3, &[1.0, 0.0]).is_err(), "dim is checked");
        // Re-embedding overwrites instead of duplicating.
        save_vector(&conn, aid, VECTOR_MODEL, 2, &[0.0, 1.0]).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM asset_vectors", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);

        // Deleting the asset cascades to its vector.
        conn.execute("DELETE FROM assets WHERE id = ?1", params![aid]).unwrap();
        let left: i64 = conn.query_row("SELECT COUNT(*) FROM asset_vectors", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn similar_to_ranks_by_cosine_and_excludes_self() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), 1.0, "identical vectors score 1.0");
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), 0.0, "orthogonal scores 0.0");
        assert_eq!(cosine(&[], &[]), 0.0, "empty is degenerate, not NaN");
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0, "zero norm sorts last");
        assert_eq!(cosine(&[1.0], &[1.0, 0.0]), 0.0, "length mismatch scores 0.0");
        let bytes = floats_to_bytes(&[1.0, -0.5]);
        assert_eq!(bytes_to_floats(&bytes).unwrap(), vec![1.0, -0.5]);
        assert!(bytes_to_floats(&[1, 2, 3]).is_none(), "truncated blobs rejected");

        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/c.jpg", 1, 3.0), scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        save_vector(&conn, id_for("/a.jpg"), VECTOR_MODEL, 2, &[1.0, 0.0]).unwrap();
        save_vector(&conn, id_for("/b.jpg"), VECTOR_MODEL, 2, &[0.9, 0.1]).unwrap();
        save_vector(&conn, id_for("/c.jpg"), VECTOR_MODEL, 2, &[0.0, 1.0]).unwrap();

        // "b" is missing-gated below; the ranking itself is cosine order.
        let hits = similar_to(&conn, id_for("/a.jpg"), VECTOR_MODEL, 10).unwrap();
        assert_eq!(hits.len(), 2, "the query photo is excluded");
        assert_eq!(hits[0].path, "/b.jpg");
        assert_eq!(hits[1].path, "/c.jpg");
        assert!(hits[0].score > 0.99, "near-parallel scores ~1, got {}", hits[0].score);
        assert_eq!(hits[1].score, 0.0);
        assert!(similar_to(&conn, 9999, VECTOR_MODEL, 10).is_err(), "unembedded query errors");

        conn.execute("UPDATE assets SET missing = 1 WHERE path = '/b.jpg'", []).unwrap();
        let gated = similar_to(&conn, id_for("/a.jpg"), VECTOR_MODEL, 10).unwrap();
        assert_eq!(gated.len(), 1);
        assert_eq!(gated[0].path, "/c.jpg");
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
    fn pet_sightings_store_filter_and_search() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/dog.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/plain.jpg", 1, 2.0), scan).unwrap();

        let ids = save_sightings(
            &conn,
            "/dog.jpg",
            &[serde_json::json!({"label": "Dog", "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.92})],
        )
        .unwrap();
        assert_eq!(ids.len(), 1);
        // Re-saving replaces: no duplicates on rescan.
        save_sightings(&conn, "/dog.jpg", &[]).unwrap();
        assert_eq!(sightings_count(&conn).unwrap(), 0);
        save_sightings(
            &conn,
            "/dog.jpg",
            &[serde_json::json!({"label": "Dog", "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.92})],
        )
        .unwrap();

        let pets = LibraryQuery { has_pet: Some(true), limit: 10, ..Default::default() };
        let rows = library(&conn, &pets).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/dog.jpg");
        assert_eq!(library_count(&conn, &pets).unwrap(), 1);
        let nopets = LibraryQuery { has_pet: Some(false), limit: 10, ..Default::default() };
        assert_eq!(library(&conn, &nopets).unwrap().len(), 1);

        // Labels are searchable like everything else.
        fn q(s: &str) -> LibraryQuery<'_> {
            LibraryQuery { query: Some(s), limit: 10, ..Default::default() }
        }
        let found = library(&conn, &q("dog")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/dog.jpg");

        let list = sightings_list(&conn).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label, "Dog");
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
    fn unassign_face_detaches_and_recounts() {
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
        let ids = mk("/a.jpg");
        let cid = save_cluster(&conn, &ids).unwrap();

        // Detaching shrinks the cluster and empties its member list.
        assert_eq!(unassign_face(&conn, ids[0]).unwrap(), cid);
        assert!(faces_in_cluster(&conn, cid).unwrap().is_empty());
        assert_eq!(clusters_list(&conn).unwrap()[0].face_count, 0);

        // Repeats, ghosts, and double-detaches fail cleanly.
        assert!(unassign_face(&conn, ids[0]).is_err(), "already unassigned errors");
        assert!(unassign_face(&conn, 9999).is_err(), "unknown face errors");
    }

    #[test]
    fn assign_face_reattaches_and_recounts() {
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
        let ids = mk("/a.jpg");
        let cid = save_cluster(&conn, &ids).unwrap();
        unassign_face(&conn, ids[0]).unwrap();
        assert!(faces_in_cluster(&conn, cid).unwrap().is_empty());

        // Undo re-links the face and restores the count.
        assign_face(&conn, ids[0], cid).unwrap();
        assert_eq!(faces_in_cluster(&conn, cid).unwrap().len(), 1);
        assert_eq!(clusters_list(&conn).unwrap()[0].face_count, 1);

        // Re-attaching to the same cluster is a no-op, not an error.
        assign_face(&conn, ids[0], cid).unwrap();

        // Ghosts fail cleanly.
        assert!(assign_face(&conn, 9999, cid).is_err(), "unknown face errors");
        assert!(assign_face(&conn, ids[0], 9999).is_err(), "unknown cluster errors");
    }

    #[test]
    fn restore_cluster_recreates_split_after_merge() {
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
        let from_faces: Vec<i64> =
            faces_in_cluster(&conn, a).unwrap().iter().map(|f| f.id).collect();

        // Merge drops the source row; undo re-inserts it and pulls the
        // faces back out of the survivor.
        merge_clusters(&conn, a, b).unwrap();
        assert_eq!(clusters_list(&conn).unwrap().len(), 1);
        let nid = restore_cluster(&conn, Some("Maya"), &from_faces).unwrap();
        let clusters = clusters_list(&conn).unwrap();
        assert_eq!(clusters.len(), 2, "split restores two clusters");
        let back = clusters.iter().find(|c| c.id == nid).unwrap();
        assert_eq!(back.name.as_deref(), Some("Maya"));
        assert_eq!(back.face_count, from_faces.len() as i64);
        assert_eq!(faces_in_cluster(&conn, nid).unwrap().len(), from_faces.len());
        assert_eq!(
            clusters.iter().find(|c| c.id == b).unwrap().face_count,
            1,
            "survivor recounts after losing members"
        );

        // Empty and ghost restores fail cleanly.
        assert!(restore_cluster(&conn, Some("Nobody"), &[]).is_err(), "empty errors");
        assert!(restore_cluster(&conn, Some("Nobody"), &[9999]).is_err(), "unknown face errors");
    }

    #[test]
    fn asset_paths_and_clear_missing_round_trip() {
        // DB halves of the Trash undo: the ~/.Trash move itself runs on the
        // main thread in the app and is mock-covered in e2e.
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let (a, b) = (id_for("/a.jpg"), id_for("/b.jpg"));

        let rows = asset_paths(&conn, &[a, b]).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&(a, "/a.jpg".to_string())));

        // Flagged missing (as trash_assets leaves them), then cleared.
        conn.execute("UPDATE assets SET missing = 1 WHERE id IN (?1, ?2)", params![a, b]).unwrap();
        assert_eq!(clear_missing(&conn, &[a]).unwrap(), 1);
        let missing: i64 =
            conn.query_row("SELECT missing FROM assets WHERE id = ?1", params![a], |r| r.get(0)).unwrap();
        assert_eq!(missing, 0, "restored row is visible again");
        let still: i64 =
            conn.query_row("SELECT missing FROM assets WHERE id = ?1", params![b], |r| r.get(0)).unwrap();
        assert_eq!(still, 1, "untouched rows stay flagged");
        assert_eq!(clear_missing(&conn, &[]).unwrap(), 0);
    }

    fn person_tags(conn: &Connection, asset_id: i64) -> Vec<String> {
        asset_tags(conn, asset_id).unwrap().into_iter().map(|t| t.tag).collect()
    }

    #[test]
    fn person_tags_stamp_on_rename_and_replace_on_rerename() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/p1.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/p2.jpg", 1, 2.0), scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let (a1, a2) = (id_for("/p1.jpg"), id_for("/p2.jpg"));

        // A scene tag proves person sync never touches sibling tags.
        store_tags(&conn, a1, &[("mountain".to_string(), 0.9)]).unwrap();

        let mk = |path: &str| {
            save_faces(
                &conn,
                path,
                &[serde_json::json!({"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
            )
            .unwrap()
        };
        let cid = save_cluster(&conn, &[mk("/p1.jpg")[0], mk("/p2.jpg")[0]]).unwrap();

        rename_cluster(&conn, cid, "Maya").unwrap();
        for aid in [a1, a2] {
            assert!(
                person_tags(&conn, aid).contains(&"person:Maya".to_string()),
                "rename stamps asset {aid}: {:?}",
                person_tags(&conn, aid)
            );
        }
        assert!(person_tags(&conn, a1).contains(&"mountain".to_string()), "scene tag survives");

        // A second name replaces the old stamp instead of stacking it.
        rename_cluster(&conn, cid, "Noor").unwrap();
        for aid in [a1, a2] {
            let tags = person_tags(&conn, aid);
            assert!(tags.contains(&"person:Noor".to_string()), "new stamp on {aid}: {tags:?}");
            assert!(!tags.contains(&"person:Maya".to_string()), "old stamp replaced on {aid}: {tags:?}");
        }
        assert!(person_tags(&conn, a1).contains(&"mountain".to_string()), "scene tag still intact");
    }

    #[test]
    fn person_tags_follow_merge_and_drop_source_stamp() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        for p in ["/a.jpg", "/b.jpg", "/c.jpg", "/d.jpg"] {
            upsert_asset(&conn, &asset(p, 1, 1.0), scan).unwrap();
        }
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let mk = |path: &str| {
            save_faces(
                &conn,
                path,
                &[serde_json::json!({"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
            )
            .unwrap()[0]
        };

        // Both sides named: the survivor keeps its own name, the source
        // stamp is dropped from every moved photo.
        let a = save_cluster(&conn, &[mk("/a.jpg")]).unwrap();
        let b = save_cluster(&conn, &[mk("/b.jpg")]).unwrap();
        rename_cluster(&conn, a, "Maya").unwrap();
        rename_cluster(&conn, b, "Leo").unwrap();
        merge_clusters(&conn, a, b).unwrap();
        for path in ["/a.jpg", "/b.jpg"] {
            let tags = person_tags(&conn, id_for(path));
            assert!(tags.contains(&"person:Leo".to_string()), "{path} carries survivor: {tags:?}");
            assert!(!tags.contains(&"person:Maya".to_string()), "{path} drops source: {tags:?}");
        }
        assert!(clusters_list(&conn).unwrap().iter().all(|c| c.id != a), "source row is gone");

        // Unnamed survivor inherits the source name onto every moved photo.
        let c = save_cluster(&conn, &[mk("/c.jpg")]).unwrap();
        let d = save_cluster(&conn, &[mk("/d.jpg")]).unwrap();
        rename_cluster(&conn, c, "Asha").unwrap();
        merge_clusters(&conn, c, d).unwrap();
        for path in ["/c.jpg", "/d.jpg"] {
            assert!(
                person_tags(&conn, id_for(path)).contains(&"person:Asha".to_string()),
                "{path} inherits the carried name"
            );
        }
    }

    #[test]
    fn person_tags_unassign_last_removes_but_sibling_keeps() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/solo.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/lone.jpg", 1, 2.0), scan).unwrap();
        let id_for = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let (solo, lone) = (id_for("/solo.jpg"), id_for("/lone.jpg"));

        let mk = |path: &str, x: f64| {
            save_faces(
                &conn,
                path,
                &[serde_json::json!({"x": x, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
            )
            .unwrap()[0]
        };
        // /solo.jpg shows the person twice; /lone.jpg once.
        let (s1, s2, l1) = (mk("/solo.jpg", 0.1), mk("/solo.jpg", 0.5), mk("/lone.jpg", 0.2));
        let cid = save_cluster(&conn, &[s1, s2, l1]).unwrap();
        rename_cluster(&conn, cid, "Maya").unwrap();
        assert!(person_tags(&conn, solo).contains(&"person:Maya".to_string()));
        assert!(person_tags(&conn, lone).contains(&"person:Maya".to_string()));

        // Detaching one of two links keeps the photo's stamp.
        unassign_face(&conn, s1).unwrap();
        assert!(person_tags(&conn, solo).contains(&"person:Maya".to_string()), "sibling face keeps the stamp");

        // Detaching the last link drops it; the other photo is untouched.
        unassign_face(&conn, s2).unwrap();
        assert!(!person_tags(&conn, solo).contains(&"person:Maya".to_string()), "last link drops the stamp");
        assert!(person_tags(&conn, lone).contains(&"person:Maya".to_string()), "other photo keeps its stamp");

        unassign_face(&conn, l1).unwrap();
        assert!(!person_tags(&conn, lone).contains(&"person:Maya".to_string()), "empty cluster stamps nothing");
    }

    fn b64_encode_test(bytes: &[u8]) -> String {
        const ALPH: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = *chunk.get(1).unwrap_or(&0) as u32;
            let b2 = *chunk.get(2).unwrap_or(&0) as u32;
            let n = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPH[((n >> 18) & 63) as usize] as char);
            out.push(ALPH[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPH[((n >> 6) & 63) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 { ALPH[(n & 63) as usize] as char } else { '=' });
        }
        out
    }

    fn descriptor_json(x: f64, vec: &[f32]) -> serde_json::Value {
        serde_json::json!({
            "x": x, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9,
            "descriptor": b64_encode_test(&floats_to_bytes(vec)),
        })
    }

    #[test]
    fn face_descriptor_roundtrip_through_save_load() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();

        // New style: with descriptor. Old style: without (pre-migration NULL).
        let with = descriptor_json(0.1, &[1.0, -0.5, 0.25]);
        let without = serde_json::json!({"x": 0.5, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.8});
        let ids_a = save_faces(&conn, "/a.jpg", &[with]).unwrap();
        let ids_b = save_faces(&conn, "/b.jpg", &[without]).unwrap();
        assert_eq!(ids_a.len() + ids_b.len(), 2);

        let stored = face_descriptors(&conn).unwrap();
        assert_eq!(stored.len(), 1, "NULL/empty rows are skipped");
        assert_eq!(stored[0].0, ids_a[0]);
        assert_eq!(stored[0].1, vec![1.0, -0.5, 0.25]);

        // Dim mismatch is corrupt: must error, not silently cluster.
        let other = descriptor_json(0.1, &[1.0, 2.0]);
        save_faces(&conn, "/c.jpg", &[other]).unwrap();
        assert!(
            face_descriptors(&conn).is_err(),
            "inconsistent dims must error"
        );
    }

    #[test]
    fn replace_clusters_is_idempotent_and_keeps_names() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();

        let mk = |path: &str, v: &[f32]| {
            save_faces(&conn, path, &[descriptor_json(0.1, v)]).unwrap()[0]
        };
        let a = mk("/a.jpg", &[0.0, 0.0]);
        let b = mk("/b.jpg", &[0.0, 0.0]);
        let c = mk("/c.jpg", &[10.0, 10.0]);
        let d = mk("/d.jpg", &[10.0, 10.0]);

        let first = replace_clusters(&conn, vec![vec![a, b], vec![c, d]]).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(faces_count(&conn).unwrap(), 4);
        rename_cluster(&conn, first[0], "Maya").unwrap();

        let before_counts = (faces_count(&conn).unwrap(), clusters_list(&conn).unwrap().len());
        // Rerun with identical groups: same counts, name intact.
        let second = replace_clusters(&conn, vec![vec![a, b], vec![c, d]]).unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(
            (faces_count(&conn).unwrap(), clusters_list(&conn).unwrap().len()),
            (before_counts.0, before_counts.1),
            "rerun changes no counts"
        );
        let names: Vec<Option<String>> =
            clusters_list(&conn).unwrap().into_iter().map(|c| c.name).collect();
        assert!(names.contains(&Some("Maya".to_string())), "name survives rerun: {names:?}");
        // No dupes: still 4 faces total.
        assert_eq!(faces_count(&conn).unwrap(), 4);
    }

    #[test]
    fn replace_clusters_carries_names_by_mutual_majority() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        let mk = |path: &str| {
            save_faces(
                &conn,
                path,
                &[serde_json::json!({"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
            )
            .unwrap()[0]
        };
        // Split: old 3-face cluster named Maya splits 2 vs 1.
        let (a, b, c) = (mk("/a.jpg"), mk("/b.jpg"), mk("/c.jpg"));
        let old = replace_clusters(&conn, vec![vec![a, b, c]]).unwrap();
        rename_cluster(&conn, old[0], "Maya").unwrap();
        let split = replace_clusters(&conn, vec![vec![a, b], vec![c]]).unwrap();
        assert_eq!(split.len(), 2);
        let rows = clusters_list(&conn).unwrap();
        let named: Vec<_> = rows.iter().filter(|r| r.name.is_some()).collect();
        assert_eq!(named.len(), 1, "exactly the majority keeps the name");
        assert_eq!(named[0].name.as_deref(), Some("Maya"));
        assert_eq!(named[0].face_count, 2, "majority group keeps the name");

        // Merge: two named clusters merge majority into one group.
        let (d, e) = (mk("/d.jpg"), mk("/e.jpg"));
        // Rebuild to two groups: [a,b]=Maya, [c,d,e] fresh; name the second.
        let two = replace_clusters(&conn, vec![vec![a, b], vec![c, d, e]]).unwrap();
        // two[0] should carry Maya (majority of old [a,b]); name the other.
        let rows = clusters_list(&conn).unwrap();
        assert!(rows.iter().any(|r| r.name.as_deref() == Some("Maya")));
        let other_id = rows.iter().find(|r| r.name.is_none()).map(|r| r.id).unwrap_or(two[1]);
        rename_cluster(&conn, other_id, "Leo").unwrap();
        // Merge all into one: Maya (2 faces) vs Leo (3 faces) -> Leo wins majority.
        let merged = replace_clusters(&conn, vec![vec![a, b, c, d, e]]).unwrap();
        assert_eq!(merged.len(), 1);
        let rows = clusters_list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        // Old Leo cluster (3 faces) has inter=3 vs Maya inter=2; best overlap wins.
        assert_eq!(rows[0].name.as_deref(), Some("Leo"), "best overlap wins the merge");

        // Tie: two old clusters equally overlap one new group -> lowest old id wins.
        let (f, g, h, i) = (mk("/f.jpg"), mk("/g.jpg"), mk("/h.jpg"), mk("/i.jpg"));
        let olds = replace_clusters(&conn, vec![vec![f, g], vec![h, i]]).unwrap();
        // olds[0] has lower cluster id (inserted first).
        rename_cluster(&conn, olds[0], "First").unwrap();
        rename_cluster(&conn, olds[1], "Second").unwrap();
        let tied = replace_clusters(&conn, vec![vec![f, h]]).unwrap();
        assert_eq!(tied.len(), 1);
        let rows = clusters_list(&conn).unwrap();
        // The single-face leftovers (g,i) are ungrouped? No: we only passed [f,h],
        // so g,i are unassigned; the one group overlaps both olds 1-1 with mutual
        // majority (1*2>=2 and 1*2>=2). Tie -> lowest old cluster id.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name.as_deref(), Some("First"), "tie goes to lowest cluster id");
    }

    #[test]
    fn cross_chunk_faces_unify_in_one_regroup() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        migrate(&conn).unwrap();
        // Simulate two separate detection calls (two "chunks") for one person:
        // identical descriptors saved in two independent save_faces calls.
        let same = [0.1f32, 0.2, 0.3];
        let chunk1 = save_faces(&conn, "/chunk1-a.jpg", &[descriptor_json(0.1, &same)]).unwrap();
        let chunk1b = save_faces(&conn, "/chunk1-b.jpg", &[descriptor_json(0.2, &same)]).unwrap();
        let chunk2 = save_faces(&conn, "/chunk2-a.jpg", &[descriptor_json(0.1, &same)]).unwrap();
        let chunk2b = save_faces(&conn, "/chunk2-b.jpg", &[descriptor_json(0.2, &same)]).unwrap();
        assert_eq!(face_descriptors(&conn).unwrap().len(), 4);

        // One global regroup containing members from both chunks -> one person.
        let all: Vec<i64> =
            [chunk1, chunk1b, chunk2, chunk2b].into_iter().flatten().collect();
        let ids = replace_clusters(&conn, vec![all]).unwrap();
        assert_eq!(ids.len(), 1, "cross-chunk members land in one group");
        let rows = clusters_list(&conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].face_count, 4);
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
    fn library_rows_carry_place_and_people_for_grouping() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        let aid_a: i64 =
            conn.query_row("SELECT id FROM assets WHERE path = '/a.jpg'", [], |r| r.get(0)).unwrap();
        // /a.jpg belongs to a placed event; /b.jpg to an unplaced one.
        conn.execute(
            "INSERT INTO events (name, started_at, ended_at, latitude, longitude, asset_count, face_count, created_at, place)
             VALUES ('Trip', 1, 2, 1.0, 2.0, 1, 0, 1, 'Moab, Utah')",
            [],
        )
        .unwrap();
        let eid: i64 = conn.query_row("SELECT last_insert_rowid()", [], |r| r.get(0)).unwrap();
        conn.execute("INSERT INTO event_assets (event_id, asset_id) VALUES (?1, ?2)", params![eid, aid_a])
            .unwrap();
        // One named face on /a.jpg.
        let face_ids = save_faces(
            &conn,
            "/a.jpg",
            &[serde_json::json!({"x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2, "confidence": 0.9})],
        )
        .unwrap();
        let cluster = save_cluster(&conn, &face_ids).unwrap();
        rename_cluster(&conn, cluster, "Maya").unwrap();

        let rows = library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 2);
        let a = rows.iter().find(|r| r.path == "/a.jpg").unwrap();
        assert_eq!(a.place.as_deref(), Some("Moab, Utah"));
        assert_eq!(a.people, vec!["Maya".to_string()]);
        let b = rows.iter().find(|r| r.path == "/b.jpg").unwrap();
        assert_eq!(b.place, None);
        assert!(b.people.is_empty());
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
    fn library_search_matches_names_paths_and_escapes_wildcards() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/photos/birthday-brunch.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/photos/trip-100%.jpg", 1, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/other/mountain.png", 1, 3.0), scan).unwrap();

        fn q(s: &str) -> LibraryQuery<'_> {
            LibraryQuery { query: Some(s), limit: 10, ..Default::default() }
        }
        assert_eq!(library(&conn, &q("brunch")).unwrap().len(), 1);
        assert_eq!(library_count(&conn, &q("brunch")).unwrap(), 1);
        // Path match, case-insensitive.
        assert_eq!(library(&conn, &q("MOUNTAIN")).unwrap().len(), 1);
        // A literal % in the query matches only the file that has one.
        let pct = library(&conn, &q("100%")).unwrap();
        assert_eq!(pct.len(), 1);
        assert_eq!(pct[0].name, "trip-100%.jpg");
        // Bare % never becomes a wildcard.
        assert_eq!(library(&conn, &q("%")).unwrap().len(), 1);
        assert!(library(&conn, &q("zzz")).unwrap().is_empty());
    }

    #[test]
    fn library_search_ranks_name_over_tag_over_event_over_person() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        // a: filename hit (tier 0). b: scene-tag hit (tier 1).
        upsert_asset(&conn, &asset("/photos/brunch spread.jpg", 1, 4.0), scan).unwrap();
        upsert_asset(&conn, &asset("/photos/lake morning.jpg", 1, 3.0), scan).unwrap();
        // c: event name hit (tier 2). d: person-name hit (tier 3).
        upsert_asset(&conn, &asset("/photos/hike.jpg", 1, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/photos/portrait.jpg", 1, 1.0), scan).unwrap();
        let bid: i64 = conn
            .query_row("SELECT id FROM assets WHERE path = '/photos/lake morning.jpg'", [], |r| r.get(0))
            .unwrap();
        let cid: i64 = conn
            .query_row("SELECT id FROM assets WHERE path = '/photos/hike.jpg'", [], |r| r.get(0))
            .unwrap();
        store_tags(&conn, bid, &[("brunch".to_string(), 0.9)]).unwrap();
        conn.execute(
            "INSERT INTO events (name, started_at, ended_at, asset_count, face_count, created_at)
             VALUES ('Brunch outing', 1.0, 2.0, 0, 0, 1)",
            [],
        )
        .unwrap();
        let eid: i64 = conn.query_row("SELECT id FROM events", [], |r| r.get(0)).unwrap();
        conn.execute(
            "INSERT INTO event_assets (event_id, asset_id) VALUES (?1, ?2)",
            params![eid, cid],
        )
        .unwrap();
        let face_ids = save_faces(
            &conn,
            "/photos/portrait.jpg",
            &[serde_json::json!({"x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9})],
        )
        .unwrap();
        let cluster = save_cluster(&conn, &face_ids).unwrap();
        rename_cluster(&conn, cluster, "Brunch Club").unwrap();

        fn q(s: &str) -> LibraryQuery<'_> {
            LibraryQuery { query: Some(s), limit: 10, ..Default::default() }
        }
        let rows = library(&conn, &q("brunch")).unwrap();
        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        // The named cluster stamps `person:Brunch Club` on the portrait, so
        // it now matches at tag tier (tier 1) instead of person tier (3);
        // within the tier, recency still orders lake morning first.
        assert_eq!(
            paths,
            vec![
                "/photos/brunch spread.jpg",
                "/photos/lake morning.jpg",
                "/photos/portrait.jpg",
                "/photos/hike.jpg"
            ],
            "name > tag > event; person stamp ranks at tag tier"
        );
        assert_eq!(library_count(&conn, &q("brunch")).unwrap(), 4);
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
        geocode_cache_store(&conn, round_coord(37.7749), round_coord(-122.4194), "San Francisco, California", None)
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
        // The first half inherits the old span name (majority overlap);
        // the second half, with its old match taken, gets a fresh bare date.
        assert_eq!(rows.iter().filter(|r| r.name.contains('–')).count(), 1);
        assert_eq!(rows.iter().filter(|r| !r.name.contains('–')).count(), 1);
    }

    #[test]
    fn cluster_events_preserves_names_and_places_across_rebuilds() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1_700_000_000.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 1_700_000_000.0 + 1800.0), scan).unwrap();

        cluster_events(&conn, 4.0, 500.0).unwrap();
        let first = events_list(&conn).unwrap();
        assert_eq!(first.len(), 1);
        rename_event(&conn, first[0].id, "Maya's birthday").unwrap();
        set_event_place(&conn, first[0].id, "San Francisco, California").unwrap();

        // Rebuild with identical members: manual name and place survive.
        cluster_events(&conn, 4.0, 500.0).unwrap();
        let second = events_list(&conn).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].name, "Maya's birthday");
        assert_eq!(second[0].place.as_deref(), Some("San Francisco, California"));

        // Rebuild with disjoint membership: no match, fresh default name.
        upsert_asset(&conn, &asset("/z.jpg", 1, 1_800_000_000.0), scan).unwrap();
        cluster_events(&conn, 4.0, 500.0).unwrap();
        let third = events_list(&conn).unwrap();
        assert_eq!(third.len(), 2);
        assert!(third.iter().any(|r| r.name == "Maya's birthday"), "overlapping event keeps its name");
        assert!(third.iter().any(|r| r.name != "Maya's birthday"), "disjoint event gets a fresh name");
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
    fn asset_ids_in_root_matches_scan_roots_rule() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let s1 = begin_scan(&conn, "/photos", true).unwrap();
        upsert_asset(&conn, &asset("/photos/a.jpg", 1, 1.0), s1).unwrap();
        upsert_asset(&conn, &asset("/photos/sub/b.jpg", 1, 1.0), s1).unwrap();
        let s2 = begin_scan(&conn, "/photos2", true).unwrap();
        upsert_asset(&conn, &asset("/photos2/d.jpg", 1, 1.0), s2).unwrap();
        let s3 = begin_scan(&conn, "/my%photos", true).unwrap();
        upsert_asset(&conn, &asset("/my%photos/e.jpg", 1, 1.0), s3).unwrap();

        let ids = asset_ids_in_root(&conn, "/photos").unwrap();
        assert_eq!(ids.len(), 2, "/photos2 must not leak into /photos");
        let ids = asset_ids_in_root(&conn, "/photos/").unwrap();
        assert_eq!(ids.len(), 2, "trailing slash accepted");
        let ids = asset_ids_in_root(&conn, "/my%photos").unwrap();
        assert_eq!(ids.len(), 1, "% in root matches literally");
        let ids = asset_ids_in_root(&conn, "/nowhere").unwrap();
        assert!(ids.is_empty());
    }

    #[test]
    fn remove_root_forgets_scans_assets_faces_and_prunes_clusters() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let s1 = begin_scan(&conn, "/photos", true).unwrap();
        upsert_asset(&conn, &asset("/photos/a.jpg", 1, 1.0), s1).unwrap();
        upsert_asset(&conn, &asset("/photos/sub/b.jpg", 1, 2.0), s1).unwrap();
        let s2 = begin_scan(&conn, "/photos2", true).unwrap();
        upsert_asset(&conn, &asset("/photos2/d.jpg", 1, 3.0), s2).unwrap();
        let face = || serde_json::json!({"x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2, "confidence": 0.9});
        let fa = save_faces(&conn, "/photos/a.jpg", &[face()]).unwrap();
        let fd = save_faces(&conn, "/photos2/d.jpg", &[face()]).unwrap();
        let ca = save_cluster(&conn, &fa).unwrap();
        rename_cluster(&conn, ca, "Maya").unwrap();
        let _cd = save_cluster(&conn, &fd).unwrap();
        conn.execute(
            "INSERT INTO animal_sightings (path, label, x, y, width, height, confidence, detected_at)
             VALUES ('/photos/a.jpg', 'dog', 0, 0, 1, 1, 0.9, 1)",
            [],
        )
        .unwrap();

        let summary = remove_root(&conn, "/photos").unwrap();
        assert_eq!(
            summary,
            RemoveRootSummary {
                root: "/photos".to_string(),
                removed_assets: 2,
                removed_faces: 1,
                removed_sightings: 1,
                pruned_clusters: 1,
                removed_scans: 1,
            }
        );
        // The sibling root is untouched: scans, assets, and its cluster.
        let roots = scan_roots(&conn).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].root, "/photos2");
        assert_eq!(roots[0].assets, 1);
        let rows = library(&conn, &LibraryQuery { limit: 10, ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "/photos2/d.jpg");
        let clusters = clusters_list(&conn).unwrap();
        assert_eq!(clusters.len(), 1, "only Maya's emptied cluster is pruned");
        // Removing again is a no-op zero summary, not an error.
        let again = remove_root(&conn, "/photos").unwrap();
        assert_eq!(again.removed_assets, 0);
        assert_eq!(again.removed_scans, 0);
        assert_eq!(scan_roots(&conn).unwrap().len(), 1);
    }

    #[test]
    fn remove_root_matches_canonicalized_symlink_spellings() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        // Symlinked root: the scan record keeps the literal spelling while
        // the import walk stores canonicalized asset paths (macOS /tmp).
        let dir = std::env::temp_dir().join("ff-remove-root-canonical");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("real")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("real"), dir.join("link")).unwrap();
        let link = dir.join("link").to_string_lossy().into_owned();
        let canonical =
            std::fs::canonicalize(&link).unwrap().to_string_lossy().into_owned();
        assert_ne!(link, canonical, "test needs a resolving symlink");
        let scan = begin_scan(&conn, &link, true).unwrap();
        upsert_asset(&conn, &asset(&format!("{canonical}/a.jpg"), 1, 1.0), scan).unwrap();

        let summary = remove_root(&conn, &link).unwrap();
        assert_eq!(summary.removed_assets, 1);
        assert_eq!(summary.removed_scans, 1);
        assert!(scan_roots(&conn).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn map_pins_returns_only_geotagged_present_assets() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let scan = begin_scan(&conn, "/tmp", true).unwrap();
        upsert_asset(&conn, &asset("/a.jpg", 1, 1.0), scan).unwrap();
        upsert_asset(&conn, &asset("/b.jpg", 1, 2.0), scan).unwrap();
        upsert_asset(&conn, &asset("/c.jpg", 1, 3.0), scan).unwrap();
        conn.execute("UPDATE assets SET latitude = 37.7, longitude = -122.4 WHERE path = '/a.jpg'", []).unwrap();
        conn.execute("UPDATE assets SET latitude = 38.0, longitude = -122.0 WHERE path IN ('/b.jpg', '/c.jpg')", [])
            .unwrap();
        conn.execute("UPDATE assets SET missing = 1 WHERE path = '/c.jpg'", []).unwrap();

        let pins = map_pins(&conn).unwrap();
        assert_eq!(pins.len(), 2, "unlocated and missing rows are skipped");
        assert_eq!(pins[0].path, "/a.jpg");
        assert!((pins[0].latitude - 37.7).abs() < 1e-9);
        assert_eq!(pins[1].path, "/b.jpg");
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
        geocode_cache_store(&conn, 37.775, -122.419, "San Francisco, California", None).unwrap();
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

    #[test]
    fn geocode_covering_reuses_fences() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        geocode_cache_store(
            &conn, 37.775, -122.419, "Mission District",
            Some((37.76, 37.78, -122.43, -122.41)),
        )
        .unwrap();
        // Same neighbourhood, different 100m cell: the fence answers.
        assert_eq!(
            geocode_covering(&conn, 37.771, -122.415).unwrap().as_deref(),
            Some("Mission District")
        );
        // Elsewhere: no fence.
        assert!(geocode_covering(&conn, 40.7128, -74.006).unwrap().is_none());
        // Overlapping boxes: the smallest wins.
        geocode_cache_store(
            &conn, 37.7, -122.4, "San Francisco, California",
            Some((37.0, 38.0, -123.0, -122.0)),
        )
        .unwrap();
        assert_eq!(
            geocode_covering(&conn, 37.771, -122.415).unwrap().as_deref(),
            Some("Mission District")
        );
    }

    #[test]
    fn coarse_marked_events_stay_refinement_eligible() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO events (name, started_at, ended_at, latitude, longitude, asset_count, face_count, created_at)
             VALUES ('Oct 9', 1.0, 2.0, 37.7749, -122.4194, 1, 0, 1)",
            [],
        )
        .unwrap();
        let eid: i64 = conn.query_row("SELECT id FROM events", [], |r| r.get(0)).unwrap();

        // Unlabeled: needs a place.
        assert_eq!(events_missing_places(&conn).unwrap().len(), 1);
        // Coarse static label: still eligible for network refinement.
        set_event_place(&conn, eid, "≈ California, United States").unwrap();
        let still = events_missing_places(&conn).unwrap();
        assert_eq!(still.len(), 1);
        assert_eq!(still[0].event_id, eid);
        // Precise label: done.
        set_event_place(&conn, eid, "San Francisco, California").unwrap();
        assert!(events_missing_places(&conn).unwrap().is_empty());
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
