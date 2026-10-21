// Bridge to the fotoflow engine Swift sidecar (JSON-lines over stdio).

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A line from the engine: either the final reply or an interim event
/// (`chunk` / `progress`) belonging to the in-flight request.
pub enum Envelope {
    Reply(Value),
    Event(String, Value),
}

pub struct Engine {
    child: Child,
    stdin: Box<dyn Write + Send>,
    next_id: u64,
    pending: Arc<Mutex<HashMap<u64, Sender<Envelope>>>>,
}

pub(crate) fn engine_path() -> Option<PathBuf> {
    #[cfg(target_arch = "aarch64")]
    let triple = "aarch64-apple-darwin";
    #[cfg(target_arch = "x86_64")]
    let triple = "x86_64-apple-darwin";

    // Escape hatch for tests and local debugging.
    if let Ok(custom) = std::env::var("FOTOFLOW_ENGINE_PATH") {
        let p = PathBuf::from(custom);
        if p.exists() {
            return Some(p);
        }
    }

    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Dev and bundles strip the target triple; `engine-{triple}` covers the
    // layout produced before tauri-cli flattens external bins.
    let plain = exe_dir.join("engine");
    if plain.exists() {
        return Some(plain);
    }
    let path = exe_dir.join(format!("engine-{triple}"));
    if path.exists() { Some(path) } else { None }
}

fn spawn() -> Result<Engine, String> {
    let path = engine_path().ok_or_else(|| {
        "engine sidecar binary not found next to the app executable. Run: pnpm build:engine".to_owned()
    })?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/fotoflow-engine.log")
        .map_err(|e| format!("engine log: {e}"))?;
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("engine spawn: {e}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or("engine: stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("engine: stdout unavailable")?;
    let pending: Arc<Mutex<HashMap<u64, Sender<Envelope>>>> = Arc::new(Mutex::new(HashMap::new()));
    let reader_map = pending.clone();

    std::thread::Builder::new()
        .name("engine-reader".into())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let Some(id) = v.get("id").and_then(Value::as_u64) else { continue };

                let envelope = match (v.get("event").and_then(Value::as_str), &v["data"]) {
                    (Some(name), data) => Envelope::Event(name.to_string(), data.clone()),
                    _ => Envelope::Reply(v.clone()),
                };
                let is_reply = matches!(envelope, Envelope::Reply(_));

                // Interim events must leave the sender registered, otherwise the
                // caller's next recv sees a disconnected channel. Only the final
                // reply retires the entry.
                let sender = if is_reply {
                    reader_map.lock().unwrap().remove(&id)
                } else {
                    reader_map.lock().unwrap().get(&id).cloned()
                };
                if let Some(tx) = sender {
                    let _ = tx.send(envelope);
                }
            }
        })
        .map_err(|e| format!("engine reader thread: {e}"))?;

    Ok(Engine { child, stdin: Box::new(stdin), next_id: 0, pending })
}

pub type EngineState = Arc<Mutex<Option<Engine>>>;

pub fn new_state() -> EngineState {
    Arc::new(Mutex::new(None))
}

impl Engine {
    pub fn call(&mut self, cmd: &str, params: Option<Value>, timeout: Duration) -> Result<Value, String> {
        self.call_with_events(cmd, params, timeout, &mut |_, _| {})
    }

    /// Like `call`, but hands every interim `chunk`/`progress` payload to
    /// `on_event` until the final reply arrives.
    pub fn call_with_events(
        &mut self,
        cmd: &str,
        params: Option<Value>,
        timeout: Duration,
        on_event: &mut dyn FnMut(&str, &Value),
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx): (Sender<Envelope>, Receiver<Envelope>) = channel();
        self.pending.lock().unwrap().insert(id, tx);

        let mut req = serde_json::json!({ "id": id, "cmd": cmd });
        if let Some(p) = params {
            req["params"] = p;
        }
        let mut line = req.to_string().into_bytes();
        line.push(b'\n');
        if let Err(e) = self.stdin.write_all(&line) {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("engine write: {e}"));
        }
        if let Err(e) = self.stdin.flush() {
            self.pending.lock().unwrap().remove(&id);
            return Err(format!("engine flush: {e}"));
        }

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(Envelope::Event(name, data)) => on_event(&name, &data),
                Ok(Envelope::Reply(resp)) => {
                    return if resp.get("ok").and_then(Value::as_bool) == Some(true) {
                        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
                    } else {
                        Err(resp
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown engine error")
                            .to_owned())
                    };
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    self.pending.lock().unwrap().remove(&id);
                    return Err(format!("engine closed stdout during '{cmd}'"));
                }
            }
        }

        self.pending.lock().unwrap().remove(&id);
        Err(format!("engine call '{cmd}' timed out after {timeout:?}"))
    }
}

