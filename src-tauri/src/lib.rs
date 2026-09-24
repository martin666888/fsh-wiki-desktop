use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener, Manager, Position, Size, WebviewUrl, WindowEvent};
use tauri::webview::{Color, WebviewBuilder};

const TAB_BAR_HEIGHT: f64 = 42.0;
const DEFAULT_URL: &str = "https://www.feishu.cn/drive/home/";
const INIT_JS: &str = include_str!("init.js");
// 设置窗尺寸：居中定位与建窗必须共用同一组值，否则双屏下窗口位置会漂
const SETTINGS_W: f64 = 430.0;
const SETTINGS_H: f64 = 480.0;

// 所有运行数据（WebView 缓存、登录态、字体配置）都收进安装目录下的 data/，
// 不散到 AppData：删安装目录即删干净
fn data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("data")))
        .unwrap_or_else(|| PathBuf::from("data"))
}

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

fn font_config_path() -> PathBuf {
    data_dir().join("fonts.json")
}

fn load_font_config() -> FontConfig {
    fs::read_to_string(font_config_path())
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

// document-start 注入：初始化脚本跑在 HTML 解析之前，那时 head/documentElement
// 可能还不存在，用 MutationObserver 等文档根出现再插 <style>，赶在首帧前生效
fn font_init_js(css: &str) -> String {
    format!(
        r#"(function(){{var CSS={css};function apply(){{var d=document;var el=d.getElementById('feishu-font-force');if(!el){{var host=d.head||d.documentElement;if(!host)return false;el=d.createElement('style');el.id='feishu-font-force';host.appendChild(el);}}if(el.textContent!==CSS)el.textContent=CSS;return true;}}if(apply())return;var mo=new MutationObserver(function(){{if(apply())mo.disconnect();}});mo.observe(document,{{childList:true,subtree:true}});}})();"#,
        css = serde_json::to_string(css).unwrap_or_default()
    )
}

fn apply_font_config(app: &AppHandle) {
    let cfg = load_font_config();
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
fn get_font_config() -> FontConfig {
    load_font_config()
}

// 取应用版本号给「关于」页显示。用 package_info 而不是 env!("CARGO_PKG_VERSION")：
// 前者来自 tauri.conf.json 编译进去的版本，也就是 updater 比对时用的那个，保证一致。
#[tauri::command]
fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
fn set_font_config(app: AppHandle, config: FontConfig) -> Result<(), String> {
    let path = font_config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, serde_json::to_string_pretty(&config).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    apply_font_config(&app);
    Ok(())
}

// 必须 async：同步命令在主线程重入创建 WebView2 控制器会产出空白 WebView
#[tauri::command]
async fn open_settings(app: AppHandle) {
    // 设置窗要落在主窗口中心：Tauri 新窗口默认位置与主显示器绑定，
    // 双屏下会跑到左屏原点，必须按主窗口外框坐标手动算
    let pos = app.get_window("main").and_then(|main| {
        let scale = main.scale_factor().ok()?;
        let outer = main.outer_position().ok()?;
        let size = main.outer_size().ok()?;
        let (w, h) = (SETTINGS_W * scale, SETTINGS_H * scale);
        Some((
            (outer.x as f64 + (size.width as f64 - w) / 2.0) / scale,
            (outer.y as f64 + (size.height as f64 - h) / 2.0) / scale,
        ))
    });

    if let Some(w) = app.get_webview_window("settings") {
        if let Some((x, y)) = pos {
            let _ = w.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(x, y)));
        }
        let _ = w.set_focus();
        return;
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        WebviewUrl::App("settings.html".into()),
    )
    .title("设置")
    .inner_size(SETTINGS_W, SETTINGS_H)
    .resizable(false)
    .data_directory(data_dir().join("webview"));
    if let Some((x, y)) = pos {
        builder = builder.position(x, y);
    }
    let _w = builder.build();
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

    // 建 WebView 前读配置，把 CSS 烘进 document-start 初始化脚本：
    // 首帧之前字体就位，不再等 NavigationCompleted 后的 eval
    let font_css = {
        let cfg = load_font_config();
        if cfg.enabled { cfg.css() } else { String::new() }
    };
    let mut builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .background_color(Color(255, 255, 255, 255))
        .data_directory(data_dir().join("webview"))
        .initialization_script(INIT_JS);
    if !font_css.is_empty() {
        builder = builder.initialization_script(font_init_js(&font_css));
    }
    let builder = builder
        // 兜底：烘进脚本的 CSS 是建 Tab 那一刻的快照，设置页改字体后
        // 已存在的 Tab 再整页导航会先命中旧快照，这里用最新配置纠正
        .on_page_load(|webview, payload| {
            if let tauri::webview::PageLoadEvent::Finished = payload.event() {
                let cfg = load_font_config();
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
    let state = app.state::<TabState>();
    let is_last = {
        let tabs = state.tabs.lock().unwrap();
        tabs.len() == 1 && tabs.first().map(String::as_str) == Some(label)
    };
    // 内容区不允许零 WebView：关最后一个 Tab 时先补新首页 Tab 再销毁旧的，
    // 否则裸窗口客户区会露出未绘制区域（黑块残影）
    if is_last {
        let _ = spawn_tab(app, DEFAULT_URL);
    }
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
    let state = app.state::<TabState>();
    let tabs = state.tabs.lock().unwrap().clone();
    let active = state.active.lock().unwrap().clone();
    TabsSnapshot { tabs, active }
}

// 启动时检查更新：有新版就下载 → 验签 → 跑 NSIS 安装器 → 自动重启。
//
// Windows 上 download_and_install 在启动安装器后会自己 std::process::exit(0)
// （插件内部行为，见 tauri-plugin-updater/src/updater.rs），安装器再带 /R 把应用拉起来，
// 所以这里不需要手动 restart。
// 默认 installMode = passive：NSIS 以 /P /UPDATE /R 运行，只显示进度条、不弹交互。
//
// 只在 release 构建启用：dev 跑的是 debug 版，同样会去拉线上 release，没必要。
#[cfg(not(debug_assertions))]
fn spawn_update_check(app: AppHandle) {
    use tauri_plugin_updater::UpdaterExt;

    tauri::async_runtime::spawn(async move {
        let updater = match app.updater() {
            Ok(u) => u,
            Err(e) => {
                eprintln!("[updater] 初始化失败: {e}");
                return;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => {
                eprintln!(
                    "[updater] 发现新版本 {}（当前 {}），开始下载",
                    update.version, update.current_version
                );
                if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
                    eprintln!("[updater] 下载或安装失败: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => eprintln!("[updater] 检查更新失败: {e}"),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            app_version,
            set_font_config,
            open_settings
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
            let ui_builder = WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()))
                .background_color(Color(232, 234, 237, 255))
                .data_directory(data_dir().join("webview"));
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

            #[cfg(not(debug_assertions))]
            spawn_update_check(handle.clone());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
