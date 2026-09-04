//! 原子写：同目录临时文件 + rename（设计原则 7）。

use crate::error::KbResult;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(path: &Path, content: &str) -> KbResult<()> {
    let dir = path
        .parent()
        .ok_or_else(|| crate::error::KbError::invalid("entry path must have a parent directory"))?;
    fs::create_dir_all(dir)?;

    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let filename = filename.ok_or_else(|| crate::error::KbError::invalid("entry path must be a file"))?;
    let tmp = dir.join(format!("{filename}.tmp-{}-{seq}", std::process::id()));

    fs::write(&tmp, content)?;
    if fs::rename(&tmp, path).is_err() {
        // Windows：目标已存在时 rename 失败，退化为 remove + rename
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_and_overwrites() {
        let dir = std::env::temp_dir().join(format!("fluen-kb-atomic-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
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
}
