# fluen-kb 执行规划

> **依据**：`KNOWLEDGE_BASE_REDESIGN.md`（V2 设计定稿）。
> **范围**：本仓库即 crate `fluen-kb`（非 workspace 子目录），`src/lib.rs` 为唯一入口。
> **产出物**：可通过 `cargo test --all-features` 的 SDK crate + MCP server。

***

## 1. 核心思路

### 1.1 一条主轴

所有能力围绕一条不变的主轴组织：

```
读：  MD 文件 ──parse──▶ EntryDocument ──derive──▶ index.db（缓存）──▶ search
写：  变换正文(syntax) ──▶ 原子落盘(store) ──▶ 单条目重派生(index) ──▶ 追加 log
```

**MD 是唯一真相源，DB 是纯派生缓存。** 任何写操作都是这条四段管线的组合，
任何读操作都只依赖派生索引或按需读文件。模块划分即沿这条主轴切分，
不存在绕过管线的旁路。

### 1.2 三个必须做对的算法

全项目只有三处逻辑复杂度集中区，其余均为薄编排。实现时以此节为参照：

**A. 容错溯源解析（syntax/parser.rs）**

单遍线性扫描 + 开放标签栈，不建 AST：

1. 预扫描记录围栏代码块区间（` ``` `/`~~~` 配对）与行内代码区间（T4 豁免区）
2. 主扫描识别 `<ref-…>` / `</ref-…>`：不符合 `ref-`+16hex 的按字面文本跳过（T3）
3. 开标签入栈；闭标签弹出**栈中最近的同名 id**（T5 交叉嵌套：弹出到目标为止，
   被隐式关闭的标签记录待"重开"，保证后续内容归属不丢失）
4. 到达块边界（空行分隔的段落/列表项/标题/围栏块边界/EOF）时栈非空 → 记录
   T1 隐式闭合点（parse 结果等价于已闭合；normalize 才落盘显式标签）
5. 每个文本片段的归属集 = 扫描至此的整栈快照（§3.2.2 并集总则）

输出统一为**片段序列** `Vec<Span { text, sources: Vec<SourceId> }>`，
SourceTree 即片段序列的轻封装。`strip`/`filter`/`remove` 全部消费同一序列，
杜绝多套解析逻辑。

**B. 归属集剪枝（syntax/ops.rs，§3.2.6）**

`remove_source(body, s)` 逐片段判定：

| 片段归属集 A      | 动作  |      |                     |
| ------------ | --- | ---- | ------------------- |
| `A == {s}`   | 删文本 |      |                     |
| `s ∈ A` 且 \` | A   | >1\` | 保文本，输出标签时跳过 s 的开闭标签 |
| `s ∉ A`      | 原样  |      |                     |

实现要点：重建输出时按片段的**剩余归属集**重新包裹标签（同一片段的剩余
归属集标签开一次、相邻同集片段合并），而不是在原文上做区间删除——后者在
嵌套场景必然错。`replace_source = remove_source + 追加子树`。

**C. frontmatter 逐字节 round-trip（store/frontmatter.rs）**

不引入 YAML 库。手写行级解析，只认 `key: value` 标量子集（设计 §3.1 已知
字段全部是字符串）：

- 已知四键（title/type/created/updated）解析为强类型字段

- 未知键：**值以原始字符串逐字节保留**（含引号、注释、空白），存入
  `extra: BTreeMap<String, String>`

- 序列化：已知键固定顺序 + 未知键按字典序原样输出

- 违反 `key: value` 行形态的内容（极端边界）→ 整块按字面保留，
  `KbError::Invalid` 拒绝写入，绝不静默丢弃

这是"零依赖 + MUST 级 round-trip"两个约束的唯一交集解，也天然免疫
YAML 库的重排/转义副作用。

### 1.3 与设计文档的三处落地细化（非偏离，仅具体化）

1. `extra_fm` 类型定为 `BTreeMap<String, String>`（原始值文本），替代文档中
   的 `Value`——"值逐字节保留"决定了值只能是字符串，且 L1 保持零依赖
2. 内置 ID 生成器（UUID4 截断）依赖 `uuid` crate，置于默认 feature
   `index` 之下——L0/L1 的 `SourceId::parse` 只做校验，不生成
3. 本仓库独立成 crate（设计文档中 `crates/fluen-kb` 路径按仓库现状映射）

***

## 2. 模块设计

### 2.1 目录结构与职责矩阵

```
src/
├── lib.rs            入口：feature 门控 + 公开 API re-export（~80 行）
├── error.rs          KbError / KbResult（~60 行）
├── ids.rs            SourceId / WikiId / WikiType / Predicate（~150 行）
├── model.rs          EntryMeta / EntryDocument（~80 行）
│
├── syntax/           L0 溯源标记（零依赖）
│   ├── mod.rs        API 面 re-export（~30 行）
│   ├── parser.rs     容错扫描器 → Span 序列（算法 A）（~300 行）
│   └── ops.rs        filter/remove/replace/normalize（算法 B）（~250 行）
│
├── store/            L1 文件读写（零依赖）
│   ├── mod.rs        Store：路径规则、目录自愈、load/save 编排（~200 行）
│   ├── frontmatter.rs  行级解析/序列化（算法 C）（~200 行）
│   ├── filename.rs   WikiId↔路径、标题净化（§3.3）（~100 行）
│   └── atomic.rs     原子写 / 原子重命名（~80 行）
│
├── index/            L2 索引（feature `index`，rusqlite）
│   ├── mod.rs        Indexer：open/reindex_entry/rebuild（~300 行）
│   ├── schema.rs     DDL v2、连接参数、user_version（~120 行）
│   └── views.rs      index.md 生成、log.md 追加与轮转（~200 行）
│
├── ops/              L2 写操作编排（feature `index`）
│   ├── mod.rs        Ops 句柄 + 公共管线 run_pipeline（~150 行）
│   ├── create.rs     create（含去重分流）（~180 行）
│   ├── merge.rs      merge / upsert_from_source（~180 行）
│   ├── edit.rs       EditOp 三原语（~200 行）
│   ├── delete.rs     delete / delete_source / prune（~250 行）
│   ├── migrate.rs    存量迁移（幂等）（~150 行）
│   └── rename.rs     rename（~80 行）
│
├── search/           L2 检索（feature `index`）
│   ├── mod.rs        Searcher：query 分发 + QueryParams（~150 行）
│   ├── keyword.rs    ID 直查 / FTS bm25 / LIKE 回退（~200 行）
│   ├── rank.rs       分数归一、hybrid 融合、expand BFS（~180 行）
│   └── meta.rs       list_entries / entries_by_source（~100 行）
│
├── embed.rs          L2 KnowledgeEmbedding trait + stale 管理（~150 行）
├── lint.rs           L2 只读体检（七项检查）（~250 行）
├── handle.rs         Kb / KbBuilder / 子句柄装配（~250 行）
│
├── async_kb.rs       L3 AsyncKb（feature `async`）（~300 行）
└── mcp.rs            L4 MCP server（feature `mcp-server`）（~400 行）

