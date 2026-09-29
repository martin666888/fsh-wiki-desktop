pub(crate) use crate::layout::TAB_BAR_HEIGHT;
use crate::{
    data_dir, fonts,
    layout::{Layout, Pane, Viewport, PANE_BORDER, PANE_HEADER_HEIGHT},
    native, navigation, report_ui_error, NO_DEFAULT_MENU_JS,
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Mutex,
};
use std::time::Instant;
use tauri::webview::{Color, WebviewBuilder};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl};
pub(crate) const DEFAULT_URL: &str = "https://www.feishu.cn/drive/home/";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TabInfo {
    pub label: String,
    pub title: String,
    pub loading: bool,
    pub error: Option<String>,
    pub generation: u64,
    #[serde(skip)]
    pub navigation_id: u64,
}

#[derive(Clone, Serialize)]
pub(crate) struct TabGroup {
    pub id: String,
    pub layout: Layout,
}

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TabsSnapshot {
    pub tabs: Vec<TabInfo>,
    pub groups: Vec<TabGroup>,
    pub active_group: Option<String>,
    pub active: Option<String>,
    pub revision: u64,
    pub layout: Layout,
    pub viewport: Viewport,
    pub picker_session: u64,
}

#[derive(Default)]
pub(crate) struct TabState {
    pub snapshot: Mutex<TabsSnapshot>,
    counter: AtomicUsize,
    pub operations: tauri::async_runtime::Mutex<()>,
    pub previews: tauri::async_runtime::Mutex<()>,
    pub arranging: AtomicBool,
    pub dragging: AtomicBool,
    resize_pending: AtomicBool,
    last_open: Mutex<Option<(String, String, Instant)>>,
}

pub(crate) fn publish(app: &AppHandle) {
    let app = app.clone();
    let runner = app.clone();
    let _ = runner.run_on_main_thread(move || {
        let snap = app.state::<TabState>().snapshot.lock().unwrap().clone();
        if let Some(tab) = snap
            .tabs
            .iter()
            .find(|tab| Some(tab.label.as_str()) == snap.layout.retained())
        {
            if let Some(window) = app.get_window("main") {
                let _ = window.set_title(&format!("{} - 飞书文档", tab.title));
            }
        }
        let _ = app.emit_to("ui", "tabs-changed", &snap);
        let _ = app.emit_to("workspace", "tabs-changed", &snap);
    });
}

/// Native callbacks never acquire the operation mutex or wait for a UI task.
pub(crate) fn focused(app: &AppHandle, label: &str) {
    let state = app.state::<TabState>();
    if state.arranging.load(Ordering::Acquire) || state.dragging.load(Ordering::Acquire) {
        return;
    }
    let mut snap = state.snapshot.lock().unwrap();
    let Some(pane) = snap.layout.pane(label) else {
        return;
    };
    if pane == snap.layout.focused() {
        return;
    }
    snap.layout.focus(pane);
    sync_group(&mut snap);
    snap.active = snap.layout.active().map(str::to_owned);
    snap.revision += 1;
    drop(snap);
    publish(app);
}

pub(crate) fn navigation_started(app: &AppHandle, label: &str, id: u64) {
    crate::power::navigation(app, label);
    let state = app.state::<TabState>();
    let mut snap = state.snapshot.lock().unwrap();
    if let Some(tab) = snap.tabs.iter_mut().find(|tab| tab.label == label) {
        tab.navigation_id = id;
        tab.loading = true;
        tab.error = None;
        tab.generation += 1;
        snap.revision += 1;
    }
    drop(snap);
    publish(app);
}

pub(crate) fn navigation_finished(app: &AppHandle, label: &str, id: u64, error: Option<String>) {
    crate::power::navigation(app, label);
    let state = app.state::<TabState>();
    let mut snap = state.snapshot.lock().unwrap();
    if let Some(tab) = snap.tabs.iter_mut().find(|tab| tab.label == label) {
        if tab.navigation_id != 0 && id != tab.navigation_id {
            return;
        }
        tab.navigation_id = id;
        tab.loading = false;
        tab.error = error;
        tab.generation += 1;
        snap.revision += 1;
    }
    drop(snap);
    publish(app);
}

