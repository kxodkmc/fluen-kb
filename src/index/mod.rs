//! 派生管线：MD → 派生表。悬空关联行跳过不失败（设计 D25）。

pub(crate) mod schema;
pub(crate) mod views;

use crate::error::{KbError, KbResult};
use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use crate::model::{EntryDocument, EntryMeta};
use crate::store::Store;
use crate::syntax;
use rusqlite::{Connection, OptionalExtension, params};

/// 单条目重派生：读 MD → 解析 → 覆盖写 entries/relations/sources/FTS，
/// 保留该条目已有向量行（hash 不符即 stale，由读路径判定）。
/// 文件已不存在时清理 DB 行（自愈）。
pub(crate) fn reindex_entry(conn: &Connection, store: &Store, id: &WikiId) -> KbResult<()> {
    let Some(path) = store.find_entry_file(id)? else {
        return remove_entry_rows(conn, id);
    };
    let doc = store.load_path(&path)?;
    upsert_entry(conn, &doc, &store.rel_path(&path))
}

/// 全量重建：扫描 wiki/ → 事务重建全部派生表 → 重建 index.md。
/// embeddings 按 entry_id 存活保留（hash 是否相符由读路径判定）。
pub(crate) fn rebuild(conn: &Connection, store: &Store) -> KbResult<()> {
    let mut docs = Vec::new();
    for (_, path) in store.discover()? {
        let doc = store.load_path(&path)?;
        let rel = store.rel_path(&path);
        docs.push((doc, rel));
    }

    let tx = conn.unchecked_transaction()?;
    let embeddings = load_embeddings(&tx)?;
    tx.execute("DELETE FROM entries", [])?;
    tx.execute("DELETE FROM entries_fts", [])?;

    for (doc, rel) in &docs {
        insert_entry_row(&tx, doc, rel)?;
    }
    for (doc, _) in &docs {
        insert_relations(&tx, &doc.id, &doc.relations)?;
    }
    reinsert_embeddings(&tx, &embeddings, &docs)?;
    tx.commit()?;

    views::rebuild_index_md(conn, store)
}

pub(crate) fn remove_entry_rows(conn: &Connection, id: &WikiId) -> KbResult<()> {
    conn.execute("DELETE FROM entries WHERE id = ?1", params![id.as_str()])?;
    conn.execute("DELETE FROM entries_fts WHERE id = ?1", params![id.as_str()])?;
    Ok(())
}

fn upsert_entry(conn: &Connection, doc: &EntryDocument, rel_path: &str) -> KbResult<()> {
    let tx = conn.unchecked_transaction()?;
    let embedding = load_embedding(&tx, &doc.id)?;
    // 入边（其它条目指向本条目）会被删行时的 ON DELETE CASCADE 一并清除，
    // 这里先留存、重建后再回写，保证单条目重派生不破坏其它条目的出边引用。
    let incoming = load_incoming_relations(&tx, &doc.id)?;
    remove_entry_rows(&tx, &doc.id)?;
    insert_entry_row(&tx, doc, rel_path)?;
    insert_relations(&tx, &doc.id, &doc.relations)?;
    for (from, predicate) in incoming {
        tx.execute(
            "INSERT OR IGNORE INTO entry_relations(from_id, predicate, to_id) VALUES (?1, ?2, ?3)",
            params![from.as_str(), predicate.as_str(), doc.id.as_str()],
        )?;
    }
    if let Some(e) = embedding {
        insert_embedding(&tx, &doc.id, &e)?;
    }
    tx.commit()?;
    Ok(())
}

/// 取指向 `id` 的入边：`(from_id, predicate)`，供单条目重建后恢复。
fn load_incoming_relations(conn: &Connection, id: &WikiId) -> KbResult<Vec<(String, String)>> {
    let mut stmt =
        conn.prepare("SELECT from_id, predicate FROM entry_relations WHERE to_id = ?1")?;
    let rows = stmt.query_map(params![id.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.flatten().collect())
}

fn insert_entry_row(conn: &Connection, doc: &EntryDocument, rel_path: &str) -> KbResult<()> {
    let sources = syntax::collect_sources(&doc.body);
    let (plain, _) = syntax::strip(&doc.body);
    conn.execute(
        "INSERT INTO entries(id, type, title, file_path, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            doc.id.as_str(),
            doc.wiki_type.as_str(),
            doc.title,
            rel_path,
            doc.created,
            doc.updated
        ],
    )?;
    for s in &sources {
        conn.execute(
            "INSERT OR IGNORE INTO entry_sources(entry_id, ref_id) VALUES (?1, ?2)",
            params![doc.id.as_str(), s.as_str()],
        )?;
    }
    conn.execute(
        "INSERT INTO entries_fts(id, title, \"type\", content) VALUES (?1, ?2, ?3, ?4)",
        params![doc.id.as_str(), doc.title, doc.wiki_type.as_str(), plain],
    )?;
    Ok(())
}

/// 悬空引用防御（D25）：to_id 不在 entries 中的关联行跳过，不视为错误。
fn insert_relations(
    conn: &Connection,
    from: &WikiId,
    relations: &[(Predicate, WikiId)],
) -> KbResult<()> {
    for (pred, to) in relations {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM entries WHERE id = ?1",
                params![to.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            conn.execute(
                "INSERT OR IGNORE INTO entry_relations(from_id, predicate, to_id) VALUES (?1, ?2, ?3)",
                params![from.as_str(), pred.as_str(), to.as_str()],
            )?;
        }
    }
    Ok(())
}

struct EmbeddingRow {
    blob: Vec<u8>,
    model: String,
    content_hash: String,
    updated: String,
}

