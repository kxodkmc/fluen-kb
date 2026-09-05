//! 容错溯源解析器（设计 §3.2.3 T1–T8）。
//!
//! 单遍扫描 + 开放标签栈，输出统一的片段序列（`Vec<Span>`）。
//! 片段的归属集 = 覆盖该位置的所有标签区间之并集。

use crate::ids::SourceId;
use crate::syntax::{ParseStats, SourceMap, SourceTree, Span};
use std::ops::Range;

struct TagTok {
    pos: usize,
    end: usize,
    id: SourceId,
    is_close: bool,
}

struct Interval {
    id: SourceId,
    start: usize,
    end: usize,
}

pub fn parse_detailed(text: &str) -> (SourceTree, ParseStats) {
    let exempt = code_regions(text);
    let toks = scan_tags(text, &exempt);
    let block_list = blocks(text);
    let (intervals, stats) = match_tags(text, &toks, &block_list);
    let spans = build_spans(text, &toks, &intervals);
    (SourceTree { spans }, stats)
}

pub fn strip(text: &str) -> (String, SourceMap) {
    let tree = crate::syntax::parse(text);
    let mut plain = String::new();
    let mut map = Vec::new();
    for span in &tree.spans {
        if span.text.is_empty() {
            continue;
        }
        let start = plain.len();
        plain.push_str(&span.text);
        map.push((start..plain.len(), span.sources.clone()));
    }
    (plain, map)
}

fn match_tags(
    text: &str,
    toks: &[TagTok],
    block_list: &[Range<usize>],
) -> (Vec<Interval>, ParseStats) {
    let mut intervals = Vec::new();
    let mut stack: Vec<(SourceId, usize)> = Vec::new();
    let mut orphan_closes = 0;

    for tok in toks {
        if !tok.is_close {
            stack.push((tok.id.clone(), tok.end));
            continue;
        }
        // T5：闭标签弹出栈中最近的同名开标签；被跨越的开标签保持开放（隐式闭合后重开）
        if let Some(i) = stack.iter().rposition(|(id, _)| *id == tok.id) {
            let (id, content_start) = stack.remove(i);
            intervals.push(Interval {
                id,
                start: content_start,
                end: tok.pos,
            });
        } else {
            // T2：孤立闭标签
            orphan_closes += 1;
        }
    }

    let unmatched_opens = stack.len();
    for (id, content_start) in stack {
        // T1：未闭合的开标签在所在块末尾自动闭合
        let end = block_end(block_list, text.len(), content_start);
        intervals.push(Interval {
            id,
            start: content_start,
            end,
        });
    }

    intervals.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    (intervals, ParseStats {
        unmatched_opens,
        orphan_closes,
    })
}

/// 片段 = 标签 token 之间的文本区段，再按归属区间边界细分；
/// 片段归属集 = 覆盖该片段起点的全部区间之并集。
fn build_spans(text: &str, toks: &[TagTok], intervals: &[Interval]) -> Vec<Span> {
    let mut cuts = vec![0, text.len()];
    for iv in intervals {
        cuts.push(iv.start.min(text.len()));
        cuts.push(iv.end.min(text.len()));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let mut spans = Vec::new();
    let mut cursor = 0;
    for tok in toks {
        emit_range(text, cursor, tok.pos, &cuts, intervals, &mut spans);
        cursor = tok.end;
    }
    emit_range(text, cursor, text.len(), &cuts, intervals, &mut spans);
    spans
}

fn emit_range(
    text: &str,
    a: usize,
    b: usize,
    cuts: &[usize],
    intervals: &[Interval],
    spans: &mut Vec<Span>,
) {
    if a >= b {
        return;
    }
    let mut start = a;
    for &c in cuts.iter().filter(|&&c| c > a && c < b) {
        push_span(text, start, c, intervals, spans);
        start = c;
    }
    push_span(text, start, b, intervals, spans);
}

fn push_span(text: &str, a: usize, b: usize, intervals: &[Interval], spans: &mut Vec<Span>) {
    let mut sources = Vec::new();
    for iv in intervals {
        if iv.start <= a && a < iv.end && !sources.contains(&iv.id) {
            sources.push(iv.id.clone());
        }
    }
    spans.push(Span {
        text: text[a..b].to_string(),
        sources,
    });
}

fn block_end(block_list: &[Range<usize>], text_len: usize, pos: usize) -> usize {
    block_list
        .iter()
        .find(|r| r.start <= pos && pos < r.end)
        .map_or(pos.min(text_len), |r| r.end)
}

fn scan_tags(text: &str, exempt: &[Range<usize>]) -> Vec<TagTok> {
    let bytes = text.as_bytes();
    let mut toks = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'<' || exempt.iter().any(|r| r.start <= i && i < r.end) {
            continue;
        }
        if let Some(tok) = try_tag(text, i) {
            toks.push(tok);
        }
    }
    toks
}

