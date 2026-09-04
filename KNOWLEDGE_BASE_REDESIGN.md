# fluen-kb 设计方案 V2 —— 通用知识库 SDK

> **定位**：一个**通用知识库 SDK**（crate：`crates/fluen-kb`）。轻量、高效、稳定，
> 易于集成、配置与扩展。任何拥有"文档来源 → 结构化知识条目"场景的应用
> （学术写作、文献管理、Agent 记忆、文档问答）都可直接接入，不必理解 Fluen 业务。
>
> **版本**：V2（从 0 重构，替代 `crates/fluen-knowledge`）。
> **状态**：设计定稿，待实施。决策记录见 §14。

---

## 0. SDK 定位与设计原则

### 0.1 分层承诺

| 层 | 依赖 | 承诺 |
|---|---|---|
| L0 语法层（溯源标记） | **零第三方依赖** | 纯函数，可独立复用于任何 Rust 项目 |
| L1 存储层（MD 读写） | 零第三方依赖 | 纯函数 + 原子写 |
| L2 索引/检索/操作层 | rusqlite | 默认 feature，可关闭 |
| L3 异步句柄 | tokio | feature `async` |
| L4 MCP server | rmcp | feature `mcp-server`，默认不编译（对齐 socstat/socstat-mcp 模式） |

**范围边界**（对齐 socstat）：SDK 止步于**方法 + MCP server**。不为任何 agent
框架（referee-ai 等）提供工具适配层——宿主若需将方法注册为 agent 工具，
在宿主侧写薄适配（数十行）即可。SDK 保持对宿主技术栈零感知。

### 0.2 设计原则（八条）

1. **MD 是唯一真相源**：条目文件承载全部信息；`index.db` 是纯派生缓存，
   任何时刻可删除重建，不存在"同步"问题，只有"重新派生"一个动作。
2. **溯源是一等公民**：每块正文内容都能回答"来自哪个来源"。
3. **宽容解析，严格生成**：解析器必须容错（AI 产物不规范是常态），
   生成端（prompt、文档、模板）保持严格规范。
4. **未知即保留**：解析器无权销毁它不认识的数据（frontmatter 未知键、
   标签未知属性一律 round-trip）。
5. **不 panic**：库代码零 `unwrap`/`expect`/`panic!`，所有失败走 `KbError`。
6. **无全局状态**：一切通过句柄与显式配置；不读环境变量、不写 home 目录、
   不假设项目结构（根路径由调用方给出）。
7. **写路径原子**：所有文件写入走"临时文件 + rename"；DB 走事务。
8. **Markdown 优先，标签最少**：自定义语法只做 Markdown 表达不了的两件事
   ——来源归属与类型化关联。

### 0.3 三层可变性模型

| 层 | 位置 | 可变性 | 归属 |
|---|---|---|---|
| 原始来源 | `{root}/sources/`（由宿主应用定义，如 Fluen 的 `references/raw/`） | **不可变**（捕获后只读） | 宿主应用 |
| 知识条目 | `{root}/wiki/` | **持续维护**（create/merge/upsert/delete/edit） | 本 SDK |
| 索引与日志 | `{root}/wiki/index.db`、`{root}/wiki/log.md` | 派生 / 追加 | 本 SDK |

SDK 只管理第 2、3 层；原始来源的存储格式与 SDK 无关，SDK 仅通过
**来源 ID（SourceId）** 与之关联。

---

## 1. 总体架构

```text
┌────────────────────────────────────────────────────────┐
│ 宿主应用：Fluen 构建流水线 / Agent 工具 / MCP 客户端 / 前端 │
└───────────────┬────────────────────────────────────────┘
                │ L4 MCP server（feature 门控）
┌───────────────▼────────────────────────────────────────┐
│ fluen-kb                                               │
│                                                        │
│  syntax    溯源标记 / 关联链接 解析·剥离·过滤·替换·规范化  │
│  model     EntryMeta / EntryDocument / WikiType / IDs   │
│  store     frontmatter(含未知键 round-trip)·文件名·原子写 │
│  ops       create·merge·upsert·delete·edit·lint         │
│  index     schema v2·reindex·rebuild·index.md·log.md    │
│  search    ID直查·keyword·hybrid·邻域扩展               │
│  embed     KnowledgeEmbedding trait（可选注入）          │
└───────────────┬────────────────────────────────────────┘
                │
   {root}/wiki/*.md        ← 唯一真相源
   {root}/wiki/index.db    ← 派生缓存（可随时重建）
   {root}/wiki/log.md      ← 追加式操作日志
```

写操作的标准形态：**改 MD（原子写）→ 重新派生该条目索引 → 追加日志**。
读操作只查派生索引（或按需读文件）。

---

## 2. 核心概念与术语