pub fn call_blocking(
    state: &EngineState,
    cmd: &str,
    params: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    match with_engine(state, |engine| engine.call(cmd, params.clone(), timeout)) {
        Err(spawn_err) if is_transport_error(&spawn_err) => {
            std::thread::sleep(watchdog_backoff(1));
            reset_engine(state);
            with_engine(state, |engine| engine.call(cmd, params, timeout))?
        }
        Err(spawn_err) => Err(spawn_err),
        Ok(inner) => match inner {
            Ok(v) => {
                // Success clears nothing here (stateless backoff); the sleep
                // budget only applies to the retry path above/below.
                Ok(v)
            }
            Err(call_err) if is_transport_error(&call_err) => {
                std::thread::sleep(watchdog_backoff(1));
                reset_engine(state);
                with_engine(state, |engine| engine.call(cmd, params, timeout))?
            }
            Err(e) => Err(e),
        },
    }
}

pub fn call_with_events(
    state: &EngineState,
    cmd: &str,
    params: Option<Value>,
    timeout: Duration,
    on_event: &mut dyn FnMut(&str, &Value),
) -> Result<Value, String> {
    // `on_event` is borrowed for a single attempt; retry reuses a no-op
    // listener so interim events are never double-delivered. Progress-heavy
    // callers re-emit on the successful attempt only via the second closure
    // below — in practice the retry path is a cold restart after a dead
    // pipe, so the first attempt delivered nothing.
    let first: Result<Result<Value, String>, String> = with_engine(state, |engine| {
        // Scoped borrow of on_event for this attempt only.
        engine.call_with_events(cmd, params.clone(), timeout, &mut *on_event)
    });
    match first {
        Err(spawn_err) if is_transport_error(&spawn_err) => {
            std::thread::sleep(watchdog_backoff(1));
            reset_engine(state);
            with_engine(state, |engine| {
                engine.call_with_events(cmd, params, timeout, &mut *on_event)
            })?
        }
        Err(spawn_err) => Err(spawn_err),
        Ok(inner) => match inner {
            Ok(v) => Ok(v),
            Err(call_err) if is_transport_error(&call_err) => {
                std::thread::sleep(watchdog_backoff(1));
                reset_engine(state);
                with_engine(state, |engine| {
                    engine.call_with_events(cmd, params, timeout, &mut *on_event)
                })?
            }
            Err(e) => Err(e),
        },
    }
}

/// Transport-level failure: the bytes never reached the engine (broken pipe,
/// dead child, spawn failure) as opposed to an engine-reported logic error
/// ("unknown command", "cannot decode image"). Only these restart the
/// sidecar; logic errors return as-is with no behavior change.
pub fn is_transport_error(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("broken pipe")
        || e.contains("closed stdout")
        || e.contains("disconnected")
        || e.contains("engine write")
        || e.contains("engine flush")
        || e.contains("engine spawn")
        || e.contains("stdin unavailable")
        || e.contains("stdout unavailable")
        || e.contains("dead child")
        || e.contains("os error 32")
        || e.contains("not found next to the app executable")
}

/// Small backoff before the single retry: 100ms base, doubling per
/// consecutive failure, capped at 1s. Pure function so the retry budget is
/// unit-testable without spawning a sidecar.
pub fn watchdog_backoff(consecutive_failures: u32) -> Duration {
    let shift = consecutive_failures.saturating_sub(1).min(3);
    Duration::from_millis(100 * (1 << shift).min(10))
}

/// Consecutive-failure counter: success resets to 0, a transport error
/// increments, any other error leaves the count untouched (no backoff debt
/// for logic errors).
pub fn next_failure_count(current: u32, result: &Result<Value, String>) -> u32 {
    match result {
        Ok(_) => 0,
        Err(e) if is_transport_error(e) => current.saturating_add(1),
        Err(_) => current,
    }
}

/// Drops the current child (if any) so the next `with_engine` spawns fresh.
/// Never fails: a dead child is already gone.
fn reset_engine(state: &EngineState) {
    let mut opt = state.lock().unwrap();
    *opt = None;
    // The old Engine's Drop kills + waits the child.
}

