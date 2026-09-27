//! Durable, cancellable updates. Network discovery uses Tauri's updater; the
//! Windows-only offline installer accepts a version-bound, signed NSIS package.
use std::{fs, future::Future, path::Path, sync::Mutex, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::future::{select, Either};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::oneshot;

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const CACHE_SCHEMA: u32 = 1;
const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
const MAX_CACHE_BYTES: u64 = (MAX_PACKAGE_BYTES as u64 * 4 / 3) + 1024 * 1024;
const NSIS_HEADER: &[u8] = b"\xef\xbe\xad\xdeNullsoftInst";

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatePrefs {
    auto_check: bool,
    auto_download: bool,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        Self {
            auto_check: true,
            auto_download: false,
        }
    }
}

#[derive(Clone, Serialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate {
        checked_at: i64,
    },
    Available {
        version: String,
    },
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    Downloaded {
        version: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        notice: Option<String>,
    },
    Installing {
        version: String,
    },
    Error {
        message: String,
    },
}

/// One atomic file contains both metadata and package. An interrupted write can
/// never leave a new manifest referring to an incomplete or different package.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedPackage {
    schema: u32,
    version: String,
    signature: String,
    download_url: String,
    installer: String,
    package_base64: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OperationKind {
    Check,
    Download,
    Install,
}

struct ActiveOperation {
    id: u64,
    kind: OperationKind,
    cancel: Option<oneshot::Sender<()>>,
}

#[derive(Default)]
struct UpdateInner {
    status: UpdateStatus,
    pending: Option<Update>,
    cached: Option<CachedPackage>,
    prefs: UpdatePrefs,
    active: Option<ActiveOperation>,
    sequence: u64,
}

impl UpdateInner {
    fn begin(
        &mut self,
        kind: OperationKind,
        status: UpdateStatus,
    ) -> Result<(u64, oneshot::Receiver<()>), String> {
        if self.active.is_some() {
            return Err("另一个更新操作正在进行".into());
        }
        self.sequence += 1;
        let (tx, rx) = oneshot::channel();
        self.active = Some(ActiveOperation {
            id: self.sequence,
            kind,
            cancel: Some(tx),
        });
        self.status = status;
        Ok((self.sequence, rx))
    }

    fn is_current(&self, id: u64) -> bool {
        self.active.as_ref().is_some_and(|op| op.id == id)
    }

    fn may_download_after(&self, id: u64) -> bool {
        self.sequence == id && self.active.is_none() && self.prefs.auto_download
    }

    fn cancel(&mut self) -> Result<(), String> {
        if self
            .active
            .as_ref()
            .is_some_and(|op| op.kind == OperationKind::Install)
        {
            return Err("安装已开始，无法取消".into());
        }
        // Invalidate the handoff between a completed check and auto-download.
        self.sequence += 1;
        if let Some(mut operation) = self.active.take() {
            if let Some(cancel) = operation.cancel.take() {
                let _ = cancel.send(());
            }
            self.status = self.resting_status(None);
        }
        Ok(())
    }

    fn resting_status(&self, notice: Option<String>) -> UpdateStatus {
        if let Some(cache) = &self.cached {
            UpdateStatus::Downloaded {
                version: cache.version.clone(),
                notice,
            }
        } else if let Some(update) = &self.pending {
            UpdateStatus::Available {
                version: update.version.clone(),
            }
        } else {
            UpdateStatus::Idle
        }
    }
}

pub struct UpdateState(Mutex<UpdateInner>);

impl UpdateState {
    pub fn new() -> Self {
        let prefs = fs::read(crate::data_dir().join("updates.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self(Mutex::new(UpdateInner {
            prefs,
            ..UpdateInner::default()
        }))
    }
}

fn emit_status(app: &AppHandle) {
    let status = app.state::<UpdateState>().0.lock().unwrap().status.clone();
    // The remote document WebViews must never receive application IPC events.
    let _ = app.emit_to("ui", "update-status", &status);
    let _ = app.emit_to("settings", "update-status", &status);
}

fn cache_path() -> std::path::PathBuf {
    crate::data_dir().join("update").join("pending.json")
}

fn configured_public_key(app: &AppHandle) -> Result<String, String> {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|value| value.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "更新签名公钥未配置".into())
}

fn decode_text(value: &str, description: &str) -> Result<String, String> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| format!("{description}格式无效"))?;
    String::from_utf8(bytes).map_err(|_| format!("{description}编码无效"))
}