| 术语 | 定义 |
|---|---|
| **SourceId** | 来源标识，`ref-` + 16 位小写 hex（UUID4 前 16 位）。指向宿主管理的原始来源，SDK 不解释其内容 |
| **WikiId** | 条目标识，`wiki-` + 16 位小写 hex |
| **WikiType** | 条目类型：`summary`（综述页，对应一个来源）/ `concept`（概念页）/ `entity`（实体页）。**固定三类**（决策 D-kept；`type` 字段为字符串，格式不阻碍未来扩展） |
| **Predicate** | 类型化关系的谓词，如 `应用了`、`隶属于`；缺省为 `related` |
| **SourceTree** | 正文解析产物：以"未溯源"为根、来源标签为节点的归属树 |
| **EntryMeta** | 条目索引视图（DB 派生，字段永远完整） |
| **EntryDocument** | 条目文件视图（frontmatter + 原始正文 + SourceTree） |

---

## 3. 语法规范（完整版）

### 3.1 条目文件

路径：`{root}/wiki/{concepts|entities|summaries}/{WikiId}-{标题}.md`

```markdown
---
title: 知识追踪
type: concept            # summary | concept | entity
created: 2026-09-03T12:00:00+00:00
updated: 2026-09-03T12:30:00+00:00
---                      # ↑ 已知字段；任意未知键同样合法（见 3.1.2）

正文（Markdown + 溯源标记，见 3.2）

## 关联页面
- [[wiki-3d09a8bb72c14e5f]]                       # 无类型 → related
- 应用了 :: [[concepts/wiki-91f2c43bf4de401e-BKT]]  # 类型化关系
- 作者 :: [[entities/wiki-aaa-张三]]                # summary 的作者（类型化关系）
```

#### 3.1.1 已知字段

| 字段 | 类型 | 约束 |
|---|---|---|
| `title` | string | 必填；非空 |
| `type` | string | 必填；`summary` / `concept` / `entity` |
| `created` / `updated` | string | RFC3339 |

**frontmatter 三类条目完全同构**（无 summary 专属字段）：

- **来源归属只存在于正文溯源标签**。summary 的来源 = 其正文包裹的
  `<ref-xxx>`（由 `entry_sources` 派生），不设 frontmatter `source` 字段——
  单一表示，杜绝两处不一致
- **作者 = 类型化关系**：`作者 :: [[wikiId]]`，与其它关联同区管理、同表
  （`entry_relations`）存储，可随图谱查询，无需独立字段

#### 3.1.2 未知键 round-trip（硬保证）

- 解析：未知键收进 `extra: BTreeMap<String, Value>`；**不报错、不丢弃**
- 序列化：已知键按固定顺序输出，未知键原样（值逐字节保留）追加
- 推论：任何"解析→重写"路径（edit/merge/upsert/rebuild 重建文件）都不丢失
  宿主自定义元数据。此条为 SDK 的 MUST 级保证

#### 3.1.3 `## 关联页面` 区

- **权威模型：文件为准**（与原则 1 一致）。该区是 relations 的真相源：
  reindex/rebuild 从本区解析派生 `entry_relations`，语义同 frontmatter
- 解析规则：**导入所有语法有效的行**；语法无效的行不导入并由 lint 报
  warning（原样保留在文件中，不做静默修改）
- `create` 的 body **入参**中若含该区则剥离——因为关联由 `relations` 参数
  显式提供，避免同一批关系双写；这只针对入参去重，不构成对文件权威性的
  限制。文件写定后，任何符合语法的行（含后续经工具/manual 编辑产生的）
  在 reindex 时一视同仁
- 语法：

```ebnf
relation_line  ::= "- " [predicate " :: "] link
predicate      ::= 1*32(LETTER | DIGIT | "_" | CJK)     ; 如 应用了 / uses / 属于
link           ::= "[[" target "]]"
target         ::= [type_dir "/"] wiki_id ["-" title]    ; title 部分仅注释性
```

- 无谓词 → 谓词 = `related`；谓词原样存储、大小写敏感
- 同 (from, predicate, to) 去重

### 3.2 溯源标记

#### 3.2.1 语法

```ebnf
source_tag  ::= open_tag body close_tag
open_tag    ::= "<" ref_id attr* ">"
close_tag   ::= "</" ref_id ">"
attr        ::= attr_name "=" quoted_value        ; 预留：解析时接受并忽略
ref_id      ::= "ref-" 16HEXDIG                   ; 大小写不敏感，归一为小写
body        ::= 任意 Markdown 内容（可跨块、可嵌套 source_tag）
```

示例（嵌套 + 多来源）：

```markdown
<ref-3d09a8bb72c14e5f>
知识追踪模型的早期工作以贝叶斯方法为主。
<ref-91f2c43bf4de401e>
其中贝叶斯知识追踪（BKT）由 Corbett 与 Anderson 于 1995 年提出。
</ref-91f2c43bf4de401e>
后续深度知识追踪（DKT）将 RNN 引入该任务。
</ref-3d09a8bb72c14e5f>

此段无包裹，归属为「未溯源」。
```

#### 3.2.2 归属语义（唯一总则）

