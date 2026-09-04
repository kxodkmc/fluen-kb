//! `migrate`：存量一次性迁移（设计 §10.3）。幂等：转换后键已移除，二次执行为 no-op。

use super::Ops;
use crate::error::{KbError, KbResult};
use crate::ids::{SourceId, WikiId, WikiType};
use crate::index::views;
use crate::ops::commit;
use crate::store;
use crate::syntax;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MigrateReport {
    pub wrapped: usize,
    pub authors_converted: usize,
    pub tags_removed: usize,
}

impl Ops {
    pub fn migrate(&self) -> KbResult<MigrateReport> {
        let inner = &self.0;
        let mut report = MigrateReport::default();

        for (id, path) in inner.store.discover()? {
            let mut doc = inner.store.load_path(&path)?;
            let mut changed = false;

            if doc.wiki_type == WikiType::Summary {
                if let Some(raw) = doc.extra.remove("source") {
                    let src = normalize_source_value(raw.trim())?;
                    match syntax::collect_sources(&doc.body).as_slice() {
                        [] => {
                            doc.body = format!("<{src}>{}</{src}>", doc.body);
                            report.wrapped += 1;
                        }
                        [only] if *only == src => {}
                        other => {
                            return Err(KbError::invalid(format!(
                                "migrate: source key {src} conflicts with body sources {other:?} in {id}"
                            )));
                        }
                    }
                    changed = true;
                }

                if let Some(raw) = doc.extra.remove("authors") {
                    for author in extract_wiki_ids(&raw) {
                        let already = doc.relation_lines.iter().any(|l| {
                            store::parse_relation_line(l)
                                .is_some_and(|(_, to)| to == author)
                        });
                        if !already {
                            doc.relation_lines
                                .push(store::relation_line(&author_predicate()?, &author));
                        }
                        report.authors_converted += 1;
                    }
                    changed = true;
                }
            }

            if doc.extra.remove("tags").is_some() {
                report.tags_removed += 1;
                changed = true;
            }

            if changed {
                doc.had_relations_section |= !doc.relation_lines.is_empty();
                doc.relations = doc
                    .relation_lines
                    .iter()
                    .filter_map(|l| store::parse_relation_line(l))
                    .collect();
                doc.updated = store::time::now_rfc3339();
                inner.store.save(&doc)?;
                commit(inner, &id, &doc.title, "migrate", &["存量条目格式迁移".to_string()])?;
            }
        }
        {
            let conn = inner.lock_conn()?;
            views::rebuild_index_md(&conn, &inner.store)?;
        }
        Ok(report)
    }
}

/// `raw/ref-xxx.pdf` / `ref-xxx.pdf` / `ref-xxx` → `ref-xxx`。
fn normalize_source_value(value: &str) -> KbResult<SourceId> {
    let value = value.trim_matches('"');
    let base = value.rsplit('/').next().unwrap_or(value);
    let stem = base.rsplit_once('.').map_or(base, |(s, _)| s);
    SourceId::new(stem)
}

fn author_predicate() -> KbResult<crate::ids::Predicate> {
    crate::ids::Predicate::new("作者")
}

/// 从旧 authors 值中提取全部 wiki-xxx 标识。
fn extract_wiki_ids(raw: &str) -> Vec<WikiId> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + WikiId::PREFIX.len() + 16 <= bytes.len() {
        if raw[i..].starts_with(WikiId::PREFIX) {
            let end = i + WikiId::PREFIX.len() + 16;
            if let Some(id) = WikiId::parse(&raw[i..end]) {
                if !out.contains(&id) {
                    out.push(id);
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_value_normalization() {
        assert_eq!(
            normalize_source_value("ref-aaaaaaaaaaaaaaaa").unwrap(),
            SourceId::new("ref-aaaaaaaaaaaaaaaa").unwrap()
        );
        assert_eq!(
            normalize_source_value("raw/ref-aaaaaaaaaaaaaaaa.pdf").unwrap(),
            SourceId::new("ref-aaaaaaaaaaaaaaaa").unwrap()
        );
        assert!(normalize_source_value("not-a-ref").is_err());
    }

    #[test]
    fn wiki_id_extraction() {
        let raw = "[\"wiki-91f2c43bf4de401e\", \"wiki-3d09a8bb72c14e5f\"]";
        let ids = extract_wiki_ids(raw);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&WikiId::new("wiki-3d09a8bb72c14e5f").unwrap()));
    }
}
