//! L3 异步句柄：`spawn_blocking` 薄包装，方法面与 `Kb` 一一对应。

use crate::error::{KbError, KbResult};
use crate::handle::KbInner;
use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use crate::model::{EntryDocument, EntryMeta};
use crate::ops::{CreateOutcome, EditOp, Ops};
use crate::search::{QueryParams, SearchHit, Searcher};
use std::sync::Arc;

#[derive(Clone)]
pub struct AsyncKb(pub(crate) Arc<KbInner>);

async fn run<T, F>(op: F) -> KbResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> KbResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|e| KbError::Cancelled(format!("blocking task failed: {e}")))?
}

impl AsyncKb {
    pub async fn create(
        &self,
        wiki_type: WikiType,
        title: String,
        body: String,
        relations: Vec<(Predicate, WikiId)>,
        source: Option<SourceId>,
        authors: Vec<WikiId>,
    ) -> KbResult<CreateOutcome> {
        let inner = self.0.clone();
        run(move || {
            Ops(inner).create(
                wiki_type,
                &title,
                &body,
                &relations,
                source.as_ref(),
                &authors,
            )
        })
        .await
    }

    pub async fn merge(
        &self,
        id: WikiId,
        body: String,
        relations: Vec<(Predicate, WikiId)>,
    ) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || Ops(inner).merge(&id, &body, &relations)).await
    }

    pub async fn upsert_from_source(
        &self,
        id: WikiId,
        src: SourceId,
        new_body: String,
    ) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || Ops(inner).upsert_from_source(&id, &src, &new_body)).await
    }

    pub async fn edit(&self, id: WikiId, ops: Vec<EditOp>) -> KbResult<Vec<KbResult<()>>> {
        let inner = self.0.clone();
        run(move || Ops(inner).edit(&id, ops)).await
    }

    pub async fn rename(&self, id: WikiId, new_title: String) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || Ops(inner).rename(&id, &new_title)).await
    }

    pub async fn delete(&self, id: WikiId) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || Ops(inner).delete(&id)).await
    }

    pub async fn delete_source(&self, src: SourceId) -> KbResult<crate::ops::DeleteSourceReport> {
        let inner = self.0.clone();
        run(move || Ops(inner).delete_source(&src)).await
    }

    pub async fn prune(&self) -> KbResult<crate::ops::PruneReport> {
        let inner = self.0.clone();
        run(move || Ops(inner).prune()).await
    }

    pub async fn migrate(&self) -> KbResult<crate::ops::MigrateReport> {
        let inner = self.0.clone();
        run(move || Ops(inner).migrate()).await
    }

    pub async fn lint(&self) -> KbResult<Vec<crate::lint::LintIssue>> {
        let inner = self.0.clone();
        run(move || Ops(inner).lint()).await
    }

    pub async fn query(&self, params: QueryParams) -> KbResult<Vec<SearchHit>> {
        let inner = self.0.clone();
        run(move || Searcher(inner).query(&params)).await
    }

    pub async fn get_entry(&self, id: WikiId) -> KbResult<EntryDocument> {
        let inner = self.0.clone();
        run(move || Searcher(inner).get_entry(&id)).await
    }

    pub async fn list_entries(&self, wiki_type: Option<WikiType>) -> KbResult<Vec<EntryMeta>> {
        let inner = self.0.clone();
        run(move || Searcher(inner).list_entries(wiki_type)).await
    }

    pub async fn entries_by_source(&self, src: SourceId) -> KbResult<Vec<EntryMeta>> {
        let inner = self.0.clone();
        run(move || Searcher(inner).entries_by_source(&src)).await
    }

    pub async fn rebuild(&self) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || {
            let conn = inner.lock_conn()?;
            crate::index::rebuild(&conn, &inner.store)
        })
        .await
    }

    pub async fn attach_embedding(
        &self,
        provider: Arc<dyn crate::embed::KnowledgeEmbedding>,
    ) -> KbResult<()> {
        let inner = self.0.clone();
        run(move || crate::embed::EmbedHandle(inner).attach(provider)).await
    }
}