> **一段内容的有效来源 = 从根到该内容的开放标签栈中所有 SourceId 的并集。**

- `<s1>甲</s1>` → 甲: {s1}
- `<s1><s2>乙</s2></s1>` → 乙: {s1, s2}（共同归属）
- `<s1>甲<s2>乙</s2>丙</s1>` → 甲 {s1}，乙 {s1,s2}，丙 {s1}
- 无包裹 → 归属 ∅（**未溯源**，合法状态：AI 通用知识、存量迁移内容）

#### 3.2.3 解析容错规则

| # | 情形 | 处理 |
|---|---|---|
| T1 | 未闭合的开标签 | 在**当前块**末尾自动闭合（块 = 空行分隔的段落 / 列表项 / 标题 / 围栏代码块边界 / 文件末尾） |
| T2 | 孤立的闭标签 | 忽略该闭标签 |
| T3 | 标签名不符合 `ref-` + 16 hex | 不识别为溯源标签，按字面文本保留 |
| T4 | 围栏代码块 / 行内代码内 | 不解析，字面保留 |
| T5 | 交叉嵌套 `<s1>甲<s2>乙</s1>丙</s2>` | 规范化为树（先闭 s1 时隐式闭 s2 并在其后重开），不报错 |
| T6 | 同一 open_tag 出现未知属性 | 接受并忽略属性（前向兼容），归一化时不保留 |
| T7 | 空标签（开闭间无内容） | 解析为空归属节点，normalize 时可清除 |
| T8 | 标签跨越多个 Markdown 块 | 允许；归属计算不受块边界影响 |

#### 3.2.4 规范粒度

- **规范用法为块级**：一个标签包裹一个或多个完整块；行内允许但不鼓励
- 生成端（prompt/文档）只示范块级用法
- `normalize()`：落地 T1 的自动闭合（把隐式边界写成显式闭合标签）、折叠 T5、
  清除 T7。**不改变归属语义**

#### 3.2.5 syntax 模块 API（纯函数，零依赖）

```rust
pub struct SourceId(String);            // newtype，new/parse 校验 ref- + 16 hex

pub fn parse(body: &str) -> SourceTree;                       // 归属树
pub fn strip(body: &str) -> (String, SourceMap);              // 纯文本 + 字符区间归属（FTS 用）
pub fn collect_sources(body: &str) -> Vec<SourceId>;          // 来源清单（entry_sources 派生）
pub fn filter_by_source(body: &str, s: &SourceId) -> String;  // 抽取归属含 s 的内容
pub fn remove_source(body: &str, s: &SourceId) -> (String, bool); // 按归属集剪枝（见 3.2.6）
pub fn replace_source(body: &str, s: &SourceId, new: &str) -> String; // 替换/追加子树
pub fn normalize(body: &str) -> String;
```

#### 3.2.6 来源移除语义（按归属集剪枝）

`remove_source(body, s)` **不是**"摘除 s 的语法子树"——嵌套布局下那会连带
删除共同归属内容，违反跨来源零干扰。正确定义（与 §3.2.2 并集模型自洽）：

对每个内容片段按其归属集 A 处理：

| 归属集 A 与 s 的关系 | 处理 |
|---|---|
| `A == {s}`（独属） | **删除**该内容 |
| `s ∈ A` 且 `A ⊃ {s}`（共同归属，如 {a,b} 移除 a） | **摘除 s 标签边界，保留内容**，归属降为 `A \ {s}` |
| `s ∉ A` | 原样保留 |

```text
反例验证：正文 = <ref-a>结论X。<ref-b>细节Y。</ref-b></ref-a> <ref-b>补充Z。</ref-b>
归属：X={a}，Y={a,b}，Z={b}
remove_source(a) → 删"结论X。"；Y 摘掉 a 边界，保留并降归属为 {b}；Z 不动
结果：<ref-b>细节Y。补充Z。</ref-b>（normalize 合并相邻同源片段）
```

保证：移除来源 s **绝不删除**其他来源也归属的内容；只可能丢失"仅 s 独属"
的信息——这正是"移除来源"的本意。`replace_source` = `remove_source` +
追加新子树，因此重导入天然满足跨来源零干扰。

### 3.3 文件名

- 格式：`{WikiId}-{净化标题}.md`；标题为空 → `{WikiId}.md`
- 净化：剔除 `/ \ : * ? " < > |`，截断至 50 字符，trim
- 标题变更 → store 层原子重命名（写新 + 删旧 + 更新 DB `file_path`）

### 3.4 index.md（派生，格式统一）

```markdown
---
title: 知识库索引
type: index
created: RFC3339
updated: RFC3339
---

# index
## Summaries
- [[summaries/{WikiId}-{标题}]]
## Concepts
- [[concepts/{WikiId}-{标题}]]
## Entities
- [[entities/{WikiId}-{标题}]]
```

- 三区统一格式（V1 中 Concepts/Entities 带的 `<@id>` 前缀**废除**；
  wikiID 从链接路径统一提取）
