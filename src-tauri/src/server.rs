//! Localhost read API: the same backend behind the Tauri commands,
//! served over HTTP so the web UI and Playwright exercise the real
//! database instead of the in-memory mock.
//!
//! Thin slice: reads only (no imports, no engine calls, no progress
//! streams). Every response reuses the exact structs the Tauri commands
//! return, so the JSON shapes are identical by construction.
//!
//! Bound to 127.0.0.1 only; permissive CORS is safe on loopback and lets
//! the Vite dev server (another loopback port) call it from a browser.

use axum::{
    extract::{Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
}

fn cors() -> [(HeaderName, HeaderValue); 1] {
    [(HeaderName::from_static("access-control-allow-origin"), HeaderValue::from_static("*"))]
}

fn err(e: String) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, cors(), Json(serde_json::json!({ "error": e }))).into_response()
}

fn lock(state: &ApiState) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, Response> {
    state.db.lock().map_err(|_| err("database lock poisoned".to_string()))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LibraryParams {
    kind: Option<String>,
    blurred_only: Option<bool>,
    person_cluster: Option<i64>,
    has_location: Option<bool>,
    captured_min: Option<f64>,
    captured_max: Option<f64>,
    favorite_only: Option<bool>,
    scene_tag: Option<String>,
    query: Option<String>,
    has_pet: Option<bool>,
    offset: Option<i64>,
    limit: Option<i64>,
}

fn library_query(p: &LibraryParams, offset: Option<i64>, limit: Option<i64>) -> crate::db::LibraryQuery<'_> {
    crate::db::LibraryQuery {
        kind: p.kind.as_deref().filter(|k| *k == "image" || *k == "video"),
        blurred_only: p.blurred_only.unwrap_or(false),
        person_cluster: p.person_cluster,
        has_location: p.has_location,
        captured_min: p.captured_min,
        captured_max: p.captured_max,
        favorite_only: p.favorite_only,
        scene_tag: p.scene_tag.as_deref(),
        query: p.query.as_deref(),
        has_pet: p.has_pet,
        offset: offset.unwrap_or(0).max(0),
        limit: limit.unwrap_or(200).clamp(1, 1000),
    }
}

async fn ping() -> Response {
    (cors(), Json(serde_json::json!({ "pong": true }))).into_response()
}

