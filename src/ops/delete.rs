//! `delete` / `delete_source` / `prune`：删除与悬空引用治理（设计 §8.1、D24、D25）。

use super::{Ops, commit};
use crate::error::KbResult;
use crate::ids::{SourceId, WikiId};
use crate::index::{self, views};
use crate::model::EntryDocument;
use crate::store;
use crate::syntax;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteSourceReport {
    /// 随来源一并删除的条目（单源 summary、摘除后为空的条目）。
    pub removed: Vec<WikiId>,
    /// 摘除来源后仍存留的条目。
    pub updated: Vec<WikiId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneReport {
    /// (条目, 清除的悬空引用数)。
    pub entries: Vec<(WikiId, usize)>,
}

impl Ops {
    pub fn delete(&self, id: &WikiId) -> KbResult<()> {
        let inner = &self.0;
        let _write = inner.lock_write()?;
        delete_entry(inner, id)
    }

    pub fn delete_source(&self, src: &SourceId) -> KbResult<DeleteSourceReport> {
        let inner = &self.0;
        let _write = inner.lock_write()?;
        let metas = {
            let conn = inner.lock_conn()?;
            index::metas_by_source(&conn, src)?
        };

        let mut report = DeleteSourceReport {
            removed: Vec::new(),
            updated: Vec::new(),
        };
        for meta in metas {
            let mut doc = inner.store.load(&meta.id)?;
            let sources = syntax::collect_sources(&doc.body);

            // 来源恰为 src 的 summary 随之整体删除
            if doc.wiki_type == crate::ids::WikiType::Summary
                && sources.as_slice() == [src.clone()]
            {
                delete_entry(inner, &meta.id)?;
                report.removed.push(meta.id);
                continue;
            }

            let (new_body, _) = syntax::remove_source(&doc.body, src);
            doc.body = new_body;
            let (plain, _) = syntax::strip(&doc.body);
            if plain.trim().is_empty() && doc.relations.is_empty() {
                inner.store.delete_file(&doc.id)?;
                let conn = inner.lock_conn()?;
                index::remove_entry_rows(&conn, &doc.id)?;
                views::rebuild_index_md(&conn, &inner.store)?;
                drop(conn);
                views::append_log(
                    &inner.store,
                    "delete",
                    doc.id.as_str(),
                    &doc.title,
                    &[format!("来源 {src} 移除后条目为空，自动删除")],
                )?;
                report.removed.push(doc.id);
            } else {
                doc.updated = crate::store::time::now_rfc3339();
                inner.store.save(&doc)?;
                commit(
                    inner,
                    &doc.id,
                    &doc.title,
                    "delete",
                    &[format!("来源 {src} 移除")],
                )?;
                report.updated.push(doc.id);
            }
        }
        Ok(report)
    }

    /// 悬空引用修复原语：落盘清除，与 lint 构成"检测 → 修复"闭环。
    pub fn prune(&self) -> KbResult<PruneReport> {
        let inner = &self.0;
        let _write = inner.lock_write()?;
        let files = inner.store.discover()?;
        let existing: HashSet<WikiId> = files.iter().map(|(id, _)| id.clone()).collect();

        let mut report = PruneReport { entries: Vec::new() };
        for (id, path) in files {
            let mut doc = inner.store.load_path(&path)?;
            let removed = clean_references(&mut doc, |t| !existing.contains(t));
            if removed > 0 {
                inner.store.save(&doc)?;
                commit(
                    inner,
                    &id,
                    &doc.title,
                    "prune",
                    &[format!("清除 {removed} 处悬空引用")],
                )?;
                report.entries.push((id, removed));
            }
        }
        Ok(report)
    }
}

/// 删除条目并清理全库对其的引用（调用方须持有写锁）。
fn delete_entry(inner: &crate::handle::KbInner, id: &WikiId) -> KbResult<()> {
    let doc = inner.store.load(id)?;

    let mut cleaned = Vec::new();
    for (other_id, path) in inner.store.discover()? {
        if other_id == *id {
            continue;
        }
        let mut other = inner.store.load_path(&path)?;
        if clean_references(&mut other, |t| t == id) > 0 {
            inner.store.save(&other)?;
            cleaned.push(other_id);
        }
    }

    inner.store.delete_file(id)?;
    {
        let conn = inner.lock_conn()?;
        index::remove_entry_rows(&conn, id)?;
        for oid in &cleaned {
            index::reindex_entry(&conn, &inner.store, oid)?;
        }
        views::rebuild_index_md(&conn, &inner.store)?;
    }

    let mut details = Vec::new();
    if !cleaned.is_empty() {
        details.push(format!("cleaned references in {} entries", cleaned.len()));
    }
    views::append_log(&inner.store, "delete", id.as_str(), &doc.title, &details)
}

/// 清除指向 `matches` 目标的引用：关联区整行删；正文 `[[…]]` 只摘链接
/// token、保留标题提示文本与所在句子（D24）。返回清除数。
pub(crate) fn clean_references(doc: &mut EntryDocument, matches: impl Fn(&WikiId) -> bool) -> usize {
    let mut count = 0;

    let before = doc.relation_lines.len();
    doc.relation_lines.retain(|line| match store::parse_relation_line(line) {
        Some((_, to)) => !matches(&to),
        None => true,
    });
    count += before - doc.relation_lines.len();
    doc.relations.retain(|(_, to)| !matches(to));

    doc.body = strip_body_links(&doc.body, &matches, &mut count);
    count
}

/// `[[dir/]]wikiId[-title]]` → title（无提示则删除 token）。
/// 与 lint 的 body_links 同一识别规则（前缀解析，含标题提示）；
/// 代码区内的链接是文档示例，不参与清理。
fn strip_body_links(body: &str, matches: &impl Fn(&WikiId) -> bool, count: &mut usize) -> String {
    let exempt = syntax::code_regions(body);
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        let Some(rel_end) = rest[start..].find("]]") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let end = start + rel_end + 2;
        let offset = body.len() - rest.len() + start;
        let target = rest[start + 2..end - 2].rsplit('/').next().unwrap_or("");
        match WikiId::split_prefix(target) {
            Some((id, hint)) if matches(&id) && !exempt.iter().any(|r| r.contains(&offset)) => {
                *count += 1;
                out.push_str(hint.strip_prefix('-').unwrap_or(""));
            }
            _ => out.push_str(&rest[start..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}