- 每次条目写操作后由 SDK 从 DB 全量重建（原子写）
- 仅供人类浏览与宿主快速导览；**SDK 自身不解析它**（V1 的 IndexSnapshot
  依赖由此消除——宿主要快照请调 `list_entries`）

### 3.5 log.md（追加式操作日志）

```markdown
## [2026-09-03T12:30:00Z] upsert | wiki-91f2… 知识追踪
- ref-3d09… 子树替换（旧 842 字符 → 新 917 字符）
- 触发：宿主操作（如 Fluen 的 reference_import）

## [2026-09-03T12:31:10Z] merge | wiki-3d09… 自适应学习
- 合并来源 ref-91f2…（+320 字符）；+1 relation
```

- 每次写操作追加一条；格式 `## [RFC3339] 动词 | WikiId 标题` + 明细行
- 动词集：`create | merge | upsert | delete | edit | rename | prune | rebuild | migrate`
- 仅追加；单文件超过 1 MB 时轮转为 `log-{YYYYMM}.md`；同月第二次及以后的
  轮转使用 `log-{YYYYMM}-{n}.md`（n 从 2 递增）
- **定位是审计日志，不是 undo 机制**：只记操作与规模，不存内容快照；
  内容级恢复依赖宿主的版本控制（Fluen 项目本身是 git 仓库，wiki/ 全程可
  回溯）。需要内容快照的宿主可自行在写路径挂钩子
- 用途：AI 自主写入的审计、变更追溯、宿主展示"最近变更"

---

## 4. 数据模型

```rust
pub struct SourceId(String);   // ref-xxxxxxxxxxxxxxxx
pub struct WikiId(String);     // wiki-xxxxxxxxxxxxxxxx

pub enum WikiType { Summary, Concept, Entity }   // 固定三类

/// 索引视图（DB 派生；无"半填充"歧义——V1 的 content/authors 空字段问题已消除）
pub struct EntryMeta {
    pub id: WikiId,
    pub wiki_type: WikiType,
    pub title: String,
    pub file_path: String,            // 相对 root，如 wiki/concepts/wiki-xxx-标题.md
    pub relations: Vec<(Predicate, WikiId)>,  // ★ 类型化（含 summary 的「作者」关系）
    pub sources: Vec<SourceId>,       // ★ 来源 = 正文标签派生（summary 应恰为 1 个）
    pub created: String,
    pub updated: String,
}

/// 文件视图
pub struct EntryDocument {
    pub meta: EntryMeta,
    pub body: String,                 // 原始正文（含标记）
    pub extra_fm: BTreeMap<String, Value>, // frontmatter 未知键（round-trip 载体）
}
```

ID 生成：SDK 内置 UUID4 截断生成器；宿主可通过 `KbConfig` 注入自定义生成器
（如云端分配）。

---

## 5. 存储层（store）

- frontmatter：解析（含 unknown-key 捕获）/ 序列化（已知键定序 + 未知键原样）/ 原子写
- 原子写：`{file}.tmp-{pid}-{rand}` → `rename`（同目录保证原子性）
- 重命名：标题变更封装为单操作（新文件写 + 旧文件删 + DB 更新）
- 目录自愈：读/写时目录不存在则创建（`init` 显式建库，其他操作宽松处理）

---

## 6. 索引层（index，feature `index`）

### 6.1 SQLite schema v2（`PRAGMA user_version = 2`）

```sql
CREATE TABLE entries (
    id        TEXT PRIMARY KEY,        -- WikiId
    type      TEXT NOT NULL,           -- summary|concept|entity
    title     TEXT NOT NULL,
    file_path TEXT NOT NULL,
    created   TEXT NOT NULL,
    updated   TEXT NOT NULL
);

CREATE TABLE entry_relations (
    from_id   TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    predicate TEXT NOT NULL DEFAULT 'related',
    to_id     TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    PRIMARY KEY (from_id, predicate, to_id)
);

-- 来源清单（正文溯源标签派生）：按文献反查贡献条目的索引
CREATE TABLE entry_sources (
    entry_id TEXT NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    ref_id   TEXT NOT NULL,
    PRIMARY KEY (entry_id, ref_id)
);

-- FTS：content 必须为 strip() 后的纯文本（标签词不进索引）
CREATE VIRTUAL TABLE entries_fts USING fts5(
    id, title, type, content, tokenize='trigram'
);

CREATE TABLE embeddings (
    entry_id     TEXT PRIMARY KEY REFERENCES entries(id) ON DELETE CASCADE,
    embedding    BLOB NOT NULL,        -- f32 小端
    model        TEXT NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',  -- 新鲜度防护（见 6.3）
    updated      TEXT NOT NULL
);

CREATE INDEX idx_sources_ref ON entry_sources(ref_id);
CREATE INDEX idx_relations_to ON entry_relations(to_id);
```

连接参数：`journal_mode=WAL`、`foreign_keys=ON`、`busy_timeout=5000`。

### 6.2 派生管线

