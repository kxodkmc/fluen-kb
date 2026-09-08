#![cfg(feature = "index")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{src, temp_kb};
use fluen_kb::ids::WikiType::*;
use fluen_kb::{
    KnowledgeEmbedding, KbResult, Predicate, QueryParams, SearchMethod, Searcher,
};
use std::sync::Arc;

fn created(kb: &fluen_kb::Kb, wiki_type: fluen_kb::WikiType, title: &str, body: &str) -> fluen_kb::WikiId {
    match kb.ops().create(wiki_type, title, body, &[], None, &[]).unwrap() {
        fluen_kb::CreateOutcome::Created(id) => id,
        fluen_kb::CreateOutcome::MergedInto(id) => id,
    }
}

#[test]
fn stale_index_self_heals_on_query() {
    let (dir, kb) = temp_kb();
    let id = created(&kb, Concept, "被外部删除的概念", "独有内容XYZ。");

    // 外部直接删除 md 文件，未经 ops.delete
    let concepts = dir.path().join("wiki/concepts");
    let file: std::path::PathBuf = std::fs::read_dir(&concepts)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.file_name().unwrap().to_string_lossy().starts_with(id.as_str()))
        .unwrap();
    std::fs::remove_file(file).unwrap();

    let hits = kb
        .search()
        .query(&QueryParams { query: "被外部删除".into(), ..Default::default() })
        .unwrap();
    assert!(hits.is_empty(), "幽灵命中应被过滤");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "被外部删除".into(),
            include_content: true,
            ..Default::default()
        })
        .unwrap();
    assert!(hits.is_empty(), "include_content 时不得整体报错");
}

#[test]
fn like_fallback_treats_wildcards_literally() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "下划线条目", "包含 a_b 标识符。");
    created(&kb, Concept, "伪装条目", "包含 axb 标识符。");

    // "a_" 两个字符 → LIKE 回退路径；`_` 不得匹配任意单字符
    let hits = kb
        .search()
        .query(&QueryParams { query: "a_".into(), ..Default::default() })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.title, "下划线条目");
}

/// 中英混合查询：<3 字符的 CJK 短词剔除后仍应命中英文内容（报告 §四）。
#[test]
fn mixed_language_query_hits_after_dropping_short_cjk() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "Transformer", "The dominant sequence transduction models are based on self-attention.");
    created(&kb, Concept, "无关条目", "完全无关的内容。");

    let hits = kb
        .search()
        .query(&QueryParams { query: "self-attention 机制".into(), ..Default::default() })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.title, "Transformer");

    // 长 CJK 词可成 trigram，参与 AND
    let hits = kb
        .search()
        .query(&QueryParams { query: "注意力机制 attention".into(), ..Default::default() })
        .unwrap();
    assert!(hits.is_empty(), "含未出现 CJK 长词的 AND 查询应为空");
}

/// 多词查询为 AND 语义：全部词出现才命中。
#[test]
fn multi_word_query_requires_all_terms() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "甲", "包含 alpha 一词。");
    created(&kb, Concept, "乙", "同时包含 alpha 与 beta 两词。");

    let hits = kb
        .search()
        .query(&QueryParams { query: "alpha beta".into(), ..Default::default() })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.title, "乙");
}

#[test]
fn list_entries_page_paginates_in_id_order() {
    let (_dir, kb) = temp_kb();
    for i in 0..5 {
        created(&kb, Concept, &format!("条目{i}"), "正文。");
    }
    let search = kb.search();

    let page1 = search.list_entries_page(None, 2, 0).unwrap();
    let page2 = search.list_entries_page(None, 2, 2).unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page2.len(), 2);
    assert_eq!(search.count_entries(None).unwrap(), 5);
    assert_eq!(search.count_entries(Some(Concept)).unwrap(), 5);

    // 跨页无重叠、按 id 升序
    let all: Vec<_> = page1.iter().chain(page2.iter()).map(|m| m.id.clone()).collect();
    let mut sorted = all.clone();
    sorted.sort();
    assert_eq!(all, sorted);
    let id_set: std::collections::HashSet<_> = all.iter().collect();
    assert_eq!(id_set.len(), 4);

    // 越界偏移返回空页
    assert!(search.list_entries_page(None, 2, 100).unwrap().is_empty());
}

#[test]
fn recent_entries_orders_by_updated_desc() {
    let (_dir, kb) = temp_kb();
    let first = created(&kb, Concept, "先建", "正文一。");
    let second = created(&kb, Concept, "后建", "正文二。");
    // 确保时间戳不同
    std::thread::sleep(std::time::Duration::from_millis(1100));
    kb.ops().merge(&first, "追加正文。", &[]).unwrap();

    let recent = kb.search().recent_entries(10).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].id, first, "最近更新者在前");
    assert_eq!(recent[1].id, second);
    let limited = kb.search().recent_entries(1).unwrap();
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].id, first);
}

#[test]
fn count_by_type_groups_entries() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "概念", "正文。");
    created(&kb, Concept, "另一概念", "正文。");
    created(&kb, Entity, "实体", "正文。");

    let counts = kb.search().count_by_type().unwrap();
    let get = |t: fluen_kb::WikiType| counts.iter().find(|(wt, _)| *wt == t).map(|(_, n)| *n);
    assert_eq!(get(Concept), Some(2));
    assert_eq!(get(Entity), Some(1));
    assert_eq!(get(Summary), None);
}