async fn library_list(State(state): State<ApiState>, Query(p): Query<LibraryParams>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::library(&conn, &library_query(&p, p.offset, p.limit)) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

async fn library_count(State(state): State<ApiState>, Query(p): Query<LibraryParams>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::library_count(&conn, &library_query(&p, None, None)) {
        Ok(n) => (cors(), Json(serde_json::json!({ "count": n }))).into_response(),
        Err(e) => err(e),
    }
}

async fn library_months(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::library_months(&conn) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

async fn stats(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::stats(&conn) {
        Ok(s) => (cors(), Json(s)).into_response(),
        Err(e) => err(e),
    }
}

async fn roots(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::scan_roots(&conn) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

#[derive(Deserialize)]
struct RootAssetsParams {
    root: String,
}

async fn root_assets(State(state): State<ApiState>, Query(p): Query<RootAssetsParams>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::asset_ids_in_root(&conn, &p.root) {
        Ok(ids) => (cors(), Json(ids)).into_response(),
        Err(e) => err(e),
    }
}

async fn clusters(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::clusters_list(&conn) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

async fn events(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::events_list(&conn) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

async fn albums(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::albums_list(&conn) {
        Ok(rows) => (cors(), Json(rows)).into_response(),
        Err(e) => err(e),
    }
}

async fn tags(State(state): State<ApiState>) -> Response {
    let conn = match lock(&state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    match crate::db::distinct_tags(&conn) {
        Ok(rows) => (
            cors(),
            Json(
                rows.into_iter()
                    .map(|(tag, count)| super::SceneTagCount { tag, count })
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(e) => err(e),
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/ping", get(ping))
        .route("/api/library", get(library_list))
        .route("/api/library/count", get(library_count))
        .route("/api/library/months", get(library_months))
        .route("/api/stats", get(stats))
        .route("/api/roots", get(roots))
        .route("/api/root-assets", get(root_assets))
        .route("/api/clusters", get(clusters))
        .route("/api/events", get(events))
        .route("/api/albums", get(albums))
        .route("/api/tags", get(tags))
        .with_state(state)
}

/// Demo seed mirroring the web mock: 8 assets under /photos so the API
/// E2E run has deterministic data without any imports. Only seeds an
/// empty database, so reruns against the same file are stable.
pub fn seed_demo(conn: &rusqlite::Connection) -> Result<bool, String> {
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))
        .map_err(|e| format!("demo count: {e}"))?;
    if n > 0 {
        return Ok(false);
    }
    let scan = crate::db::begin_scan(conn, "/photos", true)?;
    let rows = [
        ("IMG_20261007_185817.jpg", "image", "image/jpeg", 1_237_000i64),
        ("IMG_20261007_185822.jpg", "image", "image/jpeg", 1_274_000),
        ("IMG_20261007_185824.jpg", "image", "image/jpeg", 1_311_000),
        ("IMG_20261007_191433.jpg", "image", "image/jpeg", 1_348_000),
        ("IMG_20261007_191434.jpg", "image", "image/jpeg", 1_385_000),
        ("IMG_20261007_191434_copy.jpg", "image", "image/jpeg", 1_385_000),
        ("VID_20261007_192001.mp4", "video", "video/mp4", 4_200_000),
        ("VID_20261007_204512.mp4", "video", "video/mp4", 5_100_000),
    ];
    for (i, (name, kind, mime, size)) in rows.iter().enumerate() {
        crate::db::upsert_asset(
            conn,
            &crate::db::Value2 {
                path: format!("/photos/{name}"),
                name: name.to_string(),
                ext: name.rsplit('.').next().unwrap_or("").to_string(),
                kind: kind.to_string(),
                mime: Some(mime.to_string()),
                size_bytes: *size,
                modified: 1_760_000_000.0 + i as f64 * 60.0,
            },
            scan,
        )?;
    }
    Ok(true)
}

/// Serves the read API on 127.0.0.1. Used headless by the API E2E run
/// (`--serve`); the desktop app keeps using Tauri IPC for now.
pub async fn serve(db_path: &std::path::Path, port: u16, demo: bool) -> Result<(), String> {
    let conn = crate::db::open(db_path)?;
    let applied = crate::db::migrate(&conn)?;
    if !applied.is_empty() {
        eprintln!("fotoflow serve: applied migrations {applied:?}");
    }
    if demo {
        let seeded = seed_demo(&conn)?;
        eprintln!(
            "fotoflow serve: demo seed {} (db {})",
            if seeded { "written" } else { "skipped, db not empty" },
            db_path.display()
        );
    }
    let state = ApiState { db: Arc::new(Mutex::new(conn)) };
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| format!("serve bind 127.0.0.1:{port}: {e}"))?;
    eprintln!("fotoflow serve: listening on http://127.0.0.1:{port} (db {})", db_path.display());
    axum::serve(listener, router(state))
        .await
        .map_err(|e| format!("serve: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_and_library_reads_serve_seeded_rows() {
        // Multi-thread: the blocking ureq client must not starve the server task.
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let conn = crate::db::open(std::path::Path::new(":memory:")).unwrap();
            crate::db::migrate(&conn).unwrap();
            assert!(seed_demo(&conn).unwrap(), "first seed fills the empty db");
            assert!(!seed_demo(&conn).unwrap(), "second seed is a no-op");
            let state = ApiState { db: Arc::new(Mutex::new(conn)) };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
            tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
            let ping: serde_json::Value =
                ureq::get(&format!("{base}/api/ping")).call().unwrap().body_mut().read_json().unwrap();
            assert_eq!(ping.get("pong"), Some(&serde_json::Value::Bool(true)));
            let rows: Vec<serde_json::Value> = ureq::get(&format!("{base}/api/library"))
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(rows.len(), 8);
            let count: serde_json::Value = ureq::get(&format!("{base}/api/library/count?kind=video"))
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(count.get("count"), Some(&serde_json::json!(2)));
            let roots: Vec<serde_json::Value> = ureq::get(&format!("{base}/api/roots"))
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(roots.len(), 1);
            assert_eq!(roots[0].get("root"), Some(&serde_json::json!("/photos")));
            let ids: Vec<i64> = ureq::get(&format!("{base}/api/root-assets?root=%2Fphotos"))
                .call()
                .unwrap()
                .body_mut()
                .read_json()
                .unwrap();
            assert_eq!(ids.len(), 8);
            // Unknown route: axum 404, still loopback-confined.
            let missing = ureq::get(&format!("{base}/api/nope")).call();
            assert!(missing.is_err());
        });
    }
}
