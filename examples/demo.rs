//! fluen-kb 端到端演示：建库 → 建条目 → 检索 → lint → prune → 全量重建。
//! 默认使用临时目录，不污染工作区。

use fluen_kb::{EditOp, KbBuilder, Predicate, QueryParams, SearchMethod, SourceId, WikiType};
use std::sync::Arc;

fn main() -> fluen_kb::KbResult<()> {
    // 1. 打开（不存在则初始化）一个知识库。
    let label = "fluen-kb-demo";
    let dir = std::env::temp_dir().join(format!("{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let kb = KbBuilder::new(&dir).open()?;

    // 2. 定义一个"来源"。
    let src_a = SourceId::new("ref-aaaaaaaaaaaaaaaa")?;

    // 3. 创建一个 concept：正文用 <ref-…> 标签标注溯源。
    let a = match kb.ops().create(
        WikiType::Concept,
        "Rust 内存安全",
        "<ref-aaaaaaaaaaaaaaaa>所有权系统在编译期保证内存安全。</ref-aaaaaaaaaaaaaaaa>",
        &[],
        None,
        &[],
    )? {
        fluen_kb::CreateOutcome::Created(id) => id,
        _ => unreachable!("should create"),
    };
    // 4. 再创建一个 concept，两条相互关联（缺省谓词 related）。
    let b = match kb.ops().create(
        WikiType::Concept,
        "借用检查器",
        "<ref-bbbbbbbbbbbbbbbb>借用检查器是编译器的一部分。</ref-bbbbbbbbbbbbbbbb>",
        &[(Predicate::related(), a.clone())],
        None,
        &[],
    )? {
        fluen_kb::CreateOutcome::Created(id) => id,
        _ => unreachable!("should create"),
    };

    // 5. 编辑：search_replace 原语。
    let edit_results = kb.ops().edit(
        &a,
        vec![EditOp::SearchReplace {
            search: "内存安全".to_string(),
            replace: "内存与线程安全".to_string(),
            scope: None,
        }],
    )?;
    println!("edit applied: {}", edit_results.iter().all(|r| r.is_ok()));

    // 6. 关键词检索。
    let hits = kb.search().query(&QueryParams {
        query: "借用检查器".to_string(),
        method: SearchMethod::Keyword,
        ..Default::default()
    })?;
    println!("keyword hits: {}", hits.iter().map(|h| h.meta.title.as_str()).collect::<Vec<_>>().join(", "));

    // 7. 来源反查：src_a 贡献了哪些条目。
    let by_src = kb.search().entries_by_source(&src_a)?;
    println!("entries from src_a: {}", by_src.len());
    kb.embed().attach(Arc::new(DummyEmbed))?;
    kb.embed().refresh_all()?;
    let sem = kb.search().query(&QueryParams {
        query: "编译器 安全".to_string(),
        method: SearchMethod::Hybrid,
        expand: 1,
        include_content: true,
        ..Default::default()
    })?;
    println!("hybrid/expanded hits: {}", sem.len());

    // 9. lint 体检 + prune（此刻应无悬空引用）。
    println!("lint issues: {}", kb.ops().lint()?.len());
    println!("pruned entries: {}", kb.ops().prune()?.entries.len());

    // 10. 展示落盘的 MD 结构。
    let doc = kb.search().get_entry(&a)?;
    println!(
        "md file: wiki/{}/{}-{}.md",
        doc.wiki_type.dir(),
        doc.id,
        doc.title
    );

    // 11. rename：变更标题触发文件原子重命名 + 派生刷新。
    kb.ops().rename(&b, &"借用检查器（Borrow Checker）".to_string())?;

    // 12. 全量重建：关闭句柄后删除 index.db，仅凭 MD 恢复全部派生数据。
    drop(kb);
    for suffix in ["index.db", "index.db-wal", "index.db-shm"] {
        let _ = std::fs::remove_file(dir.join("wiki").join(suffix));
    }
    let kb2 = KbBuilder::new(&dir).open()?;
    kb2.index().rebuild()?;
    let after = kb2.search().list_entries(None)?;
    println!("after deleting index.db and rebuilding: {}", after.len());

    println!("demo ok @ {}", dir.display());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// 固定向量的假嵌入 provider，仅用于演示语义检索链路。
struct DummyEmbed;
impl fluen_kb::KnowledgeEmbedding for DummyEmbed {
    fn embed(&self, _text: &str) -> fluen_kb::KbResult<Vec<f32>> {
        // 内容无关的伪向量：保证维度一致即可。
        Ok(vec![0.1, 0.2, 0.3, 0.1, 0.4, 0.2])
    }
    fn model_name(&self) -> &str {
        "dummy"
    }
}