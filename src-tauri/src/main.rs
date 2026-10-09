#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod mcp;

use tauri::{menu::Menu, Manager, WebviewWindow};

fn allowed(window: &WebviewWindow) -> Result<(), String> {
    let url = window.url().map_err(|e| e.to_string())?;
    let local = url.scheme() == "tauri"
        || (matches!(url.scheme(), "http" | "https") && url.host_str() == Some("tauri.localhost"));
    let development = cfg!(debug_assertions)
        && window
            .app_handle()
            .config()
            .build
            .dev_url
            .as_ref()
            .is_some_and(|dev| dev.origin() == url.origin());
    if window.label() == "main" && (local || development) {
        Ok(())
    } else {
        Err("Desktop controls is available only in the local Playground window".into())
    }
}

fn set_window_top(window: &tauri::WebviewWindow, enabled: bool) -> Result<bool, String> {
    window
        .set_always_on_top(enabled)
        .map_err(|e| e.to_string())?;
    // macOS applies the window level asynchronously. An immediate getter can
    // still report the previous level; acknowledge the accepted request instead.
    Ok(enabled)
}

#[tauri::command]
fn window_top_get(window: tauri::WebviewWindow) -> Result<bool, String> {
    allowed(&window)?;
    window.is_always_on_top().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_top_set(window: tauri::WebviewWindow, enabled: bool) -> Result<bool, String> {
    allowed(&window)?;
    set_window_top(&window, enabled)
}

#[tauri::command]
fn window_reload(window: WebviewWindow) -> Result<(), String> {
    allowed(&window)?;
    window.reload().map_err(|e| e.to_string())
}

#[tauri::command]
fn window_close(window: WebviewWindow) -> Result<(), String> {
    allowed(&window)?;
    window.destroy().map_err(|e| e.to_string())
}

fn main() {
    let context = tauri::generate_context!();
    let dev_origin = if cfg!(debug_assertions) {
        context
            .config()
            .build
            .dev_url
            .as_ref()
            .map(|url| url.origin().ascii_serialization())
    } else {
        None
    };
    let desktop_script = include_str!("../../desktop/desktop-interface.js").replace(
        "__DESKTOP_DEV_ORIGIN__",
        &serde_json::to_string(&dev_origin).expect("serialize development origin"),
    );
    tauri::Builder::default()
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("desktop-controls")
                .js_init_script(desktop_script)
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            window_top_get,
            window_reload,
            window_close,
            window_top_set,
            mcp::mcp_status,
            mcp::mcp_regenerate_token,
            mcp::mcp_set,
            mcp::mcp_reply,
        ])
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if let Some(webview) = window.app_handle().get_webview_window("main") {
                        // Keep the process alive until the final IndexedDB transaction commits.
                        if webview.eval("void (window.__tetoricaCloseRequested ? window.__tetoricaCloseRequested() : window.__TAURI_INTERNALS__.invoke('window_close'));").is_ok() {
                            api.prevent_close();
                        }
                    }
                }
            }
        })
        .setup(|app| {
            app.manage(std::sync::Mutex::new(mcp::Mcp::load(
                &app.path().app_data_dir()?,
            )?));
            // Extend only the pinned module response; dist and its lock stay untouched.
            tauri::WebviewWindowBuilder::from_config(app, &app.config().app.windows[0])?
                .on_web_resource_request(|request, response| {
                    let local = request.uri().scheme_str() == Some("tauri")
                        || request.uri().host() == Some("tauri.localhost");
                    if local
                        && request.uri().path() == "/playground.js"
                        && response.status().is_success()
                    {
                        let mut bytes = response.body().to_vec();
                        bytes.extend_from_slice(b"\n;\n");
                        bytes.extend_from_slice(include_bytes!(
                            "../../desktop/project-interface.js"
                        ));
                        *response.body_mut() = std::borrow::Cow::Owned(bytes);
                        response.headers_mut().remove("content-length");
                        response.headers_mut().remove("etag");
                    }
                })
                .build()?;
            app.set_menu(Menu::default(app.handle())?)?;
            Ok(())
        })
        .run(context)
        .expect("failed to run Tetorica Playground");
}
