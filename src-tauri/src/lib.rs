use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener, Manager, Position, Size, WebviewUrl, WindowEvent};
use tauri::webview::WebviewBuilder;

const TAB_BAR_HEIGHT: f64 = 42.0;
const DEFAULT_URL: &str = "https://www.feishu.cn/drive/home/";
const INIT_JS: &str = include_str!("init.js");

pub struct TabState {
    counter: AtomicUsize,
    tabs: Mutex<Vec<String>>,
    active: Mutex<Option<String>>,
    last_open: Mutex<Option<(String, std::time::Instant)>>,
}

fn handle_open_request(app: &AppHandle, url: &str) {
    {
        let state = app.state::<TabState>();
        let mut last = state.last_open.lock().unwrap();
        if let Some((prev, at)) = last.as_ref() {
            if prev == url && at.elapsed() < std::time::Duration::from_millis(500) {
                return;
            }
        }
        *last = Some((url.to_string(), std::time::Instant::now()));
    }
    match url.parse::<tauri::Url>() {
        Ok(parsed) if parsed.host_str().is_some_and(is_feishu_host) => {
            let _ = spawn_tab(app, url);
        }
        Ok(_) => open_external(url),
        Err(_) => {}
    }
}

#[derive(Clone, Serialize)]
struct TabCreatedPayload {
    label: String,
    url: String,
}

#[derive(Clone, Serialize)]
struct TabsSnapshot {
    tabs: Vec<String>,
    active: Option<String>,
}

#[derive(Deserialize)]
struct OpenPayload {
    url: String,
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(default)]
struct FontConfig {
    enabled: bool,
    en: String,
    cn: String,
    mono: String,
}

impl FontConfig {
    fn css(&self) -> String {
        let en = quote_family(&self.en, "Cascadia Mono");
        let cn = quote_family(&self.cn, "Microsoft YaHei");
        let want = format!("{en}, {cn}, serif");
        let mono = quote_family(&self.mono, "Cascadia Code");
        format!(
            r#"html, body, #app, #root {{ font-family: {want} !important; }}
*:not(i):not(svg):not(code):not(pre):not(kbd):not([class*="icon"]):not([class*="Icon"]) {{ font-family: {want} !important; }}
code, pre, kbd, .code-block-content {{ font-family: {mono}, Consolas, monospace !important; }}"#
        )
    }
}

fn quote_family(name: &str, fallback: &str) -> String {
    if name.is_empty() {
        format!("\"{fallback}\"")
    } else {
        format!("\"{name}\"")
    }
}

fn font_config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("fonts.json"))
}

fn load_font_config(app: &AppHandle) -> FontConfig {
    let Some(path) = font_config_path(app) else {
        return FontConfig::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn font_apply_js(cfg: &FontConfig) -> String {
    let css = if cfg.enabled { cfg.css() } else { String::new() };
    format!(
        r#"(function(){{try{{var d=document;var el=d.getElementById('feishu-font-force');if(!el){{el=d.createElement('style');el.id='feishu-font-force';(d.head||d.documentElement).appendChild(el);}}el.textContent={css};}}catch(e){{}}}})();"#,
        css = serde_json::to_string(&css).unwrap_or_default()
    )
}

fn apply_font_config(app: &AppHandle) {
    let cfg = load_font_config(app);
    let js = font_apply_js(&cfg);
    for wv in app.webviews().values() {
        if wv.label().starts_with("tab-") {
            let _ = wv.eval(&js);
        }
    }
}

#[tauri::command]
fn list_fonts() -> Vec<String> {
    let mut names = font_loader::system_fonts::query_all();
    names.sort();
    names.dedup();
    names
}

#[tauri::command]
fn get_font_config(app: AppHandle) -> FontConfig {
    load_font_config(&app)
}

#[tauri::command]
fn set_font_config(app: AppHandle, config: FontConfig) -> Result<(), String> {
    let Some(dir) = font_config_path(&app) else {
        return Err("config dir unavailable".into());
    };
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&dir, serde_json::to_string_pretty(&config).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    apply_font_config(&app);
    Ok(())
}

// 必须 async：同步命令在主线程重入创建 WebView2 控制器会产出空白 WebView
#[tauri::command]
async fn open_settings(app: AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.set_focus();
        return;
    }
    let _w = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App("settings.html".into()),
    )
    .title("显示字体设置")
    .inner_size(430.0, 440.0)
    .resizable(false)
    .on_page_load(|webview, payload| {
        if let tauri::webview::PageLoadEvent::Finished = payload.event() {
            eprintln!("[feishu-desktop] settings page loaded: {}", webview.url().map(|u| u.to_string()).unwrap_or_default());
        }
    })
    .build();
}

