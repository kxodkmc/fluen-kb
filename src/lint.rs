//! 只读体检（设计 §8.3）。检测悬空引用的修复路径是 `prune()`。

use crate::error::KbResult;
use crate::handle::KbInner;
use crate::ids::{WikiId, WikiType};
use crate::index;
use crate::ops::Ops;
use crate::syntax;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintIssue {
    pub level: LintLevel,
    pub entry: Option<WikiId>,
    pub message: String,
}

impl LintIssue {
    fn warn(entry: &WikiId, message: impl Into<String>) -> Self {
        LintIssue {
            level: LintLevel::Warning,
            entry: Some(entry.clone()),
            message: message.into(),
        }
    }

    fn error(entry: &WikiId, message: impl Into<String>) -> Self {
        LintIssue {
            level: LintLevel::Error,
            entry: Some(entry.clone()),
            message: message.into(),
        }
    }
}

impl Ops {
    pub fn lint(&self) -> KbResult<Vec<LintIssue>> {
        let inner: &KbInner = &self.0;
        let files = inner.store.discover()?;
        let docs: Vec<_> = files
            .iter()
            .map(|(_, path)| inner.store.load_path(path))
            .collect::<KbResult<_>>()?;

        let titles: HashMap<WikiId, String> = docs
            .iter()
            .map(|d| (d.id.clone(), d.title.clone()))
            .collect();
        let existing: HashSet<WikiId> = titles.keys().cloned().collect();

        let db_metas: HashMap<WikiId, crate::model::EntryMeta> = {
            let conn = inner.lock_conn()?;
            index::list_metas(&conn, None)?
                .into_iter()
                .map(|m| (m.id.clone(), m))
                .collect()
        };

        let mut issues = Vec::new();
        for doc in &docs {
            lint_document(inner, doc, &existing, &titles, &mut issues);

            match db_metas.get(&doc.id) {
                None => issues.push(LintIssue::error(&doc.id, "not indexed; run rebuild")),
                Some(meta) => lint_db_consistency(doc, meta, &existing, &mut issues),
            }
        }
        for id in db_metas.keys() {
            if !existing.contains(id) {
                issues.push(LintIssue::error(id, "indexed but entry file missing; run rebuild"));
            }
        }
        Ok(issues)
    }
}

fn lint_document(
    inner: &KbInner,
    doc: &crate::model::EntryDocument,
    existing: &HashSet<WikiId>,
    titles: &HashMap<WikiId, String>,
    issues: &mut Vec<LintIssue>,
) {
    // 检查 1：未闭合溯源标签（T1 被触发）
    let (_, stats) = syntax::parse_detailed(&doc.body);
    if stats.unmatched_opens > 0 {
        issues.push(LintIssue::warn(
            &doc.id,
            format!("{} unclosed source tag(s); auto-closed at block end", stats.unmatched_opens),
        ));
    }

    // 检查 2/6：正文 [[…]] 悬空与标题提示过期
    for (target, hint) in body_links(&doc.body) {
        match titles.get(&target) {
            None => issues.push(LintIssue::error(
                &doc.id,
                format!("body link points to missing entry {target}"),
            )),
            Some(title) => {
                if let Some(h) = hint
                    && h != *title
                {
                    issues.push(LintIssue::warn(
                        &doc.id,
                        format!("link title hint {h:?} != current title {title:?} of {target}"),
                    ));
                }
            }
        }
    }

    // 检查 3：关联区悬空行与语法无效行
    for line in &doc.relation_lines {
        match crate::store::parse_relation_line(line) {
            Some((_, to)) => {
                if !existing.contains(&to) {
                    issues.push(LintIssue::error(
                        &doc.id,
                        format!("relation line {line:?} points to missing entry {to}"),
                    ));
                }
            }
            None => issues.push(LintIssue::warn(
                &doc.id,
                format!("invalid relation line kept as-is: {line:?}"),
            )),
        }
    }

    // 检查 4：来源存在性（宿主注入校验回调）
    if let Some(validator) = &inner.source_validator {
        for s in syntax::collect_sources(&doc.body) {
            if !validator(&s) {
                issues.push(LintIssue::warn(
                    &doc.id,
                    format!("source {s} unknown to host"),
                ));
            }
        }
    }

    // 检查 5：summary 恰好一个来源
    if doc.wiki_type == WikiType::Summary {
        let n = syntax::collect_sources(&doc.body).len();
        if n != 1 {
            issues.push(LintIssue::error(
                &doc.id,
                format!("summary must carry exactly one source, found {n}"),
            ));
        }
    }
}

/// 检查 7：DB 与 MD 派生一致性（悬空行被跳过属预期，不计入）。
fn lint_db_consistency(
    doc: &crate::model::EntryDocument,
    meta: &crate::model::EntryMeta,
    existing: &HashSet<WikiId>,
    issues: &mut Vec<LintIssue>,
) {
    if meta.title != doc.title {
        issues.push(LintIssue::error(
            &doc.id,
            format!("db title {:?} != file title {:?}", meta.title, doc.title),
        ));
    }
    let mut file_relations: Vec<_> = doc
        .relations
        .iter()
        .filter(|(_, to)| existing.contains(to))
        .cloned()
        .collect();
    file_relations.sort();
    let mut db_relations = meta.relations.clone();
    db_relations.sort();
    if file_relations != db_relations {
        issues.push(LintIssue::error(
            &doc.id,
            "db relations differ from relation section in file",
        ));
    }
}

/// 提取正文 `[[…]]` 链接：(目标 id, 标题提示)。
fn body_links(body: &str) -> Vec<(WikiId, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let Some(rel_end) = rest[start..].find("]]") else {
            break;
        };
        let end = start + rel_end + 2;
        let inner = &rest[start + 2..end - 2];
        let target = inner.rsplit('/').next().unwrap_or(inner);
        if target.len() >= WikiId::PREFIX.len() + 16
            && let Some(id) = WikiId::parse(&target[..WikiId::PREFIX.len() + 16])
        {
            let hint = target[WikiId::PREFIX.len() + 16..]
                .strip_prefix('-')
                .map(str::to_string);
            out.push((id, hint));
        }
        rest = &rest[end..];
    }
    out
}