tests/
├── syntax_cases.rs   T1–T8 容错矩阵 + 归属语义 + 剪枝反例
├── store_cases.rs    round-trip / 原子写 / 文件名
├── pipeline_cases.rs 金样本 rebuild 幂等 / 端到端写读
├── ops_cases.rs      重导入/删除/迁移/悬空治理
└── search_cases.rs   检索路径与 expand
```

### 2.2 职责边界（一处一能力）

| 模块       | 唯一职责                             | 明确不做                     |
| -------- | -------------------------------- | ------------------------ |
| `syntax` | 标记文本 ↔ Span 序列的纯变换               | 不懂文件、不懂 DB               |
| `store`  | EntryDocument ↔ 磁盘文件             | 不做任何业务校验（校验在 ops）        |
| `index`  | MD → DB 派生表；index.md / log.md 生成 | 不改写条目文件                  |
| `ops`    | 业务语义编排（校验、去重、分流）                 | 不直接摸 Connection 细节、不手写解析 |
| `search` | 只读查询与打分                          | 不写任何数据                   |
| `embed`  | 向量注入、content\_hash、stale 判定      | 不实现具体嵌入模型                |
| `lint`   | 只读体检，产出 `Vec<LintIssue>`         | 不修复（修复归 prune）           |
| `handle` | 装配与生命周期                          | 无业务逻辑                    |

### 2.3 共享内核

```rust
// handle.rs —— 所有子句柄共享同一内核，避免状态复制
struct KbInner {
    root: PathBuf,
    id_gen: Box<dyn Fn() -> String>,          // 可注入
    source_validator: Option<Box<dyn Fn(&SourceId) -> bool>>,
    conn: Mutex<rusqlite::Connection>,        // WAL + busy_timeout=5000
    embed: Option<Arc<dyn KnowledgeEmbedding>>,
    store: Store,                             // 无状态，纯路径函数集
}

