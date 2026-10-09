use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, WebviewWindow};

struct Sidecar {
    child: Child,
    input: ChildStdin,
    replies: mpsc::Receiver<Value>,
    sequence: u64,
}
impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default, Clone)]
pub struct Audio(Arc<Mutex<Option<Sidecar>>>);
impl Audio {
    fn request(&self, app: &tauri::AppHandle, op: &str, args: Value) -> Result<Value, String> {
        let mut state = self.0.lock().map_err(|e| e.to_string())?;
        if state.is_none() {
            let root = if cfg!(debug_assertions) {
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../audio-sidecar-bundle")
            } else {
                app.path()
                    .resource_dir()
                    .map_err(|e| e.to_string())?
                    .join("audio-sidecar")
            };
            let executable = root.join(if cfg!(windows) { "node.exe" } else { "node" });
            let mut command = Command::new(executable);
            command
                .arg(root.join("server.mjs"))
                .current_dir(&root)
                .env_remove("NODE_OPTIONS")
                .env_remove("NODE_PATH")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit());
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
            let (send, replies) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(output).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(value) = serde_json::from_str(&line) {
                        if send.send(value).is_err() {
                            break;
                        }
                    }
                }
            });
            *state = Some(Sidecar {
                child,
                input,
                replies,
                sequence: 0,
            });
        }
        let sidecar = state.as_mut().unwrap();
        sidecar.sequence += 1;
        let id = sidecar.sequence;
        let line = json!({"id":id,"op":op,"args":args}).to_string() + "\n";
        let result = (|| {
            sidecar
                .input
                .write_all(line.as_bytes())
                .map_err(|e| e.to_string())?;
            sidecar.input.flush().map_err(|e| e.to_string())?;
            let response = sidecar
                .replies
                .recv_timeout(Duration::from_secs(15))
                .map_err(|e| format!("Audify output did not respond: {e}"))?;
            if response["id"] != id {
                return Err("Invalid Audify reply".into());
            }
            if let Some(error) = response["error"].as_str() {
                return Err(error.to_owned());
            }
            Ok(response["result"].clone())
        })();
        // Recreate the sidecar after any failure; no failed audio stream is retained.
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
