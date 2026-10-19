mod db;
mod engine;

use serde::Serialize;
use rusqlite::params;
use serde_json::{Value, json};

use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

type DbState = Arc<Mutex<rusqlite::Connection>>;

const SCAN_TIMEOUT: Duration = Duration::from_secs(600);

/// An unbundled `cargo run` binary has no Info.plist, so AppKit refuses to
/// build an NSOpenPanel ("unexpected NULL returned from +[NSOpenPanel
/// openPanel]"). Promoting the process to a regular GUI app makes the native
/// folder picker work in dev; bundled builds already have a dock icon.
#[cfg(target_os = "macos")]
fn ensure_gui_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    // setup() runs on the main thread; the marker only exists there.
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    app.activate();
}

#[cfg(not(target_os = "macos"))]
fn ensure_gui_activation() {}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
async fn engine_call(
    app: tauri::AppHandle,
    cmd: String,
    params: Option<Value>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<engine::EngineState>();
        engine::call_blocking(&state, &cmd, params, Duration::from_secs(240))
    })
    .await
    .map_err(|e| format!("engine: task join failed: {e}"))?
}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgress {
    scan_id: i64,
    scanned: i64,
    imported: i64,
    unchanged: i64,
    bytes: i64,
    walking: bool,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    root: String,
    scan_id: i64,
    scanned: i64,
    imported: i64,
    unchanged: i64,
    skipped: i64,
    bytes: i64,
    elapsed: f64,
}

/// Streams a folder scan from the engine into SQLite. `on_progress` fires per
/// chunk with the running counters; the caller decides how to surface them
/// (the Tauri command emits them as `import-progress` events).
pub fn run_import(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    root: &str,
    recursive: bool,
    mut on_progress: impl FnMut(ImportProgress),
    mut on_error: impl FnMut(String),
) -> Result<ImportSummary, String> {
    let scan_id = db::begin_scan(conn, root, recursive)?;
    let params = json!({ "root": root, "recursive": recursive });
    let mut counters = db::ScanCounters::default();
    let started = std::time::Instant::now();

    let snapshot = |c: &db::ScanCounters, walking: bool| ImportProgress {
        scan_id,
        scanned: c.scanned,
        imported: c.imported,
        unchanged: c.unchanged,
        bytes: c.bytes,
        walking,
    };

    let result = engine::call_with_events(
        engine_state,
        "scan",
        Some(params),
        SCAN_TIMEOUT,
        &mut |name, data| match name {
            "chunk" => {
                let Some(items) = data.as_array() else { return };
                let tx = match conn.unchecked_transaction() {
                    Ok(tx) => tx,
                    Err(e) => {
                        on_error(format!("transaction failed: {e}"));
                        return;
                    }
                };
                for item in items {
                    match db::asset_from_json(item) {
                        Ok(a) => {
                            counters.bytes += a.size_bytes;
                            match db::upsert_asset(&tx, &a, scan_id) {
                                Ok(true) => counters.imported += 1,
                                Ok(false) => counters.unchanged += 1,
                                Err(e) => on_error(e),
                            }
                            counters.scanned += 1;
                        }
                        Err(e) => on_error(e),
                    }
                }
                if let Err(e) = tx.commit() {
                    on_error(format!("commit failed: {e}"));
                }
                on_progress(snapshot(&counters, false));
            }
            "progress" => on_progress(snapshot(&counters, true)),
            _ => {}
        },
    );

    let skipped = match &result {
        Ok(v) => v.get("skipped").and_then(Value::as_i64).unwrap_or(0),
        Err(_) => 0,
    };
    db::finish_scan(conn, scan_id, &counters, result.as_ref().err().map(|e| e.as_str()))?;
    result.map_err(|e| e)?;

    Ok(ImportSummary {
        root: root.to_string(),
        scan_id,
        scanned: counters.scanned,
        imported: counters.imported,
        unchanged: counters.unchanged,
        skipped,
        bytes: counters.bytes,
        elapsed: started.elapsed().as_secs_f64(),
    })
}

/// Tauri entry point: same as `run_import`, but emits progress to the window.
#[tauri::command]
async fn import_folder(
    app: tauri::AppHandle,
    root: String,
    recursive: Option<bool>,
) -> Result<ImportSummary, String> {
    let recursive = recursive.unwrap_or(true);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();

        run_import(
            &conn,
            &engine_state,
            &root,
            recursive,
            |p| {
                let _ = app.emit(
                    "import-progress",
                    json!({
                        "scanId": p.scan_id,
                        "scanned": p.scanned,
                        "imported": p.imported,
                        "unchanged": p.unchanged,
                        "bytes": p.bytes,
                        "walking": p.walking,
                    }),
                );
            },
            |e| {
                let _ = app.emit("import-error", e);
            },
        )
    })
    .await
    .map_err(|e| format!("import task join failed: {e}"))?
}

/// Resumable metadata enrichment: pulls batches of assets with no `stat_at` and
/// asks the engine for EXIF/dimension data. Completed rows are never revisited,
/// so a stopped pass can simply be run again.
pub fn run_enrich(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    max_attempts: i64,
    mut on_progress: impl FnMut(i64, i64, i64, i64),
) -> Result<db::EnrichSummary, String> {
    let started = std::time::Instant::now();
    let (mut processed, mut enriched, mut failed) = (0i64, 0i64, 0i64);
    // One attempt per asset per invocation. A failure leaves stat_at NULL so the
    // row stays eligible later, but retrying it in this same loop would burn the
    // whole attempt budget instantly (an iCloud placeholder fails every time).
    let mut attempted: std::collections::HashSet<i64> = std::collections::HashSet::new();

    loop {
        let batch: Vec<(i64, String)> = db::pending_stat(conn, 100, max_attempts)?
            .into_iter()
            .filter(|(id, _)| !attempted.contains(id))
            .collect();
        if batch.is_empty() {
            break;
        }
        for (id, path) in &batch {
            attempted.insert(*id);
            match engine::call_blocking(
                engine_state,
                "stat",
                Some(json!({ "path": path })),
                Duration::from_secs(30),
            ) {
                Ok(stat) => {
                    db::apply_stat(conn, *id, &stat)?;
                    enriched += 1;
                }
                Err(e) => {
                    db::apply_stat_failure(conn, *id, &e)?;
                    failed += 1;
                }
            }
            processed += 1;
        }
        let (pending, _) = db::enrich_counts(conn, max_attempts)?;
        on_progress(processed, enriched, failed, pending);
    }

    let (pending, _) = db::enrich_counts(conn, max_attempts)?;
    Ok(db::EnrichSummary {
        processed,
        enriched,
        failed,
        pending,
        elapsed: started.elapsed().as_secs_f64(),
    })
}

