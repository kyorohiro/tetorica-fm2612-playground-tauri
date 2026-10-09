use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, WebviewWindow};

#[derive(Default)]
struct DiagnosticTail(Vec<u8>);
impl DiagnosticTail {
    fn append(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
        if self.0.len() > 8192 {
            self.0.drain(..self.0.len() - 8192);
        }
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0).trim().to_owned()
    }
}
struct Sidecar {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Value>,
    diagnostics: Arc<Mutex<DiagnosticTail>>,
    stderr_done: mpsc::Receiver<()>,
    sequence: u64,
}
impl Sidecar {
    fn launch(mut command: Command) -> Result<Self, String> {
        command
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("Cannot start Audify output: {e}"))?;
        let input = child.stdin.take().ok_or("Missing sidecar input")?;
        let output = child.stdout.take().ok_or("Missing sidecar output")?;
        let mut stderr = child.stderr.take().ok_or("Missing sidecar error output")?;
        let diagnostics = Arc::new(Mutex::new(DiagnosticTail::default()));
        let (done, stderr_done) = mpsc::channel();
        let tail = diagnostics.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 1024];
            loop {
                match stderr.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        if let Ok(mut log) = tail.lock() {
                            log.append(&bytes[..size]);
                        }
                    }
                }
            }
            let _ = done.send(());
        });
        let (send, replies) = mpsc::channel();
        let tail = diagnostics.clone();
        std::thread::spawn(move || {
            // Native Windows drivers may print non-UTF8 diagnostics to stdout.
            // Only JSON replies require UTF8; such logs must not close the IPC reader.
            for line in BufReader::new(output).split(b'\n') {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_slice(&line) {
                    if send.send(value).is_err() {
                        break;
                    }
                } else if let Ok(mut log) = tail.lock() {
                    log.append(&line);
                    log.append(b"\n");
                }
            }
        });
        Ok(Self {
            child,
            input,
            replies,
            diagnostics,
            stderr_done,
            sequence: 0,
        })
    }
    fn failure(&mut self, op: &str, reason: &str) -> String {
        let mut exit = self.child.try_wait().ok().flatten();
        // stdout can close just before the OS reports exit. Wait briefly, never
        // block on a reader join (a descendant could still hold an inherited pipe).
        for _ in 0..5 {
            if exit.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            exit = self.child.try_wait().ok().flatten();
        }
        if exit.is_some() {
            let _ = self.stderr_done.recv_timeout(Duration::from_millis(100));
        }
        let state = exit
            .map(|status| match status.code() {
                Some(code) => format!("Node exited with code {code} (0x{:08X})", code as u32),
                None => format!("Node exited: {status}"),
            })
            .unwrap_or_else(|| "Node is running but its response connection failed".into());
        let tail = self
            .diagnostics
            .lock()
            .map(|log| log.text())
            .unwrap_or_default();
        format!(
            "Audify failed during {op}: {state}. {reason}{}",
            if tail.is_empty() {
                String::new()
            } else {
                format!("\n{tail}")
            }
        )
    }
    fn exchange(&mut self, op: &str, args: Value) -> Result<Value, String> {
        self.sequence += 1;
        let id = self.sequence;
        let line = json!({"id":id,"op":op,"args":args}).to_string() + "\n";
        let result: Result<Value, String> = (|| {
            self.input
                .write_all(line.as_bytes())
                .map_err(|e| e.to_string())?;
            self.input.flush().map_err(|e| e.to_string())?;
            let response = self
                .replies
                .recv_timeout(Duration::from_secs(15))
                .map_err(|e| e.to_string())?;
            if response["id"] != id {
                return Err("Invalid Audify reply".into());
            }
            Ok(response)
        })();
        let response = match result {
            Ok(value) => value,
            Err(error) => return Err(self.failure(op, &error)),
        };
        if let Some(error) = response["error"].as_str() {
            return Err(error.to_owned());
        }
        Ok(response["result"].clone())
    }
}
impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn sidecar_command(root: &Path) -> Result<Command, String> {
    let executable = root.join(if cfg!(windows) { "node.exe" } else { "node" });
    let script = root.join("server.mjs");
    for file in [&executable, &script] {
        if !file.is_file() {
            return Err(format!(
                "Audify resource is missing from the app: {}",
                file.display()
            ));
        }
    }
    let mut command = Command::new(executable);
    // Windows resource_dir can contain a verbatim \\?\ drive prefix. Node's
    // main-module realpath resolution fails on that absolute script argument
    // (nodejs/node#62446). Resolve the entry point relative to the resource cwd.
    command.arg("server.mjs").current_dir(root);
    Ok(command)
}
#[derive(Default, Clone)]
pub struct Audio(Arc<Mutex<Option<Sidecar>>>);
impl Audio {
    fn request(&self, app: &tauri::AppHandle, op: &str, args: Value) -> Result<Value, String> {
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        // Stopping an unused output must not launch a fresh Node process.
        if state.is_none() && op == "stop" {
            return Ok(Value::Null);
        }
        if state.is_none() {
            let root = if cfg!(debug_assertions) {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../audio-sidecar-bundle")
            } else {
                app.path()
                    .resource_dir()
                    .map_err(|e| e.to_string())?
                    .join("audio-sidecar")
            };
            *state = Some(Sidecar::launch(sidecar_command(&root)?)?);
        }
        let result = state.as_mut().unwrap().exchange(op, args);
        if result.is_err() {
            *state = None;
        }
        result
    }
    pub fn shutdown(&self) {
        if let Ok(mut state) = self.0.lock() {
            *state = None;
        }
    }
}
#[tauri::command]
pub async fn audio_request(
    window: WebviewWindow,
    state: tauri::State<'_, Audio>,
    op: String,
    args: Option<Value>,
) -> Result<Value, String> {
    super::allowed(&window)?;
    if !["devices", "start", "stop", "status"].contains(&op.as_str()) {
        return Err("Unsupported audio operation".into());
    }
    let audio = state.inner().clone();
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        audio.request(&app, &op, args.unwrap_or(Value::Null))
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn audio_shutdown(
    window: WebviewWindow,
    state: tauri::State<'_, Audio>,
) -> Result<(), String> {
    super::allowed(&window)?;
    let audio = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || audio.shutdown())
        .await
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(windows)]
    fn bundled_server_starts_from_a_verbatim_windows_resource_path() {
        // canonicalize deliberately produces the \\?\ path used by installed
        // Tauri resources; exercise the same command and IPC as the app.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../audio-sidecar-bundle")
            .canonicalize()
            .unwrap();
        assert!(root.as_os_str().to_string_lossy().starts_with(r"\\?\"));
        let mut child = Sidecar::launch(sidecar_command(&root).unwrap()).unwrap();
        let status = child.exchange("status", Value::Null).unwrap();
        assert_eq!(status["active"], false);
        assert!(child.exchange("devices", Value::Null).unwrap().is_array());
        assert_eq!(child.exchange("stop", Value::Null).unwrap(), Value::Null);
    }
    #[test]
    fn child_exit_preserves_stderr_and_exit_code() {
        #[cfg(windows)]
        let command = {
            let mut c = Command::new("cmd");
            c.args(["/D", "/C", "echo missing_audio.dll 1>&2 & exit /b 42"]);
            c
        };
        #[cfg(not(windows))]
        let command = {
            let mut c = Command::new("sh");
            c.args(["-c", "printf 'missing_audio.dll\\n' >&2; exit 42"]);
            c
        };
        let mut child = Sidecar::launch(command).unwrap();
        let error = child.exchange("devices", Value::Null).unwrap_err();
        assert!(error.contains("during devices"), "{error}");
        assert!(error.contains("code 42"), "{error}");
        assert!(error.contains("missing_audio.dll"), "{error}");
    }
    #[test]
    fn diagnostic_tail_is_bounded_and_keeps_recent_output() {
        let mut log = DiagnosticTail::default();
        log.append(&vec![b'x'; 20000]);
        log.append("日本語 error".as_bytes());
        assert!(log.0.len() <= 8192);
        assert!(log.text().ends_with("日本語 error"));
    }
    #[test]
    fn missing_resources_fail_before_starting_a_process() {
        let root = std::env::temp_dir().join(format!("tetorica-missing-{}", uuid::Uuid::new_v4()));
        let error = sidecar_command(&root).unwrap_err();
        assert!(error.contains("resource is missing"));
    }
    #[test]
    #[cfg(unix)]
    fn valid_reply_is_not_confused_with_stderr_or_native_stdout_logs() {
        let mut command = Command::new("sh");
        command.args(["-c","read -r request; echo 'native warning' >&2; printf '\\377native startup message\\n'; printf '%s\\n' '{\"id\":1,\"result\":[]}'"]);
        let mut child = Sidecar::launch(command).unwrap();
        assert_eq!(child.exchange("devices", Value::Null).unwrap(), json!([]));
    }
}