pub struct Kb(Arc<KbInner>);   // CheapClone；子句柄 ops()/search()/index() 按需借出
```

写操作公共管线（ops/mod.rs，所有写操作必须经过）：

```rust
fn run_pipeline(inner: &KbInner, id: &WikiId, verb: &str, detail: Vec<String>)
    -> Result<()>
{
    // 1. 调用方已完成正文变换并落盘（store.save 原子写）
    // 2. index.reindex_entry(id)          —— 含悬空关联行过滤
    // 3. embed 刷新（有 provider 时异步重算，见 §5 P4）
    // 4. views.append_log(verb, id, detail)
    // 5. views.rebuild_index_md()
}
```

***

## 3. 接口设计（公开 API 面）

```rust
// ── 句柄 ─────────────────────────────────────────────
KbBuilder::new(root: impl Into<PathBuf>)
    .with_id_generator(f)            // Option
    .with_source_validator(f)        // Option
    .open() -> KbResult<Kb>          // 不存在则初始化 wiki/ 目录结构

// ── ops() 写 ─────────────────────────────────────────
create(type, title, body, relations, source: Option<SourceId>, authors: Vec<WikiId>) -> KbResult<CreateOutcome>  // Created(id) | MergedInto(id)
merge(id, body, relations) -> KbResult<()>
upsert_from_source(id, src, new_body) -> KbResult<()>
edit(id, ops: Vec<EditOp>) -> KbResult<Vec<Result<()>>>   // 逐项成败，整体提交
rename(id, new_title) -> KbResult<()>
delete(id) -> KbResult<()>
delete_source(src) -> KbResult<DeleteSourceReport>
prune() -> KbResult<PruneReport>
migrate() -> KbResult<MigrateReport>                       // 幂等
lint() -> KbResult<Vec<LintIssue>>

// ── search() 读 ──────────────────────────────────────
query(params: QueryParams) -> KbResult<Vec<SearchHit>>    // 含 expand ≤ 2
get_entry(id) -> KbResult<EntryDocument>
list_entries(wiki_type: Option<WikiType>) -> KbResult<Vec<EntryMeta>>
entries_by_source(src: &SourceId) -> KbResult<Vec<EntryMeta>>

// ── index() 派生 ─────────────────────────────────────
reindex_entry(id) -> KbResult<()>
rebuild() -> KbResult<()>                                  // 永远可运行的最终修复手段