fn verify_signed_bytes(
    bytes: &[u8],
    version: &str,
    signature: &str,
    key: &str,
) -> Result<(), String> {
    let key = PublicKey::decode(&decode_text(key, "更新公钥")?)
        .map_err(|e| format!("更新公钥无效：{e}"))?;
    let signature = Signature::decode(&decode_text(signature, "更新签名")?)
        .map_err(|e| format!("更新签名无效：{e}"))?;
    key.verify(bytes, &signature, true)
        .map_err(|e| format!("更新包签名校验失败：{e}"))?;
    // Read the trusted comment only after its global signature is verified.
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
        .ok_or("更新签名缺少受保护的版本号，请重新下载已正确签名的版本")?;
    let announced = Version::parse(version).map_err(|_| "更新版本号无效")?;
    let signed =
        Version::parse(signed.trim_start_matches('v')).map_err(|_| "更新签名中的版本号无效")?;
    if signed != announced {
        return Err("更新包的签名版本与更新版本不一致".into());
    }
    Ok(())
}

fn verify_cache(cache: &CachedPackage, current: &str, key: &str) -> Result<Vec<u8>, String> {
    if cache.schema != CACHE_SCHEMA || cache.installer != "nsis" {
        return Err("不支持此更新缓存格式，请重新下载".into());
    }
    let version = Version::parse(&cache.version).map_err(|_| "缓存版本号无效")?;
    let current = Version::parse(current).map_err(|_| "当前版本号无效")?;
    if version <= current {
        return Err("缓存更新不高于当前版本，已停止安装".into());
    }
    let url = tauri::Url::parse(&cache.download_url).map_err(|_| "缓存下载地址无效")?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err("缓存下载地址必须使用 HTTPS".into());
    }
    if cache.package_base64.len() as u64 > MAX_CACHE_BYTES {
        return Err("更新包超过允许大小".into());
    }
    let bytes = STANDARD
        .decode(&cache.package_base64)
        .map_err(|_| "缓存更新包不完整")?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err("更新包超过允许大小".into());
    }
    verify_signed_bytes(&bytes, &cache.version, &cache.signature, key)?;
    if !bytes.starts_with(b"MZ")
        || !bytes
            .windows(NSIS_HEADER.len())
            .any(|part| part == NSIS_HEADER)
    {
        return Err("更新包不是受支持的 Windows NSIS 安装器".into());
    }
    Ok(bytes)
}

