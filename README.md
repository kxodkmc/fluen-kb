# fluen-kb

通用知识库 SDK（Rust）。

> Generic knowledge-base SDK: markdown entries with provenance tracking, typed relations, and derived SQLite index.

## 它是什么

一个**把 "Markdown 目录" 变成 "可查询、可维护、可追踪来源的半结构化知识库"** 的引擎。

核心理念只有一句话：

> **Markdown 文件是唯一的真相源，`index.db`** **是纯派生缓存——任何时刻删掉它，都能从** **`.md`** **原样重建。**

因此知识库永不发生"文件与索引漂移"（除了内容本身被改坏），备份、迁移、合并分支都只需处理 `.md`。

## 特性

- **溯源追踪（provenance）**：正文用 `<ref-…>` 标签标注每段内容的来源（`ref-` + 16 位 hex），支持按来源抽取子集、剪枝、替换、去重。

- **类型化条目**：三类固定类型 `summary` / `concept` / `entity`，按类型分目录存放；`summary` 强制单个来源。

- **类型化关系**：条目间通过谓词建立有向关系，支持谓词（缺省 `related`），如 `作者 :: [[…]]`、`应用了 :: [[…]]`。

- **多路检索**：ID/来源精确直查、FTS5 关键词（`bm25`）、语义向量（余弦相似度）、混合检索动态融合，以及沿关系邻域的 BFS 扩展。

- **静态体检与修复**：`lint` 只读检查悬空引用 / 语法不一致 / 索引漂移 / 重复条目文件，`prune` 落盘清除悬空引用（保留标题提示文本、不改写代码区），构成"检测 → 修复"闭环。

- **幂等全量重建**：删除 `index.db` 后 `rebuild()` 可从全部 `.md` 恢复等价的索引与 `index.md`。

- **可移植 / 可维护**：默认零三方依赖即可读写；索引、嵌入、异步、MCP 均由 feature 门控，按需裁剪体积。

- **可观测**：每次写操作自动维护 `index.md`（全量索引）与 `log.md`（追加式审计日志，超 1 MB 自动轮转）。

## 目录结构

打开知识库后，`root/` 下：

```
root/
└── wiki/
    ├── summaries/   # type: summary
    ├── concepts/    # type: concept
    ├── entities/    # type: entity
    ├── index.db     # 派生缓存（SQLite v2，可删除重建）
    ├── index.md     # 派生：全量条目索引
    └── log.md       # 派生：操作审计日志
```

### 一个条目的样子

`wiki/concepts/wiki-91f2c43bf4de401e-Rust 内存安全.md`

```markdown
---
title: Rust 内存安全
type: concept
created: 2026-09-03T12:00:00+00:00
updated: 2026-09-03T12:30:00+00:00
language: zh-Hans     # 任意未知键原样保留
---

<ref-aaaaaaaaaaaaaaaa>所有权系统在编译期保证内存安全。</ref-aaaaaaaaaaaaaaaa>
一些未溯源的补充说明。

## 关联页面
- [[wiki-3d09a8bb72c14e5f]]          # related 缺省谓词
- 作者 :: [[wiki-5c1e2a3b4d5e6f70]]  # 自定义谓词
```

## 快速开始

依赖 `rusqlite`（bundled）为本地编译 SQLite，无需额外运行时。

```rust
use fluen_kb::{
    KbBuilder, WikiType, SourceId, Predicate,
    QueryParams, SearchMethod, EditOp,
};

fn main() -> fluen_kb::KbResult<()> {
    // 1. 打开（不存在则初始化）知识库
    let kb = KbBuilder::new("/path/to/root").open()?;

    // 2. 建一个带溯源来源的 concept
    let src = SourceId::new("ref-aaaaaaaaaaaaaaaa")?;
    let a = match kb.ops().create(
        WikiType::Concept,
        "Rust 内存安全",
        "<ref-aaaaaaaaaaaaaaaa>所有权系统保证内存安全。</ref-aaaaaaaaaaaaaaaa>",
        &[],
        None,
        &[],
    )? {
        fluen_kb::CreateOutcome::Created(id) => id,
        _ => unreachable!(),
    };

    // 3. 建一个 concept 并关联到 a
    let b = match kb.ops().create(
        WikiType::Concept,
        "借用检查器",
        "借用检查器是编译器的一部分。",
        &[(Predicate::related(), a.clone())],
        None,
        &[],
    )? {
        fluen_kb::CreateOutcome::Created(id) => id,
        _ => unreachable!(),
    };

    // 4. 编辑原语：search_replace
    kb.ops().edit(
        &a,
        vec![EditOp::SearchReplace {
            search: "内存安全".into(),
            replace: "内存与线程安全".into(),
            scope: None,
        }],
    )?;

    // 5. 关键词检索
    let hits = kb.search().query(&QueryParams {
        query: "借用检查器".into(),
        method: SearchMethod::Keyword,
        ..Default::default()
    })?;
    println!("{}", hits[0].meta.title); // 借用检查器

    // 6. 按来源反查贡献的条目
    let count = kb.search().entries_by_source(&src)?.len();

    // 7. lint 体检 + prune 修复悬空引用
    let issues = kb.ops().lint()?;
    kb.ops().prune()?;

    // 8. 全量重建（index.db 可随时删除重建）
    kb.index().rebuild()?;
    Ok(())
}
```

