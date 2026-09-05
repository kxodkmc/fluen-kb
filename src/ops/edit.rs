//! `edit`：三原语（设计 §8.2）。scope 限定使查找可跨标签边界。

use super::{EditOp, Ops, commit};
use crate::error::{KbError, KbResult};
use crate::ids::{SourceId, WikiType, WikiId};
use crate::store::time;
use crate::syntax;
use crate::syntax::ops::render;
use crate::syntax::Span;

impl Ops {
    /// 逐项报告成败，成功的项整体提交（D22）。
    pub fn edit(&self, id: &WikiId, ops: Vec<EditOp>) -> KbResult<Vec<KbResult<()>>> {
        let inner = &self.0;
        let _write = inner.lock_write()?;
        let mut doc = inner.store.load(id)?;
        let original_sources = syntax::collect_sources(&doc.body);

        let mut results = Vec::new();
        let mut applied = 0;
        for op in ops {
            let before = doc.body.clone();
            match apply_edit(&mut doc.body, op) {
                Ok(()) => {
                    let sources_stable = doc.wiki_type != WikiType::Summary
                        || syntax::collect_sources(&doc.body) == original_sources;
                    if sources_stable {
                        results.push(Ok(()));
                        applied += 1;
                    } else {
                        doc.body = before;
                        results.push(Err(KbError::invalid(
                            "edit would change the source set of a single-source summary",
                        )));
                    }
                }
                Err(e) => {
                    doc.body = before;
                    results.push(Err(e));
                }
            }
        }

        if applied > 0 {
            doc.updated = time::now_rfc3339();
            inner.store.save(&doc)?;
            let details = vec![format!("{applied}/{} ops applied", results.len())];
            commit(inner, id, &doc.title, "edit", &details)?;
        }
        Ok(results)
    }
}

fn apply_edit(body: &mut String, op: EditOp) -> KbResult<()> {
    match op {
        EditOp::SearchReplace {
            search,
            replace,
            scope: None,
        } => {
            non_empty(&search, "search")?;
            let idx = body
                .find(&search)
                .ok_or_else(|| KbError::not_found(format!("search text {search:?}")))?;
            body.replace_range(idx..idx + search.len(), &replace);
            Ok(())
        }
        EditOp::SearchReplace {
            search,
            replace,
            scope: Some(s),
        } => scoped_splice(body, &s, &replace, |text| {
            non_empty(&search, "search")?;
            let idx = text
                .find(&search)
                .ok_or_else(|| KbError::not_found(format!("search text {search:?} in scope {s}")))?;
            Ok((idx, search.len()))
        }),
        EditOp::InsertAfter {
            anchor,
            content,
            scope: None,
        } => {
            non_empty(&anchor, "anchor")?;
            let idx = body
                .find(&anchor)
                .ok_or_else(|| KbError::not_found(format!("anchor {anchor:?}")))?;
            body.insert_str(idx + anchor.len(), &content);
            Ok(())
        }
        EditOp::InsertAfter {
            anchor,
            content,
            scope: Some(s),
        } => scoped_splice(body, &s, &content, |text| {
            non_empty(&anchor, "anchor")?;
            let idx = text
                .find(&anchor)
                .ok_or_else(|| KbError::not_found(format!("anchor {anchor:?} in scope {s}")))?;
            Ok((idx + anchor.len(), 0))
        }),
        EditOp::ReplaceSource { src, content } => {
            *body = syntax::replace_source(body, &src, &content);
            Ok(())
        }
    }
}

fn non_empty(value: &str, what: &str) -> KbResult<()> {
    if value.is_empty() {
        return Err(KbError::invalid(format!("{what} must be non-empty")));
    }
    Ok(())
}

/// 作用域内编辑：把该来源的片段文本拼接后定位匹配点，再逐片段拼接改写，
/// 新内容自动并入所在片段的归属集。整体经 render 重建。
/// `locate` 返回 (匹配起点, 删除长度)，`insert` 为待插入文本。
fn scoped_splice(
    body: &mut String,
    scope: &SourceId,
    insert: &str,
    locate: impl Fn(&str) -> KbResult<(usize, usize)>,
) -> KbResult<()> {
    let tree = syntax::parse(body);
    let in_scope: Vec<usize> = tree
        .spans
        .iter()
        .enumerate()
        .filter(|(_, s)| s.sources.contains(scope))
        .map(|(i, _)| i)
        .collect();
    if in_scope.is_empty() {
        return Err(KbError::not_found(format!("scope source {scope} not present")));
    }

    let concat: String = in_scope
        .iter()
        .map(|&i| tree.spans[i].text.as_str())
        .collect();
    let (pos, remove_len) = locate(&concat)?;

    let mut spans: Vec<Span> = tree.spans.clone();
    let mut insert_done = false;
    let mut offset = 0;
    for &i in &in_scope {
        let span = &mut spans[i];
        let len = span.text.len();
        let (a, b) = (offset, offset + len);
        offset = b;

        if b <= pos || (a >= pos + remove_len && a > pos) {
            continue;
        }
        let mut new_text = String::new();
        if pos > a {
            new_text.push_str(&span.text[..pos - a]);
        }
        if !insert_done {
            new_text.push_str(insert);
            insert_done = true;
        }
        let cut_end = (pos + remove_len).saturating_sub(a);
        if cut_end < len {
            new_text.push_str(&span.text[cut_end.min(len)..]);
        }
        span.text = new_text;
    }

    *body = render(&spans);
    Ok(())
}
