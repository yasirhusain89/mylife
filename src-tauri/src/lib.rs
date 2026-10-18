mod db;
mod engine;

use serde::Serialize;
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

#[tauri::command]
fn library_list(
    conn: tauri::State<'_, DbState>,
    kind: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<db::AssetRow>, String> {
    let conn = conn.lock().map_err(|_| "database lock poisoned")?;
    let offset = offset.unwrap_or(0).max(0);
    let limit = limit.unwrap_or(200).clamp(1, 1000);
    let kind = kind.filter(|k| k == "image" || k == "video");
    db::library(&conn, kind.as_deref(), offset, limit)
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
            app.manage(Arc::new(Mutex::new(conn)) as DbState);
            app.manage(engine::new_state());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            engine_call,
            import_folder,
            library_list,
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
}
