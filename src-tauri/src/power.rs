//! Idle document WebViews sleep without discarding their editing state.
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct PowerState(Mutex<HashMap<String, Instant>>);

pub(crate) fn visibility(app: &AppHandle, label: &str, shown: bool) {
    let state = app.state::<PowerState>();
    let mut hidden = state.0.lock().unwrap();
    if shown {
        hidden.remove(label);
    } else {
        hidden.entry(label.to_owned()).or_insert_with(Instant::now);
    }
}

pub(crate) fn navigation(app: &AppHandle, label: &str) {
    if let Some(since) = app.state::<PowerState>().0.lock().unwrap().get_mut(label) {
        *since = Instant::now();
    }
}

fn due(since: &mut Instant, now: Instant, loading: bool) -> bool {
    if loading {
        *since = now;
        return false;
    }
    now.saturating_duration_since(*since) >= IDLE_TIMEOUT
}

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(CHECK_INTERVAL).await;
            let handle = app.clone();
            if app.run_on_main_thread(move || check(&handle)).is_err() {
                break;
            }
        }
    });
}

fn check(app: &AppHandle) {
    let tabs = app.state::<crate::tabs::TabState>();
    let snap = tabs.snapshot.lock().unwrap().clone();
    let now = Instant::now();
    let candidates = {
        let state = app.state::<PowerState>();
        let mut hidden = state.0.lock().unwrap();
        hidden.retain(|label, _| snap.tabs.iter().any(|tab| &tab.label == label));
        snap.tabs
            .iter()
            .filter_map(|tab| {
                let since = hidden.get_mut(&tab.label)?;
                due(since, now, tab.loading).then(|| tab.label.clone())
            })
            .collect::<Vec<_>>()
    };
    for label in candidates {
        if let Some(view) = app.get_webview(&label) {
            try_sleep(&view);
        }
    }
}

#[cfg(windows)]
fn try_sleep(webview: &tauri::Webview) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::ICoreWebView2_3, TrySuspendCompletedHandler,
    };
    use windows::core::{Interface, BOOL};

    // All visibility changes and this final check run on the UI thread. Showing
    // the controller automatically resumes it, including an in-flight suspend.
    let _ = webview.with_webview(|platform| unsafe {
        let result = (|| -> windows::core::Result<()> {
            let controller = platform.controller();
            let mut visible = BOOL(0);
            controller.IsVisible(&mut visible)?;
            if visible.as_bool() {
                return Ok(());
            }
            let view: ICoreWebView2_3 = controller.CoreWebView2()?.cast()?;
            let mut suspended = BOOL(0);
            view.IsSuspended(&mut suspended)?;
            if !suspended.as_bool() {
                // Best effort: Chromium may refuse (for example active media).
                // A later tick retries; we never force-discard the document.
                view.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(()))))?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("Unable to suspend idle document: {error}");
        }
    });
}

#[cfg(not(windows))]
fn try_sleep(_: &tauri::Webview) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_full_idle_period() {
        let now = Instant::now();
        let mut since = now;
        assert!(!due(
            &mut since,
            now + IDLE_TIMEOUT - Duration::from_secs(1),
            false
        ));
        assert!(due(&mut since, now + IDLE_TIMEOUT, false));
    }

    #[test]
    fn loading_restarts_grace_period() {
        let now = Instant::now();
        let mut since = now;
        let loaded = now + IDLE_TIMEOUT;
        assert!(!due(&mut since, loaded, true));
        assert!(!due(&mut since, loaded + CHECK_INTERVAL, false));
        assert!(due(&mut since, loaded + IDLE_TIMEOUT, false));
    }
}
