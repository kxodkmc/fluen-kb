//! L0 溯源标记：纯函数、零第三方依赖。
//!
//! 归属总则（设计 §3.2.2）：一段内容的有效来源 = 从根到该内容的
//! 开放标签栈中所有 SourceId 的并集。

pub(crate) mod ops;
mod parser;

pub use ops::{
    collect_sources, filter_by_source, normalize, remove_source, replace_source,
};
pub use parser::{parse_detailed, strip};

use crate::ids::SourceId;
use std::ops::Range;

/// 剥离后正文与逐区间的归属映射。
pub type SourceMap = Vec<(Range<usize>, Vec<SourceId>)>;

/// 正文片段：连续文本 + 该片段的完整归属集（有序，按标签开启顺序）。
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub sources: Vec<SourceId>,
}

/// 正文解析产物：片段序列即归属树（SourceTree 的扁平等价表示）。
#[derive(Debug, Clone, PartialEq)]
pub struct SourceTree {
    pub spans: Vec<Span>,
}

/// 解析统计，供 lint 使用（T1/T2 是否被触发）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseStats {
    pub unmatched_opens: usize,
    pub orphan_closes: usize,
}

/// 解析正文为归属树。
pub fn parse(body: &str) -> SourceTree {
    parse_detailed(body).0
}

/// 剥离全部溯源标签，返回纯文本与逐区间的归属映射（FTS 索引用）。
pub fn strip_map(body: &str) -> (String, SourceMap) {
    strip(body)
}

#[cfg(test)]
mod tests;
