//! Interactive native download regression with a fresh profile and local files.
//! Run: cargo run --manifest-path src-tauri/Cargo.toml --example download-smoke
//! Add `-- --auto` to verify a real HTTP attachment, payload, and activity cleanup.
//! File extensions exercise download names; payloads are test text, not Office files.
#[path = "../src/downloads.rs"]
mod downloads;

use tauri::{Emitter, Listener, Manager};

#[derive(serde::Serialize, Clone)]
struct UiError {
    message: String,
}

fn report_ui_error(app: &tauri::AppHandle, message: String) {
    let _ = app.emit_to("ui", "ui-error", UiError { message });
}

// The harness has no idle scheduler; production uses this to restart its timer.
mod power {
    pub(crate) fn navigation(_: &tauri::AppHandle, _: &str) {}
}

fn main() {
    let auto = std::env::args().any(|arg| arg == "--auto");
    let data = tempfile::tempdir().unwrap();
    let fixture = data.path().join("fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    std::fs::write(fixture.join("index.html"), r#"<!doctype html><meta charset="utf-8">
<title>下载回归测试</title><style>body{font:16px system-ui;padding:24px}button{padding:12px;margin:8px}pre{white-space:pre-wrap}</style>
<h1>下载回归测试（独立数据）</h1><p>仅使用本机测试内容，不连接飞书。</p>
<button onclick="save('docx')">导出 Word 测试文件</button>
<button onclick="save('pdf')">导出 PDF 测试文件</button>
<button onclick="save('md')">导出 Markdown 测试文件</button>
<a href="attachment.docx" download="附件测试.docx">普通附件下载</a>
<pre id="status">等待测试</pre>
<script>
const payload='feishu-download-regression\n';
function save(ext){const a=document.createElement('a');a.href=URL.createObjectURL(new Blob([payload],{type:'application/octet-stream'}));a.download='导出测试.'+ext;document.body.append(a);a.click();a.remove();setTimeout(()=>URL.revokeObjectURL(a.href),60000);}
</script>"#).unwrap();
    std::fs::write(
        fixture.join("attachment.docx"),
        "feishu-download-regression\n",
    )
    .unwrap();
    let profile = data.path().join("profile");
    // Serve real HTTP attachments entirely on loopback, without Feishu credentials.
    let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", server.local_addr().unwrap());
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in server.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let mut request = [0u8; 4096];
            let Ok(size) = stream.read(&mut request) else {
                continue;
            };
            let attachment =
                String::from_utf8_lossy(&request[..size]).starts_with("GET /attachment.docx ");
            let body = std::fs::read(fixture.join(if attachment {
                "attachment.docx"
            } else {
                "index.html"
            }))
            .unwrap();
            let headers = if attachment {
                "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=attachment.docx\r\n"
            } else {
                "Content-Type: text/html; charset=utf-8\r\n"
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local-artifacts/download-smoke-output");
    std::fs::create_dir_all(&output).unwrap();
    let output = output
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(downloads::DownloadState::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            if auto {
                let timeout_app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    eprintln!("FAIL: timed out waiting for download completion");
                    timeout_app.exit(1);
                });
            }
            for name in ["ui-error", "ui-notice"] {
                let report = handle.clone();
                app.listen_any(name, move |event| {
                    println!(
                        "{} {} active={}",
                        name,
                        event.payload(),
                        downloads::active(&report, "ui")
                    );
                    if let Some(view) = report.get_webview("ui") {
                        let text = serde_json::to_string(event.payload()).unwrap();
                        let _ = view.eval(format!(
                            "document.getElementById('status').textContent += '\\n' + {text};"
                        ));
                    }
                    if auto {
                        let value: serde_json::Value =
                            serde_json::from_str(event.payload()).unwrap();
                        let message = value["message"].as_str().unwrap_or_default();
                        if let Some(path) = message.strip_prefix("已保存：") {
                            assert_eq!(
                                std::fs::read(path).unwrap(),
                                b"feishu-download-regression\n"
                            );
                            assert!(!downloads::active(&report, "ui"));
                            println!("PASS: saved payload and released download activity");
                            report.exit(0);
                        } else if name == "ui-error" {
                            report.exit(1);
                        }
                    }
                });
            }
            let window = tauri::WindowBuilder::new(app, "main")
                .title("下载回归测试 — 独立数据")
                .inner_size(840.0, 420.0)
                .build()?;
            let requested = std::sync::atomic::AtomicBool::new(false);
            let builder = tauri::webview::WebviewBuilder::new(
                "ui",
                tauri::WebviewUrl::External("about:blank".parse()?),
            )
            .data_directory(profile)
            .on_page_load(move |view, event| {
                println!("page {:?}: {}", event.event(), event.url());
                if auto
                    && event.url().host_str() == Some("127.0.0.1")
                    && event.url().path() == "/"
                    && matches!(event.event(), tauri::webview::PageLoadEvent::Finished)
                    && !requested.swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    view.navigate(event.url().join("attachment.docx").unwrap())
                        .unwrap();
                }
            });
            let view = window.add_child(
                builder,
                tauri::LogicalPosition::new(0.0, 0.0),
                tauri::LogicalSize::new(840.0, 420.0),
            )?;
            window.show()?;
            view.with_webview(move |platform| unsafe {
                use webview2_com::Microsoft::Web::WebView2::Win32::*;
                use windows::core::{Interface, HSTRING};
                let view = platform.controller().CoreWebView2().unwrap();
                view.add_NavigationCompleted(
                    &webview2_com::NavigationCompletedEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            let mut success = windows::core::BOOL(0);
                            let mut reason = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                            args.IsSuccess(&mut success)?;
                            args.WebErrorStatus(&mut reason)?;
                            println!(
                                "native navigation: success={} reason={}",
                                success.as_bool(),
                                reason.0
                            );
                        }
                        Ok(())
                    })),
                    &mut 0,
                )
                .unwrap();
                let profiled: ICoreWebView2_13 = view.cast().unwrap();
                let profile: ICoreWebView2Profile2 = profiled.Profile().unwrap().cast().unwrap();
                profile
                    .SetDefaultDownloadFolderPath(&HSTRING::from(output))
                    .unwrap();
                downloads::attach(&view, &handle, "ui").unwrap();
                view.Navigate(&HSTRING::from(url)).unwrap();
            })?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native download regression failed");
    drop(data);
}
