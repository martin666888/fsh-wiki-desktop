//! WebView2-only adapters. Remote pages never receive application IPC permissions.
use crate::{
    layout::{Layout, DIVIDER_WIDTH},
    tabs::{self, TabState},
};
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Manager};

#[cfg(windows)]
pub(crate) async fn attach(webview: &tauri::Webview) -> Result<(), String> {
    use webview2_com::{
        FocusChangedEventHandler, Microsoft::Web::WebView2::Win32::*,
        NavigationCompletedEventHandler, NavigationStartingEventHandler,
    };
    let app = webview.app_handle().clone();
    let label = webview.label().to_owned();
    let (send, receive) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |platform| {
            let result = (|| -> windows::core::Result<()> {
                unsafe {
                    let controller = platform.controller();
                    let view = controller.CoreWebView2()?;
                    crate::downloads::attach(&view, &app, &label)?;
                    let focus_app = app.clone();
                    let focus_label = label.clone();
                    let mut token = 0;
                    controller.add_GotFocus(
                        &FocusChangedEventHandler::create(Box::new(move |_, _| {
                            tabs::focused(&focus_app, &focus_label);
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    let start_app = app.clone();
                    let start_label = label.clone();
                    view.add_NavigationStarting(
                        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                            if let Some(args) = args {
                                let mut id = 0;
                                args.NavigationId(&mut id)?;
                                tabs::navigation_started(&start_app, &start_label, id);
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    view.add_NavigationCompleted(
                        &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                            if let Some(args) = args {
                                let mut id = 0;
                                let mut success = windows::core::BOOL(0);
                                let mut code = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                                args.NavigationId(&mut id)?;
                                args.IsSuccess(&mut success)?;
                                args.WebErrorStatus(&mut code)?;
                                let error = if success.as_bool()
                                    || code == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED
                                {
                                    None
                                } else {
                                    Some(match code {
                                        COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED => {
                                            "无法解析网站地址，请检查网络或代理".into()
                                        }
                                        COREWEBVIEW2_WEB_ERROR_STATUS_TIMEOUT => {
                                            "页面连接超时，请重试".into()
                                        }
                                        COREWEBVIEW2_WEB_ERROR_STATUS_DISCONNECTED
                                        | COREWEBVIEW2_WEB_ERROR_STATUS_CANNOT_CONNECT
                                        | COREWEBVIEW2_WEB_ERROR_STATUS_SERVER_UNREACHABLE => {
                                            "无法连接页面，请检查网络后重试".into()
                                        }
                                        _ => format!(
                                            "页面加载失败（错误 {}），可重试或选择其他文章",
                                            code.0
                                        ),
                                    })
                                };
                                tabs::navigation_finished(&app, &label, id, error);
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    Ok(())
                }
            })();
            let _ = send.send(result.map_err(|e| format!("无法连接文档窗口：{e}")));
        })
        .map_err(|e| e.to_string())?;
    receive.await.map_err(|_| "文档窗口已关闭".to_string())?
}

#[cfg(not(windows))]
pub(crate) async fn attach(_: &tauri::Webview) -> Result<(), String> {
    Err("分屏文档目前仅支持 Windows WebView2".into())
}

#[tauri::command]
pub(crate) async fn capture_tab_preview(
    app: AppHandle,
    label: String,
    generation: u64,
) -> Result<Option<String>, String> {
    let state = app.state::<TabState>();
    let _capture = state.previews.lock().await;
    let eligible = {
        let snap = state.snapshot.lock().unwrap();
        matches!(snap.layout, Layout::Picking { .. })
            && snap.tabs.iter().any(|tab| {
                tab.label == label
                    && tab.generation == generation
                    && !tab.loading
                    && tab.error.is_none()
            })
    };
    if !eligible {
        return Ok(None);
    }
    let Some(view) = app.get_webview(&label) else {
        return Ok(None);
    };
    let preview = capture(&view).await;
    let still_current = state
        .snapshot
        .lock()
        .unwrap()
        .tabs
        .iter()
        .any(|tab| tab.label == label && tab.generation == generation);
    Ok(if still_current { preview } else { None })
}

#[cfg(windows)]
async fn capture(webview: &tauri::Webview) -> Option<String> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
    };
    use windows::Win32::{
        System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET},
        UI::Shell::SHCreateMemStream,
    };
    let (send, receive) = tokio::sync::oneshot::channel();
    webview
        .with_webview(move |platform| unsafe {
            let Ok(view) = platform.controller().CoreWebView2() else {
                return;
            };
            let Some(stream) = SHCreateMemStream(None) else {
                return;
            };
            let read_stream = stream.clone();
            let callback = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let preview = (|| -> Option<String> {
                    result.ok()?;
                    let mut stat = STATSTG::default();
                    read_stream.Stat(&mut stat, STATFLAG_NONAME).ok()?;
                    if stat.cbSize == 0 || stat.cbSize > 4 * 1024 * 1024 {
                        return None;
                    }
                    read_stream.Seek(0, STREAM_SEEK_SET, None).ok()?;
                    let mut bytes = vec![0u8; stat.cbSize as usize];
                    let mut read = 0;
                    read_stream
                        .Read(
                            bytes.as_mut_ptr().cast(),
                            bytes.len() as u32,
                            Some(&mut read),
                        )
                        .ok()
                        .ok()?;
                    if read as usize != bytes.len() {
                        return None;
                    }
                    Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
                })();
                let _ = send.send(preview);
                Ok(())
            }));
            let _ = view.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_JPEG,
                &stream,
                &callback,
            );
        })
        .ok()?;
    tokio::time::timeout(Duration::from_secs(3), receive)
        .await
        .ok()?
        .ok()?
}