// ── embed() 向量 ─────────────────────────────────────
attach(provider: Arc<dyn KnowledgeEmbedding>) -> KbResult<()>
```

**接口纪律**（对齐设计 §9.2）：

- 参数用 `Option`/默认值表达可选项，无 builder 泛滥（仅句柄用 builder）

- 错误单一出口 `KbError::{NotFound, Invalid, Io, Db, Embedding, Cancelled}`

- `src/` 内禁止 `unwrap/expect/panic!`（clippy `unwrap_used` 设为 deny），
  测试代码不受限

- 全部公开类型 `Send + Sync`；无全局静态、不读环境变量、不写 home

**feature 矩阵**：

```toml
[features]
default  = ["index"]
index    = ["dep:rusqlite", "dep:uuid"]      # store+ops+index+search+lint
async    = ["dep:tokio", "index"]            # AsyncKb
mcp-server = ["dep:rmcp", "dep:tokio", "index"]

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"], optional = true }
uuid     = { version = "1", features = ["v4"], optional = true }
tokio    = { version = "1", features = ["rt"], optional = true }
rmcp     = { version = "...", optional = true }   # 实施时锁定当时稳定版
```

`cargo check --no-default-features` 必须通过（仅 syntax + model + store + error，
零第三方依赖）——每个阶段 CI 项。

***

## 4. 实施步骤

> 依赖顺序严格线性：P0 → P1 → P2 → P3 → P4 → P5。P3 依赖 P2 的全部，
> P4 依赖 P2（检索读 DB）但不依赖 P3。每阶段完成即打 tag，主应用期间
> 继续运行旧 crate（设计 §11）。

### P0 骨架（半天内性质的收口工作，不含业务）

| #   | 任务                                                              | 产出                           |
| --- | --------------------------------------------------------------- | ---------------------------- |
| 0.1 | 删除 `src/main.rs`，建 `lib.rs` + 模块骨架（空实现 + `todo!` 除外——直接留空模块不暴露） | `cargo check` 全 feature 通过   |
| 0.2 | `error.rs` / `ids.rs` / `model.rs` 完整实现（三文件无算法难度，一次到位）          | 单测：ID 校验拒绝非法格式               |
| 0.3 | Cargo.toml feature 矩阵 + clippy 配置（`unwrap_used = "deny"`）       | `--no-default-features` 编译通过 |

**验收**：三命令全绿——`cargo check`、`cargo check --no-default-features`、`cargo clippy --all-features`。

### P1 语法层（L0，算法 A + B）

| #   | 任务                                                                                                              | 关键点                                                    |
| --- | --------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| 1.1 | `syntax/parser.rs`：围栏/行内代码预扫描 → 主扫描 → `Vec<Span>`                                                               | T3/T4 豁免先行，再谈标签                                        |
| 1.2 | T1 块边界隐式闭合；T2 孤立闭标签忽略；T5 交叉嵌套规范化                                                                                | 块边界 = 空行分隔块 / 标题行 / 围栏边界 / EOF                         |
| 1.3 | `syntax/mod.rs` API：`parse` / `strip`（返回纯文本 + 区间归属 SourceMap）/ `collect_sources` / `filter_by_source`           | 全部消费同一 Span 序列                                         |
| 1.4 | `syntax/ops.rs`：`remove_source`（算法 B，按剩余归属集重建包裹）/ `replace_source` / `normalize`（T1 显式化 + T5 折叠 + T7 清空标签，语义不变） | normalize 幂等：`normalize(normalize(x)) == normalize(x)` |
| 1.5 | 测试矩阵                                                                                                            | 见验收                                                    |

**验收**：

- T1–T8 每条容错规则至少 2 个用例（含嵌套、多来源、跨块标签 T8）

- §3.2.2 四条归属语义用例逐一断言

- §3.2.6 反例原样落地：`<ref-a>结论X。<ref-b>细节Y。</ref-b></ref-a> <ref-b>补充Z。</ref-b>`
  经 `remove_source(a)` → `<ref-b>细节Y。补充Z。</ref-b>`

- `strip` 后文本不含任何标签词；`collect_sources` 去重有序

- `normalize` 幂等；round-trip（parse→重建→归属集不变）

- `cargo test --no-default-features` 通过（证明零依赖）

### P2 存储 + 索引（L1 + L2 派生）

| #   | 任务                                                                                                      | 关键点                                                                                                                             |
| --- | ------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| 2.1 | `store/frontmatter.rs`（算法 C）                                                                            | 未知键值逐字节保留；已知键定序输出；畸形块拒绝写入                                                                                                       |
| 2.2 | `store/filename.rs` + `store/atomic.rs`                                                                 | 净化规则 §3.3（剔除 `\/:*?"<>\|`、截 50、trim）；tmp 同目录 rename                                                                             |
| 2.3 | `store/mod.rs`：load（解析 frontmatter + 正文 + `## 关联页面` 区提取）/ save                                          | 关联区解析：`- [predicate :: ] [[target]]` EBNF §3.1.3；无效行保留原样并上报                                                                     |
| 2.4 | `index/schema.rs`：DDL v2（含 `content_hash`）+ 连接参数（WAL / FK / busy\_timeout）+ `user_version=2`            | 与设计 §6.1 逐列一致，不多不少                                                                                                              |
| 2.5 | `index/mod.rs`：`reindex_entry`                                                                          | 事务覆盖写 entries / entry\_relations / entry\_sources / entries\_fts；**悬空关联行过滤（MUST）**：`to_id ∉ entries` → 跳过 + lint issue 收集 + 不报错 |
| 2.6 | `index/views.rs`：index.md 全量重建（三区统一格式 §3.4）；log.md 追加 + 1MB 轮转（`log-{YYYYMM}.md`、`log-{YYYYMM}-{n}.md`） | SDK 自身不解析 index.md                                                                                                              |
| 2.7 | `handle.rs` 最小版：KbBuilder + open（初始化目录）+ index() 句柄                                                     | 仅本阶段所需能力                                                                                                                        |