fn bounds(webview: &tauri::Webview, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    webview
        .set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(x, y).into(),
            size: tauri::LogicalSize::new(width.max(1.0), height.max(1.0)).into(),
        })
        .map_err(|e| e.to_string())
}

#[derive(Clone, Copy)]
pub(crate) enum Focus {
    Keep,
    Content,
    Toolbar,
    Picker,
}

/// All bounds in one UI turn; the local workspace is below the content views.
fn apply_on_ui(app: &AppHandle, focus: Focus) -> Result<(), String> {
    let window = app.get_window("main").ok_or("主窗口未就绪")?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let width = size.width as f64 / scale;
    let height = (size.height as f64 / scale - TAB_BAR_HEIGHT).max(1.0);
    let state = app.state::<TabState>();
    let snap = {
        let mut snap = state.snapshot.lock().unwrap();
        let viewport = Viewport::calculate(width, height, scale, &snap.layout);
        if snap.viewport != viewport {
            snap.viewport = viewport;
            snap.revision += 1;
        }
        snap.clone()
    };
    if let Some(ui) = app.get_webview("ui") {
        bounds(&ui, 0.0, 0.0, width, TAB_BAR_HEIGHT)?;
    }
    let workspace = app.get_webview("workspace").ok_or("阅读工作区未就绪")?;
    bounds(&workspace, 0.0, TAB_BAR_HEIGHT, width, height)?;
    workspace.show().map_err(|e| e.to_string())?;
    let split = snap.layout.is_split();
    let suspended = snap.viewport.suspended;
    for tab in &snap.tabs {
        let Some(webview) = app.get_webview(&tab.label) else {
            continue;
        };
        let pane = snap.layout.pane(&tab.label);
        let shown = if suspended {
            snap.layout.retained() == Some(tab.label.as_str())
        } else {
            pane.is_some()
        };
        if !shown {
            webview.hide().map_err(|e| e.to_string())?;
            crate::power::visibility(app, &tab.label, false);
            continue;
        }
        let header = if suspended { PANE_HEADER_HEIGHT } else { 0.0 };
        let border = if split { PANE_BORDER } else { 0.0 };
        let (x, pane_width) = if !split || suspended {
            (0.0, width)
        } else if pane == Some(Pane::Left) {
            (0.0, snap.viewport.left_width)
        } else {
            (snap.viewport.right_x, width - snap.viewport.right_x)
        };
        bounds(
            &webview,
            x + border,
            TAB_BAR_HEIGHT + header,
            pane_width - border * 2.0,
            height - header - border,
        )?;
        webview.show().map_err(|e| e.to_string())?;
        crate::power::visibility(app, &tab.label, true);
    }
    match focus {
        Focus::Keep => {}
        Focus::Toolbar => {
            if let Some(ui) = app.get_webview("ui") {
                ui.set_focus().map_err(|e| e.to_string())?;
            }
        }
        Focus::Picker if !suspended && matches!(snap.layout, Layout::Picking { .. }) => {
            workspace.set_focus().map_err(|e| e.to_string())?;
            let _ = app.emit_to("workspace", "focus-picker", snap.picker_session);
        }
        Focus::Picker | Focus::Content => {
            if let Some(label) = snap.layout.active().or_else(|| {
                if suspended {
                    snap.layout.retained()
                } else {
                    None
                }
            }) {
                if let Some(webview) = app.get_webview(label) {
                    webview.set_focus().map_err(|e| e.to_string())?;
                }
            } else {
                workspace.set_focus().map_err(|e| e.to_string())?;
                let _ = app.emit_to("workspace", "focus-picker", snap.picker_session);
            }
        }
    }
    Ok(())
}

pub(crate) async fn apply(app: &AppHandle, focus: Focus) -> Result<(), String> {
    let (send, receive) = tokio::sync::oneshot::channel();
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let state = handle.state::<TabState>();
        state.arranging.store(true, Ordering::Release);
        let result = apply_on_ui(&handle, focus);
        state.arranging.store(false, Ordering::Release);
        let _ = send.send(result);
    })
    .map_err(|e| e.to_string())?;
    receive.await.map_err(|_| "窗口已关闭".to_string())?
}

