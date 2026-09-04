#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
#![cfg_attr(test, allow(clippy::result_large_err))]

//! fluen-kb：通用知识库 SDK。
//!
//! 分层（见 EXECUTION_PLAN.md）：
//! - L0 `syntax`：溯源标记纯函数（零依赖）
//! - L1 `store`：条目文件读写（零依赖）
//! - L2 `index` / `ops` / `search` / `embed` / `lint`（feature `index`）
//! - L3 `AsyncKb`（feature `async`）
//! - L4 MCP stdio server（feature `mcp-server`）
//!
//! 不变式：MD 是唯一真相源，`index.db` 是纯派生缓存，任何时刻可删除重建。

pub mod error;
pub mod ids;
pub mod model;
pub mod store;
pub mod syntax;

#[cfg(feature = "index")]
pub mod embed;
#[cfg(feature = "index")]
pub mod handle;
#[cfg(feature = "index")]
pub(crate) mod index;
#[cfg(feature = "index")]
pub mod lint;
#[cfg(feature = "index")]
pub mod ops;
#[cfg(feature = "index")]
pub mod search;

#[cfg(feature = "async")]
pub mod async_kb;
#[cfg(feature = "mcp-server")]
pub mod mcp;

pub use error::{KbError, KbResult};
pub use ids::{Predicate, SourceId, WikiId, WikiType};
pub use model::{EntryDocument, EntryMeta};

#[cfg(feature = "index")]
pub use embed::{EmbedHandle, KnowledgeEmbedding};
#[cfg(feature = "index")]
pub use handle::{Kb, KbBuilder};
#[cfg(feature = "index")]
pub use lint::{LintIssue, LintLevel};
#[cfg(feature = "index")]
pub use search::{QueryParams, SearchHit, SearchMethod, Searcher};

#[cfg(feature = "index")]
pub use ops::{CreateOutcome, EditOp, Ops};
