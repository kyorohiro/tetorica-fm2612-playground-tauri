#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
        .invoke_handler(tauri::generate_handler![window_top_get, window_top_set,])
        .setup(|app| {
            app.set_menu(Menu::default(app.handle())?)?;
            Ok(())
        })
        .run(context)
        .expect("failed to run Tetorica Playground");
}