fn read_cache(path: &Path) -> Result<Option<CachedPackage>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("无法读取更新缓存：{e}")),
    };
    if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
        return Err("更新缓存格式或大小无效，请重新下载".into());
    }
    let bytes = fs::read(path).map_err(|e| format!("无法读取更新缓存：{e}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "更新缓存不完整，请重新下载".into())
}

pub fn restore_pending_on_boot(app: &AppHandle) {
    let result = (|| {
        let Some(cache) = read_cache(&cache_path())? else {
            return Ok(None);
        };
        let current = app.package_info().version.to_string();
        // A successfully installed cache is harmless; retain it without offering
        // it again. Unknown legacy files are also left untouched.
        if Version::parse(&cache.version)
            .ok()
            .is_some_and(|v| v <= app.package_info().version)
        {
            return Ok(None);
        }
        verify_cache(&cache, &current, &configured_public_key(app)?)?;
        Ok::<_, String>(Some(cache))
    })();
    {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        match result {
            Ok(Some(cache)) => {
                inner.cached = Some(cache);
                inner.status = inner.resting_status(None);
            }
            Ok(None) => {}
            Err(message) => inner.status = UpdateStatus::Error { message },
        }
    }
    emit_status(app);
}

#[derive(Debug)]
enum NetworkFailure {
    Cancelled,
    Failed(String),
}

async fn run_network<T>(
    future: impl Future<Output = Result<T, String>>,
    cancel: oneshot::Receiver<()>,
    timeout: Duration,
) -> Result<T, NetworkFailure> {
    match tokio::time::timeout(timeout, select(Box::pin(future), Box::pin(cancel))).await {
        Ok(Either::Left((result, _))) => result.map_err(NetworkFailure::Failed),
        Ok(Either::Right(_)) => Err(NetworkFailure::Cancelled),
        Err(_) => Err(NetworkFailure::Failed(
            "更新请求超时，可重试或稍后检查".into(),
        )),
    }
}

fn finish_failure(app: &AppHandle, id: u64, failure: NetworkFailure) {
    {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        if !inner.is_current(id) {
            return;
        }
        inner.active = None;
        inner.status = match failure {
            NetworkFailure::Cancelled => inner.resting_status(None),
            NetworkFailure::Failed(message) if inner.cached.is_some() => {
                inner.resting_status(Some(message))
            }
            NetworkFailure::Failed(message) => UpdateStatus::Error { message },
        };
    }
    emit_status(app);
}

#[tauri::command]
pub fn get_update_status(app: AppHandle) -> UpdateStatus {
    app.state::<UpdateState>().0.lock().unwrap().status.clone()
}

#[tauri::command]
pub fn get_update_prefs(app: AppHandle) -> UpdatePrefs {
    app.state::<UpdateState>().0.lock().unwrap().prefs.clone()
}

#[tauri::command]
pub fn set_update_prefs(app: AppHandle, prefs: UpdatePrefs) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    let mut inner = state.0.lock().unwrap();
    let bytes = serde_json::to_vec_pretty(&prefs).map_err(|e| e.to_string())?;
    crate::storage::atomic_write(&crate::data_dir().join("updates.json"), &bytes)?;
    inner.prefs = prefs;
    Ok(())
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<(), String> {
    let (id, cancel) = {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        // Keep the verified package installable, including when offline. Do not
        // replace it with a fresh remote release until it has been installed.
        if inner.cached.is_some() {
            return Ok(());
        }
        inner.begin(OperationKind::Check, UpdateStatus::Checking)?
    };
    emit_status(&app);
    let network = async {
        let updater = app
            .updater_builder()
            .timeout(CHECK_TIMEOUT)
            .configure_client(|client| {
                client
                    .connect_timeout(Duration::from_secs(15))
                    .read_timeout(Duration::from_secs(30))
            })
            .build()
            .map_err(|e| e.to_string())?;
        updater.check().await.map_err(|e| e.to_string())
    };
    match run_network(network, cancel, CHECK_TIMEOUT).await {
        Ok(update) => {
            let auto_download = {
                let state = app.state::<UpdateState>();
                let mut inner = state.0.lock().unwrap();
                if !inner.is_current(id) {
                    return Ok(());
                }
                inner.active = None;
                inner.pending = update;
                inner.status = match &inner.pending {
                    Some(update) => UpdateStatus::Available {
                        version: update.version.clone(),
                    },
                    None => UpdateStatus::UpToDate {
                        checked_at: now_millis(),
                    },
                };
                inner.prefs.auto_download && inner.pending.is_some()
            };
            emit_status(&app);
            if auto_download {
                // Both manual and boot checks use this one transition. A
                // cancellation between discovery and the next task invalidates
                // the sequence, so it cannot silently restart a download.
                download_update_inner(app.clone(), Some(id)).await?;
            }
        }
        Err(failure) => finish_failure(&app, id, failure),
    }
    Ok(())
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Default)]
struct DownloadProgress {
    received: u64,
    last_percent: Option<u64>,
    last_reported: u64,
}

impl DownloadProgress {
    fn add(&mut self, chunk: usize, total: Option<u64>) -> bool {
        self.received = self.received.saturating_add(chunk as u64);
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| self.received.saturating_mul(100) / total);
        let report = percent != self.last_percent
            || self.last_reported == 0
            || (percent.is_none() && self.received.saturating_sub(self.last_reported) >= 64 * 1024);
        if report {
            self.last_percent = percent;
            self.last_reported = self.received;
        }
        report
    }
}

#[tauri::command]
pub async fn download_update(app: AppHandle) -> Result<(), String> {
    download_update_inner(app, None).await
}