```rust
/// 单条目重派生：读 MD → 解析 → 覆盖写 entries/relations（由关联区派生，见
/// §3.1.3 文件为准）/ sources / FTS；并刷新该条目的 embedding 状态：
/// 计算 content_hash（标题 + strip 后正文），与 embeddings 表不符 → 标记
/// stale（见 6.3）
///
/// **关联导入过滤规则（悬空引用防御，MUST）**：导入关联行时若
/// `to_id ∉ entries`（目标条目已消失）→ 跳过该行、收集为 lint issue、
/// **不视为错误**。entry_relations 带 FK 且 foreign_keys=ON，无此规则则
/// 一条悬空行即可让 reindex_entry 失败、甚至让整库 rebuild 事务回滚——
/// rebuild 必须永远能作为"最终修复手段"运行（V1 insert_relations_safe
/// 语义的继承）。
///
/// 注意：被跳过的入边（其他条目指向本条目的关系）在目标条目**之后**创建时，
/// 会在该目标条目下次 reindex 或 rebuild 时自动补全（MD 是真相源，文件里的
/// 行还在）。
pub fn reindex_entry(&self, id: &WikiId) -> Result<()>;

/// 全量重建：扫描 wiki/ → 事务重建全部派生表（全部由 MD 派生，无例外）→
/// 重建 index.md。embeddings 按 (entry_id, content_hash) 匹配保留：
/// hash 一致保留向量，不一致置 stale（V1"按 entry_id 盲目保留陈旧向量"的
/// 缺陷已修复）
pub fn rebuild(&self) -> Result<()>;
```

一致性模型：MD 成功 + DB 失败 → 孤立文件，由 `rebuild` 自愈；
DB 成功 + 日志失败 → 不影响正确性。**不存在需要分布式事务的场景。**

### 6.3 embedding 新鲜度（content_hash 防护）

```sql
-- embeddings 表增加：
content_hash TEXT NOT NULL DEFAULT ''   -- sha256(标题 + strip(body))，hex 前 16 位
```

- **写时刷新**：注入 provider 后，**所有**改写正文的写操作（create / merge /
  upsert / edit）在 reindex 后异步重算向量；无 provider 时不计算
- **陈旧检测**：任何读路径发现 `embeddings.content_hash ≠ 当前 hash` →
  视为 stale：semantic/hybrid 检索跳过该条目（不返回错误），日志 warning
- **rebuild**：hash 一致保留，不一致置 stale（不清空，provider 可按需重算）

---

## 7. 检索层（search）

```rust
pub struct QueryParams {
    pub query: String,
    pub wiki_type: Option<WikiType>,
    pub method: Method,            // Keyword | Semantic | Hybrid（默认 Hybrid）
    pub top_k: usize,              // 默认 10
    pub include_content: bool,     // 命中后读文件取正文
    pub expand: u8,                // ★ 关系邻域扩展深度，默认 0，上限 2
}
```

路径顺序：

1. **ID 直查**（score 1.0）：`wiki-xxx` 精确 / `ref-xxx` → 查
   `entry_sources` / 标题精确匹配（NOCASE）
2. **keyword**：FTS5 trigram + bm25 负分归一化 [0,1]；所有词 <3 字符 →
   回退 LIKE（固定 0.5）：**同时匹配标题与 FTS 的 strip 后正文**
   （`entries_fts` 列上做全表 LIKE 扫描，桌面规模可接受）。
   此为中文 2 字高频词（"模型""学习"）的兜底路径——trigram 对 <3 字符
   无 token，没有这条回退则正文检索对 2 字词失明。可选演进：CJK 预分词
   入 FTS（新 feature，不进 V2）
3. **semantic**（需注入 embed provider 且库内有非 stale 向量）：余弦相似度；
   不可用自动降级 keyword
4. **hybrid**：动态权重 kw∈[0.7,0.9]（随 kw 分线性上升），sem 补足

**邻域扩展**（`expand ≥ 1`）：命中集沿 `entry_relations` 做 BFS（深度 ≤ 2），
邻居按父分 × 0.8^(跳数) 计分并入结果集，去重后统一截断 top_k。
不做 PageRank / 重排模型（明确排除，防过度设计）。

```rust
/// 按来源反查其贡献过的全部条目（走 idx_sources_ref）
pub fn entries_by_source(&self, src: &SourceId) -> Result<Vec<EntryMeta>>;
```

---

## 8. 操作层（ops）

### 8.1 写操作

