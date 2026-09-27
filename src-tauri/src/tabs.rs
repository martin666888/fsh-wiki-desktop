use std::sync::Mutex;
use std::time::Instant;

use serde::Serialize;
use tauri::webview::{Color, WebviewBuilder};
use tauri::{AppHandle, Emitter, Manager, Position, Size, WebviewUrl};

use crate::{data_dir, fonts, navigation, report_ui_error, NO_DEFAULT_MENU_JS};

pub(crate) const TAB_BAR_HEIGHT: f64 = 42.0;
pub(crate) const DEFAULT_URL: &str = "https://www.feishu.cn/drive/home/";

#[derive(Clone, Serialize)]
pub(crate) struct TabInfo {
    label: String,
    title: String,
}

#[derive(Clone, Serialize, Default)]
pub(crate) struct TabsSnapshot {
    tabs: Vec<TabInfo>,
    active: Option<String>,
    revision: u64,
}

#[derive(Default)]
pub(crate) struct TabState {
    snapshot: Mutex<TabsSnapshot>,
    counter: std::sync::atomic::AtomicUsize,
    // Never acquired by native callbacks: a worker can be waiting for the UI thread.
    operations: tauri::async_runtime::Mutex<()>,
    last_open: Mutex<Option<(String, Instant)>>,
}

fn publish(app: &AppHandle) {
    let snap = app.state::<TabState>().snapshot.lock().unwrap().clone();
    let title_app = app.clone();
    let _ = app.run_on_main_thread(move || {
        // Read when the UI thread performs the operation, not from a stale queued snapshot.
        let snap = title_app
            .state::<TabState>()
            .snapshot
            .lock()
            .unwrap()
            .clone();
        if let Some(tab) = snap
            .tabs
            .iter()
            .find(|tab| Some(&tab.label) == snap.active.as_ref())
        {
            if let Some(window) = title_app.get_window("main") {
                let _ = window.set_title(&format!("{} - 飞书文档", tab.title));
            }
        }
    });
    let _ = app.emit_to("ui", "tabs-changed", snap);
}

pub(crate) fn content_bounds(window: &tauri::Window) -> Option<(Position, Size)> {
    let scale = window.scale_factor().ok()?;
    let inner = window.inner_size().ok()?;
    let top = (TAB_BAR_HEIGHT * scale).round() as i32;
    Some((
        Position::Physical(tauri::PhysicalPosition::new(0, top)),
        Size::Physical(tauri::PhysicalSize::new(
            inner.width,
            inner.height.saturating_sub(top as u32),
        )),
    ))
}

fn set_active(app: &AppHandle, label: &str, focus_content: bool) -> Result<(), String> {
    let window = app.get_window("main").ok_or("主窗口未就绪")?;
    if !app
        .state::<TabState>()
        .snapshot
        .lock()
        .unwrap()
        .tabs
        .iter()
        .any(|tab| tab.label == label)
    {
        return Err("该标签已关闭".into());
    }
    let target = app.get_webview(label).ok_or("标签页面未就绪")?;
    if let Some((position, size)) = content_bounds(&window) {
        target.set_position(position).map_err(|e| e.to_string())?;
        target.set_size(size).map_err(|e| e.to_string())?;
    }
    target.show().map_err(|e| e.to_string())?;
    for webview in window.webviews() {
        if webview.label().starts_with("tab-") && webview.label() != label {
            webview.hide().map_err(|e| e.to_string())?;
        }
    }
    if focus_content {
        target.set_focus().map_err(|e| e.to_string())?;
    } else if let Some(ui) = app.get_webview("ui") {
        ui.set_focus().map_err(|e| e.to_string())?;
    }
    {
        let state = app.state::<TabState>();
        let mut snap = state.snapshot.lock().unwrap();
        snap.active = Some(label.to_owned());
        snap.revision += 1;
    }
    publish(app);
    Ok(())
}