完整可运行示例见 [`examples/demo.rs`](examples/demo.rs)（`cargo run --example demo`）。

## 核心 API

入口：`KbBuilder::new(root)` → `Kb`，再拆出四类子句柄。

| 句柄            | 方法                                             | 说明                                                                                 |
| ------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------- |
| `kb.ops()`    | `create / merge / upsert_from_source`          | 创建与重导入；`create` 自动去重（summary 按来源，concept/entity 按标题+类型），命中则转入 merge                |
| <br />        | `edit`                                         | 三原语：`SearchReplace` / `InsertAfter`（可用 `scope` 限定到某来源）/ `ReplaceSource`；逐项报告成败     |
| <br />        | `delete / delete_source / prune`               | 删除条目（并清理他处引用）；按来源删除；清除悬空引用（保留标题提示，跳过代码区）                     |
| <br />        | `rename / migrate`                             | 标题变更（原子重命名，title+type 查重与 `create` 一致）；存量格式一次性迁移（幂等）                |
| <br />        | `lint`                                         | 只读体检，返回 `LintIssue` 列表                                                             |
| `kb.search()` | `query`                                        | 检索，支持 `Keyword / Semantic / Hybrid`、`expand` 邻域扩展、`wiki_type` 过滤、`include_content` |
| <br />        | `get_entry / list_entries / entries_by_source` | 元数据读取；另有 `list_entries_page`（分页）、`count_entries / count_by_type / recent_entries`（聚合，避免全量载入） |
| `kb.index()`  | `reindex_entry / rebuild`                      | 单条目重派生 / 全量重建                                                                      |
| `kb.embed()`  | `attach / refresh_all`                         | 注入向量提供方并补算向量                                                                       |

### 检索细节

- **打分**：`bm25` 取负后归一化到 `[0,1]`；语义用余弦相似度；混合检索动态融合 `w = 0.7 + 0.2*k`。

- **邻域扩展**：`expand=1|2` 沿关系双向 BFS，邻居分数 = 父分 × `0.8` 每跳衰减。

- **分词与中文兜底**：查询按空白切词，逐词 `AND` 匹配；`< 3` 字符的词（如 2 字中文词）无法成 trigram 会被剔除，全部剔除时退回 `LIKE` 匹配（2 字中文词也能命中，中英混合查询不落空）。

- **语义新鲜度**：向量带 `content_hash`；内容变更后旧向量会被读路径自动跳过，无 provider 时 `Semantic/Hybrid` 自动退化为关键词。

## 溯源标记语法

- 标签形如 `<ref-aaaaaaaaaaaaaaaa>…内容…</ref-aaaaaaaaaaaaaaaa>`。

- 一段内容的**有效来源** = 覆盖它的所有开放标签来源之并集（支持嵌套）。

- 容错解析（T1–T8）：未闭合标签在块尾自动闭合、孤立闭标签忽略、属性标签接受并忽略、代码块与行内代码内不解析等。

- 内容变换 `filter_by_source / remove_source / replace_source / normalize` 全部保证重建后的归属语义不变，可安全用于"读→改→写"。

## 安全与规范

- Cargo 层面禁止 `unsafe`、`unwrap`、`expect`、`panic`、`todo`、`unimplemented`。

- 写入全程原子：临时文件 + `rename`，崩溃不产生半截文件。

- 前端用 `rusqlite` 的 `Mutex` 串行化访问；WAL 模式 + busy timeout。

## Feature flags

| feature      | 依赖                                 | 启用能力                                     |
| ------------ | ---------------------------------- | ---------------------------------------- |
| `index`（默认）  | rusqlite / uuid / sha2             | 索引、ops、search、embed、lint                 |
| `async`      | tokio(`rt`, `rt-multi-thread`)     | `AsyncKb`（`spawn_blocking` 薄包装）          |
| `mcp-server` | rmcp / tokio / serde / serde\_json | MCP stdio server（官方 rmcp SDK，2026-07-28） |

## 作为 MCP server

基于官方 [rmcp SDK](https://github.com/modelcontextprotocol/rust-sdk)（MCP **2026-07-28** 无状态规范），启用 `mcp-server` feature：

```rust
fn main() {
    let kb = KbBuilder::new("/path/to/root").open().unwrap();
    fluen_kb::mcp::run_stdio(kb).unwrap();   // 同步阻塞；内部自建 tokio runtime
}
```

- 已运行于 tokio runtime 的宿主请用 async 入口 `fluen_kb::mcp::serve_stdio(kb)`。

- 暴露工具：`knowledge_query` / `knowledge_query_batch` / `knowledge_create_entry` / `knowledge_edit_entry` / `knowledge_get_entry` / `knowledge_list_entries`（分页，`limit`/`offset`，默认每页 100） / `knowledge_meta` / `knowledge_delete_entry` / `knowledge_lint` / `knowledge_prune`。

- 协议要点由 rmcp 保证：无 `initialize` 握手、支持 `server/discover`、结果带 `resultType`、工具列表确定性排序，符合原生规范。

## 开发

```bash
cargo test             # 全部测试（各 feature 合计 93）
cargo test --all-features
cargo run --example demo   # 端到端演示
```