| 操作 | 语义 |
|---|---|
| `create(type, title, body, relations, source?, authors?)` | `source` / `authors` 仅为**参数**（宿主便捷入口），不落 frontmatter：summary 传入 `source` 时 SDK 自动将 body 包裹为 `<ref-xxx>`（已包裹则校验一致）；`authors` 转写为 `作者 :: [[wikiId]]` 关系。**写时校验（见下）**。去重：summary 按派生来源（`entry_sources`）；concept/entity 按 title(NOCASE)+type。未命中 → 建条目；命中 → `merge` |
| `merge(id, body, relations)` | relations 去重追加；正文：带 `<ref-s>` 包裹且 s 不在现有正文 → 子树追加（异来源安全共存）；裸内容 → 追加 + lint 警告。**summary 目标条目写时强校验**：body 中出现的所有 SourceId 必须 ⊆ 该 summary 现有来源集（裸内容或同源），否则返回 `KbError::Invalid`——单源约束在写时拒绝，不让违法状态落盘（lint 仅作外部编辑的后备防线） |
| `upsert_from_source(id, src, new_body)` | **重导入核心原语**：`remove_source(旧 s)` → `replace_source(新 s)` → 写文件 → reindex → 重建向量。同来源替换、异来源追加、跨来源零干扰 |
| `edit(id, ops: Vec<EditOp>)` | 见 8.2；逐项报告成败，整体仍提交 |
| `rename(id, new_title)` | 标题变更：文件原子重命名（§3.3）+ DB `file_path`/`title` 更新 + reindex + 同步 index.md |
| `delete(id)` | 删 MD + DB 级联 + 清理他条目中对本条目的引用 + 清悬空关系 + 同步 index.md。**引用清理范围限定**：仅 `## 关联页面` 区内删除整行；正文中出现的 `[[…该 id…]]` 链接只摘除链接 token 本身、保留所在句子（整行删除对散文是破坏性的） |
| `delete_source(src)` | `entries_by_source` 反查 → 各条目 `remove_source` → 摘除后正文为空且无关系的条目**自动删除**（记日志）→ 来源恰为 src 的 summary 随之整体删除 |
| `prune()` | **悬空引用修复原语**：全库扫描各条目文件，清除指向不存在 WikiId 的引用——关联区内的悬空行整行删除，正文中的悬空 `[[…]]` 只摘链接 token（复用 D24 机制）；随后逐条目 reindex。返回修复报告（条目 × 清除数）。与 lint 的悬空检测构成"检测（lint）→ 修复（prune）"闭环：reindex/rebuild 对悬空行只跳过不修复，落盘清理由 prune 承担。日志动词 `prune` |
| `lint()` | 只读体检，见 8.3 |

### 8.2 编辑原语

```rust
pub enum EditOp {
    /// 作用域内查找替换：scope=None 全文；Some(src) 仅在该来源子树内
    SearchReplace { search: String, replace: String, scope: Option<SourceId> },
    /// 锚点后插入（自动并入所在来源作用域）
    InsertAfter { anchor: String, content: String, scope: Option<SourceId> },
    /// 整子树替换（upsert 的操作化形态）
    ReplaceSource { src: SourceId, content: String },
}
```

`scope` 解决 V1 缺陷：search 串跨标签边界必然失败的问题。

**多重匹配语义**（写死，防实施分歧）：`SearchReplace` 与 `InsertAfter` 在
锚点/搜索串命中多处时一律取**作用域内首个**匹配（与 V1 `replacen(1)` 一致）；
替换/插入不传播到后续匹配。需要全量替换时由调用方循环调用并检查返回值。

### 8.3 lint（只读体检）

| 检查项 | 级别 |
|---|---|
| 未闭合溯源标签（T1 被触发） | warning |
| 正文 `[[…]]` 指向不存在的 WikiId | error |
| `entry_relations` 指向不存在的条目 | error |
| 来源 SourceId 不存在于宿主来源清单（由宿主注入校验回调，可选） | warning |
| summary 正文未恰好包裹一个 `<ref-xxx>`（0 个或多个） | **error**（来源唯一表示，必须成立） |
| `[[…]]` 链接的标题部分与目标条目当前标题不一致（rename 的连带效应） | warning |
| DB 与 MD 派生不一致（title/relations；悬空行被跳过属预期状态，不计入） | error |

输出 `Vec<LintIssue>`；宿主可将其暴露为工具/命令。构建流水线收尾自动执行一次。

---

## 9. SDK 接口设计

### 9.1 配置与句柄

```rust
/// 同步句柄（feature `index`，默认）
let kb = KbBuilder::new("/path/to/references")     // 根路径，唯一必填项
    .with_id_generator(my_gen)                     // 可选：自定义 ID 生成
    .with_source_validator(|id| my_sources.exists(id)) // 可选：lint 用来源校验
    .open()?;                                      // 不存在则初始化目录结构

kb.ops().create(...)?;
kb.ops().upsert_from_source(...)?;
kb.search().query(QueryParams { expand: 1, .. })?;
kb.index().rebuild()?;
kb.embed().attach(Arc::new(my_provider))?;         // 可选：向量能力

/// 异步句柄（feature `async`）：spawn_blocking 包装，Send+Sync+Clone
let akb: AsyncKb = kb.into_async();
```

### 9.2 接口纪律

- **一处一能力**：`ops`（写）、`search`（读）、`index`（派生与重建）、`embed`（向量）
  职责互不重叠；所有 API 无 panic、无隐藏 IO（除显式读写路径）