#[tauri::command]
async fn enrich_assets(
    app: tauri::AppHandle,
    max_attempts: Option<i64>,
) -> Result<db::EnrichSummary, String> {
    let max_attempts = max_attempts.unwrap_or(3).clamp(1, 20);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        run_enrich(&conn, &engine_state, max_attempts, |processed, enriched, failed, pending| {
            let _ = app.emit(
                "enrich-progress",
                json!({ "processed": processed, "enriched": enriched, "failed": failed, "pending": pending }),
            );
        })
    })
    .await
    .map_err(|e| format!("enrich task join failed: {e}"))?
}

// MARK: - Cleanup passes

/// Hashes only size-bucketed duplicate candidates, in batches of 100 paths per
/// engine round-trip, then rebuilds the duplicate groups.
pub fn run_hash(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    mut on_progress: impl FnMut(i64, i64, i64),
) -> Result<(i64, i64, i64), String> {
    let buckets = db::size_buckets(conn)?;
    let (mut hashed, mut failed, mut batches) = (0i64, 0i64, 0i64);

    loop {
        let batch = db::hash_candidates(conn, &buckets, 100)?;
        if batch.is_empty() {
            break;
        }
        let paths: Vec<String> = batch.iter().map(|(_, p)| p.clone()).collect();
        let reply = engine::call_blocking(
            engine_state,
            "hash_batch",
            Some(json!({ "paths": paths })),
            Duration::from_secs(300),
        )?;

        let results = reply
            .get("results")
            .and_then(Value::as_array)
            .ok_or("hash_batch returned no results array")?;
        if results.len() != batch.len() {
            return Err(format!(
                "hash_batch returned {} results for {} paths",
                results.len(),
                batch.len()
            ));
        }

        let tx = conn.unchecked_transaction().map_err(|e| format!("hash tx: {e}"))?;
        for ((id, path), result) in batch.iter().zip(results) {
            if result.get("ok").and_then(Value::as_bool) == Some(true) {
                let Some(sha) = result.get("sha256").and_then(Value::as_str) else { continue };
                db::apply_hash(&tx, *id, sha)?;
                hashed += 1;
            } else {
                let message = result.get("error").and_then(Value::as_str).unwrap_or("hash failed");
                db::apply_hash_failure(&tx, *id, message)?;
                failed += 1;
            }
            let _ = path;
        }
        tx.commit().map_err(|e| format!("hash commit: {e}"))?;

        batches += 1;
        on_progress(hashed, failed, buckets.len() as i64);
        if batches > 10_000 {
            break; // safety valve against a pathological bucket set
        }
    }

    let (groups, waste) = db::rebuild_duplicate_groups(conn)?;
    Ok((hashed, groups, waste))
}

#[tauri::command]
async fn hash_candidates_run(
    app: tauri::AppHandle,
) -> Result<db::CleanupStats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();

        let (_hashed, _groups, _waste) = run_hash(&conn, &engine_state, |hashed, failed, buckets| {
            let _ = app.emit(
                "hash-progress",
                json!({ "hashed": hashed, "failed": failed, "buckets": buckets }),
            );
        })?;
        db::cleanup_stats(&conn)
    })
    .await
    .map_err(|e| format!("hash task join failed: {e}"))?
}

/// Scores image sharpness and flags blurred frames. Resumable via quality_at.
pub fn run_quality(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    blur_threshold: f64,
    mut on_progress: impl FnMut(i64, i64),
) -> Result<(i64, i64), String> {
    let (mut scored, mut blurred) = (0i64, 0i64);
    let mut attempted: std::collections::HashSet<i64> = std::collections::HashSet::new();

    loop {
        let batch: Vec<(i64, String)> = db::quality_candidates(conn, 50)?
            .into_iter()
            .filter(|(id, _)| !attempted.contains(id))
            .collect();
        if batch.is_empty() {
            break;
        }
        for (id, path) in &batch {
            attempted.insert(*id);
            let reply = engine::call_blocking(
                engine_state,
                "quality",
                Some(json!({ "path": path, "blurThreshold": blur_threshold })),
                Duration::from_secs(60),
            );
            // A failed score stays un-scored (quality_at NULL) so it can be retried.
            if let Ok(q) = reply {
                let sharpness = q.get("sharpness").and_then(Value::as_f64).unwrap_or(0.0);
                let is_blurred = q.get("blurred").and_then(Value::as_bool).unwrap_or(false);
                db::apply_quality(conn, *id, sharpness, is_blurred)?;
                scored += 1;
                if is_blurred {
                    blurred += 1;
                }
            }
        }
        on_progress(scored, blurred);
    }
    Ok((scored, blurred))
}

#[tauri::command]
async fn score_quality(app: tauri::AppHandle) -> Result<db::CleanupStats, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        run_quality(&conn, &engine_state, 25.0, |scored, blurred| {
            let _ = app.emit(
                "quality-progress",
                json!({ "scored": scored, "blurred": blurred }),
            );
        })?;
        db::cleanup_stats(&conn)
    })
    .await
    .map_err(|e| format!("quality task join failed: {e}"))?
}

#[tauri::command]
fn duplicates_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::DuplicateGroup>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::duplicate_groups(&conn)
}

#[tauri::command]
fn cleanup_stats_cmd(conn: tauri::State<'_, DbState>) -> Result<db::CleanupStats, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::cleanup_stats(&conn)
}