fn spawn_tab(app: &AppHandle, url: &str, focus_content: bool) -> Result<String, String> {
    let parsed = url.parse::<tauri::Url>().map_err(|_| "链接格式无效")?;
    if navigation::classify(&parsed) != navigation::Destination::Document {
        return Err("应用内仅支持飞书或 Lark 的 HTTPS 文档链接".into());
    }
    let window = app.get_window("main").ok_or("主窗口未就绪")?;
    let (position, size) = content_bounds(&window).ok_or("无法获取窗口尺寸")?;
    let state = app.state::<TabState>();
    let n = state
        .counter
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let label = format!("tab-{n}");
    {
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.push(TabInfo {
            label: label.clone(),
            title: "新标签页".into(),
        });
        snap.revision += 1;
    }
    let navigation_app = app.clone();
    let new_window_app = app.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .background_color(Color(255, 255, 255, 255))
        .data_directory(data_dir().join("webview"))
        .initialization_script(NO_DEFAULT_MENU_JS)
        .initialization_script(fonts::current_script(app))
        .on_document_title_changed(|webview, title| {
            let app = webview.app_handle();
            let title: String = title
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect();
            let title = if title.trim().is_empty() {
                "新标签页".into()
            } else {
                title
            };
            {
                let state = app.state::<TabState>();
                let mut snap = state.snapshot.lock().unwrap();
                let Some(tab) = snap
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.label == webview.label())
                else {
                    return;
                };
                if tab.title == title {
                    return;
                }
                tab.title = title;
                snap.revision += 1;
            }
            publish(app);
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                // Always synchronize, including disabled: old document-start snapshots must be cleared.
                let _ = webview.eval(fonts::current_script(webview.app_handle()));
            }
        })
        .on_navigation(move |url| {
            // A fresh WebView can transiently navigate through this inert document.
            if url.as_str() == "about:blank" {
                return true;
            }
            match navigation::classify(url) {
                navigation::Destination::Document => true,
                navigation::Destination::Browser => {
                    if let Err(error) = navigation::open_external(url) {
                        report_ui_error(&navigation_app, error);
                    }
                    false
                }
                navigation::Destination::Denied => {
                    report_ui_error(&navigation_app, "不支持此链接协议，已阻止打开".into());
                    false
                }
            }
        })
        .on_new_window(move |url, _| {
            let app = new_window_app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = open_request(&app, url.as_str()).await {
                    report_ui_error(&app, error);
                }
            });
            tauri::webview::NewWindowResponse::Deny
        });
    if let Err(error) = window.add_child(builder, position, size) {
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.retain(|tab| tab.label != label);
        snap.revision += 1;
        drop(snap);
        publish(app);
        return Err(format!("无法创建文档标签：{error}"));
    }
    set_active(app, &label, focus_content)?;
    Ok(label)
}

async fn open_request(app: &AppHandle, url: &str) -> Result<(), String> {
    let parsed = url.parse::<tauri::Url>().map_err(|_| "链接格式无效")?;
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    {
        let mut last = state.last_open.lock().unwrap();
        if last.as_ref().is_some_and(|(previous, at)| {
            previous == url && at.elapsed() < std::time::Duration::from_millis(500)
        }) {
            return Ok(());
        }
        *last = Some((url.to_owned(), Instant::now()));
    }
    match navigation::classify(&parsed) {
        navigation::Destination::Document => spawn_tab(app, url, true).map(|_| ()),
        navigation::Destination::Browser => navigation::open_external(&parsed),
        navigation::Destination::Denied => Err("不支持此链接协议，已阻止打开".into()),
    }
}

#[tauri::command]
pub(crate) async fn create_tab(app: AppHandle, url: Option<String>) -> Result<String, String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    spawn_tab(&app, url.as_deref().unwrap_or(DEFAULT_URL), true)
}

#[tauri::command]
pub(crate) async fn activate_tab(
    app: AppHandle,
    label: String,
    focus_content: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    set_active(&app, &label, focus_content.unwrap_or(true))
}

#[tauri::command]
pub(crate) async fn close_tab(
    app: AppHandle,
    label: String,
    focus_content: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let snap = state.snapshot.lock().unwrap().clone();
    let Some(index) = snap.tabs.iter().position(|tab| tab.label == label) else {
        return Ok(());
    };
    let focus_content = focus_content.unwrap_or(true);
    // Retain one live content view even when creation of the replacement fails.
    if snap.tabs.len() == 1 {
        spawn_tab(&app, DEFAULT_URL, focus_content)?;
    } else if snap.active.as_deref() == Some(&label) {
        let next = if index + 1 < snap.tabs.len() {
            index + 1
        } else {
            index - 1
        };
        set_active(&app, &snap.tabs[next].label, focus_content)?;
    }
    if let Some(webview) = app.get_webview(&label) {
        webview.close().map_err(|e| format!("无法关闭标签：{e}"))?;
    }
    {
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.retain(|tab| tab.label != label);
        snap.revision += 1;
    }
    publish(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn list_tabs(app: AppHandle) -> TabsSnapshot {
    app.state::<TabState>().snapshot.lock().unwrap().clone()
}

pub(crate) fn resize(app: &AppHandle) {
    let Some(window) = app.get_window("main") else {
        return;
    };
    let Ok(scale) = window.scale_factor() else {
        return;
    };
    let Ok(inner) = window.inner_size() else {
        return;
    };
    if let Some(ui) = app.get_webview("ui") {
        let _ = ui.set_size(Size::Physical(tauri::PhysicalSize::new(
            inner.width,
            (TAB_BAR_HEIGHT * scale).round() as u32,
        )));
    }
    // Update all view bounds, avoiding a stale active snapshot racing with a switch.
    if let Some((position, size)) = content_bounds(&window) {
        for webview in window.webviews() {
            if webview.label().starts_with("tab-") {
                let _ = webview.set_position(position);
                let _ = webview.set_size(size);
            }
        }
    }
}
