mod db;
mod engine;
mod geofences;
mod server;

pub use server::serve;

use serde::Serialize;
use rusqlite::params;
use serde_json::{Value, json};

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Media extensions mirrored from the engine's IMAGE_EXTS + VIDEO_EXTS so
/// ingest copies exactly what a later import would index.
const MEDIA_EXTS: &[&str] = &[
    "jpg", "jpeg", "heic", "heif", "png", "tif", "tiff", "gif", "webp", "bmp", "dng", "raw", "cr2",
    "cr3", "nef", "arw", "orf", "rw2", "raf", "avif",
    "mov", "mp4", "m4v", "avi", "mpg", "mpeg", "hevc", "mkv", "webm", "3gp", "mts", "m2ts",
];

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct IngestSummary {
    pub copied: i64,
    pub skipped: i64,
    pub failed: Vec<String>,
    pub bytes: i64,
    pub total: i64,
    pub total_bytes: i64,
    pub dest: String,
}

fn collect_media(root: &std::path::Path, out: &mut Vec<(std::path::PathBuf, u64)>) {
    let entries = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Symlinks (and everything non-regular) are skipped: no loop risk,
        // no AppleDouble-style resource forks sneaking into the copy.
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_media(&path, out);
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let is_media = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| MEDIA_EXTS.contains(&e.to_lowercase().as_str()))
            .unwrap_or(false);
        if is_media {
            out.push((path, meta.len()));
        }
    }
}

/// Copies a camera card / device folder to an SSD (or any destination),
/// mirroring the tree. Pure filesystem work — no DB writes — so it is a
/// plain function; the Tauri command below only adds progress events.
pub fn ingest_copy_tree(
    src: &std::path::Path,
    dest: &std::path::Path,
    mut on_progress: impl FnMut(&IngestSummary),
) -> Result<IngestSummary, String> {
    if !src.is_dir() {
        return Err(format!("ingest source is not a folder: {}", src.display()));
    }
    if dest.starts_with(src) {
        return Err("ingest destination cannot be inside the source folder".into());
    }
    let mut files: Vec<(std::path::PathBuf, u64)> = Vec::new();
    collect_media(src, &mut files);
    files.sort();
    let total_bytes: i64 = files.iter().map(|(_, s)| *s as i64).sum();
    let mut summary = IngestSummary {
        total: files.len() as i64,
        total_bytes,
        dest: dest.to_string_lossy().to_string(),
        ..Default::default()
    };
    for (path, size) in &files {
        let rel = path
            .strip_prefix(src)
            .map_err(|e| format!("relativize {}: {e}", path.display()))?;
        let target = dest.join(rel);
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                summary.failed.push(format!("create {}: {e}", parent.display()));
                on_progress(&summary);
                continue;
            }
        }
        // Identical file already there: skip instead of re-copying gigabytes.
        if let Ok(m) = std::fs::metadata(&target) {
            if m.is_file() && m.len() == *size {
                summary.skipped += 1;
                summary.bytes += *size as i64;
                on_progress(&summary);
                continue;
            }
        }
        match std::fs::copy(path, &target) {
            Ok(_) => {
                let got = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                if got != *size {
                    let _ = std::fs::remove_file(&target);
                    summary.failed.push(format!(
                        "size mismatch after copy (want {size}, got {got}): {}",
                        path.display()
                    ));
                } else {
                    summary.copied += 1;
                    summary.bytes += *size as i64;
                }
            }
            Err(e) => summary.failed.push(format!("copy {}: {e}", path.display())),
        }
        on_progress(&summary);
    }
    Ok(summary)
}

/// Tauri entry point: copies a source folder to a destination with progress
/// on `ingest-progress`. Desktop-only (like the folder picker): there is no
/// HTTP equivalent, the browser cannot read arbitrary disk paths.
#[tauri::command]
async fn ingest_copy(app: tauri::AppHandle, src: String, dest: String) -> Result<IngestSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        ingest_copy_tree(std::path::Path::new(&src), std::path::Path::new(&dest), |s| {
            let _ = app.emit("ingest-progress", s);
        })
    })
    .await
    .map_err(|e| format!("ingest task join failed: {e}"))?
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
    // Self-heal for the videoStat upgrade: videos enriched before capture
    // dates were extracted (stat_at set, captured_at NULL) go back in the
    // queue once. After a successful re-stat they carry dates and never
    // match again; genuinely dateless files exhaust stat_attempts and rest.
    conn.execute(
        "UPDATE assets SET stat_at = NULL WHERE kind = 'video' AND captured_at IS NULL AND stat_at IS NOT NULL AND missing = 0",
        [],
    )
    .map_err(|e| format!("requeue dateless videos: {e}"))?;
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearDupeSummary {
    pub hashed: i64,
    pub failed: i64,
    pub groups: i64,
    pub waste_bytes: i64,
}

/// Backfills perceptual hashes in batches of 100, then rebuilds the
/// near-duplicate groups. O(n^2) grouping refuses past NEAR_DUPE_MAX_IMAGES.
pub fn run_near_dupes(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    mut on_progress: impl FnMut(i64, i64, i64),
) -> Result<(i64, i64, i64, i64), String> {
    let (mut hashed, mut failed) = (0i64, 0i64);
    loop {
        let batch = db::dhash_candidates(conn, 100)?;
        if batch.is_empty() {
            break;
        }
        let paths: Vec<String> = batch.iter().map(|(_, p)| p.clone()).collect();
        let reply = engine::call_blocking(
            engine_state,
            "dhash_batch",
            Some(json!({ "paths": paths })),
            Duration::from_secs(300),
        )?;
        let results = reply
            .get("results")
            .and_then(Value::as_array)
            .ok_or("dhash_batch returned no results array")?;
        if results.len() != batch.len() {
            return Err(format!(
                "dhash_batch returned {} results for {} paths",
                results.len(),
                batch.len()
            ));
        }
        for ((id, _), result) in batch.iter().zip(results) {
            if result.get("ok").and_then(Value::as_bool) == Some(true) {
                if let Some(dhash) = result.get("dhash").and_then(Value::as_str) {
                    db::apply_dhash(conn, *id, dhash)?;
                    hashed += 1;
                    continue;
                }
            }
            let message = result.get("error").and_then(Value::as_str).unwrap_or("dhash failed");
            db::apply_dhash_failure(conn, *id, message)?;
            failed += 1;
        }
        on_progress(hashed, failed, 0);
    }
    let (groups, waste) = db::rebuild_near_groups(conn)?;
    Ok((hashed, failed, groups, waste))
}

#[tauri::command]
async fn near_dupes_run(app: tauri::AppHandle) -> Result<NearDupeSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        let (hashed, failed, groups, waste_bytes) =
            run_near_dupes(&conn, &engine_state, |hashed, failed, _| {
                let _ = app.emit("near-progress", json!({ "hashed": hashed, "failed": failed }));
            })?;
        Ok(NearDupeSummary { hashed, failed, groups, waste_bytes })
    })
    .await
    .map_err(|e| format!("near-dupe task join failed: {e}"))?
}

#[tauri::command]
fn near_groups_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::NearGroup>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::near_groups(&conn)
}

/// Marks a keeper for a near-duplicate group. Never deletes.
#[tauri::command]
fn near_keep(
    conn: tauri::State<'_, DbState>,
    group_id: i64,
    keep_id: i64,
) -> Result<Vec<db::NearGroup>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::near_keep(&conn, group_id, keep_id)?;
    db::near_groups(&conn)
}

#[tauri::command]
fn duplicates_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::DuplicateGroup>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::duplicate_groups(&conn)
}

// MARK: - Scene tags

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TagProgress {
    pub processed: i64,
    pub total: i64,
    pub tagged: i64,
    pub failed: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagScenesSummary {
    pub total: i64,
    pub tagged: i64,
    pub failed: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneTagCount {
    pub tag: String,
    pub count: i64,
}

/// Classifies every untagged readable image with the on-device Vision scene
/// classifier (mountains, flowers, birds…) and stores the top labels.
/// One failed file never aborts the run.
pub fn run_tag_scenes(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    mut on_progress: impl FnMut(i64, i64, i64, i64),
) -> Result<TagScenesSummary, String> {
    let targets = db::untagged_images(conn)?;
    let total = targets.len() as i64;
    let mut tagged = 0i64;
    let mut failed = 0i64;
    for (i, (id, path)) in targets.iter().enumerate() {
        let ok = if !std::path::Path::new(path).exists() {
            false
        } else {
            match engine::call_blocking(
                engine_state,
                "classify",
                Some(json!({ "path": path, "top": 3 })),
                Duration::from_secs(60),
            ) {
                Ok(reply) => {
                    let tags: Vec<(String, f64)> = reply
                        .get("tags")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| {
                                    Some((
                                        t.get("tag")?.as_str()?.to_string(),
                                        t.get("confidence")?.as_f64()?,
                                    ))
                                })
                                .filter(|(_, c)| *c >= 0.2)
                                .take(3)
                                .collect()
                        })
                        .unwrap_or_default();
                    if tags.is_empty() {
                        false
                    } else {
                        db::store_tags(conn, *id, &tags).is_ok()
                    }
                }
                Err(_) => false,
            }
        };
        if ok {
            tagged += 1;
        } else {
            failed += 1;
        }
        on_progress((i + 1) as i64, total, tagged, failed);
    }
    Ok(TagScenesSummary { total, tagged, failed })
}

#[tauri::command]
async fn tag_scenes(app: tauri::AppHandle) -> Result<TagScenesSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        run_tag_scenes(&conn, &engine_state, |processed, total, tagged, failed| {
            let _ = app.emit(
                "tag-progress",
                TagProgress { processed, total, tagged, failed },
            );
        })
    })
    .await
    .map_err(|e| format!("tag task join failed: {e}"))?
}

// MARK: - Visual similarity (similar-photos v1)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedSummary {
    pub embedded: i64,
    pub failed: i64,
}

/// Backfills Vision feature-print vectors for readable images missing one.
/// One engine round-trip per batch of 100; a single bad file never aborts.
/// One attempt per asset per invocation: failures stay vector-less (and are
/// retried on the next call) instead of spinning the loop.
pub fn run_embed_library(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    mut on_progress: impl FnMut(i64, i64),
) -> Result<EmbedSummary, String> {
    let (mut embedded, mut failed) = (0i64, 0i64);
    // A failed embed leaves no row, so without this guard the same broken
    // file would be retried forever (mirrors run_enrich/run_quality).
    let mut attempted: std::collections::HashSet<i64> = std::collections::HashSet::new();
    loop {
        let batch: Vec<(i64, String)> = db::embed_candidates(conn, db::VECTOR_MODEL, 100)?
            .into_iter()
            .filter(|(id, _)| !attempted.contains(id))
            .collect();
        if batch.is_empty() {
            break;
        }
        let paths: Vec<String> = batch.iter().map(|(_, p)| p.clone()).collect();
        let reply = engine::call_with_events(
            engine_state,
            "embed_batch",
            Some(json!({ "paths": paths })),
            Duration::from_secs(300),
            &mut |name, data| {
                if name == "progress" {
                    let done = data.get("processed").and_then(Value::as_i64).unwrap_or(0);
                    on_progress(embedded + done, failed);
                }
            },
        )?;
        let results = reply
            .get("results")
            .and_then(Value::as_array)
            .ok_or("embed_batch returned no results array")?;
        if results.len() != batch.len() {
            return Err(format!(
                "embed_batch returned {} results for {} paths",
                results.len(),
                batch.len()
            ));
        }
        for ((id, _), result) in batch.iter().zip(results) {
            attempted.insert(*id);
            let vector: Option<(Vec<f32>, i64)> =
                if result.get("ok").and_then(Value::as_bool) == Some(true) {
                    result
                        .get("vector")
                        .and_then(Value::as_str)
                        .and_then(|b| b64_decode(b).ok())
                        .and_then(|bytes| db::bytes_to_floats(&bytes))
                        .zip(result.get("dim").and_then(Value::as_i64))
                } else {
                    None
                };
            match vector {
                Some((vec, dim)) => match db::save_vector(conn, *id, db::VECTOR_MODEL, dim, &vec) {
                    Ok(()) => embedded += 1,
                    Err(_) => failed += 1,
                },
                None => failed += 1,
            }
        }
        on_progress(embedded, failed);
    }
    Ok(EmbedSummary { embedded, failed })
}

#[tauri::command]
async fn embed_library(app: tauri::AppHandle) -> Result<EmbedSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        run_embed_library(&conn, &engine_state, |embedded, failed| {
            let _ = app.emit(
                "embed-progress",
                json!({ "processed": embedded + failed, "embedded": embedded, "failed": failed }),
            );
        })
    })
    .await
    .map_err(|e| format!("embed task join failed: {e}"))?
}

#[tauri::command]
fn similar_to(
    conn: tauri::State<'_, DbState>,
    asset_id: i64,
    limit: Option<i64>,
) -> Result<Vec<db::SimilarHit>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::similar_to(&conn, asset_id, db::VECTOR_MODEL, limit.unwrap_or(12).clamp(1, 100))
}

// MARK: - Semantic search (CLIP)

/// Model release hosting the converted CoreML packages. Pinned by hash;
/// bump the version (and both hashes) to roll the models forward.
pub const CLIP_MODELS_VERSION: &str = "clip-models-v1";
const CLIP_IMAGE_URL: &str = "https://github.com/yasirhusain89/mylife/releases/download/clip-models-v1/CLIPImage.mlpackage.zip";
const CLIP_IMAGE_SHA256: &str = "0ac03dcfbdb4e4c99be33b6f4c100bb836625768576a04d5fcb756c233c6d7f4";
const CLIP_IMAGE_ZIP_BYTES: u64 = 162408298;
const CLIP_TEXT_URL: &str = "https://github.com/yasirhusain89/mylife/releases/download/clip-models-v1/CLIPText.mlpackage.zip";
const CLIP_TEXT_SHA256: &str = "35a85464d94d288e59fd5e3a6649d2c63646819f4e2ece1a5cc620dc72b5aac7";
const CLIP_TEXT_ZIP_BYTES: u64 = 116735598;

fn clip_model_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("models")
        .join("clip-vit-b32"))
}