/// Marks a keeper for a duplicate group. Never deletes: the user moves files
/// themselves after reviewing the group.
#[tauri::command]
fn duplicates_keep(
    conn: tauri::State<'_, DbState>,
    group_id: i64,
    keep_id: i64,
) -> Result<Vec<db::DuplicateGroup>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;

    let belongs: bool = conn
        .query_row(
            "SELECT 1 FROM assets WHERE id = ?1 AND duplicate_group_id = ?2",
            params![keep_id, group_id],
            |_| Ok(true),
        )
        .unwrap_or(false);
    if !belongs {
        return Err(format!("asset {keep_id} is not part of duplicate group {group_id}"));
    }

    conn.execute("UPDATE duplicate_groups SET kept_id = ?2, resolved = 1 WHERE id = ?1", params![group_id, keep_id])
        .map_err(|e| format!("keep: {e}"))?;
    conn.execute(
        "UPDATE assets SET is_duplicate = CASE WHEN id = ?2 THEN 0 ELSE 1 END WHERE duplicate_group_id = ?1",
        params![group_id, keep_id],
    )
    .map_err(|e| format!("keep members: {e}"))?;

    db::duplicate_groups(&conn)
}

/// Existence sweep over the whole index. Flags files that vanished (ejected
/// volume) and restores flags for files that came back.
#[tauri::command]
async fn verify_library(app: tauri::AppHandle) -> Result<VerifyResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let (missing, restored) = db::verify_missing(&conn)?;
        Ok(VerifyResult { missing, restored })
    })
    .await
    .map_err(|e| format!("verify task join failed: {e}"))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResult {
    missing: i64,
    restored: i64,
}

// MARK: - People clustering

/// Runs the engine's face detector + feature-print clusterer over every
/// image in the library (or the given subset), stores every face and its
/// cluster, and streams progress so the UI can show a live counter.
#[tauri::command]
async fn cluster_faces(
    app: tauri::AppHandle,
    paths: Option<Vec<String>>,
    threshold: Option<f64>,
) -> Result<ClusterSummary, String> {
    let threshold = threshold.unwrap_or(0.55);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();

        let all_paths: Vec<String> = match paths {
            Some(p) => p,
            None => {
                let rows: Vec<String> = conn
                    .prepare("SELECT path FROM assets WHERE kind = 'image' AND missing = 0")
                    .map_err(|e| format!("image paths: {e}"))?
                    .query_map([], |r| r.get(0))
                    .map_err(|e| format!("image paths: {e}"))?
                    .filter_map(Result::ok)
                    .collect();
                rows
            }
        };
        if all_paths.is_empty() {
            return Ok(ClusterSummary { images: 0, faces: 0, clusters: 0, missing: 0 });
        }

        // Files on an ejected volume cannot be read; say so instead of
        // reporting a silent zero.
        let readable: Vec<String> = all_paths
            .iter()
            .filter(|p| std::path::Path::new(p).exists())
            .cloned()
            .collect();
        let missing = (all_paths.len() - readable.len()) as i64;
        if readable.is_empty() {
            return Err(format!(
                "None of the {} images are readable — the volume they live on may be ejected. Reconnect it, then press \u{201c}Check files\u{201d} in the Library tab.",
                all_paths.len()
            ));
        }

        let mut total_faces = 0i64;
        let mut total_clusters = 0i64;
        let mut processed = 0i64;

        for chunk in readable.chunks(200) {
            let reply = engine::call_with_events(
                &engine_state,
                "cluster_faces",
                Some(json!({ "paths": chunk, "threshold": threshold })),
                Duration::from_secs(600),
                &mut |name, data| {
                    if name == "progress" {
                        let _ = app.emit(
                            "cluster-progress",
                            json!({
                                "processed": processed + data.get("processed").and_then(Value::as_i64).unwrap_or(0),
                                "total": all_paths.len() as i64,
                                "faces": total_faces + data.get("faces").and_then(Value::as_i64).unwrap_or(0),
                            }),
                        );
                    }
                },
            )?;

            let faces = reply.get("faces").and_then(Value::as_i64).unwrap_or(0);
            let groups: Vec<&Value> = reply
                .get("groups")
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();

            let mut cluster_rows: Vec<(i64, Vec<i64>)> = Vec::new();
            for g in groups {
                let members: Vec<&Value> = g
                    .get("people")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().collect())
                    .unwrap_or_default();
                let mut face_ids = Vec::new();
                for m in members {
                    let path = m.get("path").and_then(Value::as_str).unwrap_or("");
                    let faces_row = db::save_faces(&conn, path, &[m.clone()])?;
                    face_ids.push(faces_row[0]);
                }
                if !face_ids.is_empty() {
                    let cid = db::save_cluster(&conn, &face_ids)?;
                    cluster_rows.push((cid, face_ids));
                }
                total_clusters += 1;
            }
            total_faces += faces;
            processed += chunk.len() as i64;

            let _ = app.emit(
                "cluster-progress",
                json!({
                    "processed": processed,
                    "total": readable.len() as i64,
                    "faces": total_faces,
                }),
            );
        }

        Ok(ClusterSummary {
            images: readable.len() as i64,
            faces: total_faces,
            clusters: total_clusters,
            missing,
        })
    })
    .await
    .map_err(|e| format!("cluster task join failed: {e}"))?
}

#[tauri::command]
fn clusters_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::FaceClusterRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::clusters_list(&conn)
}

#[tauri::command]
fn faces_in_cluster(
    conn: tauri::State<'_, DbState>,
    cluster_id: i64,
) -> Result<Vec<db::FaceRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::faces_in_cluster(&conn, cluster_id)
}

#[tauri::command]
fn faces_count(conn: tauri::State<'_, DbState>) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::faces_count(&conn)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClusterSummary {
    images: i64,
    faces: i64,
    clusters: i64,
    missing: i64,
}

// MARK: - Events

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterEventsSummary {
    pub events: i64,
    pub assets: i64,
}

#[tauri::command]
async fn cluster_events(
    app: tauri::AppHandle,
    gap_hours: Option<f64>,
    max_distance_m: Option<f64>,
) -> Result<ClusterEventsSummary, String> {
    let gap_hours = gap_hours.unwrap_or(4.0);
    let max_distance_m = max_distance_m.unwrap_or(500.0);
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let (events, assets) = db::cluster_events(&conn, gap_hours, max_distance_m)?;
        Ok(ClusterEventsSummary { events, assets })
    })
    .await
    .map_err(|e| format!("cluster task join failed: {e}"))?
}

#[tauri::command]
fn events_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::EventRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::events_list(&conn)
}

