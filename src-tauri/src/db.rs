// SQLite library index. Schema lives in ../migrations/*.sql and is embedded at
// compile time so bundled builds need no external files.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/001_init.sql"))];

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

// MARK: - Queries

pub fn library(
    conn: &Connection,
    kind: Option<&str>,
    offset: i64,
    limit: i64,
) -> Result<Vec<AssetRow>, String> {
    let sql = "SELECT id, path, name, ext, kind, mime, size_bytes, modified, captured_at,
                      width, height, duration, favorite, missing, sha256
               FROM assets
               WHERE (?1 IS NULL OR kind = ?1) AND missing = 0
               ORDER BY COALESCE(captured_at, modified) DESC, id DESC
               LIMIT ?2 OFFSET ?3";
    let mut stmt = conn.prepare(sql).map_err(|e| format!("prepare library: {e}"))?;
    let rows = stmt
        .query_map(params![kind, limit, offset], |r| {
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
        assert_eq!(migrate(&conn).unwrap(), vec![1]);
        assert!(migrate(&conn).unwrap().is_empty());
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
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

        let all = library(&conn, None, 0, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].path, "/old.jpg", "newest capture first");

        let paged = library(&conn, None, 1, 10).unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].path, "/new.jpg", "offset skips the newest");

        let videos = library(&conn, Some("video"), 0, 10).unwrap();
        assert!(videos.is_empty(), "kind filter excludes images");

        let s = stats(&conn).unwrap();
        assert_eq!(s.photos, 2);
        assert_eq!(s.videos, 0);
        assert_eq!(s.bytes, 2);
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