**验收**：

- 金样本（≥5 个条目，含嵌套溯源、类型化关系、未知 frontmatter 键）：
  `rebuild()` 两次，DB dump 完全一致（幂等）

- 未知键 round-trip：load → save → 文件字节级等价（已知键顺序外）

- FTS content 断言不含 `ref-` 标签词（strip 生效）

- 2 字中文词（"模型"）在 LIKE 回退路径命中正文

- 悬空关联行：reindex 不失败、行被跳过、文件未被修改

- log 轮转命名序列正确；index.md 三区格式与 §3.4 一致

### P3 操作层（L2 编排）

| #   | 任务                                                                                                                                                | 关键点                                                  |
| --- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------- |
| 3.1 | `ops/mod.rs` 公共管线 `run_pipeline`（§2.3）                                                                                                            | 所有写操作唯一出口                                            |
| 3.2 | `create.rs`：写时校验（title 非空、type 合法、summary 的 source 包裹一致性）；入参 body 剥离关联区（D17）；去重分流：summary 按派生来源、concept/entity 按 title(NOCASE)+type → 命中转 `merge` | `authors` 参数 → `作者 :: [[id]]` 关系行；返回 `CreateOutcome` |
| 3.3 | `merge.rs`：relations 去重追加；正文按 `<ref-s>` 子树追加 / 裸内容追加 + lint 警告；**summary 单源写时强校验（D21）**：body 新来源 ⊄ 现有来源集 → `KbError::Invalid`                     | 违法状态不落盘                                              |
| 3.4 | `merge.rs`：`upsert_from_source` = `remove_source(旧)` → `replace_source(新)` → 管线                                                                   | 跨来源零干扰由 P1 算法 B 保证                                   |
| 3.5 | `edit.rs`：`SearchReplace` / `InsertAfter`（scope 限定，作用域内**首个**匹配，D22）/ `ReplaceSource`                                                             | 返回逐项成败；整体提交                                          |
| 3.6 | `rename.rs`：新文件写 + 旧文件删 + DB file\_path/title + reindex + index.md                                                                                | 原子重命名封装为单操作                                          |
| 3.7 | `delete.rs`：`delete`（D24：关联区删整行、正文摘链接 token 保句子）；`delete_source`（反查 → 剪枝 → 空且无关系自动删 + 日志 → 单源 summary 随之删除）；`prune`（落盘清除悬空引用，返回报告）                | 引用清理逻辑与 prune 共用同一实现，不写两遍                            |
| 3.8 | `lint.rs`：七项检查（设计 §8.3）                                                                                                                           | 只读；`Vec<LintIssue>`                                  |
| 3.9 | `migrate.rs`：五步转换（§10.3），键已移除则 no-op                                                                                                              | 幂等可重复执行                                              |

