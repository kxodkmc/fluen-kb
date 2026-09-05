//! 原子写：同目录临时文件 + rename（设计原则 7）。

use crate::error::KbResult;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, content: &str) -> KbResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| crate::error::KbError::invalid("entry path must have a parent directory"))?;
    fs::create_dir_all(dir)?;

    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let filename = path
        .file_name()
        .ok_or_else(|| crate::error::KbError::invalid("entry path must be a file"))?
        .to_string_lossy()
        .into_owned();
    let tmp = dir.join(format!("{filename}.tmp-{}-{seq}", std::process::id()));

    let mut file = File::create(&tmp)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    drop(file);

    if fs::rename(&tmp, path).is_err() {
        // Windows：目标被占用等场景退化为「备份 → 替换」；再失败则清理临时文件并回滚
        let backup = fs::read(path).ok();
        let _ = fs::remove_file(path);
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            if let Some(old) = backup {
                let _ = fs::write(path, old);
            }
            return Err(e.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir_of(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fluen-kb-atomic-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_creates_and_overwrites() {
        let dir = temp_dir_of("basic");
        let path = dir.join("a.md");
        atomic_write(&path, "one").unwrap();
        atomic_write(&path, "two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let leftover: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_failure_leaves_no_tmp_and_keeps_target() {
        let dir = temp_dir_of("fail");
        let target = dir.join("a.md");
        fs::write(&target, "原内容").unwrap();
        let doomed = dir.join("b.md");
        fs::create_dir_all(&doomed).unwrap(); // 目标是目录 → rename 必败

        assert!(atomic_write(&doomed, "x").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "原内容", "失败不得破坏既有文件");
        let leftover: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty(), "失败后不得残留临时文件");
        let _ = fs::remove_dir_all(&dir);
    }
}
