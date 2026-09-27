use std::fs;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::{data_dir, storage};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct FontConfig {
    enabled: bool,
    en: String,
    cn: String,
    mono: String,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            en: "Cascadia Mono".into(),
            cn: "Microsoft YaHei".into(),
            mono: "Cascadia Code".into(),
        }
    }
}

pub(crate) struct FontState(Mutex<FontConfig>);

impl FontState {
    pub(crate) fn new() -> Self {
        let config = fs::read_to_string(data_dir().join("fonts.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self(Mutex::new(config))
    }
}

fn quote_family(name: &str, fallback: &str) -> String {
    let name = if name.is_empty() { fallback } else { name };
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

impl FontConfig {
    fn css(&self) -> String {
        if !self.enabled {
            return String::new();
        }
        let en = quote_family(&self.en, "Cascadia Mono");
        let cn = quote_family(&self.cn, "Microsoft YaHei");
        let mono = quote_family(&self.mono, "Cascadia Code");
        // :where keeps exclusions at zero specificity. Protect whole code and icon subtrees.
        format!(
            r#"html, body, #app, #root {{ font-family: {en}, {cn}, sans-serif !important; }}
*:not(:where(i, i *, svg, svg *, [class*="icon"], [class*="icon"] *, [class*="Icon"], [class*="Icon"] *, code, code *, pre, pre *, kbd, kbd *, .code-block-content, .code-block-content *)) {{ font-family: {en}, {cn}, sans-serif !important; }}
:is(code, pre, kbd, .code-block-content), :is(code, pre, kbd, .code-block-content) * {{ font-family: {mono}, Consolas, monospace !important; }}"#
        )
    }
}

/// Safe in both document-start and page-finished callbacks, including disabled state.
pub(crate) fn current_script(app: &AppHandle) -> String {
    let css = app.state::<FontState>().0.lock().unwrap().css();
    format!(
        r#"(()=>{{const css={css};const apply=()=>{{const d=document;let el=d.getElementById('feishu-font-force');if(!css){{el?.remove();return true;}}if(!el){{const host=d.head||d.documentElement;if(!host)return false;el=d.createElement('style');el.id='feishu-font-force';host.appendChild(el);}}el.textContent=css;return true;}};if(!apply()){{const observer=new MutationObserver(()=>{{if(apply())observer.disconnect();}});observer.observe(document,{{childList:true,subtree:true}});}}}})();"#,
        css = serde_json::to_string(&css).unwrap()
    )
}

#[tauri::command]
pub(crate) fn list_fonts() -> Vec<String> {
    let mut names = font_loader::system_fonts::query_all();
    names.sort();
    names.dedup();
    names
}

#[tauri::command]
pub(crate) fn get_font_config(app: AppHandle) -> FontConfig {
    app.state::<FontState>().0.lock().unwrap().clone()
}

#[tauri::command]
pub(crate) fn set_font_config(app: AppHandle, config: FontConfig) -> Result<(), String> {
    for name in [&config.en, &config.cn, &config.mono] {
        if name.len() > 512 || name.chars().any(char::is_control) {
            return Err("字体名称无效".into());
        }
    }
    {
        let state = app.state::<FontState>();
        let mut current = state.0.lock().unwrap();
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
        storage::atomic_write(&data_dir().join("fonts.json"), &bytes)?;
        *current = config;
    }
    let script = current_script(&app);
    let mut failed = false;
    for webview in app.webviews().values() {
        if webview.label().starts_with("tab-") && webview.eval(script.clone()).is_err() {
            failed = true;
        }
    }
    if failed {
        let _ = app.emit_to(
            "settings",
            "settings-warning",
            crate::UiError {
                message: "配置已保存，部分页面暂未应用，请刷新对应文档页面".into(),
            },
        );
    }
    Ok(())
}