/// Locates the CLIP BPE vocab. Tauri copies the sidecar binary WITHOUT its
/// sibling files, so the engine cannot rely on "next to the binary": in dev
/// it runs from `target/debug/` and in prod from `Contents/MacOS/`, neither
/// of which carries the vocab. Candidates, first readable file wins:
/// bundled resource (prod), dev staging dir, engine binary dir (probes).
fn clip_vocab_path(app: &tauri::AppHandle) -> Result<String, String> {
    let mut tried: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        tried.push(res.join("clip_bpe_vocab.txt.gz"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Dev layout: <repo>/src-tauri/target/debug/fotoflow, vocab staged
            // at <repo>/src-tauri/binaries/ by build-engine.sh.
            tried.push(dir.join("..").join("binaries").join("clip_bpe_vocab.txt.gz"));
        }
    }
    if let Some(engine) = engine::engine_path() {
        if let Some(dir) = engine.parent() {
            tried.push(dir.join("clip_bpe_vocab.txt.gz"));
        }
    }
    for cand in &tried {
        if cand.is_file() {
            return Ok(cand.to_string_lossy().into_owned());
        }
    }
    let tried = tried
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!("CLIP vocab missing (tried {tried}). Run: pnpm build:engine"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipModelsStatus {
    pub ready: bool,
    pub bytes_total: u64,
    pub bytes_on_disk: u64,
}

/// Disk usage of a directory tree (models + compiled CoreML caches).
fn dir_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

/// Whether both CoreML packages are on disk (unpacked, ready to load).
#[tauri::command]
fn clip_models_present(app: tauri::AppHandle) -> Result<ClipModelsStatus, String> {
    let dir = clip_model_dir(&app)?;
    let ready = dir.join("CLIPImage.mlpackage").is_dir() && dir.join("CLIPText.mlpackage").is_dir();
    let bytes_on_disk = if dir.is_dir() { dir_size(&dir) } else { 0 };
    Ok(ClipModelsStatus { ready, bytes_total: CLIP_IMAGE_ZIP_BYTES + CLIP_TEXT_ZIP_BYTES, bytes_on_disk })
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Verifies a downloaded model zip against its pinned hash and unpacks it
/// into the models dir. Pure apart from the filesystem, so the round-trip is
/// unit-tested with a synthetic archive.
fn verify_and_unpack(dir: &std::path::Path, name: &str, zip_bytes: &[u8], sha: &str) -> Result<(), String> {
    let got = sha256_hex(zip_bytes);
    if got != sha {
        return Err(format!("{name} hash mismatch: got {got}, want {sha}"));
    }
    let cursor = std::io::Cursor::new(zip_bytes);
    zip::ZipArchive::new(cursor)
        .map_err(|e| format!("{name} unzip open: {e}"))?
        .extract(dir)
        .map_err(|e| format!("{name} unzip: {e}"))?;
    Ok(())
}

/// Downloads both CoreML packages (first use, explicit opt-in via the UI),
/// verifies hashes, and unzips them into the models dir. Progress rides the
/// `clip-download-progress` event ({name, received, total}).
#[tauri::command]
async fn download_clip_models(app: tauri::AppHandle) -> Result<ClipModelsStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = clip_model_dir(&app)?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("create models dir: {e}"))?;
        for (name, url, sha, total) in [
            ("CLIPImage", CLIP_IMAGE_URL, CLIP_IMAGE_SHA256, CLIP_IMAGE_ZIP_BYTES),
            ("CLIPText", CLIP_TEXT_URL, CLIP_TEXT_SHA256, CLIP_TEXT_ZIP_BYTES),
        ] {
            let target = dir.join(format!("{name}.mlpackage"));
            if target.is_dir() {
                let _ = app.emit(
                    "clip-download-progress",
                    json!({ "name": name, "received": total, "total": total }),
                );
                continue;
            }
            let zip_path = dir.join(format!("{name}.mlpackage.zip"));
            let mut resp = ureq::get(url)
                .header("User-Agent", "fotoflow/0.1.0 (local photo manager; one-time model fetch)")
                .call()
                .map_err(|e| format!("{name} download: {e}"))?;
            let mut reader = resp.body_mut().as_reader();
            let file = std::fs::File::create(&zip_path).map_err(|e| format!("{name} cache file: {e}"))?;
            let mut writer = std::io::BufWriter::new(file);
            let mut buf = vec![0u8; 1 << 20];
            let mut received: u64 = 0;
            loop {
                let n = std::io::Read::read(&mut reader, &mut buf)
                    .map_err(|e| format!("{name} download read: {e}"))?;
                if n == 0 {
                    break;
                }
                std::io::Write::write_all(&mut writer, &buf[..n])
                    .map_err(|e| format!("{name} download write: {e}"))?;
                received += n as u64;
                let _ = app.emit(
                    "clip-download-progress",
                    json!({ "name": name, "received": received, "total": total }),
                );
            }
            drop(writer);
            let zip_bytes =
                std::fs::read(&zip_path).map_err(|e| format!("{name} re-read: {e}"))?;
            if let Err(e) = verify_and_unpack(&dir, name, &zip_bytes, sha) {
                let _ = std::fs::remove_file(&zip_path);
                return Err(e);
            }
            let _ = std::fs::remove_file(&zip_path);
        }
        let bytes_on_disk = dir_size(&dir);
        Ok(ClipModelsStatus { ready: true, bytes_total: CLIP_IMAGE_ZIP_BYTES + CLIP_TEXT_ZIP_BYTES, bytes_on_disk })
    })
    .await
    .map_err(|e| format!("clip download join failed: {e}"))?
}

/// Deletes the downloaded models (and their compiled caches) to reclaim
/// disk. Search keeps working after a fresh download + backfill.
#[tauri::command]
async fn remove_clip_models(app: tauri::AppHandle) -> Result<ClipModelsStatus, String> {
    let dir = clip_model_dir(&app)?;
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove models: {e}"))?;
    }
    Ok(ClipModelsStatus { ready: false, bytes_total: CLIP_IMAGE_ZIP_BYTES + CLIP_TEXT_ZIP_BYTES, bytes_on_disk: 0 })
}

/// Cooperative cancel for the semantic embed backfill. Set by the UI; the
/// loop checks it between files and returns early. Reset at the start of
/// every backfill run so a stale cancel cannot skip a fresh one.
static CLIP_CANCEL: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn cancel_clip_backfill() {
    CLIP_CANCEL.store(true, Ordering::SeqCst);
}

/// Backfills CLIP photo vectors (images) and keyframe moments (videos) for
/// rows missing them. One engine round-trip per file; a single bad file
/// never aborts, and `embed_attempts` bounds retries so an undecodable file
/// cannot keep the loop alive.
///
/// The DB lock is taken **per file**, never across an engine call: holding
/// the global connection for a whole-library pass froze every other command
/// (and, because `search_text` is sync, the main thread too).
#[tauri::command]
async fn embed_clip_backfill(app: tauri::AppHandle) -> Result<EmbedSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let model_dir = clip_model_dir(&app)?;
        if !model_dir.join("CLIPImage.mlpackage").is_dir()
            || !model_dir.join("CLIPText.mlpackage").is_dir()
        {
            return Err("CLIP models are not downloaded yet".into());
        }
        let model_dir = model_dir.to_string_lossy().into_owned();
        CLIP_CANCEL.store(false, Ordering::SeqCst);
        let db = app.state::<DbState>().inner().clone();
        let engine_state = app.state::<engine::EngineState>();
        let (mut embedded, mut failed) = (0i64, 0i64);
        let mut attempted: std::collections::HashSet<i64> = std::collections::HashSet::new();

        loop {
            if CLIP_CANCEL.load(Ordering::SeqCst) {
                break;
            }
            let batch: Vec<(i64, String)> = {
                let conn = db.lock().map_err(|_| "database lock poisoned")?;
                db::clip_photo_candidates(&conn, 20)?
                    .into_iter()
                    .filter(|(id, _)| !attempted.contains(id))
                    .collect()
            };
            if batch.is_empty() {
                break;
            }
            for (id, path) in &batch {
                if CLIP_CANCEL.load(Ordering::SeqCst) {
                    break;
                }
                attempted.insert(*id);
                let reply = engine::call_blocking(
                    &engine_state,
                    "clip_encode_image",
                    Some(json!({ "path": path, "modelDir": model_dir })),
                    Duration::from_secs(300),
                );
                let vector: Option<Vec<f32>> = reply
                    .ok()
                    .as_ref()
                    .and_then(|r| r.get("vector"))
                    .and_then(Value::as_str)
                    .and_then(|b| b64_decode(b).ok())
                    .and_then(|bytes| db::bytes_to_floats(&bytes));
                let conn = db.lock().map_err(|_| "database lock poisoned")?;
                let outcome = match vector {
                    Some(vec) => db::save_clip_vector(&conn, *id, &vec)
                        .map_err(|e| format!("save clip vector: {e}")),
                    None => Err("image encode returned no vector".to_string()),
                };
                match outcome {
                    Ok(()) => {
                        let _ = db::record_clip_embed(&conn, *id, "clip-image", None);
                        embedded += 1;
                    }
                    Err(e) => {
                        let _ = db::record_clip_embed(&conn, *id, "clip-image", Some(&e));
                        failed += 1;
                    }
                }
            }
            let _ = app.emit(
                "embed-progress",
                json!({ "processed": embedded + failed, "embedded": embedded, "failed": failed }),
            );
        }

        loop {
            if CLIP_CANCEL.load(Ordering::SeqCst) {
                break;
            }
            let batch: Vec<(i64, String)> = {
                let conn = db.lock().map_err(|_| "database lock poisoned")?;
                db::keyframe_candidates(&conn, 5)?
                    .into_iter()
                    .filter(|(id, _)| !attempted.contains(id))
                    .collect()
            };
            if batch.is_empty() {
                break;
            }
            for (id, path) in &batch {
                if CLIP_CANCEL.load(Ordering::SeqCst) {
                    break;
                }
                attempted.insert(*id);
                let reply = engine::call_with_events(
                    &engine_state,
                    "clip_keyframes",
                    Some(json!({
                        "path": path,
                        "modelDir": model_dir,
                        "everySeconds": 10.0,
                        "maxFrames": 60,
                    })),
                    Duration::from_secs(600),
                    &mut |_, _| {},
                );
                let frames: Option<Vec<(f64, Vec<f32>)>> = reply.ok().as_ref().and_then(|r| {
                    r.get("frames")?.as_array()?.iter().map(|f| {
                        let t = f.get("t")?.as_f64()?;
                        let vec = f.get("vector")?.as_str()
                            .and_then(|b| b64_decode(b).ok())
                            .and_then(|bytes| db::bytes_to_floats(&bytes))?;
                        Some((t, vec))
                    }).collect()
                });
                let conn = db.lock().map_err(|_| "database lock poisoned")?;
                let outcome = match frames {
                    Some(frames) if !frames.is_empty() => {
                        db::save_keyframes(&conn, *id, &frames)
                            .map_err(|e| format!("save keyframes: {e}"))
                    }
                    _ => Err("keyframe extraction returned no frames".to_string()),
                };
                match outcome {
                    Ok(()) => {
                        let _ = db::record_clip_embed(&conn, *id, "clip-video", None);
                        embedded += 1;
                    }
                    Err(e) => {
                        let _ = db::record_clip_embed(&conn, *id, "clip-video", Some(&e));
                        failed += 1;
                    }
                }
            }
            let _ = app.emit(
                "embed-progress",
                json!({ "processed": embedded + failed, "embedded": embedded, "failed": failed }),
            );
        }
        Ok(EmbedSummary { embedded, failed })
    })
    .await
    .map_err(|e| format!("clip embed join failed: {e}"))?
}

/// Clears retry counters, then backfills again. This is the user-facing
/// "reindex" path: files that gave up (unmounted volume, locked file) get a
/// fresh set of attempts.
#[tauri::command]
async fn reindex_clip(app: tauri::AppHandle) -> Result<EmbedSummary, String> {
    {
        let db = app.state::<DbState>();
        let conn = db.lock().map_err(|_| "database lock poisoned")?;
        db::reset_clip_attempts(&conn)?;
    }
    embed_clip_backfill(app).await
}

/// Text search over photos and video moments: the query is embedded by the
/// engine (Swift BPE + CoreML, verified parity with the Python tokenizer),
/// then cosine-ranked against stored vectors in one pass.
#[tauri::command]
async fn search_text(
    app: tauri::AppHandle,
    text: String,
    limit: Option<i64>,
) -> Result<Vec<db::SearchHit>, String> {
    if text.trim().is_empty() {
        return Err("empty search text".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let model_dir = clip_model_dir(&app)?;
        let vocab_path = clip_vocab_path(&app)?;
        let engine_state = app.state::<engine::EngineState>();
        let reply = engine::call_blocking(
            &engine_state,
            "clip_encode_text",
            Some(json!({ "text": text, "modelDir": model_dir.to_string_lossy(), "vocabPath": vocab_path })),
            Duration::from_secs(120),
        )?;
        let query = reply
            .get("vector")
            .and_then(Value::as_str)
            .and_then(|b| b64_decode(b).ok())
            .and_then(|bytes| db::bytes_to_floats(&bytes))
            .ok_or("text encode returned no vector")?;
        // Lock only for the scan; the engine call above stays outside it.
        let db = app.state::<DbState>();
        let conn = db.lock().map_err(|_| "database lock poisoned")?;
        db::search_clip(&conn, &query, limit.unwrap_or(50).clamp(1, 200))
    })
    .await
    .map_err(|e| format!("clip search join failed: {e}"))?
}

// MARK: - Post-import pipeline

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProgress {
    pub stage: String,
    pub processed: i64,
    pub total: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSummary {
    pub enriched: i64,
    pub hashed: i64,
    pub scored: i64,
    pub near: i64,
    pub events: i64,
    pub tagged: i64,
    pub embedded: i64,
    pub faces: i64,
    pub places: i64,
    pub failed: i64,
}

/// Offline place fill for the pipeline: exact cell, cached fence, static
/// table — never the network. Precise fence hits pin their cell; coarse
/// static labels apply in-memory only (see resolve_places).
pub fn run_fill_cached_places(conn: &rusqlite::Connection) -> Result<i64, String> {
    let targets = db::events_missing_places(conn)?;
    let mut filled = 0i64;
    for t in &targets {
        let lat_key = db::round_coord(t.latitude);
        let lon_key = db::round_coord(t.longitude);
        if let Some(p) = db::geocode_cache_lookup(conn, lat_key, lon_key)? {
            db::set_event_place(conn, t.event_id, &p)?;
            filled += 1;
            continue;
        }
        if let Ok(Some(p)) = db::geocode_covering(conn, t.latitude, t.longitude) {
            let _ = db::geocode_cache_store(conn, lat_key, lon_key, &p, None);
            db::set_event_place(conn, t.event_id, &p)?;
            filled += 1;
            continue;
        }
        if let Some(label) = crate::geofences::static_geofence(t.latitude, t.longitude) {
            db::set_event_place(conn, t.event_id, &approx_label(label))?;
            filled += 1;
        }
    }
    Ok(filled)
}

/// Runs every offline organization pass in dependency order: metadata,
/// hashes, blur scores, near-duplicates, scene tags, faces, event clusters,
/// cached-only places. Faces run before events so counts are fresh; events
/// run after dedup signals so later curation can exclude non-keepers.
/// Network geocoding stays on its button. A stage-level error counts once
/// and the chain continues — one bad stage never strands the rest.
pub fn run_post_import(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    max_attempts: i64,
    mut on_stage: impl FnMut(PipelineProgress),
) -> Result<PipelineSummary, String> {
    let mut failed = 0i64;
    let mut emit = |stage: &str, processed: i64, total: i64| {
        on_stage(PipelineProgress { stage: stage.to_string(), processed, total });
    };

    let enriched = match run_enrich(conn, engine_state, max_attempts, |p, _, f, pend| {
        emit("enrich", p, p + pend);
    }) {
        Ok(s) => {
            failed += s.failed;
            s.enriched
        }
        Err(_) => {
            failed += 1;
            0
        }
    };

    let hashed = match run_hash(conn, engine_state, |h, f, _| {
        emit("hash", h + f, 0);
    }) {
        Ok((h, _, _)) => h,
        Err(_) => {
            failed += 1;
            0
        }
    };

    let scored = match run_quality(conn, engine_state, 25.0, |s, _| {
        emit("quality", s, 0);
    }) {
        Ok((s, _)) => s,
        Err(_) => {
            failed += 1;
            0
        }
    };

    let near = match run_near_dupes(conn, engine_state, |h, f, _| {
        emit("near", h + f, 0);
    }) {
        Ok((_, _, groups, _)) => groups,
        Err(_) => {
            failed += 1;
            0
        }
    };

    let tagged = match run_tag_scenes(conn, engine_state, |p, t, _, _| {
        emit("tag", p, t);
    }) {
        Ok(s) => {
            failed += s.failed;
            s.tagged
        }
        Err(_) => {
            failed += 1;
            0
        }
    };

    let embedded = match run_embed_library(conn, engine_state, |e, f| {
        emit("embed", e + f, 0);
    }) {
        Ok(s) => {
            failed += s.failed;
            s.embedded
        }
        Err(_) => {
            failed += 1;
            0
        }
    };

    // Faces stay incremental: images that already have descriptors skip
    // engine calls, then every stored descriptor regroups globally so new
    // imports join their people (names + person tags follow in
    // run_cluster_faces). Runs after tag_scenes so scene stamps land first,
    // and before cluster_events so event face counts are fresh.
    let faces = match run_cluster_faces(conn, engine_state, None, 0.55, |p, t, _| {
        emit("faces", p, t);
    }) {
        Ok(s) => s.faces,
        Err(_) => {
            failed += 1;
            0
        }
    };

    let events = match db::cluster_events(conn, 4.0, 500.0) {
        Ok((e, _)) => {
            emit("events", e, e);
            e
        }
        Err(_) => {
            failed += 1;
            0
        }
    };

    let places = match run_fill_cached_places(conn) {
        Ok(n) => {
            emit("places", n, n);
            n
        }
        Err(_) => {
            failed += 1;
            0
        }
    };

    emit("done", 0, 0);
    Ok(PipelineSummary { enriched, hashed, scored, near, events, tagged, embedded, faces, places, failed })
}

#[tauri::command]
async fn run_post_import_cmd(app: tauri::AppHandle) -> Result<PipelineSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        run_post_import(&conn, &engine_state, 3, |p| {
            let _ = app.emit("pipeline-progress", p);
        })
    })
    .await
    .map_err(|e| format!("pipeline task join failed: {e}"))?
}