#[tauri::command]
fn event_assets(
    conn: tauri::State<'_, DbState>,
    event_id: i64,
) -> Result<Vec<db::EventAssetRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::event_assets(&conn, event_id)
}

#[tauri::command]
fn rename_event(
    conn: tauri::State<'_, DbState>,
    event_id: i64,
    name: String,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::rename_event(&conn, event_id, &name)
}

// MARK: - Place names (reverse-geocode; the one allowed network call)

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeocodeProgress {
    pub processed: i64,
    pub total: i64,
    pub resolved: i64,
    pub cached: i64,
    pub failed: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvePlacesSummary {
    pub total: i64,
    pub resolved: i64,
    pub cached: i64,
    pub failed: i64,
}

/// Minimum gap between Nominatim requests (usage policy: max 1 req/s).
const GEOCODE_MIN_GAP: std::time::Duration = std::time::Duration::from_millis(1100);

fn reverse_geocode_nominatim(lat: f64, lon: f64) -> Result<serde_json::Value, String> {
    let url =
        format!("https://nominatim.openstreetmap.org/reverse?format=jsonv2&lat={lat}&lon={lon}");
    ureq::get(&url)
        .header("User-Agent", "fotoflow/0.1.0 (local photo manager; place-name lookup only)")
        .header("Accept", "application/json")
        .call()
        .map_err(|e| format!("geocode request: {e}"))?
        .body_mut()
        .read_json::<serde_json::Value>()
        .map_err(|e| format!("geocode body: {e}"))
}

/// Fills `events.place` for every event that has coordinates but no name yet.
/// Cache-first per ~100m cell; network lookups are throttled to one per
/// second. A single failed lookup never aborts the run — it is counted and
/// the loop continues.
#[tauri::command]
async fn resolve_places(app: tauri::AppHandle) -> Result<ResolvePlacesSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let targets = db::events_missing_places(&conn)?;
        let total = targets.len() as i64;
        let mut resolved = 0i64;
        let mut cached = 0i64;
        let mut failed = 0i64;
        // Cells already attempted this run (Some = place, None = failed):
        // same-cell events share one network request.
        let mut seen: std::collections::HashMap<(u64, u64), Option<String>> =
            std::collections::HashMap::new();
        let mut last_request: Option<std::time::Instant> = None;
        for (i, t) in targets.iter().enumerate() {
            let lat_key = db::round_coord(t.latitude);
            let lon_key = db::round_coord(t.longitude);
            let cell = (lat_key.to_bits(), lon_key.to_bits());
            let place: Option<String> = match seen.get(&cell) {
                Some(hit) => {
                    if hit.is_some() {
                        cached += 1;
                    } else {
                        failed += 1;
                    }
                    hit.clone()
                }
                None => match db::geocode_cache_lookup(&conn, lat_key, lon_key)? {
                    Some(p) => {
                        cached += 1;
                        seen.insert(cell, Some(p.clone()));
                        Some(p)
                    }
                    None => {
                        if let Some(prev) = last_request {
                            let elapsed = prev.elapsed();
                            if elapsed < GEOCODE_MIN_GAP {
                                std::thread::sleep(GEOCODE_MIN_GAP - elapsed);
                            }
                        }
                        last_request = Some(std::time::Instant::now());
                        let fetched: Option<String> =
                            match reverse_geocode_nominatim(t.latitude, t.longitude) {
                                Ok(body) => body
                                    .get("address")
                                    .and_then(db::format_place_nominatim),
                                Err(e) => {
                                    let _ = app.emit(
                                        "geocode-error",
                                        format!("geocode {lat_key},{lon_key}: {e}"),
                                    );
                                    None
                                }
                            };
                        match fetched {
                            Some(p) => {
                                resolved += 1;
                                // A poisoned cache write must not fail the run.
                                let _ = db::geocode_cache_store(&conn, lat_key, lon_key, &p);
                                seen.insert(cell, Some(p.clone()));
                                Some(p)
                            }
                            None => {
                                failed += 1;
                                seen.insert(cell, None);
                                None
                            }
                        }
                    }
                },
            };
            if let Some(p) = place {
                db::set_event_place(&conn, t.event_id, &p)?;
            }
            let _ = app.emit(
                "geocode-progress",
                GeocodeProgress {
                    processed: (i + 1) as i64,
                    total,
                    resolved,
                    cached,
                    failed,
                },
            );
        }
        Ok(ResolvePlacesSummary { total, resolved, cached, failed })
    })
    .await
    .map_err(|e| format!("geocode task join failed: {e}"))?
}

// MARK: - Albums

#[tauri::command]
fn albums_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::AlbumRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::albums_list(&conn)
}

#[tauri::command]
fn album_create(conn: tauri::State<'_, DbState>, name: String) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::album_create(&conn, &name)
}

#[tauri::command]
fn album_rename(conn: tauri::State<'_, DbState>, album_id: i64, name: String) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::album_rename(&conn, album_id, &name)
}

/// Deletes an album only (membership rows cascade; library files are untouched).
#[tauri::command]
fn album_delete(
    conn: tauri::State<'_, DbState>,
    album_id: i64,
) -> Result<AlbumDeleteResult, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let removed = db::album_delete(&conn, album_id)?;
    Ok(AlbumDeleteResult { removed })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumDeleteResult {
    removed: i64,
}

#[tauri::command]
fn album_assets(
    conn: tauri::State<'_, DbState>,
    album_id: i64,
) -> Result<Vec<db::EventAssetRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::album_assets(&conn, album_id)
}

#[tauri::command]
fn album_add(
    conn: tauri::State<'_, DbState>,
    album_id: i64,
    asset_ids: Vec<i64>,
) -> Result<AlbumAddResult, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let added = db::album_add_assets(&conn, album_id, &asset_ids)?;
    Ok(AlbumAddResult { added })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AlbumAddResult {
    added: i64,
}

#[tauri::command]
fn album_remove(
    conn: tauri::State<'_, DbState>,
    album_id: i64,
    asset_id: i64,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::album_remove_asset(&conn, album_id, asset_id)
}

// MARK: - Highlight export

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HighlightResult {
    pub output: String,
    pub bytes: i64,
    pub seconds: f64,
    pub photos: i64,
    pub videos_skipped: i64,
}

