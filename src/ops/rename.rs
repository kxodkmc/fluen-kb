//! `rename`：标题变更 → 文件原子重命名（§3.3）+ 派生刷新。

use super::{Ops, commit};
use crate::error::{KbError, KbResult};
use crate::ids::WikiId;
use crate::store::time;

impl Ops {
    pub fn rename(&self, id: &WikiId, new_title: &str) -> KbResult<()> {
        let title = new_title.trim();
        if title.is_empty() {
            return Err(KbError::invalid("title must be non-empty"));
        }
        let inner = &self.0;
        let mut doc = inner.store.load(id)?;
        if doc.title == title {
            return Ok(());
        }

        let old_path = inner.store.find_entry_file(id)?;
        let old_title = doc.title.clone();
        doc.title = title.to_string();
        doc.updated = time::now_rfc3339();
        inner.store.save(&doc)?;

        if let Some(old) = old_path {
            let (new_abs, _) = inner.store.entry_path(id, doc.wiki_type, title);
            if old != new_abs {
                std::fs::remove_file(old)?;
            }
        }

        commit(
            inner,
            id,
            title,
            "rename",
            &[format!("标题「{old_title}」→「{title}」")],
        )
    }
}
