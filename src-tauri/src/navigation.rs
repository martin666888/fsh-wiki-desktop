use tauri::Url;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Destination {
    Document,
    Browser,
    Denied,
}

pub(crate) fn classify(url: &Url) -> Destination {
    if !url.username().is_empty() || url.password().is_some() {
        return Destination::Denied;
    }
    let Some(host) = url.host_str() else {
        return Destination::Denied;
    };
    let feishu = host == "feishu.cn"
        || host.ends_with(".feishu.cn")
        || host == "larksuite.com"
        || host.ends_with(".larksuite.com");
    if url.scheme() == "https" && feishu && url.port_or_known_default() == Some(443) {
        Destination::Document
    } else if matches!(url.scheme(), "https" | "http") {
        Destination::Browser
    } else {
        Destination::Denied
    }
}

pub(crate) fn open_external(url: &Url) -> Result<(), String> {
    if classify(url) != Destination::Browser {
        return Err("仅允许在系统浏览器中打开 HTTP 或 HTTPS 链接".into());
    }
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url.as_str()])
            .creation_flags(0x08000000)
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url.as_str()).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(url.as_str())
        .spawn();
    result
        .map(|_| ())
        .map_err(|e| format!("无法打开系统浏览器：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_urls_without_launching_handlers() {
        for (input, expected) in [
            ("https://www.feishu.cn/drive/home/", Destination::Document),
            ("https://tenant.feishu.cn:443/wiki/a", Destination::Document),
            ("https://a.b.larksuite.com/docx/a", Destination::Document),
            ("http://www.feishu.cn/", Destination::Browser),
            ("https://tenant.feishu.cn:8443/", Destination::Browser),
            ("https://feishu.cn.evil.example/", Destination::Browser),
            ("https://notfeishu.cn/", Destination::Browser),
            ("https://example.com/", Destination::Browser),
            ("https://user:secret@www.feishu.cn/", Destination::Denied),
            ("file:///C:/example.exe", Destination::Denied),
            ("javascript:alert(1)", Destination::Denied),
            ("data:text/html,hello", Destination::Denied),
            ("about:blank", Destination::Denied),
            ("lark://client/open", Destination::Denied),
        ] {
            assert_eq!(classify(&Url::parse(input).unwrap()), expected, "{input}");
        }
    }

    #[test]
    fn reject_non_browser_inputs_before_native_launch() {
        for input in [
            "file:///C:/example.exe",
            "custom:open",
            "https://www.feishu.cn/",
        ] {
            assert!(open_external(&Url::parse(input).unwrap()).is_err());
        }
    }
}