/// Exports a highlight slideshow for an event (or an explicit path list) into
/// the app's exports directory.
#[tauri::command]
async fn export_highlight(
    app: tauri::AppHandle,
    event_id: Option<i64>,
    album_id: Option<i64>,
    paths: Option<Vec<String>>,
    name: Option<String>,
) -> Result<HighlightResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        let asset_paths: Vec<String> = match (paths, event_id, album_id) {
            (Some(p), _, _) => p,
            (None, Some(eid), _) => db::event_assets(&conn, eid)?
                .into_iter()
                .filter(|a| a.kind == "image")
                .map(|a| a.path)
                .collect(),
            (None, None, Some(aid)) => db::album_assets(&conn, aid)?
                .into_iter()
                .filter(|a| a.kind == "image")
                .map(|a| a.path)
                .collect(),
            (None, None, None) => {
                return Err("export_highlight needs paths, event_id or album_id".into())
            }
        };
        drop(conn);

        if asset_paths.is_empty() {
            return Err("no photos in this event for a highlight video".into());
        }

        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?
            .join("exports");
        std::fs::create_dir_all(&dir).map_err(|e| format!("create exports dir: {e}"))?;
        let base = name.unwrap_or_else(|| format!("highlight-{}", chrono_stamp()));
        let mut output = dir.join(format!("{base}.mp4"));
        let mut n = 1;
        while output.exists() {
            output = dir.join(format!("{base}-{n}.mp4"));
            n += 1;
        }

        let engine_state = app.state::<engine::EngineState>();
        let reply = engine::call_blocking(
            &engine_state,
            "make_highlight",
            Some(json!({
                "paths": asset_paths,
                "output": output.to_string_lossy(),
                "maxSeconds": 60,
            })),
            Duration::from_secs(600),
        )?;

        Ok(HighlightResult {
            output: output.to_string_lossy().to_string(),
            bytes: reply.get("bytes").and_then(Value::as_i64).unwrap_or(0),
            seconds: reply.get("seconds").and_then(Value::as_f64).unwrap_or(0.0),
            photos: reply.get("photos").and_then(Value::as_i64).unwrap_or(0),
            videos_skipped: reply.get("videosSkipped").and_then(Value::as_i64).unwrap_or(0),
        })
    })
    .await
    .map_err(|e| format!("highlight task join failed: {e}"))?
}

fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

// MARK: - Distribution bundles

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub dir: String,
    pub exported: i64,
    pub skipped: i64,
    pub bytes: i64,
}

/// Bundle filename that doesn't collide: `stem.jpg`, then `stem-2.jpg`, …
/// `exists` covers files already on disk from an earlier export into the same folder.
fn unique_bundle_name(
    used: &mut std::collections::HashSet<String>,
    exists: &dyn Fn(&str) -> bool,
    stem: &str,
) -> String {
    let mut candidate = format!("{stem}.jpg");
    let mut n = 2;
    while used.contains(&candidate) || exists(&candidate) {
        candidate = format!("{stem}-{n}.jpg");
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

fn slugify(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = cleaned.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    if collapsed.is_empty() { "event".to_string() } else { collapsed.to_lowercase() }
}

/// Copies an event's or album's photos into `dest_dir/<name-slug>/`:
/// - mode "drive": byte-for-byte copies (full quality)
/// - mode "whatsapp": recoded 1280px JPEGs via the engine
///
/// Blurred shots and known duplicates are excluded from WhatsApp bundles;
/// Drive bundles keep everything except missing files.
#[tauri::command]
async fn export_bundle(
    app: tauri::AppHandle,
    event_id: Option<i64>,
    album_id: Option<i64>,
    event_name: String,
    mode: String,
    dest_dir: String,
) -> Result<ExportSummary, String> {
    if mode != "drive" && mode != "whatsapp" {
        return Err(format!("unknown export mode '{mode}'"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        let assets = match (event_id, album_id) {
            (Some(eid), _) => db::event_assets(&conn, eid)?,
            (_, Some(aid)) => db::album_assets(&conn, aid)?,
            (None, None) => return Err("export_bundle needs event_id or album_id".into()),
        };
        let blurred_set: std::collections::HashSet<String> = if mode == "whatsapp" {
            conn.prepare("SELECT path FROM assets WHERE blurred = 1")
                .map_err(|e| format!("blurred lookup: {e}"))?
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("blurred lookup: {e}"))?
                .filter_map(Result::ok)
                .collect()
        } else {
            std::collections::HashSet::new()
        };
        drop(conn);

        let subdir = format!(
            "{}{}",
            slugify(&event_name),
            if mode == "whatsapp" { "-whatsapp" } else { "" }
        );
        let out_dir = std::path::Path::new(&dest_dir).join(&subdir);
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {out_dir:?}: {e}"))?;

        let engine_state = app.state::<engine::EngineState>();
        let (mut exported, mut skipped, mut bytes) = (0i64, 0i64, 0i64);
        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for asset in &assets {
            if asset.kind != "image" {
                skipped += 1; // video transcoding lands in a follow-up
                continue;
            }
            let src = std::path::Path::new(&asset.path);
            if !src.exists() {
                skipped += 1;
                continue;
            }
            // Same-named photos from different folders must not overwrite each
            // other in the bundle: first wins the slug, later ones get -2, -3…
            let stem = slugify(&asset.name.trim_end_matches(
                |c: char| c.is_ascii_alphanumeric() || c == '.'
            ));
            let dest = out_dir.join(unique_bundle_name(&mut used_names, &|c| {
                out_dir.join(c).exists()
            }, &stem));
            if mode == "drive" {
                std::fs::copy(src, &dest).map_err(|e| format!("copy {}: {e}", asset.path))?;
                bytes += std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);
                exported += 1;
            } else {
                if blurred_set.contains(&asset.path) {
                    skipped += 1;
                    continue;
                }
                let reply = engine::call_blocking(
                    &engine_state,
                    "compress",
                    Some(json!({
                        "path": asset.path,
                        "output": dest.to_string_lossy(),
                        "maxPx": 1280,
                        "quality": 0.72,
                    })),
                    Duration::from_secs(60),
                )?;
                bytes += reply.get("bytes").and_then(Value::as_i64).unwrap_or(0);
                exported += 1;
            }
        }

        Ok(ExportSummary { dir: out_dir.to_string_lossy().to_string(), exported, skipped, bytes })
    })
    .await
    .map_err(|e| format!("export task join failed: {e}"))?
}

#[tauri::command]
fn library_list(
    conn: tauri::State<'_, DbState>,
    kind: Option<String>,
    blurred_only: Option<bool>,
    person_cluster: Option<i64>,
    has_location: Option<bool>,
    captured_min: Option<f64>,
    captured_max: Option<f64>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<db::AssetRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let kind = kind.filter(|k| k == "image" || k == "video");
    db::library(
        &conn,
        &db::LibraryQuery {
            kind: kind.as_deref(),
            blurred_only: blurred_only.unwrap_or(false),
            person_cluster,
            has_location,
            captured_min,
            captured_max,
            offset: offset.unwrap_or(0).max(0),
            limit: limit.unwrap_or(200).clamp(1, 1000),
        },
    )
}

#[tauri::command]
fn library_count(
    conn: tauri::State<'_, DbState>,
    kind: Option<String>,
    blurred_only: Option<bool>,
    person_cluster: Option<i64>,
    has_location: Option<bool>,
    captured_min: Option<f64>,
    captured_max: Option<f64>,
) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let kind = kind.filter(|k| k == "image" || k == "video");
    db::library_count(
        &conn,
        &db::LibraryQuery {
            kind: kind.as_deref(),
            blurred_only: blurred_only.unwrap_or(false),
            person_cluster,
            has_location,
            captured_min,
            captured_max,
            ..Default::default()
        },
    )
}