**验收**（对齐设计 §11 P3 并具体化）：

- 同 ref 重导入 = 子树替换（旧内容不残留）；异 ref = 追加共存；
  删 ref = 归属集剪枝且**共同归属内容无损**

- summary 塞入第二来源 → `KbError::Invalid`，文件未变

- `migrate()` 连跑两次，第二次报告全 no-op

- 悬空关联行：reindex 跳过不失败 → `prune()` 落盘清除 → `lint()` 该项归零

- `edit` 多重匹配取首个、不传播；scope=Some(src) 时跨标签边界搜索正确

- `delete` 后：DB 级联干净、他条目关联区整行移除、正文 `[[…]]` 摘 token 留句

- log.md 每操作一条，动词与规模正确（§3.5 示例格式）

### P4 检索 + 向量（L2 读路径）

| #   | 任务                                                                                                                                                        | 关键点                                         |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| 4.1 | `search/keyword.rs`：路径 1 ID 直查（wiki-/ref- 前缀分流，score 1.0）；路径 2 FTS bm25 负分归一 \[0,1] + <3 字符 LIKE 回退（标题 + strip 正文，固定 0.5）                                 | ref- 直查走 `idx_sources_ref`                  |
| 4.2 | `search/rank.rs`：hybrid 动态权重（kw∈\[0.7,0.9] 随 kw 分线性上升）；expand BFS 深度 ≤2、父分 × 0.8^跳数、去重截断 top\_k                                                           | 不做 PageRank/重排（D11 明确排除）                    |
| 4.3 | `embed.rs`：`KnowledgeEmbedding` trait；content\_hash = sha256(标题+strip 正文) hex 前 16 位（手写 sha256 或引入 `sha2`——**决策：引入 sha2**，放 `index` feature，理由：密码学实现不宜手写） | 无 provider 不计算；读路径发现 hash 不符 → 跳过 + warning |
| 4.4 | `search/mod.rs`：`query` 分发（semantic 不可用自动降级 keyword）；`search/meta.rs`：`list_entries` / `entries_by_source` / `get_entry`                                  | `include_content` 命中后读文件                    |
| 4.5 | 写路径接入：`run_pipeline` 内 embed 刷新（有 provider 时）                                                                                                             | P3 管线预留的挂点在此填充                              |

**验收**：

- ID 直查 / keyword / semantic 不可用降级 / hybrid 四路径行为符合 §7 顺序

- expand=1：直接命中 + 一跳邻居（0.8×父分）；expand=2 衰减正确；去重

- hash 不符的向量被 semantic/hybrid 跳过且不报错

- `entries_by_source` 返回与 `entry_sources` 一致

- 2 字中文词经 LIKE 回退命中（P2 已测，此处回归）

### P5 异步 + MCP（L3/L4）

