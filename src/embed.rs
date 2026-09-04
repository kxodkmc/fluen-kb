//! 向量能力：`KnowledgeEmbedding` trait 可选注入 + content_hash 新鲜度防护（设计 §6.3）。

use crate::error::{KbError, KbResult};
use crate::handle::KbInner;
use crate::ids::WikiId;
use crate::syntax;
use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// 宿主注入的向量提供方。SDK 不实现具体嵌入模型。
pub trait KnowledgeEmbedding: Send + Sync {
    fn embed(&self, text: &str) -> KbResult<Vec<f32>>;
    fn model_name(&self) -> &str;
}

/// sha256(标题 + strip 后正文) hex 前 16 位。
pub fn content_hash(title: &str, plain_body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(plain_body.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex.truncate(16);
    hex
}

pub fn encode_vector(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn decode_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// 嵌入文本 = 标题 + 空行 + strip 后正文。
pub(crate) fn embed_text(title: &str, plain_body: &str) -> String {
    format!("{title}\n{plain_body}")
}

/// 写路径刷新：有 provider 时对单条目重算向量。无 provider 静默跳过。
/// 向量是派生缓存——本函数失败会让操作返回错误，但已落盘的 MD 与索引保持一致。
pub(crate) fn refresh_entry(
    inner: &KbInner,
    conn: &Connection,
    id: &WikiId,
) -> KbResult<()> {
    let provider = current_provider(inner);
    let Some(provider) = provider else {
        return Ok(());
    };
    let doc = inner.store.load(id)?;
    let (plain, _) = syntax::strip(&doc.body);
    let vector = provider
        .embed(&embed_text(&doc.title, &plain))
        .map_err(|e| KbError::Embedding(format!("{e}")))?;
    if vector.is_empty() {
        return Err(KbError::Embedding("provider returned empty vector".into()));
    }
    let hash = content_hash(&doc.title, &plain);
    let now = crate::store::time::now_rfc3339();
    conn.execute(
        "INSERT INTO embeddings(entry_id, embedding, model, content_hash, updated)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(entry_id) DO UPDATE SET
            embedding = excluded.embedding,
            model = excluded.model,
            content_hash = excluded.content_hash,
            updated = excluded.updated",
        params![
            id.as_str(),
            encode_vector(&vector),
            provider.model_name(),
            hash,
            now
        ],
    )?;
    Ok(())
}

pub(crate) fn current_provider(inner: &KbInner) -> Option<Arc<dyn KnowledgeEmbedding>> {
    inner.embedding.lock().ok().and_then(|slot| slot.clone())
}

pub struct EmbedHandle(pub(crate) std::sync::Arc<KbInner>);

impl EmbedHandle {
    /// 注入向量提供方。已入库条目的向量由 `refresh_all` 显式补算。
    pub fn attach(&self, provider: Arc<dyn KnowledgeEmbedding>) -> KbResult<()> {
        let mut slot = self
            .0
            .embedding
            .lock()
            .map_err(|_| KbError::Cancelled("embedding slot poisoned".into()))?;
        *slot = Some(provider);
        Ok(())
    }

    /// 全库补算向量。
    pub fn refresh_all(&self) -> KbResult<()> {
        let ids = {
            let conn = self
                .0
                .conn
                .lock()
                .map_err(|_| KbError::Cancelled("db lock poisoned".into()))?;
            let mut stmt = conn.prepare("SELECT id FROM entries ORDER BY id")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.flatten()
                .map(|id| WikiId::new(&id))
                .collect::<KbResult<Vec<_>>>()?
        };
        let conn = self
            .0
            .conn
            .lock()
            .map_err(|_| KbError::Cancelled("db lock poisoned".into()))?;
        for id in &ids {
            refresh_entry(&self.0, &conn, id)?;
        }
        Ok(())
    }
}
