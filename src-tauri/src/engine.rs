// Bridge to the fotoflow engine Swift sidecar (JSON-lines over stdio).

use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Engine {
    child: Child,
    stdin: Box<dyn Write + Send>,
    next_id: u64,
    pending: Arc<Mutex<HashMap<u64, Sender<Value>>>>,
}

fn engine_path() -> Option<PathBuf> {
    #[cfg(target_arch = "aarch64")]
    let triple = "aarch64-apple-darwin";
    #[cfg(target_arch = "x86_64")]
    let triple = "x86_64-apple-darwin";
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
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

    let pending: Arc<Mutex<HashMap<u64, Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
    let reader_map = pending.clone();

    std::thread::Builder::new()
        .name("engine-reader".into())
        .spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l, Err(_) => break,
                };
                if line.trim().is_empty() { continue; }
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                let Some(id) = v.get("id").and_then(Value::as_u64) else { continue };
                if let Some(tx) = reader_map.lock().unwrap().remove(&id) {
                    let _ = tx.send(v);
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
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx): (Sender<Value>, Receiver<Value>) = channel();
        self.pending.lock().unwrap().insert(id, tx);

        let mut req = serde_json::json!({ "id": id, "cmd": cmd });
        if let Some(p) = params { req["params"] = p; }
        let mut line = req.to_string().into_bytes();
        line.push(b'\n');
        self.stdin.write_all(&line).map_err(|e| format!("engine write: {e}"))?;
        self.stdin.flush().map_err(|e| format!("engine flush: {e}"))?;

        let resp = rx.recv_timeout(timeout).map_err(|_| {
            self.pending.lock().unwrap().remove(&id);
            format!("engine call '{cmd}' timed out after {timeout:?}")
        })?;

        if resp.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(resp.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(resp
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown engine error")
                .to_owned())
        }
    }
}

pub fn call_blocking(
    state: &EngineState,
    cmd: &str,
    params: Option<Value>,
    timeout: Duration,
) -> Result<Value, String> {
    {
        let mut opt = state.lock().unwrap();
        if opt.is_none() {
            *opt = Some(spawn()?);
        }
    }
    let mut opt = state.lock().unwrap();
    let engine = opt.as_mut().ok_or("engine unavailable")?;
    engine.call(cmd, params, timeout)
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
