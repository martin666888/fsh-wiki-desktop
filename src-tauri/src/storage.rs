use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static WRITE_ID: AtomicU64 = AtomicU64::new(0);

/// Write on the same volume, sync, then atomically replace the destination.
/// An unsuccessful write preserves the previous file.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径没有父目录")?;
    fs::create_dir_all(parent).map_err(|e| format!("无法创建数据目录：{e}"))?;
    let name = path.file_name().ok_or("配置文件名无效")?.to_string_lossy();
    let temp = parent.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        WRITE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| format!("无法写入数据目录：{e}"))?;
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    let result = written.and_then(|_| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("保存失败，原配置未替换：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_commits_a_complete_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new complete settings").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new complete settings");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_keeps_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("settings.json");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("keep"), b"previous").unwrap();
        assert!(atomic_write(&destination, b"next").is_err());
        assert_eq!(fs::read(destination.join("keep")).unwrap(), b"previous");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
