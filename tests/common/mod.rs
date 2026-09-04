#![cfg(feature = "index")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use fluen_kb::{Kb, KbBuilder};

#[allow(dead_code)]
pub fn temp_kb() -> (tempfile::TempDir, Kb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let kb = KbBuilder::new(dir.path()).open().expect("open kb");
    (dir, kb)
}

#[allow(dead_code)]
pub fn src(v: &str) -> fluen_kb::SourceId {
    fluen_kb::SourceId::new(v).expect("valid SourceId")
}

#[allow(dead_code)]
pub fn wiki(v: &str) -> fluen_kb::WikiId {
    fluen_kb::WikiId::new(v).expect("valid WikiId")
}

/// 全部派生表拼接成的规范串，用于 rebuild 幂等断言。
#[allow(dead_code)]
pub fn db_dump(root: &std::path::Path) -> String {
    use rusqlite::Connection;
    let conn = Connection::open(root.join("wiki/index.db")).expect("open db");
    let mut out = String::new();
    for table in [
        "SELECT id, type, title, file_path, created, updated FROM entries ORDER BY id",
        "SELECT from_id, predicate, to_id FROM entry_relations ORDER BY from_id, predicate, to_id",
        "SELECT entry_id, ref_id FROM entry_sources ORDER BY entry_id, ref_id",
        "SELECT id, title, \"type\", content FROM entries_fts ORDER BY id",
    ] {
        let mut stmt = conn.prepare(table).expect("prepare");
        let n = stmt.column_count();
        let rows = stmt
            .query_map([], |r| {
                let mut line = String::new();
                for i in 0..n {
                    let v: String = r.get(i).expect("col");
                    line.push_str(&v);
                    line.push('|');
                }
                Ok(line)
            })
            .expect("query");
        for row in rows {
            out.push_str(&row.expect("row"));
            out.push('\n');
        }
    }
    out
}
