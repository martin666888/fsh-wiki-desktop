mod fonts;
mod layout;
mod native;
mod navigation;
mod storage;
mod tabs;
mod updates;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::webview::{Color, WebviewBuilder};
use tauri::{AppHandle, Emitter, Manager, Position, Size, WebviewUrl, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

const SETTINGS_W: f64 = 430.0;
const SETTINGS_H: f64 = 480.0;
// Suppress only the browser's default menu; Feishu's event handlers still run.
const NO_DEFAULT_MENU_JS: &str =
    r#"document.addEventListener("contextmenu", function (e) { e.preventDefault(); }, true);"#;

fn data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("data")))
        .unwrap_or_else(|| PathBuf::from("data"))
}

#[derive(Serialize, Clone)]
struct UiError {
    message: String,
}

fn report_ui_error(app: &AppHandle, message: String) {
    let _ = app.emit_to("ui", "ui-error", UiError { message });
}

#[derive(Default)]
struct SettingsState {
    tab: Mutex<Option<String>>,
    operations: tauri::async_runtime::Mutex<()>,
}

#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn take_settings_tab(app: AppHandle) -> Option<String> {
    app.state::<SettingsState>().tab.lock().unwrap().take()
}

fn center_settings(app: &AppHandle, settings: &tauri::WebviewWindow) -> Result<(), String> {
    let main = app.get_window("main").ok_or("主窗口未就绪")?;
    let scale = main.scale_factor().map_err(|e| e.to_string())?;
    let outer = main.outer_position().map_err(|e| e.to_string())?;
    let size = main.outer_size().map_err(|e| e.to_string())?;
    let x = outer.x as f64 + (size.width as f64 - SETTINGS_W * scale) / 2.0;
    let y = outer.y as f64 + (size.height as f64 - SETTINGS_H * scale) / 2.0;
    // Do not pass coordinates scaled for the main window as target LogicalPosition.
    settings
        .set_position(Position::Physical(tauri::PhysicalPosition::new(
            x.round() as i32,
            y.round() as i32,
        )))
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_settings(app: AppHandle, tab: Option<String>) -> Result<(), String> {
    if tab
        .as_deref()
        .is_some_and(|tab| !matches!(tab, "font" | "about"))
    {
        return Err("未知设置页面".into());
    }
    let state = app.state::<SettingsState>();
    let _operation = state.operations.lock().await;
    *state.tab.lock().unwrap() = tab.clone();
    if let Some(window) = app.get_webview_window("settings") {
        window.unminimize().map_err(|e| e.to_string())?;
        center_settings(&app, &window)?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        if let Some(tab) = tab {
            app.emit_to("settings", "settings-tab", tab)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let window =
        tauri::WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
            .title("设置")
            .inner_size(SETTINGS_W, SETTINGS_H)
            .resizable(false)
            .minimizable(false)
            .visible(false)
            .skip_taskbar(true)
            .initialization_script(NO_DEFAULT_MENU_JS)
            .data_directory(data_dir().join("webview"))
            .build()
            .map_err(|e| format!("无法打开设置：{e}"))?;
    center_settings(&app, &window)?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            app.dialog()
                .message("飞书文档轻客户端已经在运行，无需重复启动。")
                .title("程序已在运行")
                .kind(MessageDialogKind::Info)
                .show(|_| {});
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updates::UpdateState::new())
        .manage(SettingsState::default())
        .manage(tabs::TabState::default())
        .manage(fonts::FontState::new())
        .invoke_handler(tauri::generate_handler![
            tabs::create_tab,
            tabs::close_tab,
            tabs::activate_tab,
            tabs::list_tabs,
            tabs::set_split,
            tabs::choose_split_tab,
            tabs::create_split_page,
            tabs::focus_pane,
            tabs::set_split_ratio,
            tabs::reload_tab,
            tabs::report_workspace_error,
            native::capture_tab_preview,
            native::begin_split_resize,
            fonts::list_fonts,
            fonts::get_font_config,
            fonts::set_font_config,
            app_version,
            open_settings,
            take_settings_tab,
            updates::get_update_status,
            updates::get_update_prefs,
            updates::set_update_prefs,
            updates::check_update,
            updates::download_update,
            updates::install_update,
            updates::cancel_update
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let window = tauri::WindowBuilder::new(app, "main")
                .title("飞书文档")
                .inner_size(1280.0, 860.0)
                .min_inner_size(800.0, 600.0)
                .decorations(false)
                .background_color(Color(255, 255, 255, 255))
                .center()
                .build()?;
            let scale = window.scale_factor()?;
            let inner = window.inner_size()?;
            let ui_builder = WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()))
                .background_color(Color(232, 234, 237, 255))
                .initialization_script(NO_DEFAULT_MENU_JS)
                .data_directory(data_dir().join("webview"));
            window.add_child(
                ui_builder,
                Position::Physical(tauri::PhysicalPosition::new(0, 0)),
                Size::Physical(tauri::PhysicalSize::new(
                    inner.width,
                    (tabs::TAB_BAR_HEIGHT * scale).round() as u32,
                )),
            )?;
            // The local workspace owns pane headers, the picker, and the divider.
            // Content WebViews are created later and positioned above this surface.
            let workspace =
                WebviewBuilder::new("workspace", WebviewUrl::App("workspace.html".into()))
                    .background_color(Color(249, 249, 249, 255))
                    .initialization_script(NO_DEFAULT_MENU_JS)
                    .data_directory(data_dir().join("webview"));
            window.add_child(
                workspace,
                tauri::LogicalPosition::new(0.0, tabs::TAB_BAR_HEIGHT),
                tauri::LogicalSize::new(
                    inner.width as f64 / scale,
                    (inner.height as f64 / scale - tabs::TAB_BAR_HEIGHT).max(1.0),
                ),
            )?;
            let event_app = handle.clone();
            window.on_window_event(move |event| match event {
                WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                    tabs::resize(&event_app);
                }
                WindowEvent::CloseRequested { .. } => {
                    if let Some(settings) = event_app.get_webview_window("settings") {
                        let _ = settings.close();
                    }
                }
                _ => {}
            });
            let startup_app = handle.clone();
            // WebView add/close hop to the UI thread: perform them from an async worker.
            tauri::async_runtime::spawn(async move {
                if let Err(error) = tabs::create_tab(startup_app.clone(), None).await {
                    report_ui_error(&startup_app, error);
                }
            });
            updates::restore_pending_on_boot(&handle);
            #[cfg(not(debug_assertions))]
            updates::spawn_update_boot(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