fn try_tag(text: &str, pos: usize) -> Option<TagTok> {
    let b = text.as_bytes();
    let prefix = SourceId::PREFIX;
    let mut i = pos + 1;
    let is_close = b.get(i) == Some(&b'/');
    if is_close {
        i += 1;
    }
    if !text.get(i..)?.starts_with(prefix) {
        return None;
    }
    i += prefix.len();
    let hex_end = i + b[i..]
        .iter()
        .take_while(|&&c| c.is_ascii_hexdigit())
        .count();
    // T3：不符合 ref- + 16..=32 hex，按字面文本
    let id = SourceId::parse(&text[i - prefix.len()..hex_end].to_ascii_lowercase())?;
    i = hex_end;

    if is_close {
        return (b.get(i) == Some(&b'>')).then(|| TagTok {
            pos,
            end: i + 1,
            id,
            is_close,
        });
    }

    if b.get(i) == Some(&b'>') {
        return Some(TagTok {
            pos,
            end: i + 1,
            id,
            is_close,
        });
    }
    // T6：开标签可携带属性，解析时接受并忽略
    attr_tail(b, i).map(|end| TagTok {
        pos,
        end,
        id,
        is_close,
    })
}

/// 解析 `name="value" ... >` 属性尾巴；属性不得跨行、不得包含 `<`。
fn attr_tail(b: &[u8], mut i: usize) -> Option<usize> {
    loop {
        if b.get(i) == Some(&b'>') {
            return Some(i + 1);
        }
        if !b.get(i).is_some_and(|c| c.is_ascii_whitespace()) || b[i] == b'\n' {
            return None;
        }
        while b.get(i).is_some_and(|c| c.is_ascii_whitespace() && *c != b'\n') {
            i += 1;
        }
        let name_start = i;
        while b
            .get(i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
        {
            i += 1;
        }
        if i == name_start {
            return None;
        }
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            return None;
        }
        i += 1;
        while b.get(i) == Some(&b' ') {
            i += 1;
        }
        if b.get(i) != Some(&b'"') {
            return None;
        }
        i += 1;
        while b.get(i).is_some_and(|c| *c != b'"' && *c != b'\n') {
            i += 1;
        }
        if b.get(i) != Some(&b'"') {
            return None;
        }
        i += 1;
        if b.get(i) != Some(&b'>') && !b.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
            return None;
        }
    }
}

/// T4 豁免区：围栏代码块整行 + 行内代码区间。
pub(crate) fn code_regions(text: &str) -> Vec<Range<usize>> {
    let mut regions = Vec::new();
    let mut fence: Option<(u8, usize, usize)> = None;
    let mut pos = 0;
    let len = text.len();

    while pos < len {
        let line_end = text[pos..]
            .find('\n')
            .map_or(len, |n| pos + n);
        let trimmed = text[pos..line_end].trim_start();

        match fence {
            Some((ch, flen, fstart)) => {
                if is_fence(trimmed, ch) && fence_len(trimmed) >= flen {
                    regions.push(fstart..line_end);
                    fence = None;
                }
            }
            None => {
                if let Some((ch, flen)) = fence_open(trimmed) {
                    fence = Some((ch, flen, pos));
                } else {
                    regions.extend(inline_code(&text[pos..line_end], pos));
                }
            }
        }
        pos = line_end + 1;
    }
    if let Some((_, _, fstart)) = fence {
        regions.push(fstart..len);
    }
    regions
}