| #   | 任务                                                         | 关键点                                                                 |
| --- | ---------------------------------------------------------- | ------------------------------------------------------------------- |
| 5.1 | `async_kb.rs`：`AsyncKb`，方法面与 `Kb` 一一对应，`spawn_blocking` 包装 | 手写薄包装不引宏依赖；`Send+Sync+Clone`                                        |
| 5.2 | `mcp.rs`：stdio server，工具面设计 §9.4 十个工具一一映射                  | `create_entry` schema 中说明溯源包裹要求；`edit_entry` 暴露 scope/ReplaceSource |
| 5.3 | MCP 集成测试：spawn 子进程 + JSON-RPC 握手 + 各工具 happy path          | rmcp 版本实施时锁定，写入 Cargo.toml 注释                                       |

**验收**：`AsyncKb` 并发调用无死锁（Mutex 不跨 await 持有）；MCP 工具
list\_tools/各工具调用 e2e 通过；`cargo test --all-features` 全绿。

### P6 宿主切换（本仓库外，列出供追踪）

- [ ] src-tauri `kb_tools.rs` 薄适配（\~百行），替换旧 `KnowledgeToolProvider`

- [ ] builder prompts：产出正文整体包裹 `<ref-当前 SourceId>`

- [ ] merge 路径 → `upsert_from_source`；收尾跑 `lint()`

- [ ] 删除 IndexSnapshot 解析依赖 → `list_entries()`

- [ ] 前端：markdown-it 溯源插件 / 去 tags UI / 按来源筛选

- [ ] 全项目 `rg "fluen-knowledge"` 为空后删除旧 crate（子仓库，需授权）

***

## 5. 测试与质量门

| 门       | 命令                                                                  | 阶段      |
| ------- | ------------------------------------------------------------------- | ------- |
| 零依赖     | `cargo check --no-default-features`                                 | P0 起每阶段 |
| 无 panic | `cargo clippy --all-features -- -D warnings -W clippy::unwrap_used` | 每阶段     |
| 全量      | `cargo test --all-features`                                         | P2 起每阶段 |
| 行数      | 单文件 ≤ 500 行（脚本检查，超限即拆）                                              | 每阶段收尾   |

测试分层纪律：

- **单元测试**贴模块文件（`#[cfg(test)]`），只测本模块契约

- **集成测试**（tests/）只测跨模块管线（写→派生→读、rebuild 自愈、migrate）

- 金样本目录 `tests/fixtures/golden/` 固化一份完整 wiki，作为 P2 起的回归基线

- 容错规则（T1–T8）用例以表驱动组织，一条规则一个测试函数，禁止共享大 fixture

***

## 6. 风险与预置对策

| 风险                          | 对策                                       |
| --------------------------- | ---------------------------------------- |
| T5 交叉嵌套规范化实现出错              | P1 用例先行：先写 §3.2.6 反例为失败测试，再实现            |
| frontmatter 手写解析遇边界（多行值/注释） | 范围明确为标量子集；超集形态 → 字面保留 + Invalid 拒写，不猜    |
| trigram FTS 中文 2 字盲区        | LIKE 回退为指定路径（非兜底 hack），有专属测试             |
| WAL 跨平台（Windows rename 语义）  | 原子写 tmp 与目标同目录；P2 补 Windows 专属用例         |
| rmcp API 变动                 | P5 才引入；版本锁定 + 集成测试覆盖握手                   |
| embed 异步刷新与写管线竞争            | provider 刷新在 reindex 完成后触发；hash 判定保证最终一致 |

***

## 7. 总验收（DoD）

1. 设计 §11 P1–P4 验收列全部通过，且本文 §4 各阶段验收项全部通过
2. `cargo test --all-features` / `--no-default-features` / clippy 三绿
3. `src/` 无 `unwrap/expect/panic!`；单文件 ≤ 500 行
4. 公开 API 面与本文 §3 一致（`cargo doc` 无 broken intra-doc link）
5. 金样本 rebuild 幂等 + 任意时刻删 `index.db` 后 `rebuild()` 恢复等价状态
6. MCP server 十工具 e2e 通过