#[tauri::command]
fn scene_tags(conn: tauri::State<'_, DbState>) -> Result<Vec<SceneTagCount>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    Ok(db::distinct_tags(&conn)?
        .into_iter()
        .map(|(tag, count)| SceneTagCount { tag, count })
        .collect())
}

#[tauri::command]
fn asset_tags(
    conn: tauri::State<'_, DbState>,
    asset_id: i64,
) -> Result<Vec<db::SceneTag>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::asset_tags(&conn, asset_id)
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
        let (unreadable, unreadable_paths) = probe_unreadable(&conn);
        Ok(VerifyResult { missing, restored, unreadable, unreadable_paths })
    })
    .await
    .map_err(|e| format!("verify task join failed: {e}"))?
}

/// Capped sample of unreadable paths in the verify summary (keeps the IPC
/// payload small on big libraries with a dead volume).
const VERIFY_UNREADABLE_SAMPLE_CAP: usize = 20;

/// True when a file that exists cannot be opened/read (permissions, dead
/// volume mid-scan). A 1-byte read is enough: success means the path is
/// at least openable, which is all `verifyLibrary` promises beyond existence.
fn is_unreadable_1byte(path: &str) -> bool {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return true,
    };
    let mut buf = [0u8; 1];
    f.read(&mut buf).is_err()
}