fn fence_open(trimmed: &str) -> Option<(u8, usize)> {
    for ch in [b'`', b'~'] {
        let n = fence_len_starting(trimmed, ch);
        if n >= 3 {
            return Some((ch, n));
        }
    }
    None
}

fn is_fence(trimmed: &str, ch: u8) -> bool {
    trimmed.as_bytes().first() == Some(&ch)
}

fn fence_len(trimmed: &str) -> usize {
    trimmed
        .as_bytes()
        .first()
        .map_or(0, |&c| fence_len_starting(trimmed, c))
}

fn fence_len_starting(trimmed: &str, ch: u8) -> usize {
    trimmed
        .as_bytes()
        .iter()
        .take_while(|&&c| c == ch)
        .count()
}

fn inline_code(line: &str, base: usize) -> Vec<Range<usize>> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'`' {
            i += 1;
            continue;
        }
        let n = b[i..].iter().take_while(|&&c| c == b'`').count();
        let mut j = i + n;
        let mut close = None;
        while j < b.len() {
            if b[j] == b'`' {
                let m = b[j..].iter().take_while(|&&c| c == b'`').count();
                if m == n {
                    close = Some(j + n);
                    break;
                }
                j += m;
            } else {
                j += 1;
            }
        }
        match close {
            Some(end) => {
                out.push(base + i..base + end);
                i = end;
            }
            None => i += n,
        }
    }
    out
}

/// 块边界（T1 的闭合位置）：空行分隔的段落 / 列表项 / 标题 / 围栏代码块 / 文件末尾。
fn blocks(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut para: Option<(usize, usize)> = None;
    let mut fence: Option<(u8, usize, usize)> = None;
    let mut pos = 0;
    let len = text.len();

    while pos < len {
        let line_end = text[pos..].find('\n').map_or(len, |n| pos + n);
        let trimmed = text[pos..line_end].trim_start();

        if let Some((ch, flen, fstart)) = fence {
            if is_fence(trimmed, ch) && fence_len(trimmed) >= flen {
                out.push(fstart..line_end);
                fence = None;
            }
        } else {
            let flush = |para: &mut Option<(usize, usize)>, out: &mut Vec<Range<usize>>| {
                if let Some((s, e)) = para.take() {
                    out.push(s..e);
                }
            };
            if trimmed.is_empty() {
                flush(&mut para, &mut out);
            } else if let Some((ch, flen)) = fence_open(trimmed) {
                flush(&mut para, &mut out);
                fence = Some((ch, flen, pos));
            } else if is_heading(trimmed) || is_list_item(trimmed) {
                flush(&mut para, &mut out);
                out.push(pos..line_end);
            } else {
                match &mut para {
                    Some((_, end)) => *end = line_end,
                    None => para = Some((pos, line_end)),
                }
            }
        }
        pos = line_end + 1;
    }

    if let Some((s, e)) = para {
        out.push(s..e);
    }
    if let Some((_, _, fstart)) = fence {
        out.push(fstart..len);
    }
    out
}

fn is_heading(trimmed: &str) -> bool {
    trimmed.starts_with('#')
}

fn is_list_item(trimmed: &str) -> bool {
    let b = trimmed.as_bytes();
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return true;
    }
    let digits = b.iter().take_while(|&&c| c.is_ascii_digit()).count();
    digits > 0 && b.get(digits) == Some(&b'.') && b.get(digits + 1) == Some(&b' ')
}