- **错误**：单一 `KbError`（thiserror），细分 `NotFound / Invalid / Io / Db /
  Embedding / Cancelled`；`KbResult<T>` 别名
- **线程模型**：`Kb` 内部 `Arc<Mutex<Connection>>`；单进程内并发安全；
  跨进程依赖 SQLite WAL + busy_timeout，不做文件锁
- **版本化**：DB 用 `user_version`；文件格式靠 round-trip 原则向前兼容，
  不引入格式版本号

### 9.3 feature 矩阵

| feature | 引入依赖 | 内容 |
|---|---|---|
| （默认）`index` | rusqlite, serde | store + ops + index + search |
| `async` | tokio | `AsyncKb` 句柄 |
| `mcp-server` | rmcp | MCP stdio server（工具面见 §9.4） |

`cargo check --no-default-features` 仅编译 L0/L1（syntax + model + store），
零第三方依赖。

### 9.4 MCP 工具面（`mcp-server`）

MCP server 将方法一一映射为工具，stdio 传输：

`knowledge_query` / `knowledge_query_batch` / `knowledge_create_entry` /
`knowledge_edit_entry` / `knowledge_get_entry` / `knowledge_list_entries` /
`knowledge_meta`（仅 Overview/Recent，Tags 查询删除）/ `knowledge_delete_entry`、
外加 `knowledge_lint` 与 `knowledge_prune`。`create_entry` 的 `content` schema
说明强制溯源包裹要求；`edit_entry` 暴露 scope/ReplaceSource。

宿主若要将同一组工具注册进 agent 框架（如 referee-ai 的 ToolRegistry），
在宿主侧写薄适配（映射方法 → Tool trait，数十行），SDK 不含该层。

---

## 10. 宿主接入指南（以 Fluen 为例）

### 10.1 构建流水线（knowledge_builder）适配点

| 项 | 改动 |
|---|---|
| **工具适配** | src-tauri 侧新建 `kb_tools.rs` 薄适配（~百行）：将 `Kb` 方法包装为 referee-ai `Tool` 并注册 ToolRegistry（替代旧 `KnowledgeToolProvider`）；工具白名单仍为 query/query_batch/create_entry/edit_entry/get_entry 五个 |
| prompts | 创建类 prompt：产出正文必须整体包裹 `<ref-当前文献 SourceId>`；关联用类型化语法或留空 |
| merge 路径 | 命中已有条目 → `upsert_from_source` 语义（同来源替换 / 异来源追加） |
| 收尾 | 流水线完成前跑 `lint()`，有 error 级问题标记任务失败原因 |
| IndexSnapshot | 删除对 index.md 的解析依赖，改用 `list_entries()` 构建快照 |
| 事件 | `kb-build:failed` 等宿主事件机制不变 |

### 10.2 前端适配点

- markdown-it 溯源插件：`<ref-xxx>` → 溯源标注（角标/底色），点击跳转来源；
  （标签为合法 custom element，html:true 下自然透传）
- CodeMirror 半预览：源码视图原样显示；块渲染插件忽略未知元素
- wiki explorer：移除 tags UI；新增"按来源筛选"（`entries_by_source`）
- 关系展示可按 predicate 分组

### 10.3 数据迁移（存量）

**`migrate()`：一次性显式迁移操作**（幂等，可重复执行；`rebuild` 不改写条目
文件，因此文件级转换由 `migrate` 独立承担）。执行顺序：先 `migrate()` 再
`rebuild()`。

`migrate()` 对每个条目文件：

1. frontmatter 含 `source` 的 summary → 将正文包裹为 `<ref-该值>`（已包裹则
   校验一致），移除该 frontmatter 键
2. frontmatter 含 `authors` 的 summary → 转写为 `作者 :: [[wikiId]]` 关系行
   （追加进关联区），移除该 frontmatter 键
3. frontmatter 含 `tags` → 移除该键（标签数据弃用，决策 D6）
4. `raw/ref-xxx.pdf` 形式的 source 值 → 归一为 `ref-xxx`
5. 原子写回（round-trip 保证其余未知键不丢）；幂等性来自"转换后键已移除，
   二次执行为 no-op"

| 项 | 策略 |
|---|---|
| 存量正文（无溯源标记） | 不迁移，语义即"未溯源"；不做 AI 回填 |
| frontmatter 旧键（`tags`/`source`/`authors`） | 由 `migrate()` 按上述规则转换；未跑 migrate 前解析忽略（进 extra 键），不报错 |
| index.db | 删除重建（schema v2）；embeddings 按 content_hash 判断存活（见 §6.1） |
| 旧 crate `fluen-knowledge` | P6 完成后整体删除（子仓库操作，需授权） |

---

## 11. 实施阶段