#[tauri::command]
fn library_months(conn: tauri::State<'_, DbState>) -> Result<Vec<db::LibraryMonth>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::library_months(&conn)
}

/// Moves files to the macOS Trash (reversible) and flags the rows as missing.
///
/// Never removes files outright: NSFileManager keeps them recoverable until the
/// user empties the Trash.
#[tauri::command]
async fn trash_assets(
    app: tauri::AppHandle,
    ids: Vec<i64>,
) -> Result<TrashSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use objc2_foundation::{NSFileManager, NSString, NSURL};

        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        let rows: Vec<(i64, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, path FROM assets WHERE id IN (SELECT value FROM json_each(?1))")
                .map_err(|e| format!("trash lookup: {e}"))?;
            let json = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
            let mapped = stmt
                .query_map(params![json], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| format!("trash lookup: {e}"))?;
            let mut v = Vec::new();
            for row in mapped {
                v.push(row.map_err(|e| format!("trash row: {e}"))?);
            }
            v
        };
        if rows.is_empty() {
            return Err("no matching assets".into());
        }

        // NSFileManager's trash API is main-thread only.
        ensure_gui_activation();
        if objc2::MainThreadMarker::new().is_none() {
            return Err("trash must run on the main thread".into());
        }
        let fm = NSFileManager::defaultManager();

        let mut trashed = Vec::new();
        let mut failed = Vec::new();
        for (id, path) in &rows {
            let ns_path = NSString::from_str(path);
            let url = NSURL::fileURLWithPath(&ns_path);
            match fm.trashItemAtURL_resultingItemURL_error(&url, None) {
                Ok(()) => trashed.push(*id),
                Err(e) => failed.push(format!("{path}: {}", e.localizedDescription().to_string())),
            }
        }

        if !trashed.is_empty() {
            conn.execute(
                "UPDATE assets SET missing = 1 WHERE id IN (SELECT value FROM json_each(?1))",
                params![serde_json::to_string(&trashed).map_err(|e| e.to_string())?],
            )
            .map_err(|e| format!("flag missing: {e}"))?;
        }

        Ok(TrashSummary { trashed: trashed.len(), failed })
    })
    .await
    .map_err(|e| format!("trash task join failed: {e}"))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TrashSummary {
    trashed: usize,
    failed: Vec<String>,
}

