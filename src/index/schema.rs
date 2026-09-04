//! SQLite schema v2（设计 §6.1）。

use crate::error::{KbError, KbResult};
use rusqlite::Connection;
use std::path::Path;
use std::time::Duration;

pub(crate) const USER_VERSION: i32 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
    id        TEXT PRIMARY KEY,
    type      TEXT NOT NULL,
    title     TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created   TEXT NOT NULL,
    updated   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS entry_relations (
    from_id   TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    predicate TEXT NOT NULL DEFAULT 'related',
    to_id     TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    PRIMARY KEY (from_id, predicate, to_id)
);

CREATE TABLE IF NOT EXISTS entry_sources (
    entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    ref_id   TEXT NOT NULL,
    PRIMARY KEY (entry_id, ref_id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    id, title, \"type\", content, tokenize='trigram'
);

CREATE TABLE IF NOT EXISTS embeddings (
    entry_id     TEXT PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
    embedding    BLOB NOT NULL,
    model        TEXT NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',
    updated      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sources_ref ON entry_sources(ref_id);
CREATE INDEX IF NOT EXISTS idx_relations_to ON entry_relations(to_id);
";

pub(crate) fn open(path: &Path) -> KbResult<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}

/// 空库初始化为 v2；已是 v2 则幂等通过；其余版本拒绝（删除 index.db 后 rebuild 恢复）。
pub(crate) fn init(conn: &Connection) -> KbResult<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    match version {
        0 => {
            conn.execute_batch(SCHEMA)?;
            conn.pragma_update(None, "user_version", USER_VERSION)?;
            Ok(())
        }
        USER_VERSION => Ok(()),
        other => Err(KbError::invalid(format!(
            "unsupported index.db schema version {other}; delete wiki/index.db and run rebuild"
        ))),
    }
}
