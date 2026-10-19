// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Headless API mode for automation: `fotoflow --serve [--port N]
/// [--db PATH] [--demo]` serves the read API on 127.0.0.1 without opening
/// a window. Everything else boots the desktop app.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--serve") {
        let port = args
            .windows(2)
            .find(|w| w[0] == "--port")
            .and_then(|w| w[1].parse::<u16>().ok())
            .unwrap_or(1423);
        let db = args
            .windows(2)
            .find(|w| w[0] == "--db")
            .map(|w| std::path::PathBuf::from(&w[1]))
            .unwrap_or_else(|| std::env::temp_dir().join("fotoflow-api.db"));
        let demo = args.iter().any(|a| a == "--demo");
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        if let Err(e) = rt.block_on(fotoflow_lib::serve(&db, port, demo)) {
            eprintln!("fotoflow serve failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    fotoflow_lib::run()
}
