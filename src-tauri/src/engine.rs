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

fn engine_path() -> Option<PathBuf> {
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
    with_engine(state, |engine| engine.call(cmd, params, timeout))?
}

pub fn call_with_events(
    state: &EngineState,
    cmd: &str,
    params: Option<Value>,
    timeout: Duration,
    on_event: &mut dyn FnMut(&str, &Value),
) -> Result<Value, String> {
    with_engine(state, |engine| engine.call_with_events(cmd, params, timeout, on_event))?
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
}