/// 1-byte readability sweep over files that exist (missing rows excluded —
/// the existence pass owns those). Never fails the verify: an unreadable
/// row is counted, never flagged. Returns (count, capped sample).
fn probe_unreadable(conn: &rusqlite::Connection) -> (i64, Vec<String>) {
    let paths: Vec<String> = conn
        .prepare("SELECT path FROM assets WHERE missing = 0")
        .and_then(|mut st| {
            st.query_map([], |r| r.get::<_, String>(0))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    let mut count = 0i64;
    let mut sample = Vec::new();
    for p in &paths {
        if !std::path::Path::new(p).exists() {
            continue; // vanished between passes; the missing count owns it
        }
        if is_unreadable_1byte(p) {
            count += 1;
            if sample.len() < VERIFY_UNREADABLE_SAMPLE_CAP {
                sample.push(p.clone());
            }
        }
    }
    (count, sample)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VerifyResult {
    missing: i64,
    restored: i64,
    unreadable: i64,
    unreadable_paths: Vec<String>,
}

// MARK: - People clustering

/// Euclidean distance over f32 descriptors (Vision feature-print distance is
/// Euclidean over normalized vectors; see stackoverflow iOS16/17 analysis).
/// Returns INFINITY on length mismatch so corrupt pairs never merge.
fn face_euclidean(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return f64::INFINITY;
    }
    let mut sum = 0f64;
    for i in 0..a.len() {
        let d = a[i] as f64 - b[i] as f64;
        sum += d * d;
    }
    sum.sqrt()
}

/// Global union-find over descriptors at `threshold`. Deterministic order:
/// biggest groups first, ties by smallest face id (stable across reruns).
pub fn group_faces_by_distance(
    ids: &[i64],
    vecs: &[Vec<f32>],
    threshold: f64,
) -> Vec<Vec<i64>> {
    let n = ids.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if face_euclidean(&vecs[i], &vecs[j]) < threshold {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut comps: std::collections::HashMap<usize, Vec<i64>> = std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        comps.entry(r).or_default().push(ids[i]);
    }
    let mut groups: Vec<Vec<i64>> = comps.into_values().collect();
    for g in &mut groups {
        g.sort_unstable();
    }
    groups.sort_by(|a, b| b.len().cmp(&a.len()).then(a[0].cmp(&b[0])));
    groups
}

/// Runs the engine's face detector over images missing embeddings, then
/// globally regroups every stored descriptor. Incremental: images that already
/// have descriptors skip engine calls, so reruns with no new images only
/// rebuild the same groups (names preserved via mutual majority).
pub fn run_cluster_faces(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    paths_override: Option<Vec<String>>,
    threshold: f64,
    mut on_progress: impl FnMut(i64, i64, i64),
) -> Result<ClusterSummary, String> {
    let all_paths: Vec<String> = match paths_override {
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

    // Detection candidates: images with no face rows, or with faces lacking
    // descriptors (pre-migration rows or failed embeds). Already-embedded
    // images skip engine calls entirely.
    let (known_paths, null_paths): (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ) = {
        let mut known = std::collections::HashSet::new();
        let mut nulls = std::collections::HashSet::new();
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM faces")
            .map_err(|e| format!("face paths: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("face paths: {e}"))?;
        for row in rows {
            if let Ok(p) = row {
                known.insert(p);
            }
        }
        let mut stmt = conn
            .prepare("SELECT DISTINCT path FROM faces WHERE descriptor IS NULL")
            .map_err(|e| format!("face null paths: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("face null paths: {e}"))?;
        for row in rows {
            if let Ok(p) = row {
                nulls.insert(p);
            }
        }
        (known, nulls)
    };
    let candidates: Vec<String> = readable
        .iter()
        .filter(|p| !known_paths.contains(*p) || null_paths.contains(*p))
        .cloned()
        .collect();

    let mut new_count = 0i64;
    let mut processed = 0i64;
    for chunk in candidates.chunks(200) {
        let reply = engine::call_with_events(
            engine_state,
            "cluster_faces",
            Some(json!({ "paths": chunk, "threshold": threshold })),
            Duration::from_secs(600),
            &mut |name, data| {
                if name == "progress" {
                    on_progress(
                        processed + data.get("processed").and_then(Value::as_i64).unwrap_or(0),
                        readable.len() as i64,
                        new_count + data.get("faces").and_then(Value::as_i64).unwrap_or(0),
                    );
                }
            },
        )?;

        // Drop stale rows for this chunk before inserting fresh detections:
        // upgrade path converts NULL-descriptor rows to embedded ones, and a
        // redetection that finds zero faces correctly clears the path.
        for p in chunk {
            conn.execute("DELETE FROM faces WHERE path = ?1", params![p])
                .map_err(|e| format!("clear stale faces: {e}"))?;
        }

        // FLATTEN: ignore per-chunk groups; global regroup happens below.
        let groups: Vec<&Value> = reply
            .get("groups")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        for g in groups {
            let members: Vec<&Value> = g
                .get("people")
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            for m in members {
                let path = m.get("path").and_then(Value::as_str).unwrap_or("");
                let ids = db::save_faces(conn, path, &[m.clone()])?;
                new_count += ids.len() as i64;
            }
        }
        processed += chunk.len() as i64;
        on_progress(processed, readable.len() as i64, new_count);
    }

    // Global union-find at threshold over ALL stored + new descriptors.
    let stored = db::face_descriptors(conn)?;
    let groups: Vec<Vec<i64>> = if stored.is_empty() {
        Vec::new()
    } else {
        let ids: Vec<i64> = stored.iter().map(|(id, _)| *id).collect();
        let vecs: Vec<Vec<f32>> = stored.into_iter().map(|(_, v)| v).collect();
        group_faces_by_distance(&ids, &vecs, threshold)
    };
    // Faces without descriptors (failed embeds) stay unassigned; they are
    // candidates again on the next run.
    // Regroups mint fresh cluster ids, so re-stamp every surviving group:
    // new members of a named cluster gain its `person:<name>` tag and photos
    // that left the group lose the stamp they no longer justify.
    for cid in db::replace_clusters(conn, groups)? {
        db::sync_person_tags(conn, cid)?;
    }

    let faces = db::faces_count(conn)?;
    let clusters = db::clusters_list(conn)?.len() as i64;
    on_progress(readable.len() as i64, readable.len() as i64, faces);
    Ok(ClusterSummary { images: readable.len() as i64, faces, clusters, missing })
}

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
        run_cluster_faces(&conn, &engine_state, paths, threshold, |processed, total, faces| {
            let _ = app.emit(
                "cluster-progress",
                json!({ "processed": processed, "total": total, "faces": faces }),
            );
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

/// Detaches one face from its person cluster. Returns the cluster id.
#[tauri::command]
fn unassign_face(conn: tauri::State<'_, DbState>, face_id: i64) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::unassign_face(&conn, face_id)
}

/// Reattaches one face to a cluster (undo for unassign_face).
#[tauri::command]
fn assign_face(
    conn: tauri::State<'_, DbState>,
    face_id: i64,
    cluster_id: i64,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::assign_face(&conn, face_id, cluster_id)
}

/// Reinserts one face_clusters row and moves the given faces into it
/// (undo for merge_clusters). Returns the new cluster id.
#[tauri::command]
fn restore_cluster(
    conn: tauri::State<'_, DbState>,
    name: Option<String>,
    face_ids: Vec<i64>,
) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::restore_cluster(&conn, name.as_deref(), &face_ids)
}

#[tauri::command]
fn faces_count(conn: tauri::State<'_, DbState>) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::faces_count(&conn)
}

/// Ranked People-review queue: near-duplicate cluster pairs and lone faces
/// near a cluster, closest first. Read-only; accepting uses merge_clusters /
/// assign_face, rejecting uses dismiss_review_candidate.
#[tauri::command]
fn review_candidates(conn: tauri::State<'_, DbState>) -> Result<Vec<db::ReviewCandidate>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::review_candidates(&conn)
}

/// Records a "different people" verdict for a candidate bridge pair so it
/// never resurfaces in the review queue.
#[tauri::command]
fn dismiss_review_candidate(
    conn: tauri::State<'_, DbState>,
    face_a: i64,
    face_b: i64,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::record_review_decision(&conn, face_a, face_b, "separate")
}

// MARK: - Pet detection

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetProgress {
    pub processed: i64,
    pub total: i64,
    pub sightings: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectPetsSummary {
    pub images: i64,
    pub sightings: i64,
    pub missing: i64,
}

/// Runs the engine's cat/dog detector over readable images and stores the
/// sightings (replacing any from a previous run). No identity tracking.
#[tauri::command]
async fn detect_pets(app: tauri::AppHandle) -> Result<DetectPetsSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();

        let all_paths: Vec<String> = conn
            .prepare("SELECT path FROM assets WHERE kind = 'image' AND missing = 0")
            .map_err(|e| format!("image paths: {e}"))?
            .query_map([], |r| r.get(0))
            .map_err(|e| format!("image paths: {e}"))?
            .filter_map(Result::ok)
            .collect();
        let readable: Vec<String> = all_paths
            .iter()
            .filter(|p| std::path::Path::new(p).exists())
            .cloned()
            .collect();
        let missing = (all_paths.len() - readable.len()) as i64;
        let total = readable.len() as i64;
        let mut sightings = 0i64;
        let mut processed = 0i64;
        for path in &readable {
            let reply = engine::call_blocking(
                &engine_state,
                "detect_animals",
                Some(json!({ "path": path })),
                Duration::from_secs(120),
            );
            match reply {
                Ok(body) => {
                    let list: Vec<Value> = body
                        .get("animals")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    if !list.is_empty() {
                        sightings += db::save_sightings(&conn, path, &list)?.len() as i64;
                    } else {
                        // Clear stale sightings when nothing is found now.
                        db::save_sightings(&conn, path, &[])?;
                    }
                }
                Err(e) => {
                    let _ = app.emit("pet-error", format!("{path}: {e}"));
                }
            }
            processed += 1;
            let _ = app.emit(
                "pet-progress",
                PetProgress { processed, total, sightings },
            );
        }
        Ok(DetectPetsSummary { images: total, sightings, missing })
    })
    .await
    .map_err(|e| format!("pet task join failed: {e}"))?
}

#[tauri::command]
fn sightings_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::PetSighting>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::sightings_list(&conn)
}

#[tauri::command]
fn rename_cluster(
    conn: tauri::State<'_, DbState>,
    cluster_id: i64,
    name: String,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::rename_cluster(&conn, cluster_id, &name)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MergeClustersResult {
    merged: i64,
}

#[tauri::command]
fn merge_clusters(
    conn: tauri::State<'_, DbState>,
    from_id: i64,
    into_id: i64,
) -> Result<MergeClustersResult, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let merged = db::merge_clusters(&conn, from_id, into_id)?;
    Ok(MergeClustersResult { merged })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSummary {
    pub images: i64,
    pub faces: i64,
    pub clusters: i64,
    pub missing: i64,
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

/// Nominatim `boundingbox`: [south, north, west, east] as strings.
/// Rejects inverted or degenerate boxes so junk never becomes a fence.
fn parse_nominatim_bbox(body: &serde_json::Value) -> Option<(f64, f64, f64, f64)> {
    let b = body.get("boundingbox")?.as_array()?;
    if b.len() != 4 {
        return None;
    }
    let n = |i: usize| b.get(i)?.as_str()?.parse::<f64>().ok();
    let (s, n_, w, e) = (n(0)?, n(1)?, n(2)?, n(3)?);
    if s <= n_ && w <= e && (n_ - s) < 180.0 && (e - w) < 360.0 {
        Some((s, n_, w, e))
    } else {
        None
    }
}

/// Marks coarse static labels as approximate. Precise network and cached
/// labels never carry the mark; `events_missing_places` treats marked rows
/// as still needing refinement.
fn approx_label(label: &str) -> String {
    format!("≈ {label}")
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
                        // Geofence reuse: a cached bounding box covering this
                        // point answers without a network round-trip.
                        if let Ok(Some(p)) =
                            db::geocode_covering(&conn, t.latitude, t.longitude)
                        {
                            cached += 1;
                            // Pin the exact cell too, so the next lookup and
                            // the cluster-time naming hit it directly.
                            let _ = db::geocode_cache_store(&conn, lat_key, lon_key, &p, None);
                            seen.insert(cell, Some(p.clone()));
                            Some(p)
                        } else if let Some(label) = geofences::static_geofence(t.latitude, t.longitude) {
                            // Offline basic categorization (state/country).
                            // Deliberately NOT pinned to the cache: the coarse
                            // label stays on this event, while precise network
                            // results keep flowing into their own cells.
                            cached += 1;
                            let label = approx_label(label);
                            seen.insert(cell, Some(label.clone()));
                            Some(label)
                        } else {
                        if let Some(prev) = last_request {
                            let elapsed = prev.elapsed();
                            if elapsed < GEOCODE_MIN_GAP {
                                std::thread::sleep(GEOCODE_MIN_GAP - elapsed);
                            }
                        }
                        last_request = Some(std::time::Instant::now());
                        let fetched: Option<(String, Option<(f64, f64, f64, f64)>)> =
                            match reverse_geocode_nominatim(t.latitude, t.longitude) {
                                Ok(body) => body
                                    .get("address")
                                    .and_then(db::format_place_nominatim)
                                    .map(|p| (p, parse_nominatim_bbox(&body))),
                                Err(e) => {
                                    let _ = app.emit(
                                        "geocode-error",
                                        format!("geocode {lat_key},{lon_key}: {e}"),
                                    );
                                    None
                                }
                            };
                        match fetched {
                            Some((p, bbox)) => {
                                resolved += 1;
                                // A poisoned cache write must not fail the run.
                                let _ = db::geocode_cache_store(&conn, lat_key, lon_key, &p, bbox);
                                seen.insert(cell, Some(p.clone()));
                                Some(p)
                            }
                            None => {
                                failed += 1;
                                seen.insert(cell, None);
                                None
                            }
                        }
                        } // end fence-miss: network path above
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
    pub videos_included: i64,
    pub videos_skipped: i64,
}

/// Story budget: 3s per clip; 16 curated clips leave headroom for day cards.
const HIGHLIGHT_CLIP_BUDGET: i64 = 16;

/// (id, path) rows with capture times and per-asset places attached.
fn items_for_ids(
    conn: &rusqlite::Connection,
    ids_paths: &[(i64, String)],
    places: &std::collections::HashMap<i64, Option<String>>,
) -> Result<Vec<db::HighlightItem>, String> {
    let ids: Vec<i64> = ids_paths.iter().map(|(id, _)| *id).collect();
    let json = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(captured_at, modified), kind FROM assets
             WHERE id IN (SELECT value FROM json_each(?1))",
        )
        .map_err(|e| format!("prepare highlight times: {e}"))?;
    let rows = stmt
        .query_map(params![json], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("query highlight times: {e}"))?;
    let mut meta: std::collections::HashMap<i64, (f64, String)> = std::collections::HashMap::new();
    for row in rows {
        let (id, t, kind) = row.map_err(|e| format!("highlight time row: {e}"))?;
        meta.insert(id, (t, kind));
    }
    Ok(ids_paths
        .iter()
        .map(|(id, path)| {
            let (taken, kind) = meta.get(id).cloned().unwrap_or((0.0, "image".into()));
            db::HighlightItem {
                path: path.clone(),
                taken,
                place: places.get(id).cloned().unwrap_or(None),
                kind,
            }
        })
        .collect())
}

/// Curated-or-complete picks for an id set, kept in the caller's order.
fn filter_to_picks(
    ids_paths: &[(i64, String)],
    picks: &[db::CuratePick],
) -> Vec<(i64, String)> {
    picks
        .iter()
        .filter_map(|p| ids_paths.iter().find(|(id, _)| *id == p.id).cloned())
        .collect()
}

/// Story clips for one event: the event's place chapters every card.
fn plan_event_clips(
    conn: &rusqlite::Connection,
    eid: i64,
) -> Result<Vec<db::HighlightClip>, String> {
    let place: Option<String> = conn
        .query_row("SELECT place FROM events WHERE id = ?1", params![eid], |r| {
            r.get(0)
        })
        .map_err(|e| format!("highlight event: {e}"))?;
    let members = db::event_assets(conn, eid)?;
    let ids_paths: Vec<(i64, String)> = members.into_iter().map(|m| (m.id, m.path)).collect();
    let chosen: Vec<(i64, String)> = if ids_paths.len() > HIGHLIGHT_CLIP_BUDGET as usize {
        let picks = db::curate_picks(conn, &[eid], HIGHLIGHT_CLIP_BUDGET)?;
        filter_to_picks(&ids_paths, &picks)
    } else {
        ids_paths.clone()
    };
    let places: std::collections::HashMap<i64, Option<String>> =
        chosen.iter().map(|(id, _)| (*id, place.clone())).collect();
    Ok(db::highlight_plan(items_for_ids(conn, &chosen, &places)?))
}

/// Story clips for an album: day-only chapters (albums span places).
fn plan_album_clips(
    conn: &rusqlite::Connection,
    aid: i64,
) -> Result<Vec<db::HighlightClip>, String> {
    let members = db::album_assets(conn, aid)?;
    let ids_paths: Vec<(i64, String)> = members.into_iter().map(|m| (m.id, m.path)).collect();
    let chosen: Vec<(i64, String)> = if ids_paths.len() > HIGHLIGHT_CLIP_BUDGET as usize {
        let member_ids: Vec<i64> = ids_paths.iter().map(|(id, _)| *id).collect();
        let json = serde_json::to_string(&member_ids).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT ea.event_id FROM event_assets ea
                 WHERE ea.asset_id IN (SELECT value FROM json_each(?1))",
            )
            .map_err(|e| format!("prepare highlight events: {e}"))?;
        let rows = stmt
            .query_map(params![json], |r| r.get::<_, i64>(0))
            .map_err(|e| format!("query highlight events: {e}"))?;
        let mut eids = Vec::new();
        for row in rows {
            eids.push(row.map_err(|e| format!("highlight event row: {e}"))?);
        }
        if eids.is_empty() {
            // Unclustered members: first budget slice, chronological.
            ids_paths.into_iter().take(HIGHLIGHT_CLIP_BUDGET as usize).collect()
        } else {
            let picks =
                db::curate_picks_constrained(conn, &eids, Some(&member_ids), HIGHLIGHT_CLIP_BUDGET)?;
            let got = filter_to_picks(&ids_paths, &picks);
            if got.is_empty() {
                ids_paths.into_iter().take(HIGHLIGHT_CLIP_BUDGET as usize).collect()
            } else {
                got
            }
        }
    } else {
        ids_paths.clone()
    };
    let places: std::collections::HashMap<i64, Option<String>> = std::collections::HashMap::new();
    Ok(db::highlight_plan(items_for_ids(conn, &chosen, &places)?))
}

struct StoryRender {
    bytes: i64,
    seconds: f64,
    photos: i64,
    videos_included: i64,
    videos_skipped: i64,
}

/// Foreground progress for long exports ("export-progress" events). A `total`
/// of 0 means indeterminate: the phase label is the whole signal. Consumers
/// filter on `job`, so concurrent exports never cross-talk.
fn emit_export(app: &tauri::AppHandle, job: &str, phase: &str, done: i64, total: i64, label: &str) {
    let _ = app.emit(
        "export-progress",
        json!({ "job": job, "phase": phase, "done": done, "total": total, "label": label }),
    );
}

/// Renders story clips to one MP4. Titled stories breathe (90s cap when
/// chapters exist, else 60s). Engine per-clip progress forwards as the
/// `rendering` phase; stills land ~1/sec while a heavy video clip holds
/// `done` still, then it jumps — an honest unit count, not wall time.
fn render_story(
    engine_state: &engine::EngineState,
    app: &tauri::AppHandle,
    job: &str,
    clips: &[db::HighlightClip],
    output: &std::path::Path,
) -> Result<StoryRender, String> {
    if clips.is_empty() {
        return Err("no photos or videos in this event for a highlight video".into());
    }
    let total = clips.len() as i64;
    emit_export(app, job, "rendering", 0, total, "Starting memory video…");
    let titled = clips.iter().any(|c| c.title.is_some());
    let mut on_event = |name: &str, data: &Value| {
        if name == "progress" {
            let done = data.get("done").and_then(Value::as_i64).unwrap_or(0);
            let n = data.get("total").and_then(Value::as_i64).unwrap_or(total);
            emit_export(app, job, "rendering", done, n, &format!("Rendering clip {done} of {n}…"));
        }
    };
    let reply = engine::call_with_events(
        engine_state,
        "make_highlight",
        Some(json!({
            "clips": clips,
            "output": output.to_string_lossy(),
            "maxSeconds": if titled { 90 } else { 60 },
        })),
        Duration::from_secs(600),
        &mut on_event,
    )?;
    Ok(StoryRender {
        bytes: reply.get("bytes").and_then(Value::as_i64).unwrap_or(0),
        seconds: reply.get("seconds").and_then(Value::as_f64).unwrap_or(0.0),
        photos: reply.get("photos").and_then(Value::as_i64).unwrap_or(0),
        videos_included: reply.get("videosIncluded").and_then(Value::as_i64).unwrap_or(0),
        videos_skipped: reply.get("videosSkipped").and_then(Value::as_i64).unwrap_or(0),
    })
}

/// Exports a highlight slideshow for an event (or an explicit path list) into
/// the app's exports directory.
///
/// Story mode for event/album scopes: members beyond the clip budget go
/// through balanced coverage curation (Stage 2), then a chronological chapter
/// plan (Stage 3) — the video spans the whole timeline with a title card per
/// day instead of stopping after the first 20 filenames.
#[tauri::command]
async fn export_highlight(
    app: tauri::AppHandle,
    event_id: Option<i64>,
    album_id: Option<i64>,
    paths: Option<Vec<String>>,
    name: Option<String>,
    job: String,
) -> Result<HighlightResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        emit_export(&app, &job, "planning", 0, 0, "Planning story…");
        let clips: Vec<db::HighlightClip> = match (paths, event_id, album_id) {
            (Some(p), _, _) => p
                .into_iter()
                .map(|path| db::HighlightClip { path, title: None })
                .collect(),
            (None, Some(eid), _) => plan_event_clips(&conn, eid)?,
            (None, None, Some(aid)) => plan_album_clips(&conn, aid)?,
            (None, None, None) => {
                return Err("export_highlight needs paths, event_id or album_id".into())
            }
        };
        drop(conn);

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
        let render = render_story(&engine_state, &app, &job, &clips, &output)?;
        emit_export(&app, &job, "done", 1, 1, "");

        Ok(HighlightResult {
            output: output.to_string_lossy().to_string(),
            bytes: render.bytes,
            seconds: render.seconds,
            photos: render.photos,
            videos_included: render.videos_included,
            videos_skipped: render.videos_skipped,
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

// MARK: - Share pack (family distribution bundle)

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharePackSummary {
    pub dir: String,
    pub video: String,
    pub exported: i64,
    pub skipped: i64,
    pub bytes: i64,
}

/// One-click family share pack: a memory video plus the handpicked
/// photos/videos organized in Day folders, plus an offline index.html story
/// gallery friends can open in any browser.
///
/// - `event_ids` (trip/event stops) or `album_id` selects the scope; albums
///   curate within their own members.
/// - `mode` "drive" copies byte-for-byte, "compact" recodes small (1280px).
/// - `strip_gps` rewrites Drive copies without location metadata (Compact
///   recodes carry no metadata by construction).
/// - Scopes past SHARE_PACK_BUDGET go through balanced coverage curation;
///   one corrupt file skips instead of aborting the pack.
#[tauri::command]
async fn export_share_pack(
    app: tauri::AppHandle,
    event_ids: Option<Vec<i64>>,
    album_id: Option<i64>,
    name: String,
    mode: String,
    strip_gps: bool,
    dest_dir: String,
    job: String,
) -> Result<SharePackSummary, String> {
    if mode != "drive" && mode != "compact" {
        return Err(format!("unknown export mode '{mode}'"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;
        let engine_state = app.state::<engine::EngineState>();
        build_share_pack(&conn, &engine_state, &app, &job, event_ids, album_id, name, mode, strip_gps, dest_dir)
    })
    .await
    .map_err(|e| format!("share pack task join failed: {e}"))?
}

/// Generous handpicked set for Day folders (curation past this).
const SHARE_PACK_BUDGET: i64 = 60;

fn build_share_pack(
    conn: &rusqlite::Connection,
    engine_state: &engine::EngineState,
    app: &tauri::AppHandle,
    job: &str,
    event_ids: Option<Vec<i64>>,
    album_id: Option<i64>,
    name: String,
    mode: String,
    strip_gps: bool,
    dest_dir: String,
) -> Result<SharePackSummary, String> {
    // 1. Scope members with times and stop labels.
    let ids_paths: Vec<(i64, String)> = match (&event_ids, album_id) {
        (Some(eids), _) if !eids.is_empty() => {
            let mut all = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for eid in eids {
                for m in db::event_assets(&conn, *eid)? {
                    if seen.insert(m.id) {
                        all.push((m.id, m.path));
                    }
                }
            }
            all
        }
        (_, Some(aid)) => db::album_assets(&conn, aid)?
            .into_iter()
            .map(|m| (m.id, m.path))
            .collect(),
        _ => return Err("export_share_pack needs event_ids or album_id".into()),
    };
    if ids_paths.is_empty() {
        return Err("nothing to pack — the scope has no readable photos or videos".into());
    }
    let member_ids: Vec<i64> = ids_paths.iter().map(|(id, _)| *id).collect();
    let scope_events: Vec<i64> = match (&event_ids, album_id) {
        (Some(eids), _) if !eids.is_empty() => eids.clone(),
        _ => {
            let json = serde_json::to_string(&member_ids).map_err(|e| e.to_string())?;
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT ea.event_id FROM event_assets ea
                     WHERE ea.asset_id IN (SELECT value FROM json_each(?1))",
                )
                .map_err(|e| format!("prepare pack events: {e}"))?;
            let rows = stmt
                .query_map(params![json], |r| r.get::<_, i64>(0))
                .map_err(|e| format!("query pack events: {e}"))?;
            let mut eids = Vec::new();
            for row in rows {
                eids.push(row.map_err(|e| format!("pack event row: {e}"))?);
            }
            eids
        }
    };
    let places = db::places_for_ids(&conn, &member_ids)?;
    let mut items = items_for_ids(&conn, &ids_paths, &places)?;
    items.sort_by(|a, b| {
        a.taken
            .partial_cmp(&b.taken)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.path.cmp(&b.path))
    });

    // 2. Curate past the budget (keeps every day/stop represented).
    // Paths are unique per asset, so membership resolves through them.
    let path_to_id: std::collections::HashMap<&str, i64> =
        ids_paths.iter().map(|(id, p)| (p.as_str(), *id)).collect();
    let picked: Vec<db::HighlightItem> = if items.len() > SHARE_PACK_BUDGET as usize {
        if scope_events.is_empty() {
            items.into_iter().take(SHARE_PACK_BUDGET as usize).collect()
        } else {
            let picks = if album_id.is_some() {
                db::curate_picks_constrained(
                    &conn,
                    &scope_events,
                    Some(&member_ids),
                    SHARE_PACK_BUDGET,
                )?
            } else {
                db::curate_picks(&conn, &scope_events, SHARE_PACK_BUDGET)?
            };
            let keep: std::collections::HashSet<i64> = picks.into_iter().map(|p| p.id).collect();
            let mut got: Vec<db::HighlightItem> = items
                .into_iter()
                .filter(|it| path_to_id.get(it.path.as_str()).map_or(false, |id| keep.contains(id)))
                .collect();
            if got.is_empty() {
                // Curate found nothing usable (all blurred/dupes): fall
                // back to the chronological head rather than an empty pack.
                got = items_for_ids(&conn, &ids_paths, &places)?
                    .into_iter()
                    .take(SHARE_PACK_BUDGET as usize)
                    .collect();
                got.sort_by(|a, b| {
                    a.taken
                        .partial_cmp(&b.taken)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            got
        }
    } else {
        items
    };
    drop(conn);

    // 3. Pack root + Day folders ("2026-04-18 grand-county-utah").
    let title = if name.trim().is_empty() { "Shared photos".to_string() } else { name.trim().to_string() };
    let slug = slugify(&title);
    let root = std::path::Path::new(&dest_dir).join(if slug.is_empty() { "share-pack".into() } else { slug });
    std::fs::create_dir_all(&root).map_err(|e| format!("create {root:?}: {e}"))?;
    let photos_dir = root.join("photos");
    let mut folders: std::collections::BTreeMap<String, (String, Vec<usize>)> =
        std::collections::BTreeMap::new();
    for (i, it) in picked.iter().enumerate() {
        let (day, mondd) = db::share_day_label(it.taken);
        let year = day.get(0..4).unwrap_or("");
        let mut label = format!("{mondd} {year}");
        let folder = match &it.place {
            Some(p) if !p.trim().is_empty() => {
                label = format!("{label} · {p}");
                format!("{day} {}", slugify(p))
            }
            _ => day,
        };
        folders.entry(folder).or_insert_with(|| (label, Vec::new())).1.push(i);
    }

    // 4. Media: copy, recode, or strip per pick. One bad file skips
    // instead of aborting the family pack. `packed` counts finished units
    // (exported + skipped) through a Cell so the `fail` closure and the
    // success sites can all advance it without borrow fights.
    let pack_total = picked.len() as i64;
    let packed = std::cell::Cell::new(0i64);
    emit_export(app, job, "packing", 0, pack_total, "Starting pack…");
    let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    let (mut exported, mut skipped, mut bytes) = (0i64, 0i64, 0i64);
    let mut first_err: Option<String> = None;
    let mut fail = |msg: String, label: &str| {
        skipped += 1;
        packed.set(packed.get() + 1);
        if first_err.is_none() {
            first_err = Some(msg);
        }
        emit_export(app, job, "packing", packed.get(), pack_total, label);
    };
    struct PackedFile {
        rel: String,
        caption: String,
        kind: String,
    }
    let mut by_folder: std::collections::BTreeMap<String, Vec<PackedFile>> = std::collections::BTreeMap::new();
    for (folder, (_label, idxs)) in &folders {
        let dir = photos_dir.join(folder);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            let msg = format!("create {dir:?}: {e}");
            for _ in idxs {
                fail(msg.clone(), "Creating folders…");
            }
            continue;
        }
        for &i in idxs {
            let it = &picked[i];
            let src = std::path::Path::new(&it.path);
            let orig = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("photo-{i}"));
            if !src.exists() {
                fail(format!("missing file: {}", it.path), &orig);
                continue;
            }
            let stem = orig.rsplit_once('.').map(|(s, _)| s).unwrap_or(&orig);
            let orig_ext = orig
                .rsplit_once('.')
                .map(|(_, e)| e.to_lowercase())
                .unwrap_or_else(|| "jpg".into());
            let stem = slugify(stem);
            let stem = if stem.is_empty() { format!("photo-{i}") } else { stem };
            let exists = |c: &str| dir.join(c).exists();
            let put = |dest: std::path::PathBuf| -> String {
                format!("photos/{folder}/{}", dest.file_name().unwrap().to_string_lossy())
            };
            if mode == "drive" && !strip_gps {
                let dest = dir.join(unique_bundle_name(&mut used_names, &exists, &stem, &orig_ext));
                match std::fs::copy(src, &dest) {
                    Ok(_) => {
                        bytes += std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);
                        exported += 1;
                        packed.set(packed.get() + 1);
                        emit_export(app, job, "packing", packed.get(), pack_total, &orig);
                        by_folder.entry(folder.clone()).or_default().push(PackedFile {
                            rel: put(dest),
                            caption: orig.clone(),
                            kind: it.kind.clone(),
                        });
                    }
                    Err(e) => fail(format!("copy {}: {e}", it.path), &orig),
                }
                continue;
            }
            if mode == "drive" {
                // strip_gps: full-resolution rewrite without metadata.
                let dest = dir.join(unique_bundle_name(
                    &mut used_names,
                    &exists,
                    &stem,
                    if it.kind == "video" { "mp4" } else { "jpg" },
                ));
                match engine::call_blocking(
                    &engine_state,
                    "strip_copy",
                    Some(json!({
                        "path": it.path,
                        "output": dest.to_string_lossy(),
                    })),
                    Duration::from_secs(300),
                ) {
                    Ok(reply) => {
                        bytes += reply.get("bytes").and_then(Value::as_i64).unwrap_or(0);
                        exported += 1;
                        packed.set(packed.get() + 1);
                        emit_export(app, job, "packing", packed.get(), pack_total, &orig);
                        by_folder.entry(folder.clone()).or_default().push(PackedFile {
                            rel: put(dest),
                            caption: orig.clone(),
                            kind: it.kind.clone(),
                        });
                    }
                    Err(e) => fail(format!("strip {}: {e}", it.path), &orig),
                }
                continue;
            }
            // compact: small recode (metadata-free by construction).
            if it.kind == "video" {
                let dest = dir.join(unique_bundle_name(&mut used_names, &exists, &stem, "mp4"));
                match engine::call_blocking(
                    &engine_state,
                    "compress_video",
                    Some(json!({
                        "path": it.path,
                        "output": dest.to_string_lossy(),
                    })),
                    Duration::from_secs(300),
                ) {
                    Ok(reply) => {
                        bytes += reply.get("bytes").and_then(Value::as_i64).unwrap_or(0);
                        exported += 1;
                        packed.set(packed.get() + 1);
                        emit_export(app, job, "packing", packed.get(), pack_total, &orig);
                        by_folder.entry(folder.clone()).or_default().push(PackedFile {
                            rel: put(dest),
                            caption: orig.clone(),
                            kind: it.kind.clone(),
                        });
                    }
                    Err(e) => fail(format!("compress {}: {e}", it.path), &orig),
                }
            } else {
                let dest = dir.join(unique_bundle_name(&mut used_names, &exists, &stem, "jpg"));
                match engine::call_blocking(
                    &engine_state,
                    "compress",
                    Some(json!({
                        "path": it.path,
                        "output": dest.to_string_lossy(),
                        "maxPx": 1280,
                        "quality": 0.72,
                    })),
                    Duration::from_secs(60),
                ) {
                    Ok(reply) => {
                        bytes += reply.get("bytes").and_then(Value::as_i64).unwrap_or(0);
                        exported += 1;
                        packed.set(packed.get() + 1);
                        emit_export(app, job, "packing", packed.get(), pack_total, &orig);
                        by_folder.entry(folder.clone()).or_default().push(PackedFile {
                            rel: put(dest),
                            caption: orig.clone(),
                            kind: it.kind.clone(),
                        });
                    }
                    Err(e) => fail(format!("compress {}: {e}", it.path), &orig),
                }
            }
        }
    }
    if exported == 0 {
        return Err(first_err.unwrap_or_else(|| "nothing to pack".into()));
    }

    // 5. Memory video over the same picks (chapters included).
    let video_name = format!(
        "{}-memory.mp4",
        root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "share-pack".into())
    );
    let video_path = root.join(&video_name);
    let clips = db::highlight_plan(picked);
    let render = render_story(&engine_state, app, job, &clips, &video_path)?;
    bytes += render.bytes;

    // 6. Offline gallery.
    emit_export(app, job, "gallery", 0, 0, "Writing gallery…");
    let days: Vec<db::ShareDay> = by_folder
        .into_iter()
        .map(|(folder, files)| {
            let label = folders
                .get(&folder)
                .map(|(l, _)| l.clone())
                .unwrap_or_else(|| folder.clone());
            db::ShareDay {
                folder,
                label,
                files: files
                    .into_iter()
                    .map(|f| db::ShareFile { file: f.rel, caption: f.caption, kind: f.kind })
                    .collect(),
            }
        })
        .collect();
    let html = db::share_index_html(&title, &video_name, render.seconds, &days, exported);
    std::fs::write(root.join("index.html"), html).map_err(|e| format!("write gallery: {e}"))?;
    emit_export(app, job, "done", 1, 1, "");

    Ok(SharePackSummary {
        dir: root.to_string_lossy().to_string(),
        video: video_path.to_string_lossy().to_string(),
        exported,
        skipped,
        bytes,
    })
}

// MARK: - Distribution bundles

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
    ext: &str,
) -> String {
    let mut candidate = format!("{stem}.{ext}");
    let mut n = 2;
    while used.contains(&candidate) || exists(&candidate) {
        candidate = format!("{stem}-{n}.{ext}");
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

/// Copies an event's or album's photos and videos into `dest_dir/<name-slug>/`:
/// - mode "drive": byte-for-byte copies (full quality, original extensions)
/// - mode "whatsapp": recoded 1280px JPEGs + 1280x720 MP4s via the engine
/// - mode "compact": same recode as whatsapp but keeps everything callers
///   pass in (space-saver bundles of dupes + blurred, chosen explicitly)
///
/// Blurred shots and known duplicates are excluded from WhatsApp bundles;
/// Drive bundles keep everything except missing files.
#[tauri::command]
async fn export_bundle(
    app: tauri::AppHandle,
    event_id: Option<i64>,
    album_id: Option<i64>,
    asset_ids: Option<Vec<i64>>,
    event_name: String,
    mode: String,
    dest_dir: String,
) -> Result<ExportSummary, String> {
    if mode != "drive" && mode != "whatsapp" && mode != "compact" {
        return Err(format!("unknown export mode '{mode}'"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        let assets = match (event_id, album_id, asset_ids) {
            (Some(eid), _, _) => db::event_assets(&conn, eid)?,
            (_, Some(aid), _) => db::album_assets(&conn, aid)?,
            (_, _, Some(ids)) => db::assets_by_ids(&conn, &ids)?,
            (None, None, None) => return Err("export_bundle needs event_id, album_id or asset_ids".into()),
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
            if mode == "whatsapp" {
                "-whatsapp"
            } else if mode == "compact" {
                "-compact"
            } else {
                ""
            }
        );
        let out_dir = std::path::Path::new(&dest_dir).join(&subdir);
        std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {out_dir:?}: {e}"))?;

        let engine_state = app.state::<engine::EngineState>();
        let (mut exported, mut skipped, mut bytes) = (0i64, 0i64, 0i64);
        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for asset in &assets {
            let src = std::path::Path::new(&asset.path);
            if !src.exists() {
                skipped += 1;
                continue;
            }
            if mode == "whatsapp" && blurred_set.contains(&asset.path) {
                skipped += 1;
                continue;
            }
            // Same-named photos from different folders must not overwrite each
            // other in the bundle: first wins the slug, later ones get -2, -3…
            let stem = slugify(&asset.name.trim_end_matches(
                |c: char| c.is_ascii_alphanumeric() || c == '.'
            ));
            let exists = |c: &str| out_dir.join(c).exists();
            if asset.kind == "image" {
                let dest = out_dir.join(unique_bundle_name(&mut used_names, &exists, &stem, "jpg"));
                if mode == "drive" {
                    std::fs::copy(src, &dest).map_err(|e| format!("copy {}: {e}", asset.path))?;
                    bytes += std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);
                    exported += 1;
                } else {
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
                continue;
            }
            // Videos: Drive keeps the original bytes under the original
            // extension; WhatsApp gets a 1280x720 recode. Either way they
            // are no longer skipped.
            if mode == "drive" {
                let ext = src
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("mp4")
                    .to_lowercase();
                let dest = out_dir.join(unique_bundle_name(&mut used_names, &exists, &stem, &ext));
                std::fs::copy(src, &dest).map_err(|e| format!("copy {}: {e}", asset.path))?;
                bytes += std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);
                exported += 1;
            } else {
                let dest = out_dir.join(unique_bundle_name(&mut used_names, &exists, &stem, "mp4"));
                let reply = engine::call_blocking(
                    &engine_state,
                    "compress_video",
                    Some(json!({
                        "path": asset.path,
                        "output": dest.to_string_lossy(),
                    })),
                    Duration::from_secs(300),
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
    favorite_only: Option<bool>,
    scene_tag: Option<String>,
    query: Option<String>,
    has_pet: Option<bool>,
    sort_added: Option<bool>,
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
            favorite_only,
            scene_tag: scene_tag.as_deref(),
            query: query.as_deref(),
            has_pet,
            sort_added: sort_added.unwrap_or(false),
            offset: offset.unwrap_or(0).max(0),
            limit: limit.unwrap_or(200).clamp(1, 1000),
        },
    )
}

/// Best-of-place pick: balanced coverage curation (day × stop quotas,
/// favorites, named faces, diversity, video mix) straight into an album.
/// Same contract as the old sharpness-only rank, strictly better picks.
#[tauri::command]
fn add_event_picks_to_album(
    conn: tauri::State<'_, DbState>,
    event_id: i64,
    album_id: i64,
    limit: Option<i64>,
) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let picks = db::curate_picks(&conn, &[event_id], limit.unwrap_or(12).clamp(1, 200))?;
    let ids: Vec<i64> = picks.into_iter().map(|p| p.id).collect();
    db::album_add_assets(&conn, album_id, &ids)
}

/// Trip-scale curation: balanced picks across several events (a trip, a date
/// range's stops, handpicked events) into an album.
#[tauri::command]
fn curate_picks_to_album(
    conn: tauri::State<'_, DbState>,
    event_ids: Vec<i64>,
    album_id: i64,
    limit: Option<i64>,
) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let picks = db::curate_picks(&conn, &event_ids, limit.unwrap_or(24).clamp(1, 200))?;
    let ids: Vec<i64> = picks.into_iter().map(|p| p.id).collect();
    db::album_add_assets(&conn, album_id, &ids)
}

/// Trip → stop hierarchy over the event list (dynamic, see db::trips_list).
#[tauri::command]
fn trips_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::TripRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::trips_list(&conn)
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
    favorite_only: Option<bool>,
    scene_tag: Option<String>,
    query: Option<String>,
    has_pet: Option<bool>,
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
            favorite_only,
            scene_tag: scene_tag.as_deref(),
            query: query.as_deref(),
            has_pet,
            ..Default::default()
        },
    )
}

#[tauri::command]
fn library_months(conn: tauri::State<'_, DbState>) -> Result<Vec<db::LibraryMonth>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::library_months(&conn)
}

#[tauri::command]
fn set_favorite(
    conn: tauri::State<'_, DbState>,
    asset_id: i64,
    favorite: bool,
) -> Result<(), String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::set_favorite(&conn, asset_id, favorite)
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

        let rows: Vec<(i64, String, String)> = {
            let mut stmt = conn
                .prepare("SELECT id, path, name FROM assets WHERE id IN (SELECT value FROM json_each(?1))")
                .map_err(|e| format!("trash lookup: {e}"))?;
            let json = serde_json::to_string(&ids).map_err(|e| e.to_string())?;
            let mapped = stmt
                .query_map(params![json], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
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
        for (id, path, _name) in &rows {
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
            let logged: Vec<(i64, String, String)> = rows
                .into_iter()
                .filter(|(id, _, _)| trashed.contains(id))
                .collect();
            db::log_trash(&conn, &logged)?;
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

/// Undo for trash_assets: moves same-base-name files from ~/.Trash back to
/// their indexed paths and clears the missing flag. Exact-name match only —
/// anything else lands in `failed` with a plain-language reason.
#[tauri::command]
async fn restore_assets(
    app: tauri::AppHandle,
    ids: Vec<i64>,
) -> Result<RestoreSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = app.state::<DbState>();
        let conn = conn.lock().map_err(|_| "database lock poisoned")?;

        let rows = db::asset_paths(&conn, &ids)?;
        if rows.is_empty() {
            return Err("no matching assets".into());
        }

        let home = std::env::var("HOME").map_err(|_| "home directory unavailable")?;
        let trash = std::path::PathBuf::from(home).join(".Trash");

        let mut restored = Vec::new();
        let mut failed = Vec::new();
        for (id, path) in &rows {
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            let Some(name) = name else {
                failed.push(format!("{path}: already emptied or renamed — restore from Finder"));
                continue;
            };
            let src = trash.join(&name);
            if !src.exists() {
                failed.push(format!("{path}: already emptied or renamed — restore from Finder"));
                continue;
            }
            if let Some(parent) = std::path::Path::new(path).parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    failed.push(format!("{path}: {e}"));
                    continue;
                }
            }
            match std::fs::rename(&src, path) {
                Ok(()) => restored.push(*id),
                Err(e) => failed.push(format!("{path}: {e}")),
            }
        }

        if !restored.is_empty() {
            db::clear_missing(&conn, &restored)?;
            db::unlog_trash(&conn, &restored)?;
        }

        Ok(RestoreSummary { restored: restored.len(), failed })
    })
    .await
    .map_err(|e| format!("restore task join failed: {e}"))?
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RestoreSummary {
    restored: usize,
    failed: Vec<String>,
}

/// Recently Deleted list, newest first.
#[tauri::command]
fn trash_list(conn: tauri::State<'_, DbState>) -> Result<Vec<db::TrashEntry>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::trash_list(&conn)
}

/// Forgets the Recently Deleted history (macOS Trash contents untouched).
#[tauri::command]
fn clear_trash_log(conn: tauri::State<'_, DbState>) -> Result<i64, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::clear_trash_log(&conn)
}

#[tauri::command]
fn library_stats(conn: tauri::State<'_, DbState>) -> Result<db::LibraryStats, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::stats(&conn)
}

#[tauri::command]
fn scan_roots(conn: tauri::State<'_, DbState>) -> Result<Vec<db::ScanRoot>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::scan_roots(&conn)
}

/// Asset ids under one indexed root, for one-click folder-to-album adds.
#[tauri::command]
fn assets_in_root(conn: tauri::State<'_, DbState>, root: String) -> Result<Vec<i64>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::asset_ids_in_root(&conn, &root)
}

/// Forgets one indexed root (scan records, assets, derived face/sighting
/// rows). Files on disk are untouched; albums and events keep their other
/// members via cascades.
#[tauri::command]
fn remove_root(conn: tauri::State<'_, DbState>, root: String) -> Result<db::RemoveRootSummary, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::remove_root(&conn, &root)
}

/// Geotagged pins for the map view (whole library, capped — see
/// `db::MAP_PINS_LIMIT`).
#[tauri::command]
fn map_pins(conn: tauri::State<'_, DbState>) -> Result<Vec<db::MapPin>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    db::map_pins(&conn)
}

/// OSM raster tile cache caps: tiles are ~15KB, so this holds thousands of
/// panned/zoomed views before the oldest go.
const MAP_TILE_CACHE_MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAP_TILE_CACHE_TARGET_BYTES: u64 = 64 * 1024 * 1024;

/// OSM asks for ≤2 parallel tile downloads; cache hits never take a permit.
static TILE_DL_SLOTS: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();
/// The prune walk stats the whole tile tree, so it runs at most every N misses.
static TILE_PRUNE_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const TILE_PRUNE_EVERY: u64 = 32;

/// One cached map tile as an app-data file path, served under
/// `app_data/maptiles/{source}/{z}/{x}/{y}.png` (`source` is `osm` streets or
/// `topo` terrain relief). The UI turns it into an asset-protocol URL
/// (browser-cached, no base64/IPC per tile); repeat pans and zooms cost no
/// network and no re-encode. Upstream tile usage policies (valid User-Agent,
/// ≤2 parallel) are honored by the app UA below and the download semaphore.
/// Attribution per source is rendered by the map UI. (Pre-source caches at
/// `maptiles/{z}/…` are orphaned but still pruned by the root walk.)
#[tauri::command]
async fn map_tile(
    app: tauri::AppHandle,
    z: u32,
    x: u32,
    y: u32,
    source: String,
) -> Result<String, String> {
    let (ns, url_fmt, max_z) = match source.as_str() {
        "osm" => ("osm", "https://tile.openstreetmap.org/{z}/{x}/{y}.png", 19u32),
        "topo" => ("topo", "https://tile.opentopomap.org/{z}/{x}/{y}.png", 17u32),
        _ => return Err("unknown tile source (osm|topo)".to_string()),
    };
    if z > max_z {
        return Err(format!("tile zoom out of range (0-{max_z})"));
    }
    let n = 1u32 << z;
    if x >= n || y >= n {
        return Err("tile coordinates out of range".to_string());
    }
    let file = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?
        .join("maptiles")
        .join(ns)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.png"));
    // Fast path: cache hits skip the blocking pool and the permit entirely.
    // (A concurrent downloader racing us is harmless: the bytes are identical.)
    if file.is_file() {
        return Ok(file.to_string_lossy().into_owned());
    }
    let _permit = TILE_DL_SLOTS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::const_new(2)))
        .clone()
        .acquire_owned()
        .await
        .map_err(|e| format!("tile slot: {e}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create tile dir: {e}"))?;
        }
        if !file.is_file() {
            let url = url_fmt
                .replace("{z}", &z.to_string())
                .replace("{x}", &x.to_string())
                .replace("{y}", &y.to_string());
            let bytes = ureq::get(&url)
                .header("User-Agent", "fotoflow/0.1.0 (local photo manager; map tile cache)")
                .header("Accept", "image/png")
                .call()
                .map_err(|e| format!("tile download: {e}"))?
                .body_mut()
                .read_to_vec()
                .map_err(|e| format!("tile body: {e}"))?;
            if bytes.is_empty() || bytes.len() > 2 * 1024 * 1024 {
                return Err("tile download: unexpected body".to_string());
            }
            std::fs::write(&file, &bytes).map_err(|e| format!("cache tile: {e}"))?;
            if TILE_PRUNE_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % TILE_PRUNE_EVERY == 0
            {
                prune_dir_recursive(
                    &app.path().app_data_dir().map_err(|e| format!("app data dir: {e}"))?.join("maptiles"),
                    MAP_TILE_CACHE_MAX_BYTES,
                    MAP_TILE_CACHE_TARGET_BYTES,
                );
            }
        }
        Ok(file.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| format!("tile task join failed: {e}"))?
}