pub(crate) async fn commit(
    app: &AppHandle,
    layout: Layout,
    focus: Focus,
    new_picker: bool,
) -> Result<(), String> {
    let before = {
        let state = app.state::<TabState>();
        let mut snap = state.snapshot.lock().unwrap();
        let before = snap.layout.clone();
        snap.layout = layout;
        sync_group(&mut snap);
        snap.active = snap.layout.active().map(str::to_owned);
        if new_picker {
            snap.picker_session += 1;
        }
        snap.revision += 1;
        before
    };
    if let Err(error) = apply(app, focus).await {
        {
            let state = app.state::<TabState>();
            let mut snap = state.snapshot.lock().unwrap();
            snap.layout = before;
            sync_group(&mut snap);
            snap.active = snap.layout.active().map(str::to_owned);
            snap.revision += 1;
        }
        let _ = apply(app, Focus::Keep).await;
        publish(app);
        return Err(error);
    }
    publish(app);
    Ok(())
}

fn sync_group(snap: &mut TabsSnapshot) {
    if let Some(group) = snap
        .groups
        .iter_mut()
        .find(|g| Some(&g.id) == snap.active_group.as_ref())
    {
        group.layout = snap.layout.clone();
    }
}

async fn switch_groups(
    app: &AppHandle,
    groups: Vec<TabGroup>,
    active: String,
    focus: Focus,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let layout = groups
        .iter()
        .find(|g| g.id == active)
        .ok_or("标签已关闭")?
        .layout
        .clone();
    let (old_groups, old_active) = {
        let mut snap = state.snapshot.lock().unwrap();
        let old = (snap.groups.clone(), snap.active_group.clone());
        snap.groups = groups;
        snap.active_group = Some(active);
        old
    };
    if let Err(error) = commit(app, layout, focus, true).await {
        let mut snap = state.snapshot.lock().unwrap();
        snap.groups = old_groups;
        snap.active_group = old_active;
        snap.revision += 1;
        drop(snap);
        publish(app);
        return Err(error);
    }
    Ok(())
}

async fn spawn_tab(
    app: &AppHandle,
    url: &str,
    picker_session: Option<u64>,
    focus: Focus,
) -> Result<String, String> {
    let parsed = url.parse::<tauri::Url>().map_err(|_| "链接格式无效")?;
    if navigation::classify(&parsed) != navigation::Destination::Document {
        return Err("应用内仅支持飞书或 Lark 的 HTTPS 文档链接".into());
    }
    let window = app.get_window("main").ok_or("主窗口未就绪")?;
    let state = app.state::<TabState>();
    let label = format!("tab-{}", state.counter.fetch_add(1, Ordering::Relaxed));
    {
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.push(TabInfo {
            label: label.clone(),
            title: "新标签页".into(),
            loading: true,
            error: None,
            generation: 0,
            navigation_id: 0,
        });
        snap.revision += 1;
    }
    let navigation_app = app.clone();
    let new_window_app = app.clone();
    let source_label = label.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed))
        .background_color(Color(255, 255, 255, 255))
        .data_directory(data_dir().join("webview"))
        .initialization_script(NO_DEFAULT_MENU_JS)
        .initialization_script(fonts::current_script(app))
        .focused(false)
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
            drop(snap);
            publish(app);
        })
        .on_page_load(|webview, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = webview.eval(fonts::current_script(webview.app_handle()));
                let state = webview.app_handle().state::<TabState>();
                let mut snap = state.snapshot.lock().unwrap();
                if let Some(tab) = snap
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.label == webview.label())
                {
                    if tab.navigation_id == 0 {
                        tab.loading = false;
                        tab.generation += 1;
                        snap.revision += 1;
                    }
                }
                drop(snap);
                publish(webview.app_handle());
            }
        })
        .on_navigation(move |url| {
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
            let source = source_label.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = open_request(&app, &source, url.as_str()).await {
                    report_ui_error(&app, error);
                }
            });
            tauri::webview::NewWindowResponse::Deny
        });
    // Start offscreen; the previous article stays visible during native creation.
    let created = window.add_child(
        builder,
        tauri::LogicalPosition::new(-10000.0, TAB_BAR_HEIGHT),
        tauri::LogicalSize::new(640.0, 600.0),
    );
    let result = async {
        let webview = created.map_err(|e| format!("无法创建文档标签：{e}"))?;
        native::attach(&webview).await?;
        let snap = state.snapshot.lock().unwrap().clone();
        if let Some(session) = picker_session {
            if session != snap.picker_session || !matches!(snap.layout, Layout::Picking { .. }) {
                return Err("选页状态已改变，请重试".into());
            }
            let mut layout = snap.layout;
            layout.select(label.clone(), Pane::Right);
            commit(app, layout, focus, false).await
        } else {
            let mut groups = snap.groups;
            groups.push(TabGroup {
                id: label.clone(),
                layout: Layout::Single { tab: label.clone() },
            });
            switch_groups(app, groups, label.clone(), focus).await
        }
    }
    .await;
    if let Err(error) = result {
        if let Some(webview) = app.get_webview(&label) {
            let _ = webview.close();
        }
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.retain(|tab| tab.label != label);
        snap.revision += 1;
        drop(snap);
        publish(app);
        return Err(error);
    }
    Ok(label)
}

