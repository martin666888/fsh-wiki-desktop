//! Observe WebView2 downloads without changing destination, cancellation,
//! default download UI, cookies, or the document's export behavior.
use std::{collections::HashMap, sync::Mutex};
use tauri::{AppHandle, Manager};

#[derive(Default)]
pub(crate) struct DownloadState(Mutex<HashMap<String, usize>>);

pub(crate) fn active(app: &AppHandle, label: &str) -> bool {
    app.state::<DownloadState>()
        .0
        .lock()
        .unwrap()
        .contains_key(label)
}

#[cfg(windows)]
struct Activity {
    app: AppHandle,
    label: String,
}

#[cfg(windows)]
impl Activity {
    fn new(app: &AppHandle, label: &str) -> Self {
        *app.state::<DownloadState>()
            .0
            .lock()
            .unwrap()
            .entry(label.into())
            .or_default() += 1;
        Self {
            app: app.clone(),
            label: label.into(),
        }
    }
}

#[cfg(windows)]
impl Drop for Activity {
    fn drop(&mut self) {
        let state = self.app.state::<DownloadState>();
        let mut counts = state.0.lock().unwrap();
        if let Some(count) = counts.get_mut(&self.label) {
            *count -= 1;
            if *count == 0 {
                counts.remove(&self.label);
            }
        }
        drop(counts);
        crate::power::navigation(&self.app, &self.label);
    }
}

#[cfg(windows)]
pub(crate) unsafe fn attach(
    view: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2,
    app: &AppHandle,
    label: &str,
) -> windows::core::Result<()> {
    use std::{
        cell::{Cell, RefCell},
        path::Path,
        rc::Rc,
    };
    use tauri::Emitter;
    use webview2_com::{
        take_pwstr, DownloadStartingEventHandler, Microsoft::Web::WebView2::Win32::*,
        StateChangedEventHandler,
    };
    use windows::core::{Interface, PWSTR};

    fn notice(app: &AppHandle, message: String) {
        let _ = app.emit_to("ui", "ui-notice", crate::UiError { message });
    }

    let view: ICoreWebView2_4 = view.cast()?;
    let app = app.clone();
    let label = label.to_owned();
    let mut token = 0;
    view.add_DownloadStarting(
        &DownloadStartingEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let result = (|| -> windows::core::Result<()> {
                let operation = args.DownloadOperation()?;
                let activity = Rc::new(RefCell::new(Some(Activity::new(&app, &label))));
                let completed = Rc::new(Cell::new(false));
                let event_token = Rc::new(Cell::new(0));
                let callback_token = event_token.clone();
                let event_app = app.clone();
                let mut token = 0;
                operation.add_StateChanged(
                    &StateChangedEventHandler::create(Box::new(move |operation, _| {
                        let Some(operation) = operation else {
                            return Ok(());
                        };
                        let mut state = COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS;
                        operation.State(&mut state)?;
                        if state == COREWEBVIEW2_DOWNLOAD_STATE_IN_PROGRESS
                            || completed.replace(true)
                        {
                            return Ok(());
                        }
                        activity.borrow_mut().take();
                        let _ = operation.remove_StateChanged(callback_token.get());
                        if state == COREWEBVIEW2_DOWNLOAD_STATE_COMPLETED {
                            let mut raw = PWSTR::null();
                            match operation.ResultFilePath(&mut raw) {
                                Ok(()) => {
                                    notice(&event_app, format!("已保存：{}", take_pwstr(raw)))
                                }
                                Err(_) => {
                                    notice(&event_app, "下载已完成，请在默认下载目录查看".into())
                                }
                            }
                        } else {
                            let mut reason = COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_NONE;
                            if operation.InterruptReason(&mut reason).is_ok()
                                && reason == COREWEBVIEW2_DOWNLOAD_INTERRUPT_REASON_USER_CANCELED
                            {
                                notice(&event_app, "已取消下载".into());
                            } else {
                                crate::report_ui_error(
                                    &event_app,
                                    format!(
                                        "下载未完成（错误 {}），请检查网络和下载目录后重试",
                                        reason.0
                                    ),
                                );
                            }
                        }
                        Ok(())
                    })),
                    &mut token,
                )?;
                event_token.set(token);
                let mut raw = PWSTR::null();
                let message = if args.ResultFilePath(&mut raw).is_ok() {
                    let path = take_pwstr(raw);
                    let filename = Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    format!("正在下载：{filename}")
                } else {
                    "正在下载，请稍候…".into()
                };
                notice(&app, message);
                Ok(())
            })();
            if let Err(error) = result {
                // Notification failures must never cancel an otherwise valid export.
                crate::report_ui_error(
                    &app,
                    format!("无法跟踪下载状态，请检查默认下载目录：{error}"),
                );
            }
            Ok(())
        })),
        &mut token,
    )
}
