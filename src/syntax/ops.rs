//! 溯源标记的内容变换：过滤、按归属集剪枝（设计 §3.2.6）、替换、规范化。
//!
//! 全部变换消费同一片段序列，输出统一经 `render` 重建——
//! 重建按"剩余归属集"重新包裹标签，天然折叠 T5、合并相邻同源片段。

use crate::ids::SourceId;
use crate::syntax::{Span, parse};

/// 来源清单：首次出现顺序，去重。
pub fn collect_sources(body: &str) -> Vec<SourceId> {
    let mut out = Vec::new();
    for span in &parse(body).spans {
        for s in &span.sources {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
    }
    out
}

/// 抽取归属含 `s` 的内容；保留其余来源的包裹（`s` 自身除外）。
pub fn filter_by_source(body: &str, s: &SourceId) -> String {
    let mut spans = Vec::new();
    for span in parse(body).spans {
        if span.sources.contains(s) {
            spans.push(reduce(&span, s));
        }
    }
    render(&spans)
}

/// 按归属集剪枝：独属删除、共同归属摘边降归属、无关保留。
pub fn remove_source(body: &str, s: &SourceId) -> (String, bool) {
    let mut touched = false;
    let mut spans = Vec::new();
    for span in parse(body).spans {
        if !span.sources.contains(s) {
            spans.push(span);
            continue;
        }
        touched = true;
        if span.sources.len() > 1 {
            spans.push(reduce(&span, s));
        }
    }
    (render(&spans), touched)
}

/// `remove_source` + 追加新子树；`new` 已整体归属 `s` 时原样追加。
pub fn replace_source(body: &str, s: &SourceId, new: &str) -> String {
    let (mut out, _) = remove_source(body, s);
    if new.trim().is_empty() {
        return out;
    }
    if collect_sources(new) == vec![s.clone()] {
        append_subtree(&mut out, new);
        return out;
    }
    append_subtree(&mut out, &format!("<{s}>{new}</{s}>"));
    out
}

/// 规范化：T1 落地为显式闭合、T5 折叠、T7 清除空标签。不改变归属语义。
pub fn normalize(body: &str) -> String {
    render(&parse(body).spans)
}

fn append_subtree(out: &mut String, subtree: &str) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(subtree);
}

fn reduce(span: &Span, s: &SourceId) -> Span {
    let mut sources = span.sources.clone();
    sources.retain(|x| x != s);
    Span {
        text: span.text.clone(),
        sources,
    }
}

/// 由片段序列重建正文：归属集变化处调整包裹；相邻同集片段共用一组标签。
/// 新集合为当前开放集的超集（前缀嵌套）时只补开增量，否则全闭重开（T5 折叠）。
pub(crate) fn render(spans: &[Span]) -> String {
    let kept = edible_spans(spans);
    let mut out = String::new();
    let mut open: Vec<SourceId> = Vec::new();

    for span in kept {
        if span.sources != open {
            if is_nested_into(&open, &span.sources) {
                for id in &span.sources {
                    if !open.contains(id) {
                        out.push_str(&format!("<{id}>"));
                    }
                }
            } else {
                close_all(&mut out, &open);
                for id in &span.sources {
                    out.push_str(&format!("<{id}>"));
                }
            }
            open = span.sources.clone();
        }
        out.push_str(&span.text);
    }
    close_all(&mut out, &open);
    out
}

fn is_nested_into(open: &[SourceId], next: &[SourceId]) -> bool {
    next.len() >= open.len() && next[..open.len()] == *open
}

fn close_all(out: &mut String, open: &[SourceId]) {
    for id in open.iter().rev() {
        out.push_str(&format!("</{id}>"));
    }
}

/// 丢弃空文本片段（T7）；丢弃夹在两个同源片段之间的行内空白
/// （合并相邻同源片段时避免残留孤立空格）。
fn edible_spans(spans: &[Span]) -> Vec<&Span> {
    let idx: Vec<usize> = spans
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.text.is_empty())
        .map(|(i, _)| i)
        .collect();

    let mut keep = vec![true; spans.len()];
    for w in 1..idx.len().saturating_sub(1) {
        let (prev, cur, next) = (idx[w - 1], idx[w], idx[w + 1]);
        let inline_ws = spans[cur].sources.is_empty()
            && !spans[cur].text.contains('\n')
            && spans[cur].text.chars().all(char::is_whitespace);
        if inline_ws
            && !spans[prev].sources.is_empty()
            && spans[prev].sources == spans[next].sources
        {
            keep[cur] = false;
        }
    }
    idx.iter().filter(|&&i| keep[i]).map(|&i| &spans[i]).collect()
}