fn open_request<'a>(
    app: &'a AppHandle,
    source: &'a str,
    url: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        let parsed = url.parse::<tauri::Url>().map_err(|_| "链接格式无效")?;
        let state = app.state::<TabState>();
        let _operation = state.operations.lock().await;
        if !state
            .snapshot
            .lock()
            .unwrap()
            .tabs
            .iter()
            .any(|tab| tab.label == source)
        {
            return Ok(());
        }
        {
            let mut last = state.last_open.lock().unwrap();
            if last
                .as_ref()
                .is_some_and(|(previous_source, previous_url, at)| {
                    previous_source == source
                        && previous_url == url
                        && at.elapsed().as_millis() < 500
                })
            {
                return Ok(());
            }
            *last = Some((source.into(), url.into(), Instant::now()));
        }
        match navigation::classify(&parsed) {
            navigation::Destination::Document => {
                // Feishu's library opens documents with window.open. Inside a split
                // it acts as a document chooser: reuse the originating pane instead.
                // Actual document links and the toolbar's + keep creating normal tabs.
                if let Some(view) = app.get_webview(source) {
                    let reuse = view.url().ok().is_some_and(|url| {
                        let snap = state.snapshot.lock().unwrap();
                        snap.groups
                            .iter()
                            .any(|group| library_opens_in_pane(&group.layout, source, &url))
                    });
                    if reuse {
                        view.navigate(parsed)
                            .map_err(|e| format!("无法在分栏打开文章：{e}"))?;
                        let mut layout = state.snapshot.lock().unwrap().layout.clone();
                        if let Some(pane) = layout.pane(source) {
                            layout.focus(pane);
                            commit(app, layout, Focus::Content, false).await?;
                        }
                        return Ok(());
                    }
                }
                spawn_tab(app, url, None, Focus::Content).await.map(|_| ())
            }
            navigation::Destination::Browser => navigation::open_external(&parsed),
            navigation::Destination::Denied => Err("不支持此链接协议，已阻止打开".into()),
        }
    })
}

fn library_opens_in_pane(layout: &Layout, source: &str, url: &tauri::Url) -> bool {
    matches!(layout, Layout::Split { .. })
        && layout.pane(source).is_some()
        && navigation::classify(url) == navigation::Destination::Document
        && (url.path() == "/drive" || url.path().starts_with("/drive/"))
}

#[tauri::command]
pub(crate) async fn create_tab(app: AppHandle, url: Option<String>) -> Result<String, String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    spawn_tab(
        &app,
        url.as_deref().unwrap_or(DEFAULT_URL),
        None,
        Focus::Content,
    )
    .await
}

