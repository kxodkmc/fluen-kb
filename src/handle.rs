//! 句柄装配：`KbBuilder` → `Kb`，及四类子句柄（ops / search / index / embed）。

use crate::embed::EmbedHandle;
use crate::error::{KbError, KbResult};
use crate::ids::{SourceId, WikiId};
use crate::index::{self, schema};
use crate::ops::Ops;
use crate::search::Searcher;
use crate::store::Store;
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub(crate) type IdGenerator = Box<dyn Fn() -> String + Send + Sync>;
pub(crate) type SourceValidator = Box<dyn Fn(&SourceId) -> bool + Send + Sync>;

pub(crate) struct KbInner {
    pub(crate) store: Store,
    pub(crate) conn: Mutex<Connection>,
    /// 全量写锁：串行化 ops 层「查重 → 落盘」序列，保证并发去重不变式。
    /// 锁序恒为 write_lock → conn，避免与读路径交叉死锁。
    pub(crate) write_lock: Mutex<()>,
    pub(crate) id_generator: Option<IdGenerator>,
    pub(crate) source_validator: Option<SourceValidator>,
    pub(crate) embedding: Mutex<Option<Arc<dyn crate::embed::KnowledgeEmbedding>>>,
}

impl KbInner {
    pub(crate) fn next_id(&self) -> KbResult<WikiId> {
        let raw = match &self.id_generator {
            Some(generator) => generator(),
            None => crate::ids::generate_wiki_id(),
        };
        WikiId::new(&raw)
    }

    pub(crate) fn lock_conn(
        &self,
    ) -> KbResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| KbError::Cancelled("db lock poisoned".into()))
    }

    pub(crate) fn lock_write(&self) -> KbResult<std::sync::MutexGuard<'_, ()>> {
        self.write_lock
            .lock()
            .map_err(|_| KbError::Cancelled("write lock poisoned".into()))
    }
}

pub struct KbBuilder {
    root: PathBuf,
    id_generator: Option<IdGenerator>,
    source_validator: Option<SourceValidator>,
}

impl KbBuilder {
    /// 根路径，唯一必填项。SDK 管理 `{root}/wiki/`。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        KbBuilder {
            root: root.into(),
            id_generator: None,
            source_validator: None,
        }
    }

    /// 自定义 WikiId 生成（如云端分配），返回完整 `wiki-` + 16 hex。
    pub fn with_id_generator(
        mut self,
        generator: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        self.id_generator = Some(Box::new(generator));
        self
    }

    /// lint 用的来源存在性校验回调（可选）。
    pub fn with_source_validator(
        mut self,
        validator: impl Fn(&SourceId) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.source_validator = Some(Box::new(validator));
        self
    }

    /// 打开（不存在则初始化）知识库。
    pub fn open(self) -> KbResult<Kb> {
        let store = Store::new(&self.root);
        store.init()?;
        let conn = schema::open(&store.wiki_root().join("index.db"))?;
        schema::init(&conn)?;
        Ok(Kb(Arc::new(KbInner {
            store,
            conn: Mutex::new(conn),
            write_lock: Mutex::new(()),
            id_generator: self.id_generator,
            source_validator: self.source_validator,
            embedding: Mutex::new(None),
        })))
    }
}

#[derive(Clone)]
pub struct Kb(Arc<KbInner>);

impl Kb {
    pub fn ops(&self) -> Ops {
        Ops(self.0.clone())
    }

    pub fn search(&self) -> Searcher {
        Searcher(self.0.clone())
    }

    pub fn index(&self) -> IndexHandle {
        IndexHandle(self.0.clone())
    }

    pub fn embed(&self) -> EmbedHandle {
        EmbedHandle(self.0.clone())
    }

    #[cfg(feature = "async")]
    pub fn into_async(self) -> crate::async_kb::AsyncKb {
        crate::async_kb::AsyncKb(self.0)
    }
}

pub struct IndexHandle(pub(crate) Arc<KbInner>);

impl IndexHandle {
    pub fn reindex_entry(&self, id: &WikiId) -> KbResult<()> {
        let conn = self.0.lock_conn()?;
        index::reindex_entry(&conn, &self.0.store, id)
    }

    /// 全量重建：MD 是唯一真相源，删除 index.db 后运行本方法可完整恢复。
    pub fn rebuild(&self) -> KbResult<()> {
        let conn = self.0.lock_conn()?;
        index::rebuild(&conn, &self.0.store)
    }
}
