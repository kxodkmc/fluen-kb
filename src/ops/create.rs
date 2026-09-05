//! `create`：写时校验 + 去重分流（设计 §8.1）。

use super::{Ops, commit};
use crate::error::{KbError, KbResult};
use crate::handle::KbInner;
use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use crate::index;
use crate::model::EntryDocument;
use crate::store::{self, time};
use crate::syntax;
use std::collections::BTreeMap;

impl Ops {
    pub fn create(
        &self,
        wiki_type: WikiType,
        title: &str,
        body: &str,
        relations: &[(Predicate, WikiId)],
        source: Option<&SourceId>,
        authors: &[WikiId],
    ) -> KbResult<crate::ops::CreateOutcome> {
        let inner = &self.0;
        let title = title.trim();
        if title.is_empty() {
            return Err(KbError::invalid("title must be non-empty"));
        }
        let _write = inner.lock_write()?;

        // D17：入参 body 中的关联区剥离（关联由 relations 参数显式提供）
        let (mut body, _, _) = store::split_relations_section(body);
        if wiki_type == WikiType::Summary {
            body = prepare_summary_body(&body, source)?;
        }

        let mut all_relations: Vec<(Predicate, WikiId)> = relations.to_vec();
        for a in authors {
            all_relations.push((Predicate::new("作者")?, a.clone()));
        }

        let existing = find_duplicate(inner, wiki_type, title, &body, source)?;
        if let Some(id) = existing {
            self.merge_internal(&id, &body, &all_relations)?;
            return Ok(crate::ops::CreateOutcome::MergedInto(id));
        }

        let id = inner.next_id()?;
        let now = time::now_rfc3339();
        let mut relation_lines = Vec::new();
        for (p, t) in &all_relations {
            let line = store::relation_line(p, t);
            if !relation_lines.contains(&line) {
                relation_lines.push(line);
            }
        }
        let relations = relation_lines
            .iter()
            .filter_map(|l| store::parse_relation_line(l))
            .collect();

        let doc = EntryDocument {
            id: id.clone(),
            wiki_type,
            title: title.to_string(),
            created: now.clone(),
            updated: now,
            extra: BTreeMap::new(),
            body,
            had_relations_section: !relation_lines.is_empty(),
            relation_lines,
            relations,
        };
        inner.store.save(&doc)?;

        let mut details = vec![format!("type {wiki_type}")];
        if let Some(s) = source {
            details.push(format!("source {s}"));
        }
        if !all_relations.is_empty() {
            details.push(format!("{} relations", all_relations.len()));
        }
        commit(inner, &id, title, "create", &details)?;
        Ok(crate::ops::CreateOutcome::Created(id))
    }
}

/// summary 单源约束在写时成立（D21）：无包裹且给了 source → 包裹；
/// 已包裹 → 必须与 source 参数一致且恰为一个来源。
fn prepare_summary_body(body: &str, source: Option<&SourceId>) -> KbResult<String> {
    let found = syntax::collect_sources(body);
    match (source, found.as_slice()) {
        (Some(s), []) => Ok(format!("<{s}>{body}</{s}>")),
        (Some(s), [only]) if only == s => Ok(body.to_string()),
        (Some(s), _) => Err(KbError::invalid(format!(
            "summary body sources do not match `source` argument {s}"
        ))),
        (None, [_only]) => Ok(body.to_string()),
        (None, _) => Err(KbError::invalid(
            "summary must carry exactly one source: wrap body in a source tag or pass `source`",
        )),
    }
}

fn find_duplicate(
    inner: &KbInner,
    wiki_type: WikiType,
    title: &str,
    body: &str,
    source: Option<&SourceId>,
) -> KbResult<Option<WikiId>> {
    let conn = inner.lock_conn()?;
    match wiki_type {
        WikiType::Summary => {
            let key = match source {
                Some(s) => s.clone(),
                None => syntax::collect_sources(body)
                    .first()
                    .cloned()
                    .ok_or_else(|| KbError::invalid("summary requires a source"))?,
            };
            index::find_summary_by_source(&conn, &key)
        }
        _ => index::find_by_title(&conn, wiki_type, title),
    }
}