#[test]
fn id_direct_lookup_scores_one() {
    let (_dir, kb) = temp_kb();
    let id = created(&kb, Concept, "目标条目", "正文内容。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: id.to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.id, id);
    assert!((hits[0].score - 1.0).abs() < 1e-9);
}

#[test]
fn title_exact_lookup_is_case_insensitive() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "Knowledge Tracing", "正文。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "knowledge TRACING".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn source_lookup_returns_contributing_entries() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    let a = created(&kb, Concept, "甲", &format!("<{s}>贡献内容</{s}>"));
    let b = created(&kb, Concept, "乙", &format!("<{s}>更多贡献</{s}>"));
    created(&kb, Concept, "无关", "其他内容");

    let metas = kb.search().entries_by_source(&s).unwrap();
    let ids: Vec<_> = metas.iter().map(|m| m.id.clone()).collect();
    assert!(ids.contains(&a) && ids.contains(&b));
    assert_eq!(ids.len(), 2);

    let hits = kb
        .search()
        .query(&QueryParams {
            query: s.to_string(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 2);
    assert!(hits.iter().all(|h| (h.score - 1.0).abs() < 1e-9));
}

#[test]
fn semantic_unavailable_degrades_to_keyword() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "知识图谱", "知识图谱将领域概念连成网络。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "领域概念".into(),
            method: SearchMethod::Semantic,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 1, "无 provider 时应降级 keyword");
}

/// 确定性嵌入：文本字节哈希展开为 8 维向量。
struct HashEmbed;

impl KnowledgeEmbedding for HashEmbed {
    fn embed(&self, text: &str) -> KbResult<Vec<f32>> {
        let mut v = vec![0.0_f32; 8];
        for (i, b) in text.bytes().enumerate() {
            v[i % 8] += b as f32;
        }
        Ok(v)
    }

    fn model_name(&self) -> &str {
        "hash-embed"
    }
}

#[test]
fn semantic_search_with_provider_and_stale_skip() {
    let (dir, kb) = temp_kb();
    kb.embed().attach(Arc::new(HashEmbed)).unwrap();

    let s = src("ref-1111111111111111");
    let hit = created(&kb, Concept, "向量目标", &format!("<{s}>向量目标的内容特征。</{s}>"));
    created(&kb, Concept, "另一个条目", "完全不同的正文主题。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "向量目标的内容特征".into(),
            method: SearchMethod::Semantic,
            ..Default::default()
        })
        .unwrap();
    assert!(hits
        .iter()
        .any(|h| h.meta.id == hit), "有 provider 时应命中语义检索");

    // 篡改目标条目的 content_hash 模拟陈旧向量：
    // 语义结果中该条目必须被跳过（即使关键词能匹配它），且不报错
    {
        let conn = rusqlite::Connection::open(dir.path().join("wiki/index.db")).unwrap();
        conn.execute(
            "UPDATE embeddings SET content_hash = 'deadbeefdeadbeef' WHERE entry_id = ?1",
            [hit.as_str()],
        )
        .unwrap();
    }
    let hits = kb
        .search()
        .query(&QueryParams {
            query: "向量目标的内容特征".into(),
            method: SearchMethod::Semantic,
            ..Default::default()
        })
        .unwrap();
    assert!(hits.iter().all(|h| h.meta.id != hit), "陈旧向量应被跳过");
    assert_eq!(hits.len(), 1, "新鲜向量条目仍应命中");

    // 全部陈旧 → 语义不可用 → 自动降级 keyword，不报错
    {
        let conn = rusqlite::Connection::open(dir.path().join("wiki/index.db")).unwrap();
        conn.execute("UPDATE embeddings SET content_hash = 'deadbeefdeadbeef'", [])
            .unwrap();
    }
    let hits = kb
        .search()
        .query(&QueryParams {
            query: "向量目标的内容特征".into(),
            method: SearchMethod::Semantic,
            ..Default::default()
        })
        .unwrap();
    assert!(!hits.is_empty(), "降级 keyword 后仍应返回结果");
}

#[test]
fn neighbor_expansion_decays_by_hop() {
    let (_dir, kb) = temp_kb();
    let a = created(&kb, Concept, "中心", "中心条目正文。");
    let b = match kb
        .ops()
        .create(
            Concept,
            "邻居",
            "邻居正文。",
            &[(Predicate::related(), a.clone())],
            None,
            &[],
        )
        .unwrap()
    {
        fluen_kb::CreateOutcome::Created(id) => id,
        fluen_kb::CreateOutcome::MergedInto(id) => id,
    };

    let base = kb
        .search()
        .query(&QueryParams {
            query: "中心条目正文".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(base.len(), 1);

    let expanded = kb
        .search()
        .query(&QueryParams {
            query: "中心条目正文".into(),
            expand: 1,
            ..Default::default()
        })
        .unwrap();
    let neighbor = expanded.iter().find(|h| h.meta.id == b).expect("邻居应并入");
    let center = expanded.iter().find(|h| h.meta.id == a).unwrap();
    assert!((neighbor.score - center.score * 0.8).abs() < 1e-9);
}

#[test]
fn wiki_type_filter_applies() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "概念条目", "相同关键词内容。");
    created(&kb, Entity, "实体条目", "相同关键词内容。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "相同关键词内容".into(),
            wiki_type: Some(Entity),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.wiki_type, Entity);
}

#[test]
fn include_content_loads_body() {
    let (_dir, kb) = temp_kb();
    created(&kb, Concept, "条目", "独特的正文标记。");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "独特的正文标记".into(),
            include_content: true,
            ..Default::default()
        })
        .unwrap();
    assert!(hits[0].content.as_deref().unwrap().contains("独特的正文标记"));
}

#[test]
fn query_validation_rejects_bad_params() {
    let (_dir, kb) = temp_kb();
    let searcher: Searcher = kb.search();
    assert!(searcher
        .query(&QueryParams {
            query: "  ".into(),
            ..Default::default()
        })
        .is_err());
    assert!(searcher
        .query(&QueryParams {
            query: "词".into(),
            expand: 3,
            ..Default::default()
        })
        .is_err());
}
