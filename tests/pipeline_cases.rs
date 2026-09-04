#![cfg(feature = "index")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{db_dump, src, temp_kb, wiki};
use fluen_kb::ids::WikiType::*;
use fluen_kb::{Predicate, QueryParams};

#[test]
fn golden_rebuild_is_idempotent() {
    let (dir, kb) = temp_kb();
    seed(&kb);

    kb.index().rebuild().expect("rebuild 1");
    let first = db_dump(dir.path());
    kb.index().rebuild().expect("rebuild 2");
    let second = db_dump(dir.path());
    assert_eq!(first, second);
}

#[test]
fn deleting_index_db_and_rebuilding_restores_equivalent_state() {
    let (dir, kb) = temp_kb();
    seed(&kb);
    let before = db_dump(dir.path());
    drop(kb);

    for suffix in ["index.db", "index.db-wal", "index.db-shm"] {
        let path = dir.path().join("wiki").join(suffix);
        if path.exists() {
            std::fs::remove_file(path).expect("remove db file");
        }
    }

    let kb = fluen_kb::KbBuilder::new(dir.path()).open().expect("reopen");
    kb.index().rebuild().expect("rebuild after delete");
    let after = db_dump(dir.path());
    assert_eq!(before, after);
}

#[test]
fn fts_content_is_stripped_of_source_tags() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-3d09a8bb72c14e5f");
    kb.ops()
        .create(
            Concept,
            "深度学习",
            &format!("<{s}>深度学习模型综述内容</{s}>"),
            &[],
            None,
            &[],
        )
        .expect("create");

    // 正文可检索
    let hits = kb
        .search()
        .query(&QueryParams {
            query: "综述内容".into(),
            ..Default::default()
        })
        .expect("query");
    assert_eq!(hits.len(), 1);

    // 标签词不进索引：来源 hex 不应作为正文命中（标题与 id 均不含）
    let hits = kb
        .search()
        .query(&QueryParams {
            query: "3d09a8bb72c14e5f".into(),
            ..Default::default()
        })
        .expect("query");
    assert!(hits.is_empty());
}

#[test]
fn two_char_chinese_query_falls_back_to_like_and_hits_body() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    kb.ops()
        .create(
            Concept,
            "教育技术",
            &format!("<{s}>自适应学习系统与知识追踪模型。</{s}>"),
            &[],
            None,
            &[],
        )
        .expect("create");

    let hits = kb
        .search()
        .query(&QueryParams {
            query: "模型".into(),
            method: fluen_kb::SearchMethod::Keyword,
            ..Default::default()
        })
        .expect("query");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.title, "教育技术");
    assert!((hits[0].score - 0.5).abs() < 1e-9);
}

#[test]
fn index_md_and_log_are_derived_after_writes() {
    let (dir, kb) = temp_kb();
    let s = src("ref-2222222222222222");
    kb.ops()
        .create(Summary, "综述", &format!("<{s}>综述内容</{s}>"), &[], Some(&s), &[])
        .expect("create");

    let index_md =
        std::fs::read_to_string(dir.path().join("wiki/index.md")).expect("index.md");
    assert!(index_md.contains("## Summaries"));
    assert!(index_md.contains("[[summaries/"));
    assert!(index_md.contains("[[summaries/") || index_md.contains("- [["));

    let log = std::fs::read_to_string(dir.path().join("wiki/log.md")).expect("log.md");
    assert!(log.contains("] create | wiki-"));
    assert!(log.contains("综述"));
}

fn seed(kb: &fluen_kb::Kb) {
    let s1 = src("ref-1111111111111111");
    let s2 = src("ref-2222222222222222");
    let author = wiki("wiki-aaaaaaaaaaaaaaaa");

    let summary = kb
        .ops()
        .create(
            Summary,
            "知识追踪综述",
            &format!("<{s1}>知识追踪领域综述正文。</{s1}>"),
            &[],
            Some(&s1),
            &[author],
        )
        .expect("create summary")
        .expect_created();

    let concept = kb
        .ops()
        .create(
            Concept,
            "贝叶斯知识追踪",
            &format!("<{s1}>BKT 由 Corbett 提出。</{s1}><{s2}>扩展工作持续演进。</{s2}>"),
            &[(Predicate::new("应用了").unwrap(), summary)],
            None,
            &[],
        )
        .expect("create concept")
        .expect_created();

    kb.ops()
        .create(
            Entity,
            "Corbett",
            "人物条目，未溯源描述。",
            &[(Predicate::related(), concept)],
            None,
            &[],
        )
        .expect("create entity");
}

trait CreateOutcomeExt {
    fn expect_created(self) -> fluen_kb::WikiId;
}

impl CreateOutcomeExt for fluen_kb::CreateOutcome {
    fn expect_created(self) -> fluen_kb::WikiId {
        match self {
            fluen_kb::CreateOutcome::Created(id) => id,
            fluen_kb::CreateOutcome::MergedInto(_) => panic!("expected Created"),
        }
    }
}