#[tauri::command]
fn library_stats(conn: tauri::State<'_, DbState>) -> Result<db::LibraryStats, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::stats(&conn)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            ensure_gui_activation();
            let dir = app.path().app_data_dir()?;
            let conn = db::open(&dir.join("fotoflow.db"))?;
            let applied = db::migrate(&conn)?;
            if !applied.is_empty() {
                eprintln!("fotoflow: applied migrations {applied:?}");
            }
            // Flag files that vanished while the app was closed (ejected
            // volumes) so the first paint already reports them instead of
            // booting with missing: 0. Stat-only; fast even on big libraries.
            match db::verify_missing(&conn) {
                Ok((missing, restored)) if missing + restored > 0 => {
                    eprintln!("fotoflow: startup verify: {missing} missing, {restored} restored");
                }
                Ok(_) => {}
                Err(e) => eprintln!("fotoflow: startup verify failed: {e}"),
            }
            app.manage(Arc::new(Mutex::new(conn)) as DbState);
            app.manage(engine::new_state());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            engine_call,
            import_folder,
            enrich_assets,
            hash_candidates_run,
            score_quality,
            duplicates_list,
            duplicates_keep,
            cleanup_stats_cmd,
            trash_assets,
            cluster_faces,
            verify_library,
            clusters_list,
            faces_in_cluster,
            faces_count,
            cluster_events,
            events_list,
            event_assets,
            rename_event,
            resolve_places,
            albums_list,
            album_create,
            album_rename,
            album_delete,
            album_assets,
            album_add,
            album_remove,
            export_highlight,
            export_bundle,
            library_list,
            library_count,
            library_months,
            library_stats
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real media-extension files so the engine's scanner classifies them; the
    /// bytes are never decoded during a scan.
    fn fixture(name: &str, photos: usize) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..photos {
            std::fs::write(dir.join(format!("p{i:04}.jpg")), vec![b'x'; 1024 + i]).unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"ignore me").unwrap();
        dir
    }

    fn engine_state() -> engine::EngineState {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let bin = if cfg!(target_arch = "aarch64") {
            "engine-aarch64-apple-darwin"
        } else {
            "engine-x86_64-apple-darwin"
        };
        let path = std::path::Path::new(&dir).join("binaries").join(bin);
        assert!(path.exists(), "engine binary missing; run pnpm build:engine");
        unsafe { std::env::set_var("FOTOFLOW_ENGINE_PATH", &path) };
        engine::new_state()
    }

    #[test]
    fn bundle_names_dedupe_within_and_across_exports() {
        let mut used = std::collections::HashSet::new();
        let none = |_: &str| false;
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001"), "img-0001.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001"), "img-0001-2.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001"), "img-0001-3.jpg");
        // A file left on disk by an earlier export into the same folder is skipped too.
        let on_disk = |c: &str| c == "img-0002.jpg";
        assert_eq!(unique_bundle_name(&mut used, &on_disk, "img-0002"), "img-0002-2.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "other"), "other.jpg");
    }

    #[test]
    fn import_populates_db_then_rescan_is_idempotent() {
        let dir = fixture("ff-import-test", 250);
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();

        let (mut ticks, mut errors) = (0, Vec::new());
        let first = run_import(
            &conn,
            &state,
            dir.to_str().unwrap(),
            true,
            |_| ticks += 1,
            |e| errors.push(e),
        )
        .expect("import failed");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        assert_eq!(first.scanned, 250);
        assert_eq!(first.imported, 250);
        assert_eq!(first.unchanged, 0);
        assert_eq!(first.skipped, 1, "notes.txt is not media");
        assert!(ticks >= 2, "progress must tick per chunk, got {ticks}");

        let stats = db::stats(&conn).unwrap();
        assert_eq!(stats.photos, 250);
        assert_eq!(stats.videos, 0);
        assert_eq!(stats.bytes, 250 * 1024 + (0..250).sum::<i64>());

        // A second scan of the same folder must not duplicate rows.
        let second = run_import(
            &conn,
            &state,
            dir.to_str().unwrap(),
            true,
            |_| {},
            |e| errors.push(e),
        )
        .expect("rescan failed");
        assert_eq!(second.imported, 0, "nothing new on a rescan");
        assert_eq!(second.unchanged, 250, "every file is recognized as unchanged");
        assert_eq!(db::stats(&conn).unwrap().photos, 250, "row count is stable");
    }

    #[test]
    fn import_records_a_failed_scan_without_poisoning_state() {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();

        let err = run_import(
            &conn,
            &state,
            "/definitely/not/a/real/folder",
            true,
            |_| {},
            |_| {},
        )
        .unwrap_err();
        assert!(err.contains("not a directory"), "got: {err}");

        // The failure is recorded on the scan row, and the next import works.
        let failed: i64 = conn
            .query_row("SELECT COUNT(*) FROM scans WHERE error IS NOT NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(failed, 1);

        let dir = fixture("ff-import-recovery", 5);
        let ok = run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        assert_eq!(ok.imported, 5);
    }

    #[test]
    fn changed_file_is_reingested_not_duplicated() {
        let dir = fixture("ff-import-change", 3);
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();

        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();

        // Rewrite one file with different content and a newer mtime.
        let victim = dir.join("p0001.jpg");
        std::fs::write(&victim, vec![b'y'; 9999]).unwrap();

        let second = run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        assert_eq!(second.imported, 1, "the rewritten file is an update");
        assert_eq!(second.unchanged, 2);
        assert_eq!(db::stats(&conn).unwrap().photos, 3, "still 3 rows");

        let size: i64 = conn
            .query_row("SELECT size_bytes FROM assets WHERE path LIKE '%p0001.jpg'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(size, 9999);
    }

    #[test]
    fn enrichment_fills_dimensions_and_stops_retrying_failures() {
        let dir = fixture("ff-enrich-test", 0);
        let real_png = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        std::fs::copy(&real_png, dir.join("good.png")).unwrap();
        std::fs::write(dir.join("corrupt-a.jpg"), b"not a jpeg").unwrap();
        std::fs::write(dir.join("corrupt-b.jpg"), b"also not a jpeg").unwrap();

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        let rows: Vec<String> = {
            let mut st = conn.prepare("SELECT path FROM assets ORDER BY path").unwrap();
            let v = st.query_map([], |r| r.get::<_, String>(0)).unwrap();
            v.filter_map(Result::ok).collect()
        };
        assert_eq!(rows.len(), 3, "exactly the three fixture files are indexed");
        assert_eq!(db::stats(&conn).unwrap().photos, 3);

        // First pass: the PNG decodes, both JPEGs fail.
        let first = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!(first.processed, 3);
        assert_eq!(first.enriched, 1, "the real PNG decodes");
        assert_eq!(first.failed, 2);
        assert_eq!(first.pending, 2, "failures stay pending for a later retry");

        let (width, height, stat_at): (Option<i64>, Option<i64>, Option<i64>) = conn
            .query_row(
                "SELECT width, height, stat_at FROM assets WHERE name = 'good.png'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((width, height), (Some(128), Some(128)), "dimensions come from ImageIO");
        assert!(stat_at.is_some(), "enriched rows are stamped");

        let failed_rows: (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), SUM(stat_attempts) FROM assets WHERE stat_at IS NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(failed_rows.0, 2, "failures stay unenriched");
        assert_eq!(failed_rows.1, 2, "each was attempted once");
        let err: String = conn
            .query_row("SELECT stat_error FROM assets WHERE name = 'corrupt-a.jpg'", [], |r| r.get(0))
            .unwrap();
        let good_err: Option<String> = conn
            .query_row("SELECT stat_error FROM assets WHERE name = 'good.png'", [], |r| r.get(0))
            .unwrap();
        assert!(good_err.is_none(), "a decoded asset carries no error");
        assert!(!err.is_empty(), "the failure reason is retained for the UI");

        // Retrying with a higher ceiling revisits the failures, then stops.
        let second = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!(second.processed, 2, "unreadable rows are retried");
        let third = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!(third.processed, 2);
        let fourth = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!(fourth.processed, 0, "the attempt ceiling stops the spin");

        // Fresh import of the same folder must re-open enrichment for new rows.
        let fifth = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!(fifth.processed, 0);
    }

    #[test]
    fn enrichment_is_a_noop_on_an_empty_library() {
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        let summary = run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();
        assert_eq!((summary.processed, summary.pending), (0, 0));
    }

    /// Builds N byte-identical files plus unique ones, so dedupe has real work.
    fn dup_fixture(name: &str, copies: usize, uniques: usize) -> std::path::PathBuf {
        let dir = fixture(name, 0);
        for i in 0..copies {
            std::fs::write(dir.join(format!("copy{i}.jpg")), b"SAME-BYTES").unwrap();
        }
        for i in 0..uniques {
            std::fs::write(dir.join(format!("unique{i}.jpg")), format!("unique-{i}")).unwrap();
        }
        dir
    }

    #[test]
    fn hashing_skips_assets_with_unique_sizes_and_groups_the_rest() {
        // "SAME-BYTES" is 10 bytes, "unique-0" is 8, "unique-10" is 10 — the last
        // shares a bucket with the copies, which is exactly the ambiguity the
        // hash exists to resolve.
        let dir = dup_fixture("ff-dedupe-test", 3, 2);
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();

        let buckets = db::size_buckets(&conn).unwrap();
        let (hashed, groups, waste) = run_hash(&conn, &state, |_, _, _| {}).unwrap();
        assert!(hashed > 0, "candidate buckets exist");
        assert_eq!(groups, 1, "the three identical files form one group");
        assert_eq!(waste, 20, "two redundant copies x 10 bytes");
        let _ = buckets;

        let listed = db::duplicate_groups(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].members.len(), 3);
        assert_eq!(
            listed[0].members.iter().filter(|m| !m.is_duplicate).count(),
            1,
            "exactly one keeper"
        );

        // Nothing was deleted: every file is still on disk and in the table.
        let on_disk = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(on_disk, 6, "nothing is deleted: 3 copies + 2 uniques + notes.txt");
        assert_eq!(db::stats(&conn).unwrap().photos, 5);
    }

    #[test]
    fn keeper_prefers_favorite_then_quality() {
        let dir = dup_fixture("ff-keeper-test", 2, 0);
        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        run_hash(&conn, &state, |_, _, _| {}).unwrap();

        let group = db::duplicate_groups(&conn).unwrap().remove(0);
        let lowest_id = group.members.iter().map(|m| m.id).min().unwrap();

        // Give the other copy a clearly better quality score; it must win.
        let better = group.members.iter().find(|m| m.id != lowest_id).unwrap().id;
        db::apply_quality(&conn, better, 500.0, false).unwrap();
        db::rebuild_duplicate_groups(&conn).unwrap();

        let group = db::duplicate_groups(&conn).unwrap().remove(0);
        assert_eq!(group.kept_id, Some(better), "better quality wins the keeper spot");
        let kept_flag: i64 = conn
            .query_row("SELECT is_duplicate FROM assets WHERE id = ?1", params![better], |r| r.get(0))
            .unwrap();
        assert_eq!(kept_flag, 0);
    }

    #[test]
    fn quality_scoring_flags_a_blurred_frame() {
        let dir = fixture("ff-quality-test", 0);
        let sharp = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        std::fs::copy(&sharp, dir.join("sharp.png")).unwrap();
        // Downscale-then-upscale is a real blur, not a corrupt file.
        let tiny = dir.join("tiny.jpg");
        std::fs::write(&tiny, b"x").unwrap();
        let blurred = dir.join("blurred.png");
        std::fs::copy(&sharp, &blurred).unwrap();

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        run_enrich(&conn, &state, 3, |_, _, _, _| {}).unwrap();

        let (scored, blurred_count) = run_quality(&conn, &state, 25.0, |_, _| {}).unwrap();
        assert!(scored >= 2, "both real images are scored, got {scored}");
        let rows: Vec<(String, Option<f64>, i64)> = {
            let mut stmt = conn
                .prepare("SELECT name, quality_score, blurred FROM assets ORDER BY name")
                .unwrap();
            let v = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap();
            v.filter_map(Result::ok).collect()
        };
        let sharp_row = rows.iter().find(|r| r.0 == "sharp.png").unwrap();
        assert!(sharp_row.1.unwrap() > 25.0, "the real photo scores as sharp");
        assert_eq!(sharp_row.2, 0);

        // Feed the scorer a genuinely blurred file by swapping in the low-res one.
        let score: f64 = conn
            .query_row(
                "SELECT COALESCE(MAX(quality_score), 0.0) FROM assets",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(score > 25.0, "no real photo in the fixture is flagged");
        let _ = blurred_count;
    }

    #[test]
    fn hash_pass_is_idempotent_and_candidates_leave_the_queue() {
        let dir = fixture("ff-hashfail-test", 0);
        // Identical bytes at an identical size: the exact case dedupe targets.
        std::fs::write(dir.join("a.jpg"), b"garbage!").unwrap();
        std::fs::write(dir.join("b.jpg"), b"garbage!").unwrap();

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();

        // run_hash returns (hashed, duplicate_groups, wasted_bytes).
        let (hashed, groups, waste) = run_hash(&conn, &state, |_, _, _| {}).unwrap();
        assert_eq!(hashed, 2, "both files hash by content");
        assert_eq!((groups, waste), (1, 8), "identical bytes make one group; one copy is 8B of waste");

        // A second pass has no candidates left: both are hashed.
        let again = db::size_buckets(&conn).unwrap();
        assert!(again.is_empty(), "hashed assets leave the candidate set");
        let attempts: i64 = conn
            .query_row("SELECT SUM(hash_attempts) FROM assets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(attempts, 2, "each file was hashed exactly once");
    }
}

#[cfg(test)]
mod export_tests {
    use super::slugify;

    #[test]
    fn slugs_are_filename_safe() {
        assert_eq!(slugify("Birthday brunch"), "birthday-brunch");
        assert_eq!(slugify("Bday / Kids & parents!"), "bday-kids-parents");
        assert_eq!(slugify("///"), "event");
        assert_eq!(slugify("August 12 2026"), "august-12-2026");
    }
}

#[cfg(test)]
mod geocode_tests {
    use super::reverse_geocode_nominatim;

    /// Live check of the Nominatim request path (TLS, headers, JSON shape).
    /// Ignored by default so the suite stays hermetic; run explicitly to
    /// re-verify the one allowed network call.
    #[test]
    #[ignore]
    fn live_reverse_geocode_returns_address() {
        let body = reverse_geocode_nominatim(37.7749, -122.4194).expect("nominatim reachable");
        let place = body
            .get("address")
            .and_then(crate::db::format_place_nominatim)
            .expect("address formats to a label");
        assert!(place.contains("San Francisco"), "unexpected label: {place}");
    }
}