fn load_embedding(conn: &Connection, id: &WikiId) -> KbResult<Option<EmbeddingRow>> {
    conn.query_row(
        "SELECT embedding, model, content_hash, updated FROM embeddings WHERE entry_id = ?1",
        params![id.as_str()],
        |r| {
            Ok(EmbeddingRow {
                blob: r.get(0)?,
                model: r.get(1)?,
                content_hash: r.get(2)?,
                updated: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn load_embeddings(conn: &Connection) -> KbResult<Vec<(String, EmbeddingRow)>> {
    let mut stmt = conn.prepare(
        "SELECT entry_id, embedding, model, content_hash, updated FROM embeddings",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            EmbeddingRow {
                blob: r.get(1)?,
                model: r.get(2)?,
                content_hash: r.get(3)?,
                updated: r.get(4)?,
            },
        ))
    })?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(out)
}

fn insert_embedding(conn: &Connection, id: &WikiId, e: &EmbeddingRow) -> KbResult<()> {
    conn.execute(
        "INSERT INTO embeddings(entry_id, embedding, model, content_hash, updated) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id.as_str(), e.blob, e.model, e.content_hash, e.updated],
    )?;
    Ok(())
}

fn reinsert_embeddings(
    conn: &Connection,
    embeddings: &[(String, EmbeddingRow)],
    docs: &[(EntryDocument, String)],
) -> KbResult<()> {
    for (entry_id, e) in embeddings {
        if docs.iter().any(|(d, _)| d.id.as_str() == *entry_id) {
            let id = WikiId::new(entry_id)?;
            insert_embedding(conn, &id, e)?;
        }
    }
    Ok(())
}

// ---- 元数据查询 ----

pub(crate) fn get_meta(conn: &Connection, id: &WikiId) -> KbResult<EntryMeta> {
    let found = conn
        .query_row(
            "SELECT type, title, file_path, created, updated FROM entries WHERE id = ?1",
            params![id.as_str()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((wiki_type, title, file_path, created, updated)) = found else {
        return Err(KbError::not_found(format!("entry {id}")));
    };
    Ok(EntryMeta {
        id: id.clone(),
        wiki_type: WikiType::parse(&wiki_type)
            .ok_or_else(|| KbError::invalid(format!("unknown entry type {wiki_type:?}")))?,
        title,
        file_path,
        relations: load_relations(conn, id)?,
        sources: load_sources(conn, id)?,
        created,
        updated,
    })
}

pub(crate) fn list_metas(conn: &Connection, wiki_type: Option<WikiType>) -> KbResult<Vec<EntryMeta>> {
    let mut ids: Vec<WikiId> = match wiki_type {
        Some(t) => {
            let mut stmt =
                conn.prepare("SELECT id FROM entries WHERE type = ?1 ORDER BY title")?;
            query_ids(&mut stmt, params![t.as_str()])?
        }
        None => {
            let mut stmt = conn.prepare("SELECT id FROM entries ORDER BY type, title")?;
            query_ids(&mut stmt, params![])?
        }
    };
    ids.sort();
    ids.iter().map(|id| get_meta(conn, id)).collect()
}

pub(crate) fn metas_by_source(conn: &Connection, src: &SourceId) -> KbResult<Vec<EntryMeta>> {
    let mut stmt = conn.prepare(
        "SELECT e.id FROM entries e JOIN entry_sources s ON s.entry_id = e.id
         WHERE s.ref_id = ?1 ORDER BY e.title",
    )?;
    let ids = query_ids(&mut stmt, params![src.as_str()])?;
    ids.iter().map(|id| get_meta(conn, id)).collect()
}

pub(crate) fn find_summary_by_source(conn: &Connection, src: &SourceId) -> KbResult<Option<WikiId>> {
    conn.query_row(
        "SELECT e.id FROM entries e JOIN entry_sources s ON s.entry_id = e.id
         WHERE s.ref_id = ?1 AND e.type = 'summary' LIMIT 1",
        params![src.as_str()],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|id| WikiId::new(&id))
    .transpose()
}

pub(crate) fn find_by_title(
    conn: &Connection,
    wiki_type: WikiType,
    title: &str,
) -> KbResult<Option<WikiId>> {
    conn.query_row(
        "SELECT id FROM entries WHERE type = ?1 AND title = ?2 COLLATE NOCASE LIMIT 1",
        params![wiki_type.as_str(), title],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .map(|id| WikiId::new(&id))
    .transpose()
}

fn query_ids(
    stmt: &mut rusqlite::Statement<'_>,
    params: impl rusqlite::Params,
) -> KbResult<Vec<WikiId>> {
    let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
    let ids = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    ids.into_iter().map(|id| WikiId::new(&id)).collect()
}

fn load_relations(conn: &Connection, id: &WikiId) -> KbResult<Vec<(Predicate, WikiId)>> {
    let mut stmt =
        conn.prepare("SELECT predicate, to_id FROM entry_relations WHERE from_id = ?1")?;
    let rows = stmt.query_map(params![id.as_str()], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    out.into_iter()
        .map(|(p, t)| Ok((Predicate::new(&p)?, WikiId::new(&t)?)))
        .collect()
}

fn load_sources(conn: &Connection, id: &WikiId) -> KbResult<Vec<SourceId>> {
    let mut stmt = conn.prepare("SELECT ref_id FROM entry_sources WHERE entry_id = ?1")?;
    let rows = stmt.query_map(params![id.as_str()], |r| r.get::<_, String>(0))?;
    let out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    out.into_iter().map(|s| SourceId::new(&s)).collect()
}
