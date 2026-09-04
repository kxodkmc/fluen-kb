//! `merge` / `upsert_from_source`：重导入核心原语（设计 §8.1）。

use super::{Ops, commit};
use crate::error::{KbError, KbResult};
use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use crate::store::{self, time};
use crate::syntax;

impl Ops {
    pub fn merge(
        &self,
        id: &WikiId,
        body: &str,
        relations: &[(Predicate, WikiId)],
    ) -> KbResult<()> {
        self.merge_internal(id, body, relations)
    }

    pub(crate) fn merge_internal(
        &self,
        id: &WikiId,
        body: &str,
        relations: &[(Predicate, WikiId)],
    ) -> KbResult<()> {
        let inner = &self.0;
        let mut doc = inner.store.load(id)?;
        let (body, _, _) = store::split_relations_section(body);
        if body.trim().is_empty() && relations.is_empty() {
            return Err(KbError::invalid("merge requires body or relations"));
        }

        if doc.wiki_type == WikiType::Summary {
            let existing = syntax::collect_sources(&doc.body);
            for s in syntax::collect_sources(&body) {
                if !existing.contains(&s) {
                    return Err(KbError::invalid(format!(
                        "summary {id} must stay single-source; body introduces extra source {s}"
                    )));
                }
            }
        }

        let mut details = Vec::new();
        if !body.trim().is_empty() {
            if !doc.body.is_empty() {
                if !doc.body.ends_with('\n') {
                    doc.body.push('\n');
                }
                doc.body.push('\n');
            }
            doc.body.push_str(body.trim_end_matches('\n'));
            if syntax::collect_sources(&body).is_empty() {
                details.push(format!("追加未溯源内容（{} 字符）", body.len()));
            } else {
                details.push(format!("追加 {} 字符", body.len()));
            }
        }

        let mut added = 0;
        for (p, t) in relations {
            if !doc.relations.contains(&(p.clone(), t.clone())) {
                doc.relation_lines.push(store::relation_line(p, t));
                doc.relations.push((p.clone(), t.clone()));
                added += 1;
            }
        }
        if added > 0 {
            details.push(format!("+{added} relation"));
        }

        doc.updated = time::now_rfc3339();
        inner.store.save(&doc)?;
        commit(inner, id, &doc.title, "merge", &details)
    }

    /// 同来源替换、异来源追加、跨来源零干扰（D16 + §3.2.6）。
    pub fn upsert_from_source(
        &self,
        id: &WikiId,
        src: &SourceId,
        new_body: &str,
    ) -> KbResult<()> {
        let inner = &self.0;
        let mut doc = inner.store.load(id)?;
        let (new_body, _, _) = store::split_relations_section(new_body);
        let old_len = doc.body.chars().count();

        doc.body = syntax::replace_source(&doc.body, src, new_body.trim_end_matches('\n'));

        if doc.wiki_type == WikiType::Summary {
            let sources = syntax::collect_sources(&doc.body);
            let ok = sources.as_slice() == [src.clone()] || sources.is_empty();
            if !ok {
                return Err(KbError::invalid(format!(
                    "upsert would break single-source constraint of summary {id}"
                )));
            }
        }

        doc.updated = time::now_rfc3339();
        let new_len = doc.body.chars().count();
        let details = vec![format!(
            "来源 {src} 子树替换（旧 {old_len} 字符 → 新 {new_len} 字符）"
        )];
        inner.store.save(&doc)?;
        commit(inner, id, &doc.title, "upsert", &details)
    }
}
