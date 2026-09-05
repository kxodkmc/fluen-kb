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
        let _write = inner.lock_write()?;
        let mut doc = inner.store.load(id)?;
        if doc.title == title {
            return Ok(());
        }
        check_title_unique(inner, &doc.wiki_type, title, id)?;

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

/// 与 create 的去重规则对齐：title+type 已被其它条目占用时拒绝。
fn check_title_unique(
    inner: &crate::handle::KbInner,
    wiki_type: &crate::ids::WikiType,
    title: &str,
    self_id: &WikiId,
) -> KbResult<()> {
    let conn = inner.lock_conn()?;
    if let Some(existing) = crate::index::find_by_title(&conn, *wiki_type, title)?
        && &existing != self_id
    {
        return Err(KbError::invalid(format!(
            "title {title:?} already used by {existing}"
        )));
    }
    Ok(())
}