#[cfg(not(windows))]
async fn capture(_: &tauri::Webview) -> Option<String> {
    None
}

/// Pointer capture belongs to the native window, so crossing a child WebView or
/// leaving the application does not lose mouse-up. Sampling never blocks the UI.
#[tauri::command]
pub(crate) async fn begin_split_resize(app: AppHandle) -> Result<(), String> {
    let state = app.state::<TabState>();
    let (original, original_group) = {
        let snap = state.snapshot.lock().unwrap();
        if !snap.layout.is_split() || snap.viewport.suspended {
            return Err("当前无法调整分屏".into());
        }
        (snap.layout.ratio(), snap.active_group.clone())
    };
    if state.dragging.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let result = drag(&app, original, original_group).await;
    state.dragging.store(false, Ordering::Release);
    result
}

#[cfg(windows)]
async fn drag(
    app: &AppHandle,
    original: f64,
    original_group: Option<String>,
) -> Result<(), String> {
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetCapture, ReleaseCapture, SetCapture, VK_ESCAPE, VK_LBUTTON,
            },
            WindowsAndMessaging::{
                GetCursorPos, GetForegroundWindow, LoadCursorW, SetCursor, IDC_SIZEWE,
            },
        },
    };
    let window = app.get_window("main").ok_or("主窗口已关闭")?;
    let capture_window = window.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = capture_window
            .hwnd()
            .map(|hwnd| unsafe {
                if GetAsyncKeyState(VK_LBUTTON as i32) >= 0 {
                    return false;
                }
                SetCapture(hwnd.0);
                GetCapture() == hwnd.0
            })
            .unwrap_or(false);
        let _ = send.send(result);
    })
    .map_err(|e| e.to_string())?;
    if !receive.await.unwrap_or(false) {
        return Ok(());
    }
    let result = async {
        loop {
            if app
                .state::<TabState>()
                .snapshot
                .lock()
                .unwrap()
                .active_group
                != original_group
            {
                break;
            }
            let sample_window = window.clone();
            let (send, receive) = tokio::sync::oneshot::channel();
            app.run_on_main_thread(move || {
                let sample = (|| unsafe {
                    let hwnd = sample_window.hwnd().ok()?;
                    let cancelled = GetAsyncKeyState(VK_ESCAPE as i32) < 0;
                    if GetCapture() != hwnd.0 || GetForegroundWindow() != hwnd.0 {
                        return Some((false, cancelled, 0.0));
                    }
                    let down = GetAsyncKeyState(VK_LBUTTON as i32) < 0;
                    let mut point = POINT::default();
                    if GetCursorPos(&mut point) == 0 {
                        return None;
                    }
                    let origin = sample_window.inner_position().ok()?;
                    let scale = sample_window.scale_factor().ok()?;
                    SetCursor(LoadCursorW(std::ptr::null_mut(), IDC_SIZEWE));
                    Some((down, cancelled, (point.x - origin.x) as f64 / scale))
                })();
                let _ = send.send(sample);
            })
            .map_err(|e| e.to_string())?;
            let Some((down, cancelled, x)) = receive.await.ok().flatten() else {
                break;
            };
            if cancelled {
                let state = app.state::<TabState>();
                let _operation = state.operations.lock().await;
                let snap = state.snapshot.lock().unwrap().clone();
                if snap.active_group != original_group {
                    break;
                }
                let mut layout = snap.layout;
                if layout.set_ratio(original).is_ok() {
                    tabs::commit(app, layout, tabs::Focus::Keep, false).await?;
                }
                break;
            }
            if !down {
                break;
            }
            let snap = app.state::<TabState>().snapshot.lock().unwrap().clone();
            if !snap.layout.is_split() || snap.viewport.suspended {
                break;
            }
            let ratio = ((x - DIVIDER_WIDTH / 2.0) / (snap.viewport.width - DIVIDER_WIDTH))
                .clamp(snap.viewport.min_ratio, 1.0 - snap.viewport.min_ratio);
            if (ratio - snap.layout.ratio()).abs() > 0.0005 {
                tabs::set_split_ratio(app.clone(), ratio, original_group.clone()).await?;
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
        Ok(())
    }
    .await;
    // Always release even if a layout operation failed. Never steal another capture.
    let _ = app.run_on_main_thread(move || {
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                if GetCapture() == hwnd.0 {
                    ReleaseCapture();
                }
            }
        }
    });
    result
}

#[cfg(not(windows))]
async fn drag(_: &AppHandle, _: f64, _: Option<String>) -> Result<(), String> {
    Err("当前平台不支持原生分隔线拖动".into())
}