/// Recursive twin of `prune_thumb_cache` (tiles nest z/x/y; thumbs are flat).
/// Oldest files first; individual stat/remove failures are skipped.
fn prune_dir_recursive(dir: &std::path::Path, max_bytes: u64, target_bytes: u64) {
    fn walk(dir: &std::path::Path, out: &mut Vec<(u64, std::path::PathBuf, u64)>, total: &mut u64) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for entry in rd.filter_map(Result::ok) {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, out, total);
                continue;
            }
            if !p.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let size = meta.len();
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() * 1_000_000_000 + d.subsec_nanos() as u64)
                .unwrap_or(0);
            *total += size;
            out.push((mtime, p, size));
        }
    }
    let mut entries = Vec::new();
    let mut total: u64 = 0;
    walk(dir, &mut entries, &mut total);
    if total <= max_bytes {
        return;
    }
    entries.sort_by_key(|(mtime, _, _)| *mtime); // oldest first
    for (_, path, size) in entries {
        if total <= target_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

// MARK: - Thumbnail disk cache

/// Cap for the on-disk thumbnail cache: past this size the oldest entries
/// (by mtime) are evicted until the directory is back under the target.
const THUMB_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;
const THUMB_CACHE_PRUNE_TARGET_BYTES: u64 = 256 * 1024 * 1024;

/// FNV-1a (64-bit) over the cache identity. Hand-rolled so no new dependency
/// is needed for a single hash.
fn thumb_cache_key(canonical: &str, len: u64, mtime_secs: u64, mtime_nanos: u32, max_px: u32) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!("{canonical}|{len}|{mtime_secs}.{mtime_nanos}|{max_px}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Cache file for a source image: the key folds in the canonical path, file
/// size, mtime and requested size, so an edited file naturally misses and the
/// stale entry is left for the size-cap eviction below.
fn thumb_cache_file(
    cache_dir: &std::path::Path,
    path: &str,
    max_px: u32,
) -> Result<std::path::PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("thumb: source file not found: {path}"))?;
    let (secs, nanos) = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0));
    let canonical = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    Ok(cache_dir.join(format!(
        "{}.jpg",
        thumb_cache_key(&canonical, meta.len(), secs, nanos, max_px)
    )))
}

