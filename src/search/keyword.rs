//! 关键词路径：ID 直查 → FTS bm25 → LIKE 回退（设计 §7）。

use crate::error::KbResult;
use crate::ids::{SourceId, WikiId};
use crate::index;
use rusqlite::{Connection, OptionalExtension, params};

/// ID 直查（score 1.0）：wiki-xxx / ref-xxx / 标题精确（NOCASE）。
pub(crate) fn direct_lookup(conn: &Connection, query: &str) -> KbResult<Vec<(WikiId, f64)>> {
    let query = query.trim();
    if let Some(id) = WikiId::parse(query) {
        let exists: Option<String> = conn
            .query_row(
                "SELECT id FROM entries WHERE id = ?1",
                params![id.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(vec![(id, 1.0)]);
        }
    }
    if let Some(src) = SourceId::parse(query) {
        return Ok(index::metas_by_source(conn, &src)?
            .into_iter()
            .map(|m| (m.id, 1.0))
            .collect());
    }
    let by_title: Option<String> = conn
        .query_row(
            "SELECT id FROM entries WHERE title = ?1 COLLATE NOCASE LIMIT 1",
            params![query],
            |r| r.get(0),
        )
        .optional()?;
    match by_title {
        Some(id) => Ok(vec![(WikiId::new(&id)?, 1.0)]),
        None => Ok(Vec::new()),
    }
}

/// 关键词检索：所有分词 <3 字符 → LIKE 回退（中文 2 字词兜底，D19）；
/// 否则 FTS5 trigram + bm25 负分归一化到 [0,1]。
pub(crate) fn search(conn: &Connection, query: &str) -> KbResult<Vec<(WikiId, f64)>> {
    let trimmed = query.trim();
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    if tokens.iter().all(|t| t.chars().count() < 3) {
        return like_fallback(conn, trimmed);
    }
    fts(conn, trimmed)
}

fn fts(conn: &Connection, query: &str) -> KbResult<Vec<(WikiId, f64)>> {
    let pattern = format!("\"{}\"", query.replace('"', "\"\""));
    let mut stmt = conn.prepare(
        "SELECT id, bm25(entries_fts) FROM entries_fts WHERE entries_fts MATCH ?1",
    )?;
    let rows = stmt.query_map(params![pattern], |r| {
        Ok((r.get::<_, String>(0)?, -r.get::<_, f64>(1)?))
    })?;
    let raw: Vec<(String, f64)> = rows.flatten().collect();

    let max = raw.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    let normalize = |s: f64| if max > 0.0 { s / max } else { 1.0 };
    raw.into_iter()
        .map(|(id, s)| Ok((WikiId::new(&id)?, normalize(s))))
        .collect()
}

fn like_fallback(conn: &Connection, query: &str) -> KbResult<Vec<(WikiId, f64)>> {
    // LIKE 通配符按字面匹配，否则 `%`/`_` 会改写查询语义
    let escaped = query
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let pattern = format!("%{escaped}%");
    let mut stmt = conn.prepare(
        "SELECT id FROM entries_fts WHERE title LIKE ?1 ESCAPE '\\' OR content LIKE ?1 ESCAPE '\\'",
    )?;
    let rows = stmt.query_map(params![pattern], |r| r.get::<_, String>(0))?;
    rows.flatten()
        .map(|id| Ok((WikiId::new(&id)?, 0.5)))
        .collect()
}