async fn download_update_inner(app: AppHandle, after_check: Option<u64>) -> Result<(), String> {
    let (id, cancel, mut update) = {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        if after_check.is_some_and(|id| !inner.may_download_after(id)) {
            return Ok(());
        }
        if inner.cached.is_some() {
            return Ok(());
        }
        let update = inner.pending.clone().ok_or("请先检查更新")?;
        let (id, cancel) = inner.begin(
            OperationKind::Download,
            UpdateStatus::Downloading {
                version: update.version.clone(),
                received: 0,
                total: None,
            },
        )?;
        (id, cancel, update)
    };
    // Updater 2.12 does not copy its check timeout into the returned Update.
    update.timeout = Some(DOWNLOAD_TIMEOUT);
    emit_status(&app);
    let progress_app = app.clone();
    let version = update.version.clone();
    let mut progress = DownloadProgress::default();
    let network = async {
        update
            .download(
                move |chunk, total| {
                    if !progress.add(chunk, total) {
                        return;
                    }
                    {
                        let state = progress_app.state::<UpdateState>();
                        let mut inner = state.0.lock().unwrap();
                        if !inner.is_current(id) {
                            return;
                        }
                        inner.status = UpdateStatus::Downloading {
                            version: version.clone(),
                            received: progress.received,
                            total,
                        };
                    }
                    emit_status(&progress_app);
                },
                || {},
            )
            .await
            .map_err(|e| e.to_string())
    };
    match run_network(network, cancel, DOWNLOAD_TIMEOUT).await {
        Ok(bytes) => {
            let result = (|| {
                if bytes.len() > MAX_PACKAGE_BYTES {
                    return Err("更新包超过允许大小".into());
                }
                let cache = CachedPackage {
                    schema: CACHE_SCHEMA,
                    version: update.version.clone(),
                    signature: update.signature.clone(),
                    download_url: update.download_url.to_string(),
                    installer: "nsis".into(),
                    package_base64: STANDARD.encode(&bytes),
                };
                verify_cache(
                    &cache,
                    &app.package_info().version.to_string(),
                    &configured_public_key(&app)?,
                )?;
                let state = app.state::<UpdateState>();
                let mut inner = state.0.lock().unwrap();
                if !inner.is_current(id) {
                    return Ok(());
                }
                let serialized = serde_json::to_vec(&cache).map_err(|e| e.to_string())?;
                crate::storage::atomic_write(&cache_path(), &serialized)?;
                inner.cached = Some(cache);
                inner.active = None;
                inner.status = inner.resting_status(None);
                Ok::<_, String>(())
            })();
            match result {
                Ok(()) => emit_status(&app),
                Err(message) => finish_failure(&app, id, NetworkFailure::Failed(message)),
            }
        }
        Err(failure) => finish_failure(&app, id, failure),
    }
    Ok(())
}

#[tauri::command]
pub fn cancel_update(app: AppHandle) -> Result<(), String> {
    {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        inner.cancel()?;
    }
    emit_status(&app);
    Ok(())
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let (id, expected_version) = {
        let state = app.state::<UpdateState>();
        let mut inner = state.0.lock().unwrap();
        let version = inner
            .cached
            .as_ref()
            .ok_or("没有已验证的更新包，请先下载")?
            .version
            .clone();
        let (id, _) = inner.begin(
            OperationKind::Install,
            UpdateStatus::Installing {
                version: version.clone(),
            },
        )?;
        (id, version)
    };
    emit_status(&app);
    let verified = (|| {
        let cache = read_cache(&cache_path())?.ok_or("更新缓存已不存在，请重新下载")?;
        if cache.version != expected_version {
            return Err("更新缓存已改变，请重新下载".into());
        }
        let key = configured_public_key(&app)?;
        let bytes = verify_cache(&cache, &app.package_info().version.to_string(), &key)?;
        Ok::<_, String>((cache, bytes, key))
    })();
    let result = match verified {
        Ok((cache, bytes, key)) => launch_verified_nsis(&app, &cache, &bytes, &key),
        Err(message) => {
            let state = app.state::<UpdateState>();
            state.0.lock().unwrap().cached = None;
            Err(message)
        }
    };
    if let Err(message) = &result {
        finish_failure(&app, id, NetworkFailure::Failed(message.clone()));
    }
    result
}