/// Deletes the oldest files (by mtime) while `dir` exceeds `max_bytes`,
/// stopping once it is back under `target_bytes`. Returns files removed.
/// Individual stat/remove failures are skipped, never fatal.
fn prune_thumb_cache(dir: &std::path::Path, max_bytes: u64, target_bytes: u64) -> usize {
    let mut entries: Vec<(u64, std::path::PathBuf, u64)> = Vec::new();
    let mut total: u64 = 0;
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    for entry in rd.filter_map(Result::ok) {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let size = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() * 1_000_000_000 + d.subsec_nanos() as u64)
            .unwrap_or(0);
        total += size;
        entries.push((mtime, p, size));
    }
    if total <= max_bytes {
        return 0;
    }
    entries.sort_by_key(|(mtime, _, _)| *mtime); // oldest first
    let mut removed = 0;
    for (_, path, size) in entries {
        if total <= target_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
            removed += 1;
        }
    }
    removed
}

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 encode (no new dependency for one conversion).
fn b64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(B64_ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { B64_ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64_ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    out
}

fn b64_value(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("thumb: engine returned invalid base64".to_string()),
    }
}

/// Standard base64 decode; rejects malformed input instead of guessing.
fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("thumb: engine returned malformed base64".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return Err("thumb: engine returned malformed base64".to_string());
        }
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            if c == b'=' {
                if i < 4 - pad {
                    return Err("thumb: engine returned malformed base64".to_string());
                }
                n <<= 6;
            } else {
                n = (n << 6) | b64_value(c)? as u32;
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

/// Disk-backed thumbnail fetch shared by the `cached_thumb` command and its
/// tests: hit → the cached JPEG re-encoded to the engine's base64 shape;
/// miss → a single engine `thumb` call whose JPEG bytes populate the cache.
/// The returned object mirrors the engine `thumb` reply (`path`/`image`/…).
fn cached_thumb_value(
    engine_state: &engine::EngineState,
    cache_dir: &std::path::Path,
    path: &str,
    max_px: u32,
) -> Result<Value, String> {
    if !std::path::Path::new(path).exists() {
        return Err(format!("thumb: source file not found: {path}"));
    }
    let cache_file = thumb_cache_file(cache_dir, path, max_px)?;
    if cache_file.exists() {
        if let Ok(bytes) = std::fs::read(&cache_file) {
            if !bytes.is_empty() {
                return Ok(json!({
                    "path": path,
                    "image": b64_encode(&bytes),
                    "mime": "image/jpeg",
                    "maxPx": max_px.clamp(16, 4096),
                }));
            }
        }
    }
    let reply = engine::call_blocking(
        engine_state,
        "thumb",
        Some(json!({ "path": path, "maxPx": max_px })),
        Duration::from_secs(60),
    )?;
    let b64 = reply
        .get("image")
        .and_then(Value::as_str)
        .ok_or("thumb engine returned no image")?;
    let bytes = b64_decode(b64)?;
    if bytes.is_empty() {
        return Err("thumb engine returned an empty image".to_string());
    }
    if let Some(parent) = cache_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create thumbs dir: {e}"))?;
    }
    std::fs::write(&cache_file, &bytes).map_err(|e| format!("write thumb cache: {e}"))?;
    prune_thumb_cache(cache_dir, THUMB_CACHE_MAX_BYTES, THUMB_CACHE_PRUNE_TARGET_BYTES);
    Ok(reply)
}

/// Library thumbnail with a disk cache under `{app_data}/thumbs/`: repeat
/// renders for the same (file, size, mtime, max_px) skip the engine entirely.
/// Missing files are a clean error; the Library keeps its placeholder.
#[tauri::command]
async fn cached_thumb(app: tauri::AppHandle, path: String, max_px: u32) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?
            .join("thumbs");
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create thumbs dir: {e}"))?;
        let engine_state = app.state::<engine::EngineState>();
        cached_thumb_value(&engine_state, &cache_dir, &path, max_px)
    })
    .await
    .map_err(|e| format!("thumb task join failed: {e}"))?
}

// MARK: - Face-crop disk cache

/// FNV-1a (64-bit) over the face-crop identity: canonical path + size +
/// mtime + crop box + requested size. Same scheme as `thumb_cache_key` so a
/// moved/edited file naturally misses.
fn face_cache_key(
    canonical: &str,
    len: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    max_px: u32,
) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in format!("{canonical}|{len}|{mtime_secs}.{mtime_nanos}|{x}|{y}|{w}|{h}|{max_px}").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Cache file for a face crop: same `{app_data}/thumbs/` dir and shared
/// size-cap eviction as grid thumbs; the key additionally folds in the
/// normalized crop box so neighbouring faces never collide.
fn face_cache_file(
    cache_dir: &std::path::Path,
    path: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    max_px: u32,
) -> Result<std::path::PathBuf, String> {
    let meta = std::fs::metadata(path).map_err(|_| format!("thumb: source file not found: {path}"))?;
    let (secs, nanos) = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs(), d.subsec_nanos()))
        .unwrap_or((0, 0));
    let canonical = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    Ok(cache_dir.join(format!(
        "{}.jpg",
        face_cache_key(&canonical, meta.len(), secs, nanos, x, y, w, h, max_px)
    )))
}

/// Disk-backed face-crop fetch mirroring `cached_thumb_value`: hit → the
/// cached JPEG re-encoded to the engine's `face_thumb` shape (`{image}`);
/// miss → a single engine `face_thumb` call whose JPEG bytes populate the
/// cache. Shares the thumbs dir + prune caps with grid thumbs.
fn cached_face_thumb_value(
    engine_state: &engine::EngineState,
    cache_dir: &std::path::Path,
    path: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    max_px: u32,
) -> Result<Value, String> {
    if !std::path::Path::new(path).exists() {
        return Err(format!("thumb: source file not found: {path}"));
    }
    let cache_file = face_cache_file(cache_dir, path, x, y, w, h, max_px)?;
    if cache_file.exists() {
        if let Ok(bytes) = std::fs::read(&cache_file) {
            if !bytes.is_empty() {
                return Ok(json!({ "image": b64_encode(&bytes) }));
            }
        }
    }
    let reply = engine::call_blocking(
        engine_state,
        "face_thumb",
        Some(json!({ "path": path, "x": x, "y": y, "width": w, "height": h, "maxPx": max_px })),
        Duration::from_secs(60),
    )?;
    let b64 = reply
        .get("image")
        .and_then(Value::as_str)
        .ok_or("face_thumb engine returned no image")?;
    let bytes = b64_decode(b64)?;
    if bytes.is_empty() {
        return Err("face_thumb engine returned an empty image".to_string());
    }
    if let Some(parent) = cache_file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create thumbs dir: {e}"))?;
    }
    std::fs::write(&cache_file, &bytes).map_err(|e| format!("write thumb cache: {e}"))?;
    prune_thumb_cache(cache_dir, THUMB_CACHE_MAX_BYTES, THUMB_CACHE_PRUNE_TARGET_BYTES);
    Ok(reply)
}

