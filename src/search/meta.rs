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

    /// 按来源反查其贡献过的全部条目。
    pub fn entries_by_source(&self, src: &SourceId) -> KbResult<Vec<EntryMeta>> {
        let conn = self.0.lock_conn()?;
        index::metas_by_source(&conn, src)
    }
}