#[cfg(windows)]
fn launch_verified_nsis(
    app: &AppHandle,
    cache: &CachedPackage,
    bytes: &[u8],
    key: &str,
) -> Result<(), String> {
    use std::{
        ffi::OsStr,
        io::{Read, Write},
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    };
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOW};

    // The updater's Update cannot be reconstructed offline (its context is
    // private). This product ships only NSIS, so mirror its native NSIS launch
    // contract after independently verifying the durable signed package.
    let dir = tempfile::Builder::new()
        .prefix("feishu-verified-update-")
        .tempdir()
        .map_err(|e| format!("无法创建安装临时目录：{e}"))?;
    let path = dir.path().join("setup.exe");
    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    // Keep a read-only handle that denies writes/deletion until ShellExecute
    // succeeds. Reverify through that handle so the launched file is the exact
    // signed package, even if another process raced the temporary write.
    let mut guard = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut actual = Vec::new();
    guard.read_to_end(&mut actual).map_err(|e| e.to_string())?;
    verify_signed_bytes(&actual, &cache.version, &cache.signature, key)?;
    let mut parameters = std::ffi::OsString::from("/P /UPDATE /R /ARGS");
    for arg in app.env().args_os.iter().skip(1) {
        parameters.push(" ");
        parameters.push(escape_nsis_arg(arg));
    }
    let encode = |value: &OsStr| value.encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let file = encode(path.as_os_str());
    let parameters = encode(&parameters);
    let verb = encode(OsStr::new("open"));
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOW,
        )
    };
    if result as isize <= 32 {
        return Err(format!(
            "无法启动更新安装器（Windows 错误码 {}）",
            result as isize
        ));
    }
    let _retained = dir.keep();
    app.cleanup_before_exit();
    std::process::exit(0);
}

#[cfg(windows)]
fn escape_nsis_arg(arg: &std::ffi::OsStr) -> std::ffi::OsString {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let units = arg.encode_wide().collect::<Vec<_>>();
    let quote = units.is_empty() || units.iter().any(|unit| matches!(*unit, 9 | 32 | 34 | 47));
    let mut result = Vec::new();
    if quote {
        result.push(34);
    }
    let mut slashes = 0;
    for unit in units {
        if unit == 92 {
            slashes += 1;
        } else {
            if unit == 34 {
                result.extend(std::iter::repeat_n(92, slashes + 1));
            }
            slashes = 0;
        }
        result.push(unit);
    }
    if quote {
        result.extend(std::iter::repeat_n(92, slashes));
        result.push(34);
    }
    std::ffi::OsString::from_wide(&result)
}

#[cfg(not(windows))]
fn launch_verified_nsis(_: &AppHandle, _: &CachedPackage, _: &[u8], _: &str) -> Result<(), String> {
    Err("此应用仅支持 Windows NSIS 更新".into())
}

