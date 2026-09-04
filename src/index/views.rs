//! 派生视图：index.md（全量重建）与 log.md（追加式审计日志）。

use crate::error::KbResult;
use crate::ids::WikiType;
use crate::store::{Store, atomic, filename, time};
use rusqlite::{Connection, params};
use std::fs::{self, OpenOptions};
use std::io::Write;

const LOG_MAX_BYTES: u64 = 1024 * 1024;
const LOG_VERBS: &[&str] = &[
    "create", "merge", "upsert", "delete", "edit", "rename", "prune", "rebuild", "migrate",
];

/// 每次条目写操作后从 DB 全量重建（设计 §3.4）。SDK 自身不解析它。
pub(crate) fn rebuild_index_md(conn: &Connection, store: &Store) -> KbResult<()> {
    let now = time::now_rfc3339();
    let mut md = format!(
        "---\ntitle: 知识库索引\ntype: index\ncreated: {now}\nupdated: {now}\n---\n\n# index\n"
    );
    for t in WikiType::ALL {
        md.push_str(&format!("\n## {}\n", section_name(t)));
        let mut stmt =
            conn.prepare("SELECT id, title FROM entries WHERE type = ?1 ORDER BY title")?;
        let rows = stmt.query_map(params![t.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for (id, title) in rows.flatten() {
            let file = filename::entry_filename_id_title(&id, &title);
            md.push_str(&format!("- [[{}/{}]]\n", t.dir(), file));
        }
    }
    atomic::atomic_write(&store.wiki_root().join("index.md"), &md)
}

fn section_name(t: WikiType) -> &'static str {
    match t {
        WikiType::Summary => "Summaries",
        WikiType::Concept => "Concepts",
        WikiType::Entity => "Entities",
    }
}

/// 追加一条操作日志（设计 §3.5）。超 1 MB 先轮转。
pub(crate) fn append_log(
    store: &Store,
    verb: &str,
    id: &str,
    title: &str,
    details: &[String],
) -> KbResult<()> {
    debug_assert!(LOG_VERBS.contains(&verb), "unknown log verb {verb:?}");
    let log_path = store.wiki_root().join("log.md");
    rotate_if_large(&log_path)?;

    let now = time::now_rfc3339();
    let mut entry = format!("\n## [{now}] {verb} | {id} {title}\n");
    for d in details {
        entry.push_str(&format!("- {d}\n"));
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&log_path)?;
    file.write_all(entry.as_bytes())?;
    Ok(())
}

fn rotate_if_large(log_path: &std::path::Path) -> KbResult<()> {
    let Ok(meta) = fs::metadata(log_path) else {
        return Ok(());
    };
    if meta.len() <= LOG_MAX_BYTES {
        return Ok(());
    }
    let ym = time::now_year_month();
    let mut target = log_path.with_file_name(format!("log-{ym}.md"));
    let mut n = 2;
    while target.exists() {
        target = log_path.with_file_name(format!("log-{ym}-{n}.md"));
        n += 1;
    }
    fs::rename(log_path, target)?;
    Ok(())
}
