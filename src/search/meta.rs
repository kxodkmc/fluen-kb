//! 元数据读取：get_entry / list_entries / entries_by_source。

use super::Searcher;
use crate::error::KbResult;
use crate::ids::{SourceId, WikiType, WikiId};
use crate::index;
use crate::model::{EntryDocument, EntryMeta};

impl Searcher {
    pub fn get_entry(&self, id: &WikiId) -> KbResult<EntryDocument> {
        self.0.store.load(id)
    }

    pub fn list_entries(&self, wiki_type: Option<WikiType>) -> KbResult<Vec<EntryMeta>> {
        let conn = self.0.lock_conn()?;
        index::list_metas(&conn, wiki_type)
    }

    /// 分页列出条目元数据（id 序，跨页稳定），供 MCP 等有界消费方使用。
    pub fn list_entries_page(
        &self,
        wiki_type: Option<WikiType>,
        limit: usize,
        offset: usize,
    ) -> KbResult<Vec<EntryMeta>> {
        let conn = self.0.lock_conn()?;
        index::list_metas_page(&conn, wiki_type, limit, offset)
    }

    /// 条目总数（可按类型过滤）。
    pub fn count_entries(&self, wiki_type: Option<WikiType>) -> KbResult<usize> {
        let conn = self.0.lock_conn()?;
        index::count_entries(&conn, wiki_type)
    }

    /// 按 updated 倒序的最近条目。
    pub fn recent_entries(&self, limit: usize) -> KbResult<Vec<EntryMeta>> {
        let conn = self.0.lock_conn()?;
        index::recent_metas(&conn, limit)
    }

    /// 按类型聚合计数。
    pub fn count_by_type(&self) -> KbResult<Vec<(WikiType, usize)>> {
        let conn = self.0.lock_conn()?;
        index::count_by_type(&conn)
    }

    /// 按来源反查其贡献过的全部条目。
    pub fn entries_by_source(&self, src: &SourceId) -> KbResult<Vec<EntryMeta>> {
        let conn = self.0.lock_conn()?;
        index::metas_by_source(&conn, src)
    }
}
