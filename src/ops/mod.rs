//! L2 操作层：写操作编排。所有写操作经 `commit` 公共管线
//! （改 MD → 重派生 → 刷新向量 → 重建 index.md → 追加日志）。

mod create;
mod delete;
mod edit;
mod merge;
mod migrate;
mod rename;

pub use delete::{DeleteSourceReport, PruneReport};
pub use migrate::MigrateReport;

use crate::error::KbResult;
use crate::handle::KbInner;
use crate::ids::{SourceId, WikiId};
use crate::index::{self, views};
use std::sync::Arc;

/// 创建结果：未命中去重键 → 新建；命中 → 转入 merge。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
    Created(WikiId),
    MergedInto(WikiId),
}

/// 编辑原语（设计 §8.2）。多重匹配一律取作用域内首个，不传播（D22）。
#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    SearchReplace {
        search: String,
        replace: String,
        scope: Option<SourceId>,
    },
    InsertAfter {
        anchor: String,
        content: String,
        scope: Option<SourceId>,
    },
    ReplaceSource {
        src: SourceId,
        content: String,
    },
}

pub struct Ops(pub(crate) Arc<KbInner>);

pub(crate) fn commit(
    inner: &KbInner,
    id: &WikiId,
    title: &str,
    verb: &str,
    details: &[String],
) -> KbResult<()> {
    {
        let conn = inner.lock_conn()?;
        index::reindex_entry(&conn, &inner.store, id)?;
        crate::embed::refresh_entry(inner, &conn, id)?;
        views::rebuild_index_md(&conn, &inner.store)?;
    }
    views::append_log(&inner.store, verb, id.as_str(), title, details)
}