#[tauri::command]
pub(crate) async fn activate_tab(
    app: AppHandle,
    label: String,
    focus_content: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let snap = state.snapshot.lock().unwrap().clone();
    switch_groups(
        &app,
        snap.groups,
        label,
        if focus_content.unwrap_or(true) {
            Focus::Content
        } else {
            Focus::Toolbar
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn close_tab(
    app: AppHandle,
    label: String,
    focus_content: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let mut snap = state.snapshot.lock().unwrap().clone();
    let Some(index) = snap.groups.iter().position(|g| g.id == label) else {
        return Ok(());
    };
    let closing = snap.groups[index].layout.clone();
    let was_active = snap.active_group.as_deref() == Some(&label);
    if snap.groups.len() == 1 {
        spawn_tab(&app, DEFAULT_URL, None, Focus::Keep).await?;
        snap = state.snapshot.lock().unwrap().clone();
    }
    let mut groups = snap.groups.clone();
    groups.remove(index);
    let active = if snap.active_group.as_deref() == Some(&label) {
        groups[index.min(groups.len() - 1)].id.clone()
    } else {
        snap.active_group.clone().ok_or("没有当前标签")?
    };
    let focus = if !was_active {
        Focus::Keep
    } else if focus_content.unwrap_or(true) {
        Focus::Content
    } else {
        Focus::Toolbar
    };
    switch_groups(&app, groups, active, focus).await?;
    let mut failed = Vec::new();
    for pane in [Pane::Left, Pane::Right] {
        let Some(document) = closing.document(pane) else {
            continue;
        };
        if let Some(view) = app.get_webview(document) {
            if let Err(error) = view.close() {
                failed.push((document.to_owned(), error.to_string()));
                continue;
            }
        }
        let mut snap = state.snapshot.lock().unwrap();
        snap.tabs.retain(|t| t.label != document);
        snap.revision += 1;
    }
    // If a native close failed, keep the surviving page reachable as a normal tab.
    {
        let mut snap = state.snapshot.lock().unwrap();
        for (document, _) in &failed {
            snap.groups.push(TabGroup {
                id: format!("group-{}", state.counter.fetch_add(1, Ordering::Relaxed)),
                layout: Layout::Single {
                    tab: document.clone(),
                },
            });
        }
        snap.revision += 1;
    }
    publish(&app);
    if let Some((_, error)) = failed.first() {
        return Err(format!("部分页面无法关闭，已保留标签：{error}"));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn list_tabs(app: AppHandle) -> TabsSnapshot {
    app.state::<TabState>().snapshot.lock().unwrap().clone()
}

#[tauri::command]
pub(crate) async fn set_split(app: AppHandle, enabled: bool) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    apply(&app, Focus::Keep).await?;
    let snap = state.snapshot.lock().unwrap().clone();
    if enabled == snap.layout.is_split() {
        return Ok(());
    }
    let mut layout = snap.layout;
    if enabled {
        if !snap.viewport.can_split {
            return Err(format!(
                "窗口太窄，请先扩大到至少 {} 像素宽再分屏",
                crate::layout::MIN_SPLIT_WIDTH as u32
            ));
        }
        layout.enter()?;
    } else {
        let mut groups = snap.groups;
        let active = snap.active_group.ok_or("没有当前标签")?;
        let detached_id = format!("group-{}", state.counter.fetch_add(1, Ordering::Relaxed));
        detach_group(&mut groups, &active, detached_id)?;
        return switch_groups(&app, groups, active, Focus::Content).await;
    }
    commit(
        &app,
        layout,
        if enabled {
            Focus::Picker
        } else {
            Focus::Content
        },
        enabled,
    )
    .await
}

#[tauri::command]
pub(crate) async fn choose_split_tab(
    app: AppHandle,
    label: String,
    session: u64,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let snap = state.snapshot.lock().unwrap().clone();
    if !matches!(snap.layout, Layout::Picking { .. }) || session != snap.picker_session {
        return Err("选页状态已改变，请重新选择".into());
    }
    let mut groups = snap.groups;
    let active = snap.active_group.ok_or("没有当前标签")?;
    merge_group(&mut groups, &active, &label)?;
    switch_groups(&app, groups, active, Focus::Content).await
}

#[tauri::command]
pub(crate) async fn create_split_page(app: AppHandle, session: u64) -> Result<String, String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let snap = state.snapshot.lock().unwrap().clone();
    if session != snap.picker_session || !matches!(snap.layout, Layout::Picking { .. }) {
        return Err("选页状态已改变，请重试".into());
    }
    spawn_tab(&app, DEFAULT_URL, Some(session), Focus::Content).await
}

#[tauri::command]
pub(crate) async fn focus_pane(
    app: AppHandle,
    pane: Pane,
    focus_content: bool,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let mut layout = state.snapshot.lock().unwrap().layout.clone();
    if !layout.is_split() {
        return Ok(());
    }
    layout.focus(pane);
    commit(
        &app,
        layout,
        if focus_content {
            Focus::Content
        } else {
            Focus::Keep
        },
        false,
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_split_ratio(
    app: AppHandle,
    ratio: f64,
    group: Option<String>,
) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    let snap = state.snapshot.lock().unwrap().clone();
    if group.is_some() && snap.active_group != group {
        return Ok(());
    }
    if snap.viewport.suspended {
        return Err("请先扩大窗口再调整分屏".into());
    }
    let mut layout = snap.layout;
    layout.set_ratio(ratio.clamp(snap.viewport.min_ratio, 1.0 - snap.viewport.min_ratio))?;
    commit(&app, layout, Focus::Keep, false).await
}

#[tauri::command]
pub(crate) async fn reload_tab(app: AppHandle, label: String) -> Result<(), String> {
    let state = app.state::<TabState>();
    let _operation = state.operations.lock().await;
    if !state
        .snapshot
        .lock()
        .unwrap()
        .tabs
        .iter()
        .any(|tab| tab.label == label)
    {
        return Err("该标签已关闭".into());
    }
    app.get_webview(&label)
        .ok_or("页面未就绪")?
        .reload()
        .map_err(|e| e.to_string())
}

pub(crate) fn resize(app: &AppHandle) {
    let state = app.state::<TabState>();
    if state.resize_pending.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<TabState>();
        let _operation = state.operations.lock().await;
        state.resize_pending.store(false, Ordering::Release);
        let before = state.snapshot.lock().unwrap().viewport.suspended;
        if let Err(error) = apply(&app, Focus::Keep).await {
            report_ui_error(&app, error);
        }
        let snap = state.snapshot.lock().unwrap().clone();
        if before != snap.viewport.suspended {
            let focus = if !snap.viewport.suspended
                && matches!(
                    snap.layout,
                    Layout::Picking {
                        focused: Pane::Right,
                        ..
                    }
                ) {
                Focus::Picker
            } else {
                Focus::Content
            };
            let _ = apply(&app, focus).await;
        }
        publish(&app);
    });
}

#[tauri::command]
pub(crate) fn report_workspace_error(app: AppHandle, message: String) {
    report_ui_error(&app, message.chars().take(2048).collect());
}

fn merge_group(groups: &mut Vec<TabGroup>, active: &str, label: &str) -> Result<(), String> {
    let current = groups
        .iter()
        .position(|g| g.id == active)
        .ok_or("标签已关闭")?;
    if !matches!(groups[current].layout, Layout::Picking { .. }) {
        return Err("当前标签不在选页状态".into());
    }
    let candidate = groups
        .iter()
        .position(|g| matches!(&g.layout, Layout::Single { tab } if tab == label))
        .ok_or("该页面已关闭或已属于其他分屏标签，请重新选择")?;
    if current == candidate {
        return Err("不能重复选择当前页面".into());
    }
    groups.remove(candidate);
    groups
        .iter_mut()
        .find(|g| g.id == active)
        .unwrap()
        .layout
        .select(label.into(), Pane::Right);
    Ok(())
}

fn detach_group(
    groups: &mut Vec<TabGroup>,
    active: &str,
    detached_id: String,
) -> Result<(), String> {
    let index = groups
        .iter()
        .position(|g| g.id == active)
        .ok_or("标签已关闭")?;
    let layout = &mut groups[index].layout;
    let retained = layout.retained().map(str::to_owned);
    let detached = [layout.document(Pane::Left), layout.document(Pane::Right)]
        .into_iter()
        .flatten()
        .find(|label| Some(*label) != retained.as_deref())
        .map(str::to_owned);
    layout.exit();
    if let Some(tab) = detached {
        groups.insert(
            index + 1,
            TabGroup {
                id: detached_id,
                layout: Layout::Single { tab },
            },
        );
    }
    Ok(())
}

#[cfg(test)]
mod group_tests {
    use super::*;
    #[test]
    fn library_links_reuse_only_their_existing_split_pane() {
        let split = Layout::Split {
            left: "a".into(),
            right: "b".into(),
            focused: Pane::Right,
            ratio: 0.5,
        };
        for (url, expected) in [
            ("https://www.feishu.cn/drive/home/", true),
            ("https://tenant.feishu.cn/drive/folder/abc", true),
            ("https://tenant.larksuite.com/drive/home/?from=split", true),
            ("https://tenant.feishu.cn/docx/abc", false),
            ("https://tenant.feishu.cn/wiki/abc", false),
            ("https://accounts.feishu.cn/accounts/page/login", false),
            ("https://example.com/drive/home/", false),
        ] {
            let url = tauri::Url::parse(url).unwrap();
            assert_eq!(library_opens_in_pane(&split, "b", &url), expected);
            assert!(!library_opens_in_pane(&split, "background", &url));
            assert!(!library_opens_in_pane(
                &Layout::Single { tab: "b".into() },
                "b",
                &url
            ));
        }
    }
    fn single(id: &str) -> TabGroup {
        TabGroup {
            id: id.into(),
            layout: Layout::Single { tab: id.into() },
        }
    }
    #[test]
    fn merging_consumes_one_tab_and_preserves_other_pairs() {
        let mut groups = vec![single("a"), single("b"), single("c"), single("d")];
        groups[0].layout.enter().unwrap();
        merge_group(&mut groups, "a", "b").unwrap();
        assert_eq!(groups.len(), 3);
        groups[0].layout.set_ratio(0.6).unwrap();
        let first_pair = groups[0].layout.clone();
        groups[1].layout.enter().unwrap();
        assert!(merge_group(&mut groups, "c", "a").is_err());
        merge_group(&mut groups, "c", "d").unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].layout, first_pair);
        assert_eq!(groups[1].layout.document(Pane::Right), Some("d"));
        groups.push(single("new"));
        assert_eq!(groups[0].layout, first_pair);
        assert!(matches!(groups[2].layout, Layout::Single { .. }));
    }
    #[test]
    fn switching_retains_each_groups_focus_and_ratio() {
        let mut a = single("a");
        a.layout.enter().unwrap();
        a.layout.select("b".into(), Pane::Right);
        a.layout.set_ratio(0.6).unwrap();
        let original = a.layout.clone();
        let mut snapshot = TabsSnapshot {
            groups: vec![a, single("c")],
            active_group: Some("c".into()),
            layout: Layout::Single { tab: "c".into() },
            ..Default::default()
        };
        sync_group(&mut snapshot);
        assert_eq!(snapshot.groups[0].layout, original);
        snapshot.active_group = Some("a".into());
        snapshot.layout = snapshot.groups[0].layout.clone();
        assert_eq!(snapshot.layout.ratio(), 0.6);
        assert_eq!(snapshot.layout.active(), Some("b"));
        snapshot.layout.focus(Pane::Left);
        sync_group(&mut snapshot);
        assert_eq!(snapshot.groups[0].layout.active(), Some("a"));
        assert_eq!(
            snapshot.groups[1].layout,
            Layout::Single { tab: "c".into() }
        );
    }
    #[test]
    fn exiting_pair_detaches_without_losing_documents_or_tab_identity() {
        let mut groups = vec![single("a"), single("b")];
        groups[0].layout.enter().unwrap();
        merge_group(&mut groups, "a", "b").unwrap();
        detach_group(&mut groups, "a", "detached".into()).unwrap();
        assert_eq!(groups[0].id, "a");
        assert_eq!(groups[0].layout, Layout::Single { tab: "b".into() });
        assert_eq!(groups[1].layout, Layout::Single { tab: "a".into() });
        groups[0].layout.enter().unwrap();
        detach_group(&mut groups, "a", "unused".into()).unwrap();
        assert_eq!(groups.len(), 2);
    }
}
