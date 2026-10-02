use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Manager, WebviewWindow};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const ADDRESS: &str = "127.0.0.1:39127";
const LIMIT: usize = 3 * 1024 * 1024;
type Pending = Arc<Mutex<HashMap<String, mpsc::Sender<Value>>>>;
pub struct Mcp {
    running: Option<Arc<AtomicBool>>,
    token: String,
    token_path: PathBuf,
    generation: String,
    worker: Option<std::thread::JoinHandle<()>>,
    pub pending: Pending,
}
fn save_token(path: &Path, token: &str) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(token.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
impl Mcp {
    pub fn load(directory: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        let token_path = directory.join("mcp-token");
        let token = match std::fs::read_to_string(&token_path) {
            Ok(token) => {
                uuid::Uuid::parse_str(&token).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid saved MCP token")
                })?;
                token
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let token = uuid::Uuid::new_v4().to_string();
                save_token(&token_path, &token)?;
                token
            }
            Err(error) => return Err(error),
        };
        Ok(Self {
            worker: None,
            running: None,
            token,
            token_path,
            generation: uuid::Uuid::new_v4().to_string(),
            pending: Arc::default(),
        })
    }
    fn regenerate_token(&mut self) -> Result<(), String> {
        if self.running.is_some() {
            return Err("Turn MCP off before regenerating its token".into());
        }
        let token = uuid::Uuid::new_v4().to_string();
        save_token(&self.token_path, &token).map_err(|e| e.to_string())?;
        self.token = token;
        Ok(())
    }
}
#[tauri::command]
pub fn mcp_regenerate_token(
    window: WebviewWindow,
    state: tauri::State<'_, Mutex<Mcp>>,
) -> Result<Value, String> {
    super::allowed(&window)?;
    let mut state = state.lock().map_err(|e| e.to_string())?;
    state.regenerate_token()?;
    Ok(info(&state))
}
fn info(state: &Mcp) -> Value {
    json!({"enabled":state.running.is_some(), "url":format!("http://{ADDRESS}/mcp"), "token":state.token, "generation":state.generation})
}
#[tauri::command]
pub fn mcp_status(
    window: WebviewWindow,
    state: tauri::State<'_, Mutex<Mcp>>,
) -> Result<Value, String> {
    super::allowed(&window)?;
    let state = state.lock().map_err(|e| e.to_string())?;
    Ok(info(&state))
}
#[tauri::command]
pub async fn mcp_set(
    window: WebviewWindow,
    state: tauri::State<'_, Mutex<Mcp>>,
    enabled: bool,
) -> Result<Value, String> {
    super::allowed(&window)?;
    let mut state = state.lock().map_err(|e| e.to_string())?;
    if enabled && state.running.is_none() {
        let server =
            Server::http(ADDRESS).map_err(|e| format!("Cannot start MCP on {ADDRESS}: {e}"))?;
        state.generation = uuid::Uuid::new_v4().to_string();
        window
            .eval(format!(
                "window.__tetoricaMcpEnabled = true; window.__tetoricaMcpGeneration = {}",
                json!(state.generation)
            ))
            .map_err(|e| e.to_string())?;
        let running = Arc::new(AtomicBool::new(true));
        state.running = Some(running.clone());
        let token = state.token.clone();
        let generation = state.generation.clone();
        let pending = state.pending.clone();
        state.worker = Some(std::thread::spawn(move || {
            serve(server, window, pending, token, generation, running)
        }));
    } else if !enabled {
        window
            .eval("window.__tetoricaMcpEnabled = false")
            .map_err(|e| e.to_string())?;
        if let Some(running) = state.running.take() {
            running.store(false, Ordering::SeqCst);
        }
        state.pending.lock().unwrap().clear();
        let worker = state.worker.take();
        let response = info(&state);
        // Never hold state or block the UI thread while a pending IPC reply completes.
        drop(state);
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        return Ok(response);
    }
    Ok(info(&state))
}
#[tauri::command]
pub fn mcp_reply(
    window: WebviewWindow,
    state: tauri::State<'_, Mutex<Mcp>>,
    id: String,
    response: Value,
) -> Result<(), String> {
    super::allowed(&window)?;
    let state = state.lock().map_err(|e| e.to_string())?;
    if let Some(sender) = state.pending.lock().unwrap().remove(&id) {
        let _ = sender.send(response);
    }
    Ok(())
}
fn tools() -> Value {
    let path = json!({"type":"string","description":"Absolute virtual project path, e.g. /index.js. Not an OS path."});
    json!({"tools":[
        {"name":"list_files","description":"List files in the open Playground project (including binary metadata).","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"read_file","description":"Read current text including editor changes. Keep projectId and content for write_file.","inputSchema":{"type":"object","properties":{"path":path},"required":["path"],"additionalProperties":false}},
        {"name":"write_file","description":"Create or update a project text file, without running code. Requires exact previous content to avoid overwriting human edits. Project is in memory: user exports cassette to save.","inputSchema":{"type":"object","properties":{"path":path,"projectId":{"type":"string"},"content":{"type":"string"},"expectedContent":{"type":["string","null"],"description":"Exact content from read_file, or null to create a missing file."}},"required":["path","projectId","content","expectedContent"],"additionalProperties":false}}
    ]})
}
fn references() -> Vec<String> {
    let lock: Value = serde_json::from_str(include_str!("../../release.lock.json")).unwrap();
    lock["files"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|p| {
            p.ends_with(".d.ts")
                || *p == "llms.txt"
                || (p.starts_with("examples/")
                    && (p.ends_with(".js") || p.ends_with(".json") || p.ends_with(".md")))
        })
        .cloned()
        .collect()
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn project_dispatch(request: &Value) -> String {
    let missing = json!({"id":request["id"],"response":{"error":"Playground editor bridge is not loaded. Restart with npm run dev (tauri dev --no-dev-server), or use the packaged app. Reloading an HTTP dev-server page does not install the bridge."}});
    format!("if (typeof window.__tetoricaMcpRequest === 'function') {{ window.__tetoricaMcpRequest({request}); }} else {{ window.__TAURI_INTERNALS__.invoke('mcp_reply', {missing}); }}")
}

fn project(
    window: &WebviewWindow,
    pending: &Pending,
    name: &str,
    arguments: Value,
    generation: &str,
) -> Result<Value, String> {
    super::allowed(window)?;
    let id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(id.clone(), tx);
    let deadline = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 5000;
    let request = json!({"id":id,"name":name,"arguments":arguments,"deadline":deadline,"generation":generation});
    let result = window.eval(project_dispatch(&request)).map_err(|e|e.to_string()).and_then(|_| rx.recv_timeout(Duration::from_secs(6)).map_err(|_|"Playground editor IPC did not respond within 6 seconds. No automatic retry was performed.".into()));
    pending.lock().unwrap().remove(&id);
    result
}
fn rpc(
    request: Value,
    project: impl FnOnce(&str, Value) -> Result<Value, String>,
    asset: impl FnOnce(String) -> Option<String>,
) -> Option<Value> {
    let id = request.get("id").cloned();
    if request["jsonrpc"] != "2.0" || !request["method"].is_string() {
        return Some(error(id.unwrap_or(Value::Null), -32600, "Invalid Request"));
    }
    let method = request["method"].as_str().unwrap();
    let id = id?; // Notifications have no response.
    let params = &request["params"];
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"tetorica-playground","version":env!("CARGO_PKG_VERSION")},"instructions":"Read API/type/example resources before writing. list_files/read_file access the current in-memory project. write_file requires projectId and expectedContent; does not execute code or save to disk."})
        }
        "ping" => json!({}),
        "tools/list" => tools(),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            if !["list_files", "read_file", "write_file"].contains(&name) {
                return Some(error(id, -32602, "Unknown tool"));
            }
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if !args.is_object() {
                return Some(error(id, -32602, "arguments must be an object"));
            }
            let reply = project(name, args).unwrap_or_else(|e| json!({"error":e}));
            let failed = reply.get("error").is_some();
            json!({"content":[{"type":"text","text":if failed {reply["error"].as_str().unwrap_or("Project error").to_string()} else {reply["result"].to_string()}}],"isError":failed})
        }
        "resources/list" => {
            json!({"resources":references().iter().map(|p|json!({"uri":format!("tetorica://reference/{p}"),"name":p,"mimeType":"text/plain"})).collect::<Vec<_>>()})
        }
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap_or("");
            let path = uri.strip_prefix("tetorica://reference/").unwrap_or("");
            if !references().iter().any(|p| p == path) {
                return Some(error(id, -32602, "Unknown reference resource"));
            }
            let Some(text) = asset(path.to_string()) else {
                return Some(error(id, -32603, "Reference unavailable"));
            };
            json!({"contents":[{"uri":uri,"mimeType":"text/plain","text":text}]})
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
fn gate(host: &str, origin: bool, authorization: &str, token: &str) -> bool {
    host == ADDRESS && !origin && authorization == format!("Bearer {token}")
}
fn serve(
    server: Server,
    window: WebviewWindow,
    pending: Pending,
    token: String,
    generation: String,
    running: Arc<AtomicBool>,
) {
    while running.load(Ordering::SeqCst) {
        let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) else {
            continue;
        };
        handle_http(
            request,
            &token,
            &running,
            |name, args| project(&window, &pending, name, args, &generation),
            |path| {
                window
                    .app_handle()
                    .asset_resolver()
                    .get(path)
                    .and_then(|a| String::from_utf8(a.bytes).ok())
            },
        );
    }
}
fn handle_http(
    mut request: tiny_http::Request,
    token: &str,
    running: &AtomicBool,
    project_handler: impl FnOnce(&str, Value) -> Result<Value, String>,
    asset_handler: impl FnOnce(String) -> Option<String>,
) {
    let header = |name: &'static str| {
        request
            .headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.as_str().to_string())
    };
    let send = |request: tiny_http::Request, status: u16, body: Option<Value>| {
        let mut response = Response::from_string(body.map(|v| v.to_string()).unwrap_or_default())
            .with_status_code(StatusCode(status));
        response.add_header(Header::from_bytes("Content-Type", "application/json").unwrap());
        let _ = request.respond(response);
    };
    if !gate(
        &header("Host").unwrap_or_default(),
        header("Origin").is_some(),
        &header("Authorization").unwrap_or_default(),
        &token,
    ) {
        send(request, 403, None);
        return;
    }
    if request.url() != "/mcp" {
        send(request, 404, None);
        return;
    }
    if request.method() != &Method::Post {
        send(request, 405, None);
        return;
    }
    if let Some(version) = header("MCP-Protocol-Version") {
        if version != "2025-06-18" {
            send(request, 400, None);
            return;
        }
    }
    if !header("Content-Type")
        .unwrap_or_default()
        .starts_with("application/json")
    {
        send(request, 415, None);
        return;
    }
    if request.body_length().is_some_and(|len| len > LIMIT) {
        send(request, 413, None);
        return;
    }
    let mut bytes = Vec::new();
    if request
        .as_reader()
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > LIMIT
    {
        send(request, 413, None);
        return;
    }
    if !running.load(Ordering::SeqCst) {
        send(request, 503, None);
        return;
    }
    let response = match serde_json::from_slice(&bytes) {
        Ok(value) => rpc(value, project_handler, asset_handler),
        Err(_) => Some(error(Value::Null, -32700, "Parse error")),
    };
    send(
        request,
        if response.is_some() { 200 } else { 202 },
        response,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(method: &str, params: Value) -> Value {
        json!({"jsonrpc":"2.0","id":1,"method":method,"params":params})
    }
    #[test]
    fn token_survives_restarts_and_explicit_rotation_revokes_old_token() {
        let directory =
            std::env::temp_dir().join(format!("tetorica-token-{}", uuid::Uuid::new_v4()));
        let mut state = Mcp::load(&directory).unwrap();
        let original = state.token.clone();
        let reloaded = Mcp::load(&directory).unwrap();
        assert_eq!(original, reloaded.token);
        assert_ne!(state.generation, reloaded.generation);
        state.running = Some(Arc::new(AtomicBool::new(true)));
        assert!(state.regenerate_token().is_err());
        assert_eq!(state.token, original);
        state.running = None;
        state.regenerate_token().unwrap();
        assert_ne!(state.token, original);
        assert_eq!(Mcp::load(&directory).unwrap().token, state.token);
        assert!(!gate(
            ADDRESS,
            false,
            &format!("Bearer {original}"),
            &state.token
        ));
        assert!(gate(
            ADDRESS,
            false,
            &format!("Bearer {}", state.token),
            &state.token
        ));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&state.token_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        // A failed save must preserve the token already in use.
        let saved = state.token.clone();
        state.token_path = directory.join("missing").join("mcp-token");
        assert!(state.regenerate_token().is_err());
        assert_eq!(state.token, saved);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn missing_bridge_dispatch_has_an_explicit_ipc_reply() {
        let script = project_dispatch(
            &json!({"id":"request-1","name":"read_file","arguments":{"path":"/index.js"}}),
        );
        assert!(script.contains("typeof window.__tetoricaMcpRequest === 'function'"));
        assert!(script.contains("invoke('mcp_reply'"));
        assert!(script.contains("\"id\":\"request-1\""));
        assert!(script.contains("editor bridge is not loaded"));
    }
    #[test]
    fn protocol_and_references() {
        let reply = rpc(
            request("initialize", json!({})),
            |_, _| panic!(),
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
        assert!(reply["result"]["capabilities"]["tools"].is_object());
        assert!(rpc(
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            |_, _| panic!(),
            |_| panic!()
        )
        .is_none());
        let refs = rpc(
            request("resources/list", json!({})),
            |_, _| panic!(),
            |_| panic!(),
        )
        .unwrap();
        let resources = refs["result"]["resources"].as_array().unwrap();
        assert!(resources
            .iter()
            .any(|r| r["name"].as_str().unwrap().ends_with(".d.ts")));
        assert!(resources
            .iter()
            .any(|r| r["name"].as_str().unwrap().starts_with("examples/")));
        let read = rpc(
            request(
                "resources/read",
                json!({"uri":"tetorica://reference/../../secret"}),
            ),
            |_, _| panic!(),
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(read["error"]["code"], -32602);
    }
    #[test]
    fn forwards_tools_and_reports_conflicts() {
        let reply = rpc(
            request(
                "tools/call",
                json!({"name":"read_file","arguments":{"path":"/index.js"}}),
            ),
            |name, args| {
                assert_eq!(name, "read_file");
                assert_eq!(args["path"], "/index.js");
                Ok(json!({"result":{"content":"test"}}))
            },
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], false);
        let failed = rpc(
            request("tools/call", json!({"name":"write_file","arguments":{}})),
            |_, _| Ok(json!({"error":"File changed"})),
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(failed["result"]["isError"], true);
        let unknown = rpc(
            request("tools/call", json!({"name":"execute_code"})),
            |_, _| panic!(),
            |_| panic!(),
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }
    #[test]
    fn local_authenticated_clients_only() {
        assert!(gate(ADDRESS, false, "Bearer secret", "secret"));
        assert!(!gate("evil.example", false, "Bearer secret", "secret"));
        assert!(!gate(ADDRESS, true, "Bearer secret", "secret"));
        assert!(!gate(ADDRESS, false, "Bearer wrong", "secret"));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use std::{io::Write, net::TcpStream};
    #[test]
    fn http_handshake_tools_resources_and_rejections() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..7 {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                handle_http(
                    request,
                    "test-token",
                    &AtomicBool::new(true),
                    |name, args| {
                        assert_eq!(name, "read_file");
                        assert_eq!(args["path"], "/index.js");
                        Ok(json!({"result":{"content":"live text"}}))
                    },
                    |path| {
                        assert_eq!(path, "llms.txt");
                        Some("API references".into())
                    },
                );
            }
        });
        let send = |method: &str, extra: &str, body: Value| {
            let body = body.to_string();
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            write!(stream,"{method} /mcp HTTP/1.1\r\nHost: {ADDRESS}\r\nAuthorization: Bearer test-token\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n{extra}\r\n{body}",body.len()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            response
        };
        let init = send(
            "POST",
            "",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
        );
        assert!(init.starts_with("HTTP/1.1 200"));
        assert!(init.contains("protocolVersion"));
        assert!(send(
            "POST",
            "",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .starts_with("HTTP/1.1 202"));
        assert!(send("POST","",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/index.js"}}})).contains("live text"));
        assert!(send("POST","",json!({"jsonrpc":"2.0","id":3,"method":"resources/read","params":{"uri":"tetorica://reference/llms.txt"}})).contains("API references"));
        assert!(send("GET", "", Value::Null).starts_with("HTTP/1.1 405"));
        assert!(
            send("POST", "Origin: https://example.com\r\n", json!({})).starts_with("HTTP/1.1 403")
        );
        assert!(send("POST", "MCP-Protocol-Version: invalid\r\n", json!({}))
            .starts_with("HTTP/1.1 400"));
        worker.join().unwrap();
    }
}