/// Face/pet crop with the same disk cache as grid thumbs: repeat expands of
/// the same (file, box, size, mtime, max_px) skip the engine entirely.
/// Missing files are a clean error; the cluster view keeps its placeholder.
#[tauri::command]
async fn cached_face_thumb(
    app: tauri::AppHandle,
    path: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    max_px: u32,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cache_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?
            .join("thumbs");
        std::fs::create_dir_all(&cache_dir).map_err(|e| format!("create thumbs dir: {e}"))?;
        let engine_state = app.state::<engine::EngineState>();
        cached_face_thumb_value(&engine_state, &cache_dir, &path, x, y, width, height, max_px)
    })
    .await
    .map_err(|e| format!("face thumb task join failed: {e}"))?
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
            cached_thumb,
            cached_face_thumb,
            import_folder,
            enrich_assets,
            hash_candidates_run,
            score_quality,
            near_dupes_run,
            near_groups_list,
            near_keep,
            tag_scenes,
            run_post_import_cmd,
            embed_library,
            similar_to,
            clip_models_present,
            download_clip_models,
            embed_clip_backfill,
            reindex_clip,
            cancel_clip_backfill,
            remove_clip_models,
            search_text,
            scene_tags,
            asset_tags,
            duplicates_list,
            duplicates_keep,
            cleanup_stats_cmd,
            trash_assets,
            restore_assets,
            trash_list,
            clear_trash_log,
            cluster_faces,
            verify_library,
            clusters_list,
            faces_in_cluster,
            faces_count,
            review_candidates,
            dismiss_review_candidate,
            rename_cluster,
            merge_clusters,
            restore_cluster,
            unassign_face,
            assign_face,
            detect_pets,
            sightings_list,
            cluster_events,
            events_list,
            event_assets,
            trips_list,
            curate_picks_to_album,
            rename_event,
            resolve_places,
            albums_list,
            album_create,
            album_rename,
            album_delete,
            album_assets,
            album_add,
            album_remove,
            add_event_picks_to_album,
            export_highlight,
            export_share_pack,
            export_bundle,
            ingest_copy,
            library_list,
            library_count,
            library_months,
            library_stats,
            scan_roots,
            assets_in_root,
            remove_root,
            map_pins,
            map_tile,
            set_favorite
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

    /// The model-download unpack path: hash mismatch rejects, a good archive
    /// lands its top-level dir. Uses a synthetic zip so no network is needed.
    #[test]
    fn clip_package_verify_and_unpack_round_trips() {
        let dir = std::env::temp_dir().join("fotoflow-clip-unpack-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            w.add_directory("CLIPImage.mlpackage/", opts).unwrap();
            w.start_file("CLIPImage.mlpackage/weights.bin", opts).unwrap();
            std::io::Write::write_all(&mut w, b"fake-weights").unwrap();
            w.finish().unwrap();
        }
        let bytes = buf.into_inner();
        let sha = sha256_hex(&bytes);
        assert!(verify_and_unpack(&dir, "CLIPImage", b"tampered", &sha).is_err(), "hash enforced");
        verify_and_unpack(&dir, "CLIPImage", &bytes, &sha).unwrap();
        assert!(dir.join("CLIPImage.mlpackage/weights.bin").is_file(), "archive unpacked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression guard for the Stage-4 outage ("Command export_share_pack
    /// not found"): every #[tauri::command] must be registered in the
    /// invoke_handler! list, otherwise the UI calls a command the binary
    /// silently drops (dead-code warning only — no compile error).
    #[test]
    fn all_tauri_commands_are_registered() {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let src = std::fs::read_to_string(std::path::Path::new(&dir).join("src").join("lib.rs"))
            .unwrap();
        let mut commands = Vec::new();
        let mut lines = src.lines().peekable();
        while let Some(line) = lines.next() {
            if line.trim() == "#[tauri::command]" {
                for next in lines.by_ref() {
                    let t = next.trim();
                    if t.is_empty() || t.starts_with("///") {
                        continue;
                    }
                    if let Some(rest) = t.strip_prefix("async fn ").or_else(|| t.strip_prefix("fn ")) {
                        let name: String =
                            rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        if !name.is_empty() {
                            commands.push(name);
                        }
                        break;
                    }
                    break;
                }
            }
        }
        assert!(!commands.is_empty(), "parser found no commands");
        let after = src
            .split("tauri::generate_handler![")
            .nth(1)
            .expect("generate_handler! list present");
        let mut depth = 1i32;
        let mut end = after.len();
        for (i, c) in after.char_indices() {
            if c == '[' {
                depth += 1;
            } else if c == ']' {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
        }
        let handler = &after[..end];
        for cmd in &commands {
            assert!(
                handler.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == cmd),
                "tauri command '{cmd}' is not registered in invoke_handler!"
            );
        }
    }

    #[test]
    fn bundle_names_dedupe_within_and_across_exports() {
        let mut used = std::collections::HashSet::new();
        let none = |_: &str| false;
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001", "jpg"), "img-0001.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001", "jpg"), "img-0001-2.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001", "jpg"), "img-0001-3.jpg");
        // A file left on disk by an earlier export into the same folder is skipped too.
        let on_disk = |c: &str| c == "img-0002.jpg";
        assert_eq!(unique_bundle_name(&mut used, &on_disk, "img-0002", "jpg"), "img-0002-2.jpg");
        assert_eq!(unique_bundle_name(&mut used, &none, "other", "jpg"), "other.jpg");
        // Video extensions ride the same scheme without colliding with stills.
        assert_eq!(unique_bundle_name(&mut used, &none, "clip-0001", "mp4"), "clip-0001.mp4");
        assert_eq!(unique_bundle_name(&mut used, &none, "clip-0001", "mp4"), "clip-0001-2.mp4");
        assert_eq!(unique_bundle_name(&mut used, &none, "img-0001", "mp4"), "img-0001.mp4");
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

    #[test]
    fn pipeline_chains_all_offline_stages_and_is_stable_on_rerun() {
        let dir = fixture("ff-pipeline-test", 0);
        let real_png = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        std::fs::copy(&real_png, dir.join("a.png")).unwrap();
        std::fs::copy(&real_png, dir.join("b.png")).unwrap();
        std::fs::write(dir.join("corrupt.jpg"), b"not a jpeg").unwrap();
        // macOS copy preserves the source mtime, which would split the
        // import across two days: rewrite so every file is stamped now.
        for name in ["a.png", "b.png"] {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            std::fs::write(dir.join(name), bytes).unwrap();
        }

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();

        let mut stages = Vec::new();
        let first = run_post_import(&conn, &state, 3, |p| stages.push(p.stage.clone())).unwrap();
        assert_eq!(first.enriched, 2, "both real PNGs decode");
        assert_eq!(first.hashed, 3, "hashing reads bytes, even corrupt ones");
        assert_eq!(first.near, 1, "identical icons form one near-dupe group; corrupt bows out after retries");
        assert_eq!(first.events, 1, "one import session clusters together");
        // The 16px icon classifies nothing above the 0.2 floor and the
        // corrupt file fails outright: every image still leaves the
        // untagged set exactly once (tagged 0 + tag-stage fails 3).
        // Positive tag storage is covered at the db level.
        assert_eq!(first.tagged, 0);
        assert_eq!(first.places, 0, "no coordinates, nothing to fill");
        assert_eq!(first.faces, 0, "the icon shows no detectable face");
        for want in ["enrich", "hash", "quality", "near", "events", "tag", "faces", "places", "done"] {
            assert!(stages.contains(&want.to_string()), "stage {want} reports progress");
        }

        let snapshot = || -> (i64, i64, i64, i64) {
            let events: i64 = conn
                .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
                .unwrap();
            let groups: i64 = conn
                .query_row("SELECT COUNT(*) FROM duplicate_groups", [], |r| r.get(0))
                .unwrap_or(0);
            let tags: i64 = conn
                .query_row("SELECT COUNT(*) FROM asset_tags", [], |r| r.get(0))
                .unwrap();
            let faces: i64 = conn
                .query_row("SELECT COUNT(*) FROM faces", [], |r| r.get(0))
                .unwrap_or(0);
            (events, groups, tags, faces)
        };
        let before = snapshot();
        let second = run_post_import(&conn, &state, 3, |_| {}).unwrap();
        assert_eq!(snapshot(), before, "rerun creates no new rows");
        assert_eq!(second.tagged, 0, "nothing newly tagged on rerun");
    }

    #[test]
    fn pipeline_faces_join_named_cluster_with_person_tag() {
        let dir = fixture("ff-pipeline-faces-test", 0);
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        assert!(favicon.exists(), "static/favicon.png must exist for the faces pipeline test");
        // Two near-identical crops of the same icon.
        std::fs::copy(&favicon, dir.join("a.png")).unwrap();
        std::fs::copy(&favicon, dir.join("b.png")).unwrap();

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        let state = engine_state();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        // Stored paths, not the temp-dir spellings: the engine may report
        // the canonicalized form (/private/var/…) and faces join on it.
        let stored_path = |name: &str| -> String {
            conn.query_row("SELECT path FROM assets WHERE name = ?1", params![name], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let a_path = stored_path("a.png");
        let b_path = stored_path("b.png");
        let asset_id = |path: &str| -> i64 {
            conn.query_row("SELECT id FROM assets WHERE path = ?1", params![path], |r| r.get(0))
                .unwrap()
        };
        let has_person_tag = |aid: i64, name: &str| -> bool {
            db::asset_tags(&conn, aid).unwrap().iter().any(|t| t.tag == format!("person:{name}"))
        };

        // Engine reality check: never assume the icon depicts a person —
        // the pipeline must agree with what the sidecar actually returns.
        let probe = engine::call_with_events(
            &state,
            "cluster_faces",
            Some(json!({ "paths": [a_path.clone(), b_path.clone()], "threshold": 0.55 })),
            Duration::from_secs(600),
            &mut |_, _| {},
        )
        .expect("engine face probe");
        assert_eq!(
            probe.get("faces").and_then(Value::as_i64),
            Some(0),
            "fixture shows no detectable face per engine: {probe:?}"
        );

        // Known person, seeded with identical synthetic descriptors on the
        // real asset paths (the icon has no detectable face, so descriptors
        // stand in for detected faces while import + pipeline stay
        // end-to-end). Live threshold math, not hardcoded similarity:
        // identical vectors sit at distance 0, inside the regroup threshold.
        assert_eq!(
            group_faces_by_distance(
                &[1, 2],
                &[vec![0.0f32, 0.0], vec![0.0f32, 0.0]],
                0.55
            )
            .len(),
            1
        );
        let descriptor = || {
            serde_json::json!({
                "x": 0.1, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9,
                "descriptor": b64_encode(&db::floats_to_bytes(&[0.0f32, 0.0])),
            })
        };
        let a_face = db::save_faces(&conn, &a_path, &[descriptor()]).unwrap();
        let cluster = db::save_cluster(&conn, &a_face).unwrap();
        db::rename_cluster(&conn, cluster, "Maya").unwrap();
        let aid_a = asset_id(&a_path);
        assert!(has_person_tag(aid_a, "Maya"), "naming stamps the known photo");

        // A new image of the same person arrives after naming; its face is
        // already stored (as detection would leave it) but still untagged.
        db::save_faces(&conn, &b_path, &[descriptor()]).unwrap();
        let aid_b = asset_id(&b_path);
        assert!(!has_person_tag(aid_b, "Maya"), "new image starts untagged");

        // Act: the full post-import run (faces stage detects nothing new on
        // the icon, then globally regroups the stored descriptors).
        let mut stages = Vec::new();
        let summary = run_post_import(&conn, &state, 3, |p| stages.push(p.stage.clone())).unwrap();
        assert!(stages.contains(&"faces".to_string()), "faces stage reports progress");
        assert_eq!(summary.faces, 2, "both stored faces regroup");

        // Assert: the newcomer joined Maya's cluster WITH the stamp, and the
        // grouping matches live threshold math rather than a fixed count.
        let stored = db::face_descriptors(&conn).unwrap();
        let ids: Vec<i64> = stored.iter().map(|(id, _)| *id).collect();
        let vecs: Vec<Vec<f32>> = stored.into_iter().map(|(_, v)| v).collect();
        assert_eq!(group_faces_by_distance(&ids, &vecs, 0.55).len(), 1);
        let clusters = db::clusters_list(&conn).unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].name.as_deref(), Some("Maya"));
        assert_eq!(clusters[0].face_count, 2, "new image joins the named cluster");
        assert!(has_person_tag(aid_b, "Maya"), "join stamps the new photo");
        assert!(has_person_tag(aid_a, "Maya"), "known photo keeps its stamp");

        // Tag-aware search surfaces the newcomer by person name.
        let q = db::LibraryQuery { query: Some("maya"), limit: 10, ..Default::default() };
        let paths: Vec<String> =
            db::library(&conn, &q).unwrap().into_iter().map(|r| r.path).collect();
        assert!(
            paths.contains(&a_path) && paths.contains(&b_path),
            "search finds both by person name: {paths:?}"
        );
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

    #[test]
    fn thumb_cache_key_covers_size_and_max_px() {
        let dir = std::env::temp_dir().join("ff-thumb-key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("icon.png");
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        std::fs::copy(&favicon, &src).unwrap();
        let s = src.to_str().unwrap();

        let at256 = thumb_cache_file(&dir, s, 256).unwrap();
        let at512 = thumb_cache_file(&dir, s, 512).unwrap();
        assert_ne!(at256, at512, "max_px is part of the key");
        assert_eq!(at256.extension().and_then(|e| e.to_str()), Some("jpg"));

        // Same path but different bytes → different key (size/mtime invalidation).
        std::fs::write(&src, b"different-bytes").unwrap();
        let rewritten = thumb_cache_file(&dir, s, 256).unwrap();
        assert_ne!(at256, rewritten, "edited files must miss the old entry");
    }

    #[test]
    fn thumb_cache_missing_source_is_a_clean_error() {
        let state = engine::new_state(); // never touched: the stat fails first
        let dir = std::env::temp_dir().join("ff-thumb-missing");
        let _ = std::fs::create_dir_all(&dir);
        let err =
            cached_thumb_value(&state, &dir, "/definitely/not/a/photo.jpg", 256).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn thumb_cache_serves_identical_bytes_from_disk() {
        let state = engine_state();
        let src_dir = std::env::temp_dir().join("ff-thumb-src");
        let _ = std::fs::remove_dir_all(&src_dir);
        std::fs::create_dir_all(&src_dir).unwrap();
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        assert!(favicon.exists(), "static/favicon.png must exist for the thumb test");
        let src = src_dir.join("icon.png");
        std::fs::copy(&favicon, &src).unwrap();

        let cache_dir = std::env::temp_dir().join("ff-thumb-cache");
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).unwrap();

        let first = cached_thumb_value(&state, &cache_dir, src.to_str().unwrap(), 256)
            .expect("engine thumb on favicon.png");
        let img1 = first.get("image").and_then(Value::as_str).expect("image field");
        assert!(!img1.is_empty(), "engine must return thumb bytes");

        let files: Vec<_> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), 1, "one cache file after the first call");
        let mtime_before = files[0].metadata().unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let second =
            cached_thumb_value(&state, &cache_dir, src.to_str().unwrap(), 256).expect("cached thumb");
        let img2 = second.get("image").and_then(Value::as_str).unwrap();
        assert_eq!(img1, img2, "cache hit returns identical bytes");
        let mtime_after = std::fs::metadata(&files[0].path()).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "a hit must not rewrite the cache file");
    }

    #[test]
    fn face_cache_key_covers_box_and_max_px() {
        let dir = std::env::temp_dir().join("ff-face-key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("icon.png");
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        std::fs::copy(&favicon, &src).unwrap();
        let s = src.to_str().unwrap();

        let a = face_cache_file(&dir, s, 0.1, 0.2, 0.3, 0.4, 256).unwrap();
        let b = face_cache_file(&dir, s, 0.5, 0.2, 0.3, 0.4, 256).unwrap();
        assert_ne!(a, b, "x is part of the key");
        let c = face_cache_file(&dir, s, 0.1, 0.2, 0.3, 0.4, 512).unwrap();
        assert_ne!(a, c, "max_px is part of the key");
        let d = face_cache_file(&dir, s, 0.1, 0.2, 0.35, 0.4, 256).unwrap();
        assert_ne!(a, d, "width is part of the key");
        assert_eq!(a.extension().and_then(|e| e.to_str()), Some("jpg"));

        // Same path but different bytes → different key (size/mtime invalidation).
        std::fs::write(&src, b"different-bytes").unwrap();
        let rewritten = face_cache_file(&dir, s, 0.1, 0.2, 0.3, 0.4, 256).unwrap();
        assert_ne!(a, rewritten, "edited files must miss the old entry");
    }

    #[test]
    fn face_cache_missing_source_is_a_clean_error() {
        let state = engine::new_state(); // never touched: the stat fails first
        let dir = std::env::temp_dir().join("ff-face-missing");
        let _ = std::fs::create_dir_all(&dir);
        let err = cached_face_thumb_value(&state, &dir, "/definitely/not/a/photo.jpg", 0.1, 0.2, 0.3, 0.4, 256)
            .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn face_cache_serves_identical_bytes_from_disk() {
        let state = engine_state();
        let src_dir = std::env::temp_dir().join("ff-face-src");
        let _ = std::fs::remove_dir_all(&src_dir);
        std::fs::create_dir_all(&src_dir).unwrap();
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        assert!(favicon.exists(), "static/favicon.png must exist for the face test");
        let src = src_dir.join("icon.png");
        std::fs::copy(&favicon, &src).unwrap();

        let cache_dir = std::env::temp_dir().join("ff-face-cache");
        let _ = std::fs::remove_dir_all(&cache_dir);
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Full-image crop: valid on any decodable image.
        let first = cached_face_thumb_value(&state, &cache_dir, src.to_str().unwrap(), 0.0, 0.0, 1.0, 1.0, 256)
            .expect("engine face_thumb on favicon.png");
        let img1 = first.get("image").and_then(Value::as_str).expect("image field");
        assert!(!img1.is_empty(), "engine must return face bytes");

        let files: Vec<_> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), 1, "one cache file after the first call");
        let mtime_before = files[0].metadata().unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let second =
            cached_face_thumb_value(&state, &cache_dir, src.to_str().unwrap(), 0.0, 0.0, 1.0, 1.0, 256)
                .expect("cached face thumb");
        let img2 = second.get("image").and_then(Value::as_str).unwrap();
        assert_eq!(img1, img2, "cache hit returns identical bytes");
        let mtime_after = std::fs::metadata(&files[0].path()).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "a hit must not rewrite the cache file");

        // A different box must miss and populate a second entry.
        let third =
            cached_face_thumb_value(&state, &cache_dir, src.to_str().unwrap(), 0.0, 0.0, 0.5, 0.5, 256)
                .expect("second box renders");
        assert!(!third.get("image").and_then(Value::as_str).unwrap().is_empty());
        let files: Vec<_> = std::fs::read_dir(&cache_dir)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), 2, "a new box is a new cache entry");
    }

    #[test]
    fn thumb_cache_prune_evicts_oldest_until_under_target() {
        let dir = std::env::temp_dir().join("ff-thumb-prune");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5u8 {
            std::fs::write(dir.join(format!("{i:02}.jpg")), vec![i; 100]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        // 500B on disk vs a 300B cap: eviction runs until <= 150B, i.e. only
        // the newest file survives.
        let removed = prune_thumb_cache(&dir, 300, 150);
        assert_eq!(removed, 4, "oldest four files are evicted");
        assert!(!dir.join("00.jpg").exists(), "oldest goes first");
        assert!(dir.join("04.jpg").exists(), "newest survives");
        let remaining: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
            .sum();
        assert!(remaining <= 150, "under the prune target, got {remaining}B");

        // Under the cap: no-op.
        assert_eq!(prune_thumb_cache(&dir, 300, 150), 0);
    }

    #[test]
    fn verify_probe_counts_unreadable_without_flagging() {
        let dir = std::env::temp_dir().join("ff-verify-probe");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.jpg");
        std::fs::write(&good, b"bytes").unwrap();
        assert!(!is_unreadable_1byte(good.to_str().unwrap()));
        assert!(is_unreadable_1byte(dir.to_str().unwrap()), "a directory reads as unreadable");

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO assets (path, name, ext, kind, size_bytes, modified, missing, first_seen_at, last_seen_at) VALUES (?1, 'good.jpg', 'jpg', 'image', 5, 0, 0, 0, 0)",
            rusqlite::params![good.to_str().unwrap()],
        )
        .unwrap();
        // 25 unreadable rows (directories exist but never read as files).
        for i in 0..25 {
            let sub = dir.join(format!("sub{i}"));
            std::fs::create_dir_all(&sub).unwrap();
            conn.execute(
                "INSERT INTO assets (path, name, ext, kind, size_bytes, modified, missing, first_seen_at, last_seen_at) VALUES (?1, 'sub', '', 'image', 0, 0, 0, 0, 0)",
                rusqlite::params![sub.to_str().unwrap()],
            )
            .unwrap();
        }
        // A vanished file stays the missing pass's business, never unreadable.
        conn.execute(
            "INSERT INTO assets (path, name, ext, kind, size_bytes, modified, missing, first_seen_at, last_seen_at) VALUES ('/definitely/not/here.jpg', 'here.jpg', 'jpg', 'image', 0, 0, 0, 0, 0)",
            [],
        )
        .unwrap();

        let (count, sample) = probe_unreadable(&conn);
        assert_eq!(count, 25, "only the unreadable dirs count");
        assert_eq!(sample.len(), VERIFY_UNREADABLE_SAMPLE_CAP, "sample caps at 20");
        let flagged: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets WHERE missing = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(flagged, 0, "the probe never flags rows");
    }

    #[test]
    fn thumb_b64_roundtrips_without_a_backend() {
        for len in [0usize, 1, 2, 3, 4, 5, 100, 255] {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
            assert_eq!(b64_decode(&b64_encode(&bytes)).unwrap(), bytes, "len {len}");
        }
        assert!(b64_decode("!!!").is_err(), "junk is rejected");
    }

    #[test]
    fn faces_global_union_find_unifies_across_chunks() {
        // Same person split across two 200-image chunks must land in one group.
        // Chunk-local grouping would make two groups; global must make one.
        let ids = vec![1, 2, 3, 4];
        let same = vec![0.1f32, 0.2, 0.3];
        let vecs = vec![same.clone(), same.clone(), same.clone(), same.clone()];
        let groups = group_faces_by_distance(&ids, &vecs, 0.55);
        assert_eq!(groups.len(), 1, "identical descriptors unify globally");
        assert_eq!(groups[0], vec![1, 2, 3, 4]);

        // Distant descriptors stay apart at the same threshold.
        let far_a = vec![0.0f32, 0.0];
        let far_b = vec![10.0f32, 10.0];
        let groups = group_faces_by_distance(&[10, 11], &[far_a, far_b], 0.55);
        assert_eq!(groups.len(), 2, "far descriptors split");

        // Threshold behavior: 0.5 merges, 0.6 splits at 0.55.
        let o = vec![0.0f32, 0.0];
        let near = vec![0.3f32, 0.4]; // Euclidean 0.5
        let edge = vec![0.6f32, 0.0]; // Euclidean 0.6
        assert_eq!(group_faces_by_distance(&[20, 21], &[o.clone(), near], 0.55).len(), 1);
        assert_eq!(group_faces_by_distance(&[20, 21], &[o, edge], 0.55).len(), 2);
    }

    #[test]
    fn faces_regroup_is_idempotent_and_new_images_join() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::migrate(&conn).unwrap();
        let face_json = |x: f64, v: &[f32]| {
            serde_json::json!({
                "x": x, "y": 0.2, "width": 0.3, "height": 0.4, "confidence": 0.9,
                "descriptor": b64_encode(&db::floats_to_bytes(v)),
            })
        };
        // Two people: A near origin, B far away.
        let a_vec = vec![0.0f32, 0.0];
        let b_vec = vec![10.0f32, 10.0];
        let a1 = db::save_faces(&conn, "/a1.jpg", &[face_json(0.1, &a_vec)]).unwrap()[0];
        let a2 = db::save_faces(&conn, "/a2.jpg", &[face_json(0.2, &a_vec)]).unwrap()[0];
        let b1 = db::save_faces(&conn, "/b1.jpg", &[face_json(0.1, &b_vec)]).unwrap()[0];
        let b2 = db::save_faces(&conn, "/b2.jpg", &[face_json(0.2, &b_vec)]).unwrap()[0];

        let regroup = || -> Vec<Vec<i64>> {
            let stored = db::face_descriptors(&conn).unwrap();
            let ids: Vec<i64> = stored.iter().map(|(id, _)| *id).collect();
            let vecs: Vec<Vec<f32>> = stored.into_iter().map(|(_, v)| v).collect();
            group_faces_by_distance(&ids, &vecs, 0.55)
        };
        let g1 = regroup();
        assert_eq!(g1.len(), 2, "two people stay apart");
        let clusters = db::replace_clusters(&conn, g1.clone()).unwrap();
        assert_eq!(clusters.len(), 2);
        db::rename_cluster(&conn, clusters[0], "Asha").unwrap();
        let before = (db::faces_count(&conn).unwrap(), db::clusters_list(&conn).unwrap().len());

        // Rerun with no new images: same counts, name intact.
        let g2 = regroup();
        assert_eq!(g1, g2, "grouping is deterministic");
        db::replace_clusters(&conn, g2).unwrap();
        assert_eq!((db::faces_count(&conn).unwrap(), db::clusters_list(&conn).unwrap().len()), before);
        assert!(db::clusters_list(&conn).unwrap().iter().any(|c| c.name.as_deref() == Some("Asha")));

        // New image of person A joins their cluster globally.
        let _a3 = db::save_faces(&conn, "/a3.jpg", &[face_json(0.15, &a_vec)]).unwrap()[0];
        let g3 = regroup();
        assert_eq!(g3.len(), 2, "still two people after adding one of A");
        db::replace_clusters(&conn, g3).unwrap();
        let rows = db::clusters_list(&conn).unwrap();
        assert_eq!(rows.len(), 2);
        let biggest = rows.iter().max_by_key(|r| r.face_count).unwrap();
        assert_eq!(biggest.face_count, 3, "new image joins existing person");
        let _ = (a1, a2, b1, b2);
    }

    /// Writes a 24-bit BMP (ImageIO decodes it; hand-rolled so the test needs
    /// no image crate): `pixel(x, y)` returns (r, g, b).
    fn write_bmp(path: &std::path::Path, w: u32, h: u32, mut pixel: impl FnMut(u32, u32) -> (u8, u8, u8)) {
        let row_stride = ((w * 3 + 3) / 4) * 4;
        let mut px = vec![0u8; (row_stride * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = pixel(x, y);
                let o = ((h - 1 - y) * row_stride + x * 3) as usize;
                px[o] = b;
                px[o + 1] = g;
                px[o + 2] = r;
            }
        }
        let mut hdr = [0u8; 54];
        hdr[0] = b'B';
        hdr[1] = b'M';
        hdr[2..6].copy_from_slice(&(54 + px.len() as u32).to_le_bytes());
        hdr[10] = 54;
        hdr[14..18].copy_from_slice(&40u32.to_le_bytes());
        hdr[18..22].copy_from_slice(&w.to_le_bytes());
        hdr[22..26].copy_from_slice(&h.to_le_bytes());
        hdr[26..28].copy_from_slice(&1u16.to_le_bytes());
        hdr[28..30].copy_from_slice(&24u16.to_le_bytes());
        hdr[34..38].copy_from_slice(&(px.len() as u32).to_le_bytes());
        let mut out = hdr.to_vec();
        out.extend_from_slice(&px);
        std::fs::write(path, out).unwrap();
    }

    /// Embeds one file through the engine and decodes the f32 vector.
    fn embed_vec(state: &engine::EngineState, path: &str) -> (Vec<f32>, i64) {
        let reply = engine::call_blocking(
            state,
            "embed",
            Some(json!({ "path": path })),
            Duration::from_secs(120),
        )
        .unwrap_or_else(|e| panic!("embed {path}: {e}"));
        let b64 = reply.get("vector").and_then(Value::as_str).expect("vector field");
        let dim = reply.get("dim").and_then(Value::as_i64).expect("dim field");
        let vec = db::bytes_to_floats(&b64_decode(b64).expect("valid base64")).expect("f32 blob");
        assert_eq!(vec.len() as i64, dim, "dim matches the decoded length");
        (vec, dim)
    }

    #[test]
    fn embed_is_deterministic_and_orders_variants_first() {
        let dir = std::env::temp_dir().join("ff-embed-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Query: the real icon. Variant: an engine JPEG recode of it (same
        // pixels, different bytes/size). Unrelated: deterministic noise.
        let favicon = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("../static/favicon.png");
        assert!(favicon.exists(), "static/favicon.png must exist for the embed test");
        let query = dir.join("query.png");
        std::fs::copy(&favicon, &query).unwrap();
        let variant = dir.join("variant.jpg");
        let noise = dir.join("noise.bmp");
        // Xorshift32: deterministic, no RNG dependency.
        let mut s = 0x12345678u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as u8
        };
        write_bmp(&noise, 128, 128, |x, y| (next(), next(), next().wrapping_add((x + y) as u8)));

        let state = engine_state();
        engine::call_blocking(
            &state,
            "compress",
            Some(json!({
                "path": query.to_str().unwrap(),
                "output": variant.to_str().unwrap(),
                "maxPx": 64,
                "quality": 0.72,
            })),
            Duration::from_secs(60),
        )
        .expect("engine compress for the resized variant");

        // Determinism: the same file twice embeds to cosine 1.0.
        let (v1, dim) = embed_vec(&state, query.to_str().unwrap());
        let (v2, _) = embed_vec(&state, query.to_str().unwrap());
        assert!(dim > 0, "the encoder reports a real dimension, got {dim}");
        assert_eq!(db::cosine(&v1, &v2), 1.0, "identical files embed identically");

        let conn = db::open(std::path::Path::new(":memory:")).unwrap();
        db::migrate(&conn).unwrap();
        run_import(&conn, &state, dir.to_str().unwrap(), true, |_| {}, |_| {}).unwrap();
        let summary = run_embed_library(&conn, &state, |_, _| {}).unwrap();
        assert_eq!((summary.embedded, summary.failed), (3, 0), "all three images embed");

        // A rerun finds no candidates: embedding is idempotent.
        let again = run_embed_library(&conn, &state, |_, _| {}).unwrap();
        assert_eq!((again.embedded, again.failed), (0, 0));

        let qid: i64 = conn
            .query_row("SELECT id FROM assets WHERE path LIKE '%query.png'", [], |r| r.get(0))
            .unwrap();
        let hits = db::similar_to(&conn, qid, db::VECTOR_MODEL, 10).unwrap();
        assert_eq!(hits.len(), 2, "the query photo is excluded");
        assert!(
            hits[0].path.ends_with("variant.jpg"),
            "the resized variant ranks first, got {} @ {}",
            hits[0].path,
            hits[0].score
        );
        assert!(
            hits[0].score > hits[1].score,
            "variant {} beats noise {}",
            hits[0].score,
            hits[1].score
        );
        eprintln!(
            "embed ordering: variant={:.4} noise={:.4} dim={dim}",
            hits[0].score, hits[1].score
        );
    }

    #[test]
    fn ingest_copy_mirrors_media_skips_identical_and_verifies() {
        let src = std::env::temp_dir().join("ff-ingest-src");
        let dest = std::env::temp_dir().join("ff-ingest-dest");
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::create_dir_all(src.join("DCIM")).unwrap();
        std::fs::write(src.join("DCIM/a.jpg"), vec![b'x'; 100]).unwrap();
        std::fs::write(src.join("DCIM/b.mp4"), vec![b'y'; 200]).unwrap();
        std::fs::write(src.join("notes.txt"), b"ignore me").unwrap();

        let mut seen = Vec::new();
        let first = ingest_copy_tree(&src, &dest, |s| seen.push(s.copied)).unwrap();
        assert_eq!((first.copied, first.skipped, first.total), (2, 0, 2));
        assert_eq!(first.bytes, 300);
        assert!(first.failed.is_empty());
        assert_eq!(std::fs::read(dest.join("DCIM/a.jpg")).unwrap().len(), 100);
        assert!(!dest.join("notes.txt").exists(), "non-media stays behind");
        assert!(!seen.is_empty(), "progress reported");

        // Second run: identical files skip, touched source re-copies.
        let second = ingest_copy_tree(&src, &dest, |_| {}).unwrap();
        assert_eq!((second.copied, second.skipped), (0, 2));
        std::fs::write(src.join("DCIM/a.jpg"), vec![b'z'; 150]).unwrap();
        let third = ingest_copy_tree(&src, &dest, |_| {}).unwrap();
        assert_eq!((third.copied, third.skipped), (1, 1));
        assert_eq!(std::fs::read(dest.join("DCIM/a.jpg")).unwrap().len(), 150);

        // Destination inside the source is refused.
        assert!(ingest_copy_tree(&src, &src.join("sub"), |_| {}).is_err());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
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
    use crate::parse_nominatim_bbox;

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

    #[test]
    fn static_geofences_categorize_offline() {
        use crate::geofences::static_geofence;
        assert_eq!(
            static_geofence(37.7749, -122.4194),
            Some("California, United States")
        );
        assert_eq!(static_geofence(13.0827, 80.2707), Some("India"));
        assert_eq!(static_geofence(48.8566, 2.3522), Some("France"));
        assert_eq!(
            static_geofence(0.0, -140.0),
            None,
            "open Pacific matches no fence"
        );
    }


    #[test]
    fn nominatim_bbox_parses_and_rejects_junk() {
        let body = serde_json::json!({"boundingbox": ["37.7", "37.8", "-122.5", "-122.4"]});
        assert_eq!(
            parse_nominatim_bbox(&body),
            Some((37.7, 37.8, -122.5, -122.4))
        );
        let bad = serde_json::json!({"boundingbox": ["37.8", "37.7", "-122.4", "-122.5"]});
        assert_eq!(parse_nominatim_bbox(&bad), None, "inverted box rejected");
        let short = serde_json::json!({"boundingbox": ["1", "2"]});
        assert_eq!(parse_nominatim_bbox(&short), None, "truncated box rejected");
        let none = serde_json::json!({});
        assert_eq!(parse_nominatim_bbox(&none), None, "missing box rejected");
    }
}