fn is_feishu_host(host: &str) -> bool {
    host == "feishu.cn"
        || host.ends_with(".feishu.cn")
        || host == "larksuite.com"
        || host.ends_with(".larksuite.com")
}

fn open_external(url: &str) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn content_bounds(window: &tauri::Window) -> Option<(Position, Size)> {
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

fn find_webview(app: &AppHandle, label: &str) -> Option<tauri::Webview> {
    let window = app.get_window("main")?;
    window.webviews().into_iter().find(|w| w.label() == label)
}

fn set_active_tab(app: &AppHandle, label: &str) {
    let Some(window) = app.get_window("main") else {
        return;
    };
    let state = app.state::<TabState>();
    let tabs = state.tabs.lock().unwrap().clone();
    if !tabs.iter().any(|l| l == label) {
        return;
    }
    for wv in window.webviews() {
        if !wv.label().starts_with("tab-") {
            continue;
        }
        if wv.label() == label {
            if let Some((p, s)) = content_bounds(&window) {
                let _ = wv.set_position(p);
                let _ = wv.set_size(s);
            }
            let _ = wv.show();
            let _ = wv.set_focus();
        } else {
            let _ = wv.hide();
        }
    }
    *state.active.lock().unwrap() = Some(label.to_string());
    let _ = app.emit("tab-activated", label.to_string());
}

fn spawn_tab(app: &AppHandle, url: &str) -> Result<String, String> {
    let window = app.get_window("main").ok_or("main window not found")?;
    let parsed = url
        .parse::<tauri::Url>()
        .map_err(|e| e.to_string())?;
    let (pos, size) = content_bounds(&window).ok_or("window metrics unavailable")?;

    let app_for_nw = app.clone();
    let state = app.state::<TabState>();
    let n = state.counter.fetch_add(1, Ordering::SeqCst);
    let label = format!("tab-{n}");

    let font_app = app.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .initialization_script(INIT_JS)
        .on_page_load(move |webview, payload| {
            if let tauri::webview::PageLoadEvent::Finished = payload.event() {
                let cfg = load_font_config(&font_app);
                if cfg.enabled {
                    let _ = webview.eval(&font_apply_js(&cfg));
                }
            }
        })
        .on_navigation(|u| {
            match u.host_str() {
                Some(host) if is_feishu_host(host) => true,
                Some(_) => {
                    open_external(u.as_str());
                    false
                }
                None => true,
            }
        })
        .on_new_window(move |url, _features| {
            let u = url.as_str().to_string();
            let app = app_for_nw.clone();
            std::thread::spawn(move || handle_open_request(&app, &u));
            tauri::webview::NewWindowResponse::Deny
        });

    window
        .add_child(builder, pos, size)
        .map_err(|e| e.to_string())?;

    state.tabs.lock().unwrap().push(label.clone());
    let _ = app.emit(
        "tab-created",
        TabCreatedPayload {
            label: label.clone(),
            url: url.to_string(),
        },
    );
    set_active_tab(app, &label);
    Ok(label)
}

fn do_close_tab(app: &AppHandle, label: &str) {
    if let Some(wv) = find_webview(app, label) {
        let _ = wv.hide();
        let _ = wv.close();
    }
    let state = app.state::<TabState>();
    let was_active = state.active.lock().unwrap().as_deref() == Some(label);
    let next = {
        let mut tabs = state.tabs.lock().unwrap();
        if let Some(i) = tabs.iter().position(|l| l == label) {
            tabs.remove(i);
        }
        tabs.last().cloned()
    };
    let _ = app.emit("tab-closed", label.to_string());
    if was_active {
        match next {
            Some(next) => set_active_tab(app, &next),
            None => *state.active.lock().unwrap() = None,
        }
    }
    // 内容区不允许零 WebView：关掉最后一个 Tab 时自动补一个新首页 Tab，
    // 否则裸窗口客户区会露出未绘制区域（黑块残影）
    if state.tabs.lock().unwrap().is_empty() {
        let _ = spawn_tab(app, DEFAULT_URL);
    }
}

// 涉及 WebView 增删的命令必须保持 async：add_child/close 会 hop 到主线程并阻塞等待，
// 在主线程（同步命令）调用会自死锁。
#[tauri::command]
async fn create_tab(app: AppHandle, url: Option<String>) -> Result<String, String> {
    let url = url.unwrap_or_else(|| DEFAULT_URL.to_string());
    let result = spawn_tab(&app, &url);
    result
}

#[tauri::command]
async fn close_tab(app: AppHandle, label: String) {
    do_close_tab(&app, &label);
}

#[tauri::command]
async fn activate_tab(app: AppHandle, label: String) {
    set_active_tab(&app, &label);
}

#[tauri::command]
fn list_tabs(app: AppHandle) -> TabsSnapshot {
    eprintln!("[feishu-desktop] list_tabs");
    let state = app.state::<TabState>();
    let tabs = state.tabs.lock().unwrap().clone();
    let active = state.active.lock().unwrap().clone();
    eprintln!("[feishu-desktop] list_tabs -> {tabs:?} active={active:?}");
    TabsSnapshot { tabs, active }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(TabState {
            counter: AtomicUsize::new(1),
            tabs: Mutex::new(Vec::new()),
            active: Mutex::new(None),
            last_open: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            create_tab,
            close_tab,
            activate_tab,
            list_tabs,
            list_fonts,
            get_font_config,
            set_font_config,
            open_settings
        ])
        .setup(|app| {
            let handle = app.handle().clone();

            let window = tauri::WindowBuilder::new(app, "main")
                .title("飞书文档")
                .inner_size(1280.0, 860.0)
                .min_inner_size(800.0, 600.0)
                .center()
                .build()?;

            // 先建首页 Tab 再建 UI：UI 加载后 list_tabs 才能拿到初始 Tab
            // （tab-created 事件发出时 UI 监听器尚未就绪，顺序不能反）
            let _ = spawn_tab(&handle, DEFAULT_URL);

            let (bar_pos, bar_size) = {
                let scale = window.scale_factor()?;
                let inner = window.inner_size()?;
                let bar_h = (TAB_BAR_HEIGHT * scale).round() as u32;
                (
                    Position::Physical(tauri::PhysicalPosition::new(0, 0)),
                    Size::Physical(tauri::PhysicalSize::new(inner.width, bar_h)),
                )
            };
            let ui_builder = WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()));
            let ui = window.add_child(ui_builder, bar_pos, bar_size)?;
            let _ = ui.set_focus();

            {
                let h = handle.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Resized(_) = event {
                        let Some(win) = h.get_window("main") else {
                            return;
                        };
                        let Ok(scale) = win.scale_factor() else {
                            return;
                        };
                        let Ok(inner) = win.inner_size() else {
                            return;
                        };
                        let bar_h = (TAB_BAR_HEIGHT * scale).round() as u32;
                        if let Some(ui) = find_webview(&h, "ui") {
                            let _ = ui.set_size(Size::Physical(tauri::PhysicalSize::new(
                                inner.width, bar_h,
                            )));
                        }
                        let state = h.state::<TabState>();
                        let active = state.active.lock().unwrap().clone();
                        if let Some(label) = active {
                            if let Some(wv) = find_webview(&h, &label) {
                                if let Some((p, s)) = content_bounds(&win) {
                                    let _ = wv.set_position(p);
                                    let _ = wv.set_size(s);
                                }
                            }
                        }
                    }
                });
            }

            let h2 = handle.clone();
            app.listen("feishu-open", move |event| {
                if let Ok(payload) = serde_json::from_str::<OpenPayload>(event.payload()) {
                    handle_open_request(&h2, &payload.url);
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