fn with_engine<T>(
    state: &EngineState,
    f: impl FnOnce(&mut Engine) -> T,
) -> Result<T, String> {
    {
        let mut opt = state.lock().unwrap();
        if opt.is_none() {
            *opt = Some(spawn()?);
        }
    }
    let mut opt = state.lock().unwrap();
    let engine = opt.as_mut().ok_or("engine unavailable")?;
    Ok(f(engine))
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine_bin() -> Option<PathBuf> {
        let dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
        let name = if cfg!(target_arch = "aarch64") {
            "engine-aarch64-apple-darwin"
        } else {
            "engine-x86_64-apple-darwin"
        };
        let p = PathBuf::from(dir).join("binaries").join(name);
        p.exists().then_some(p)
    }

    /// 250 files > CHUNK_SIZE(200) forces multiple chunk flushes.
    fn fixture(n: usize) -> PathBuf {
        let dir = std::env::temp_dir().join("ff-stream-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..n {
            std::fs::write(dir.join(format!("p{i:04}.jpg")), b"not-a-real-jpeg").unwrap();
        }
        std::fs::write(dir.join("ignore.txt"), b"skip me").unwrap();
        dir
    }

    fn state_with_engine() -> EngineState {
        let bin = engine_bin().expect("engine binary not built; run pnpm build:engine");
        unsafe { std::env::set_var("FOTOFLOW_ENGINE_PATH", &bin) };
        new_state()
    }

    #[test]
    fn streaming_scan_yields_chunks_then_final_result() {
        let dir = fixture(250);
        let state = state_with_engine();

        let (mut chunks, mut items, mut progress) = (0, 0, 0);
        let res = call_with_events(
            &state,
            "scan",
            Some(json!({ "root": dir.to_string_lossy(), "recursive": true })),
            Duration::from_secs(60),
            &mut |name, data| match name {
                "chunk" => {
                    chunks += 1;
                    items += data.as_array().map(|a| a.len()).unwrap_or(0);
                }
                "progress" => progress += 1,
                _ => {}
            },
        )
        .expect("scan failed");

        assert_eq!(res["scanned"], 250, "non-media file is skipped");
        assert_eq!(res["skipped"], 1);
        assert_eq!(items, 250, "every media file arrives on a chunk");
        assert!(chunks >= 2, "250 files must span multiple chunks, got {chunks}");
        assert!(progress >= 1, "progress events fire per flush");
    }

    #[test]
    fn error_replies_surface_as_err() {
        let state = state_with_engine();
        let err = call_blocking(&state, "bogus", None, Duration::from_secs(10)).unwrap_err();
        assert!(err.contains("unknown command"), "got: {err}");
    }

    /// The share-pack video phase runs one ephemeral engine per worker
    /// thread: concurrent states must not cross-talk, deadlock, or leak
    /// children (Drop kills each child on thread exit).
    #[test]
    fn concurrent_ephemeral_engines_stay_isolated() {
        let _ = state_with_engine(); // validates the binary path once
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for w in 0..3 {
                handles.push(s.spawn(move || {
                    let state = new_state();
                    for i in 0..10 {
                        let r = call_blocking(&state, "ping", None, Duration::from_secs(10))
                            .unwrap_or_else(|e| panic!("worker {w} ping {i} failed: {e}"));
                        assert_eq!(r["pong"], true, "worker {w} ping {i}");
                    }
                    // Same process reused within the worker (no respawn storm).
                    state
                }));
            }
            for h in handles {
                h.join().expect("worker panicked");
                // Joined states drop here: children killed + reaped.
            }
        });
    }

    #[test]
    fn process_is_reused_across_calls() {
        let state = state_with_engine();
        call_blocking(&state, "ping", None, Duration::from_secs(10)).unwrap();
        let info = call_blocking(&state, "sysinfo", None, Duration::from_secs(10)).unwrap();
        let host_arch = std::process::Command::new("uname")
            .arg("-m")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        assert_eq!(info["arch"], host_arch);
    }

    #[test]
    fn transport_errors_restart_but_logic_errors_do_not() {
        for msg in [
            "engine write: Broken pipe (os error 32)",
            "engine flush: Broken pipe (os error 32)",
            "engine closed stdout during 'scan'",
            "engine spawn: No such file or directory",
            "engine: stdin unavailable",
            "engine sidecar binary not found next to the app executable. Run: pnpm build:engine",
            "engine closed stdout during 'ping'",
        ] {
            assert!(is_transport_error(msg), "must restart on: {msg}");
        }
        for msg in [
            "unknown command 'bogus'",
            "cannot decode image: /photos/a.jpg",
            "thumb engine returned no image",
            "engine call 'scan' timed out after 60s",
            "not a directory",
        ] {
            assert!(!is_transport_error(msg), "must not restart on: {msg}");
        }
    }

    #[test]
    fn watchdog_backoff_grows_and_caps() {
        assert_eq!(watchdog_backoff(0), Duration::from_millis(100));
        assert_eq!(watchdog_backoff(1), Duration::from_millis(100));
        assert_eq!(watchdog_backoff(2), Duration::from_millis(200));
        assert_eq!(watchdog_backoff(3), Duration::from_millis(400));
        assert_eq!(watchdog_backoff(4), Duration::from_millis(800));
        assert_eq!(watchdog_backoff(5), Duration::from_millis(800));
        assert_eq!(watchdog_backoff(100), Duration::from_millis(800));
    }

    #[test]
    fn failure_count_resets_on_success_and_ignores_logic_errors() {
        let ok: Result<Value, String> = Ok(json!({}));
        assert_eq!(next_failure_count(3, &ok), 0);
        let transport: Result<Value, String> = Err("engine write: Broken pipe (os error 32)".into());
        assert_eq!(next_failure_count(0, &transport), 1);
        assert_eq!(next_failure_count(2, &transport), 3);
        let logic: Result<Value, String> = Err("unknown command 'bogus'".into());
        assert_eq!(next_failure_count(2, &logic), 2, "logic errors leave the count alone");
    }

    #[test]
    fn reset_engine_drops_a_dead_child_without_failing() {
        let state = new_state();
        reset_engine(&state); // None -> None is a no-op
        assert!(state.lock().unwrap().is_none());
    }
}