#[cfg(not(debug_assertions))]
pub fn spawn_update_boot(app: AppHandle) {
    let should_check = {
        let state = app.state::<UpdateState>();
        let inner = state.0.lock().unwrap();
        inner.prefs.auto_check && inner.cached.is_none()
    };
    if !should_check {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let _ = check_update(app.clone()).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    // Independently generated public fixture. No production key or installer is
    // used; the fixture bytes are deliberately not a runnable program.
    const KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IGdlbmVyYXRlZCB0ZXN0IGZpeHR1cmUKUldRQkFnTUVCUVlIQ0hYMENzZG5nUzBWSG05T0dya2NURHZyUjdjU2t2MnN0WkJwOEFtSlozRSs=";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IGdlbmVyYXRlZCB0ZXN0IGZpeHR1cmUKUldRQkFnTUVCUVlIQ1AxOE96bG5LT25ZRXgvM3BoNU1xK2JYa2Zqd1lCQjQ2UjlPQ0ZHNXQwV0RmYkx5am9SVDBBcm5qMjE3WEcxYTRmNjBlSHlMdFRpaCtNZGZEUTh1QVEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxCWZpbGU6Zml4dHVyZS5leGUJdmVyc2lvbjoyLjAuMApIZmJkMDJCd1dacGlUZjVlOTZIb0l0clB1TjBkWDhJNFV0R2pjT1U1QkY4Z2pXbWJmWTR2bG1qS2xocno5V2M1M3dwaHpVVnFsOHFENW5RYnVZZTVBdz09";

    fn fixture() -> CachedPackage {
        let mut bytes = b"MZ".to_vec();
        bytes.extend([0; 18]);
        bytes.extend(NSIS_HEADER);
        CachedPackage {
            schema: 1,
            version: "2.0.0".into(),
            signature: SIGNATURE.into(),
            download_url: "https://example.invalid/setup.exe".into(),
            installer: "nsis".into(),
            package_base64: STANDARD.encode(bytes),
        }
    }

    #[test]
    fn cache_is_offline_verifiable_and_rejects_modified_bytes_or_versions() {
        let cache = fixture();
        assert!(verify_cache(&cache, "1.0.0", KEY).is_ok());
        let mut changed = cache.clone();
        changed.package_base64 = STANDARD.encode(b"tampered");
        assert!(verify_cache(&changed, "1.0.0", KEY)
            .unwrap_err()
            .contains("签名"));
        changed = cache.clone();
        changed.version = "3.0.0".into();
        assert!(verify_cache(&changed, "1.0.0", KEY)
            .unwrap_err()
            .contains("版本不一致"));
        assert!(verify_cache(&cache, "2.0.0", KEY).is_err());
        assert!(verify_cache(&cache, "3.0.0", KEY).is_err());
    }

    #[test]
    fn unknown_or_incomplete_cache_cannot_be_installed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending.json");
        fs::write(dir.path().join("pending-2.0.0.exe"), b"legacy").unwrap();
        assert!(read_cache(&path).unwrap().is_none());
        fs::write(&path, b"{\"schema\":1").unwrap();
        assert!(read_cache(&path).is_err());
        assert!(dir.path().join("pending-2.0.0.exe").exists());
    }

    #[test]
    fn downloaded_state_is_preserved_without_network_metadata() {
        let state = UpdateInner {
            cached: Some(fixture()),
            ..UpdateInner::default()
        };
        assert!(state.pending.is_none());
        assert!(matches!(
            state.resting_status(None),
            UpdateStatus::Downloaded { .. }
        ));
    }

    #[test]
    fn operation_claim_is_atomic_and_duplicate_install_is_rejected() {
        let state = std::sync::Arc::new(Mutex::new(UpdateInner::default()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers = (0..2)
            .map(|_| {
                let state = state.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    state
                        .lock()
                        .unwrap()
                        .begin(
                            OperationKind::Install,
                            UpdateStatus::Installing {
                                version: "2.0.0".into(),
                            },
                        )
                        .is_ok()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let claimed = workers
            .into_iter()
            .map(|worker| worker.join().unwrap() as usize)
            .sum::<usize>();
        assert_eq!(claimed, 1);
    }

    #[test]
    fn progress_accumulates_chunks_with_or_without_content_length() {
        let mut progress = DownloadProgress::default();
        for (chunk, received) in [(25, 25), (25, 50), (50, 100)] {
            assert!(progress.add(chunk, Some(100)));
            assert_eq!(progress.received, received);
        }
        let mut unknown = DownloadProgress::default();
        assert!(unknown.add(12, None));
        unknown.add(24, None);
        assert_eq!(unknown.received, 36);
    }

    #[test]
    fn cancellation_invalidates_automatic_download_but_never_installation() {
        let mut inner = UpdateInner::default();
        inner.prefs.auto_download = true;
        let (check_id, _) = inner
            .begin(OperationKind::Check, UpdateStatus::Checking)
            .unwrap();
        inner.active = None;
        assert!(inner.may_download_after(check_id));
        inner.cancel().unwrap();
        assert!(!inner.may_download_after(check_id));
        let (install_id, _) = inner
            .begin(
                OperationKind::Install,
                UpdateStatus::Installing {
                    version: "2.0.0".into(),
                },
            )
            .unwrap();
        assert!(inner.cancel().is_err());
        assert!(inner.is_current(install_id));
    }

    #[cfg(windows)]
    #[test]
    fn restart_arguments_are_quoted_for_nsis() {
        use std::ffi::OsStr;
        for (value, expected) in [
            ("plain", "plain"),
            ("", "\"\""),
            ("a b", "\"a b\""),
            ("/silent", "\"/silent\""),
            ("a\"b", "\"a\\\"b\""),
        ] {
            assert_eq!(escape_nsis_arg(OsStr::new(value)), OsStr::new(expected));
        }
    }

    #[test]
    fn stalled_network_can_be_cancelled_or_timed_out() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (cancel, receive) = oneshot::channel();
            cancel.send(()).unwrap();
            assert!(matches!(
                run_network(
                    std::future::pending::<Result<(), String>>(),
                    receive,
                    Duration::from_secs(1)
                )
                .await,
                Err(NetworkFailure::Cancelled)
            ));
            let (_cancel, receive) = oneshot::channel();
            assert!(matches!(
                run_network(
                    std::future::pending::<Result<(), String>>(),
                    receive,
                    Duration::from_millis(1)
                )
                .await,
                Err(NetworkFailure::Failed(_))
            ));
        });
    }
}
