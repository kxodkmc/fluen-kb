//! L2 检索层：只读查询与打分（设计 §7）。

mod keyword;
mod meta;
mod rank;

use crate::embed;
use crate::error::{KbError, KbResult};
use crate::handle::KbInner;
use crate::ids::WikiType;
use crate::model::EntryMeta;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMethod {
    Keyword,
    Semantic,
    #[default]
    Hybrid,
}

#[derive(Debug, Clone, Default)]
pub struct QueryParams {
    pub query: String,
    pub wiki_type: Option<WikiType>,
    pub method: SearchMethod,
    pub top_k: Option<usize>,
    pub include_content: bool,
    pub expand: u8,
}

impl QueryParams {
    fn top_k(&self) -> usize {
        self.top_k.unwrap_or(10).max(1)
    }
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub meta: EntryMeta,
    pub score: f64,
    pub content: Option<String>,
}

pub struct Searcher(pub(crate) Arc<KbInner>);

impl Searcher {
    pub fn query(&self, params: &QueryParams) -> KbResult<Vec<SearchHit>> {
        if params.query.trim().is_empty() {
            return Err(KbError::invalid("query must be non-empty"));
        }
        if params.expand > 2 {
            return Err(KbError::invalid("expand must be <= 2"));
        }

        let inner = &self.0;
        let conn = inner.lock_conn()?;

        let mut scored = keyword::direct_lookup(&conn, &params.query)?;
        if scored.is_empty() {
            let kw = keyword::search(&conn, &params.query)?;
            scored = match params.method {
                SearchMethod::Keyword => kw,
                SearchMethod::Semantic => match self.semantic(&conn, &params.query)? {
                    Some(sem) => sem,
                    None => kw,
                },
                SearchMethod::Hybrid => {
                    let sem = self.semantic(&conn, &params.query)?.unwrap_or_default();
                    rank::fuse(kw, sem)
                }
            };
        }

        if let Some(t) = params.wiki_type {
            let mut kept = Vec::new();
            for (id, score) in scored {
                if entry_type(&conn, &id)? == Some(t) {
                    kept.push((id, score));
                }
            }
            scored = kept;
        }
        if params.expand > 0 {
            scored = rank::expand(&conn, scored, params.expand)?;
        }

        // 索引是纯缓存：命中条目的 md 文件已被外部删除时清行并跳过（自愈），
        // 而非返回幽灵命中或让整个查询报错。
        let existing: std::collections::HashSet<crate::ids::WikiId> = inner
            .store
            .discover()?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        for (id, _) in &scored {
            if !existing.contains(id) {
                crate::index::remove_entry_rows(&conn, id)?;
            }
        }
        scored.retain(|(id, _)| existing.contains(id));

        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(params.top_k());

        scored
            .iter()
            .map(|(id, score)| {
                let meta = crate::index::get_meta(&conn, id)?;
                let content = if params.include_content {
                    Some(inner.store.load(id)?.body)
                } else {
                    None
                };
                Ok(SearchHit {
                    meta,
                    score: *score,
                    content,
                })
            })
            .collect()
    }

    /// 非陈旧向量上的余弦相似度；不可用（无 provider / 无新鲜向量）返回 None。
    fn semantic(
        &self,
        conn: &rusqlite::Connection,
        query: &str,
    ) -> KbResult<Option<Vec<(crate::ids::WikiId, f64)>>> {
        let inner = &self.0;
        let Some(provider) = embed::current_provider(inner) else {
            return Ok(None);
        };

        let rows = {
            let mut stmt = conn.prepare(
                "SELECT entry_id, embedding, content_hash FROM embeddings",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            rows.flatten().collect::<Vec<_>>()
        };

        let mut scored = Vec::new();
        for (entry_id, blob, stored_hash) in rows {
            let id = crate::ids::WikiId::new(&entry_id)?;
            let Ok(doc) = inner.store.load(&id) else {
                continue;
            };
            let (plain, _) = crate::syntax::strip(&doc.body);
            if embed::content_hash(&doc.title, &plain) != stored_hash {
                continue;
            }
            scored.push((id, blob));
        }
        if scored.is_empty() {
            return Ok(None);
        }

        let query_vec = provider
            .embed(query)
            .map_err(|e| KbError::Embedding(format!("{e}")))?;
        Ok(Some(
            scored
                .into_iter()
                .map(|(id, blob)| {
                    let v = embed::decode_vector(&blob);
                    (id, rank::cosine(&query_vec, &v))
                })
                .collect(),
        ))
    }
}

fn entry_type(
    conn: &rusqlite::Connection,
    id: &crate::ids::WikiId,
) -> KbResult<Option<WikiType>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT type FROM entries WHERE id = ?1",
        rusqlite::params![id.as_str()],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|t| WikiType::parse(&t).ok_or_else(|| KbError::invalid(format!("unknown type {t:?}"))))
    .transpose()
}