| 阶段 | 内容 | 验收 |
|---|---|---|
| P1 语法层 | `syntax` 全 API + SourceId | §3.2.3 全部容错用例 + roundtrip 测试通过；**§3.2.6 归属集剪枝用例通过**（嵌套反例）；`--no-default-features` 零三方依赖 |
| P2 存储+索引 | store（含 round-trip）/ model / schema v2（含 content_hash）/ rebuild | 金样本 rebuild 幂等；FTS content 已剥标签；未知 frontmatter 键 round-trip 测试通过；2 字中文查询 LIKE 回退命中正文 |
| P3 操作层 | ops（含 rename / migrate / prune）+ log.md + lint | 同 ref 重导入=替换、异 ref=追加、删 ref=归属集剪枝且共同归属内容无损；summary 单源写时拒绝；migrate 幂等；**悬空关联行 reindex 跳过不失败，prune 清除后 lint 归零**；log 追加正确 |
| P4 检索+embed | search（含 expand）/ embed trait + stale 检测 | 与 V1 检索语义对齐；entries_by_source、expand 深度/衰减测试通过；hash 不符的向量被跳过 |
| P5 上游切换 | src-tauri 换依赖 + builder prompts + 去 IndexSnapshot | 流水线 mock 端到端跑通含溯源产出；现有测试全绿 |
| P6 前端+收尾 | markdown-it 插件、UI 去 tags、按来源筛选、删旧 crate | 全项目 `rg "fluen-knowledge"` 为空 |

P1–P4 期间主应用继续运行旧 crate；新旧并存，P5 一次性切换。

---

## 12. 已定决策记录

| # | 决策 | 结论 |
|---|---|---|
| D1 | 来源 ID | **SourceId = ref-xxx**（文献 ID，稳定锚点） |
| D2 | crate | **新建 `crates/fluen-kb`**，旧库冻结至 P6 删除（需子仓库授权） |
| D3 | 删来源后空条目 | **自动删除** + 日志记录 |
| D4 | AI 裸内容 | 允许写入 + lint warning；prompt 端严格约束 |
| D5 | 来源/作者的存储位置 | **不设 frontmatter `source`/`authors` 字段**：来源 = summary 正文包裹标签（派生至 `entry_sources`）；作者 = `作者 ::` 类型化关系。`create` 保留二者为参数（SDK 自动写入包裹/关系）。frontmatter 三类条目同构 |
| D6 | tags | 彻底删除（表、字段、工具参数、meta 分支、UI） |
| D7 | 归属语义 | 开放标签栈并集（§3.2.2） |
| D8 | 类型化关系 | 采纳：`predicate :: [[link]]`，DB 加 predicate 列，缺省 related |
| D9 | 三层可变性 + log.md | 采纳：来源不可变 / 条目维护 / 索引日志派生追加 |
| D10 | 未知字段 | 采纳 round-trip（MUST 级保证，§3.1.2 / §0.2 原则 4） |
| D11 | 图邻域扩展 | 采纳**收窄版**：`expand ≤ 2`、简单衰减、不做图算法/重排 |
| D12 | lint | 采纳：七类检查（§8.3），构建收尾自动执行 |
| D13 | 双时态/社区摘要/认识论语域/候选审核流 | **暂不采纳**；标签属性语法（T6）为时态演进预留口子 |
| D14 | 三分类 | 保持固定（宿主要求）；`type` 为字符串不阻碍未来扩展 |
| D15 | SDK 范围 | **止步于方法 + MCP server**（socstat/socstat-mcp 模式）；agent 框架适配由宿主侧薄适配承担 |
| D16 | 来源移除语义 | **按归属集剪枝**（§3.2.6）：独属删除、共同归属摘边降归属、无关保留——修复嵌套下"跨来源零干扰"矛盾 |
| D17 | 关联区权威模型 | **文件为准**：语法有效行全部导入派生，无效行 lint 警告；"独占/剥离"仅针对 create 的 body 入参去重 |
| D18 | 存量迁移 | **显式 `migrate()`**（幂等，先 migrate 再 rebuild）；rebuild 不改写条目文件 |
| D19 | 检索 2 字盲区 | LIKE 回退扩展到标题 + strip 后正文；CJK 预分词留作可选演进 |
| D20 | 向量新鲜度 | embeddings 加 `content_hash`；写时刷新 + 读时跳 stale + rebuild 按 hash 保留 |
| D21 | summary 单源 | **写时强校验**（KbError::Invalid），lint 降为外部编辑后备 |
| D22 | EditOp 多重匹配 | 作用域内首个匹配，不传播；全量替换由调用方循环 |
| D23 | log.md 定位 | 审计日志（非 undo）；内容恢复依赖宿主 VCS；轮转命名 `log-{YYYYMM}-{n}.md` |
| D24 | 引用清理范围 | delete 仅在关联区删整行；正文只摘链接 token |
| D25 | 悬空引用治理 | 导入过滤（reindex/rebuild 跳过 `to_id ∉ entries` 的关联行 + lint issue，绝不失败）+ **`prune()` 修复原语**（落盘清除，检测/修复闭环）。rebuild 必须永远可运行 |
