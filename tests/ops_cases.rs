#![cfg(feature = "index")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::{src, temp_kb, wiki};
use fluen_kb::ids::WikiType::*;
use fluen_kb::{CreateOutcome, EditOp, KbError, Predicate};

fn created(outcome: CreateOutcome) -> fluen_kb::WikiId {
    match outcome {
        CreateOutcome::Created(id) => id,
        CreateOutcome::MergedInto(id) => id,
    }
}

#[test]
fn editing_referenced_entry_preserves_incoming_relations() {
    // 回归：单条目重建时入边关系不得被 ON DELETE CASCADE 冲掉。
    let (_dir, kb) = temp_kb();
    let a = created(
        kb.ops()
            .create(Concept, "目标", "正文AAA", &[], None, &[])
            .unwrap(),
    );
    let b = created(
        kb.ops()
            .create(Concept, "引用者", "正文BBB", &[(Predicate::related(), a.clone())], None, &[])
            .unwrap(),
    );

    let rels_of = |id: &fluen_kb::WikiId| -> usize {
        kb.search()
            .list_entries(None)
            .unwrap()
            .iter()
            .find(|m| m.id == *id)
            .map(|m| m.relations.len())
            .unwrap()
    };
    assert_eq!(rels_of(&b), 1);

    // 编辑被引用方 a，不应清掉 b -> a 的入边。
    kb.ops()
        .edit(
            &a,
            vec![EditOp::SearchReplace {
                search: "AAA".into(),
                replace: "XXXXXXXX".into(),
                scope: None,
            }],
        )
        .unwrap();
    assert_eq!(rels_of(&b), 1);

    // lint 不应再报"db relations differ"。
    assert!(
        kb.ops()
            .lint()
            .unwrap()
            .iter()
            .all(|i| !matches!(i.entry, Some(ref e) if *e == b))
    );
}

#[test]
fn reimport_same_source_replaces_subtree() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    let id = created(
        kb.ops()
            .create(Concept, "概念", &format!("<{s}>第一版内容。</{s}>"), &[], None, &[])
            .unwrap(),
    );

    kb.ops()
        .upsert_from_source(&id, &s, "第二版内容。")
        .unwrap();
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.body.contains("第二版内容"));
    assert!(!doc.body.contains("第一版内容"));
}

#[test]
fn reimport_different_source_appends_safely() {
    let (_dir, kb) = temp_kb();
    let s1 = src("ref-1111111111111111");
    let s2 = src("ref-2222222222222222");
    let id = created(
        kb.ops()
            .create(Concept, "概念", &format!("<{s1}>甲内容。</{s1}>"), &[], None, &[])
            .unwrap(),
    );

    kb.ops().upsert_from_source(&id, &s2, "乙内容。").unwrap();
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.body.contains("甲内容"));
    assert!(doc.body.contains("乙内容"));
    assert_eq!(
        kb.search().list_entries(None).unwrap()[0].sources.len(),
        2
    );
}

#[test]
fn delete_source_prunes_by_attribution_and_auto_deletes_empty() {
    let (_dir, kb) = temp_kb();
    let s1 = src("ref-1111111111111111");
    let s2 = src("ref-2222222222222222");
    let id = created(
        kb.ops()
            .create(
                Concept,
                "跨来源",
                &format!("<{s1}>独属甲。<{s2}>共同乙。</{s2}></{s1}><{s2}>补充丙。</{s2}>"),
                &[],
                None,
                &[],
            )
            .unwrap(),
    );

    let report = kb.ops().delete_source(&s1).unwrap();
    assert!(report.updated.contains(&id));
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(!doc.body.contains("独属甲"));
    assert!(doc.body.contains("共同乙"));
    assert!(doc.body.contains("补充丙"));

    let report = kb.ops().delete_source(&s2).unwrap();
    assert!(report.removed.contains(&id), "摘除后为空应自动删除");
    assert!(kb.search().get_entry(&id).is_err());
}

/// 回归：摘空自动删除与 delete() 同路径——其它条目对其的引用须一并清理。
#[test]
fn delete_source_auto_delete_cleans_references() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    let victim = created(
        kb.ops()
            .create(Concept, "受害者", &format!("<{s}>仅此一句。</{s}>"), &[], None, &[])
            .unwrap(),
    );
    let observer = created(
        kb.ops()
            .create(
                Concept,
                "观察者",
                &format!("正文提及 [[concepts/{victim}-受害者]] 结束。"),
                &[(Predicate::related(), victim.clone())],
                None,
                &[],
            )
            .unwrap(),
    );

    let report = kb.ops().delete_source(&s).unwrap();
    assert!(report.removed.contains(&victim), "摘空条目应自动删除");

    let doc = kb.search().get_entry(&observer).unwrap();
    assert!(doc.relations.is_empty(), "悬空关系行应被清理");
    assert!(
        !doc.body.contains(&victim.to_string()),
        "悬空正文链接应被清理"
    );
}

#[test]
fn summary_stays_single_source_enforced_at_write() {
    let (_dir, kb) = temp_kb();
    let s1 = src("ref-1111111111111111");
    let s2 = src("ref-2222222222222222");
    let id = created(
        kb.ops()
            .create(Summary, "综述", &format!("<{s1}>综述内容</{s1}>"), &[], Some(&s1), &[])
            .unwrap(),
    );

    let err = kb
        .ops()
        .merge(&id, &format!("<{s2}>异来源内容</{s2}>"), &[])
        .unwrap_err();
    assert!(matches!(err, KbError::Invalid(_)));

    // 同一 summary 塞入两个来源 → 拒绝
    let err = kb
        .ops()
        .create(
            Summary,
            "多源综述",
            &format!("<{s1}>甲</{s1}><{s2}>乙</{s2}>"),
            &[],
            None,
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, KbError::Invalid(_)));

    // source 参数与正文包裹不一致 → 拒绝
    let err = kb
        .ops()
        .create(
            Summary,
            "错配综述",
            &format!("<{s2}>内容</{s2}>"),
            &[],
            Some(&s1),
            &[],
        )
        .unwrap_err();
    assert!(matches!(err, KbError::Invalid(_)));

    // 无来源的 summary → 拒绝
    let err = kb
        .ops()
        .create(Summary, "裸综述", "没有任何包裹的正文", &[], None, &[])
        .unwrap_err();
    assert!(matches!(err, KbError::Invalid(_)));
}

#[test]
fn create_summary_deduplicates_by_source_and_merges() {
    let (_dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    let first = created(
        kb.ops()
            .create(Summary, "综述一", &format!("<{s}>第一部分</{s}>"), &[], Some(&s), &[])
            .unwrap(),
    );
    let second = kb
        .ops()
        .create(Summary, "综述二", "第二部分", &[], Some(&s), &[])
        .unwrap();
    assert_eq!(
        match second {
            CreateOutcome::MergedInto(id) => id,
            CreateOutcome::Created(_) => panic!("expected merge"),
        },
        first
    );
    let doc = kb.search().get_entry(&first).unwrap();
    assert!(doc.body.contains("第一部分"));
    assert!(doc.body.contains("第二部分"));
}

#[test]
fn concept_deduplicates_by_title_nocase() {
    let (_dir, kb) = temp_kb();
    let a = created(
        kb.ops().create(Concept, "Knowledge Tracing", "内容", &[], None, &[]).unwrap(),
    );
    let b = kb.ops().create(Concept, "knowledge tracing", "更多", &[], None, &[]).unwrap();
    match b {
        CreateOutcome::MergedInto(id) => assert_eq!(id, a),
        CreateOutcome::Created(_) => panic!("expected merge"),
    }
}

#[test]
fn migrate_is_idempotent() {
    let (dir, kb) = temp_kb();
    let id = wiki("wiki-91f2c43bf4de401e");
    let legacy = "---\ntitle: 旧综述\ntype: summary\ncreated: 2020-01-01T00:00:00+00:00\nupdated: 2020-01-01T00:00:00+00:00\nsource: raw/ref-1111111111111111.pdf\nauthors: [\"wiki-aaaaaaaaaaaaaaaa\"]\ntags: [ml]\n---\n\n存量正文。\n";
    std::fs::write(
        dir.path().join("wiki/summaries/").join(format!("{id}-旧综述.md")),
        legacy,
    )
    .unwrap();

    let report = kb.ops().migrate().unwrap();
    assert_eq!(report.wrapped, 1);
    assert_eq!(report.authors_converted, 1);
    assert_eq!(report.tags_removed, 1);

    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.body.contains("<ref-1111111111111111>"));
    assert!(doc.body.contains("存量正文"));
    assert!(doc.extra.is_empty());
    assert!(doc
        .relation_lines
        .iter()
        .any(|l| l.contains("作者 :: [[wiki-aaaaaaaaaaaaaaaa]]")));

    let second = kb.ops().migrate().unwrap();
    assert_eq!(second.wrapped, 0);
    assert_eq!(second.authors_converted, 0);
    assert_eq!(second.tags_removed, 0);

    // 原子写回不丢其余未知键
    let third = kb.ops().migrate().unwrap();
    assert_eq!(third, second);
}

#[test]
fn dangling_relation_is_skipped_then_pruned_then_lint_clears() {
    let (_dir, kb) = temp_kb();
    let ghost = wiki("wiki-deadbeefdeadbeef");
    let id = created(
        kb.ops()
            .create(
                Concept,
                "带悬空关联",
                "正文。",
                &[(Predicate::new("应用了").unwrap(), ghost)],
                None,
                &[],
            )
            .unwrap(),
    );

    // 文件中悬空行存在，DB 导入被跳过（reindex 不失败）
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.relation_lines.len() == 1);
    let meta = kb.search().list_entries(None).unwrap().into_iter().find(|m| m.id == id).unwrap();
    assert!(meta.relations.is_empty(), "悬空关联不得入 DB");

    // rebuild 永远可运行
    kb.index().rebuild().unwrap();

    // lint 报 error
    let issues = kb.ops().lint().unwrap();
    assert!(issues
        .iter()
        .any(|i| i.entry.as_ref() == Some(&id) && i.message.contains("missing entry")));

    // prune 落盘清除
    let report = kb.ops().prune().unwrap();
    assert_eq!(report.entries, vec![(id.clone(), 1)]);

    let issues = kb.ops().lint().unwrap();
    assert!(issues
        .iter()
        .all(|i| !i.message.contains("missing entry")), "悬空引用应归零");
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.relation_lines.is_empty());
}

#[test]
fn edit_takes_first_match_and_does_not_propagate() {
    let (_dir, kb) = temp_kb();
    let id = created(
        kb.ops().create(Concept, "条目", "甲 old 乙 old 丙 old", &[], None, &[]).unwrap(),
    );
    kb.ops()
        .edit(
            &id,
            vec![EditOp::SearchReplace {
                search: "old".into(),
                replace: "new".into(),
                scope: None,
            }],
        )
        .unwrap();
    let doc = kb.search().get_entry(&id).unwrap();
    assert_eq!(doc.body, "甲 new 乙 old 丙 old");
}

#[test]
fn edit_scope_search_crosses_tag_boundaries() {
    let (_dir, kb) = temp_kb();
    let s1 = src("ref-1111111111111111");
    let s2 = src("ref-2222222222222222");
    let id = created(
        kb.ops()
            .create(
                Concept,
                "作用域",
                &format!("<{s1}>甲<{s2}>mid</{s2}>乙</{s1}>"),
                &[],
                None,
                &[],
            )
            .unwrap(),
    );

    // 裸文本查找 "甲mid乙" 必失败（标签穿插）；scope 内拼接后可命中
    let results = kb
        .ops()
        .edit(
            &id,
            vec![EditOp::SearchReplace {
                search: "甲mid乙".into(),
                replace: "整体替换".into(),
                scope: Some(s1.clone()),
            }],
        )
        .unwrap();
    assert!(results[0].is_ok());
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.body.contains("整体替换"));
    assert!(!doc.body.contains("甲") || !doc.body.contains("mid"));
}

#[test]
fn edit_reports_failures_but_commits_successes() {
    let (_dir, kb) = temp_kb();
    let id = created(
        kb.ops().create(Concept, "条目", "正文目标。", &[], None, &[]).unwrap(),
    );
    let results = kb
        .ops()
        .edit(
            &id,
            vec![
                EditOp::SearchReplace {
                    search: "正文".into(),
                    replace: "改写".into(),
                    scope: None,
                },
                EditOp::InsertAfter {
                    anchor: "不存在的锚点".into(),
                    content: "x".into(),
                    scope: None,
                },
            ],
        )
        .unwrap();
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
    let doc = kb.search().get_entry(&id).unwrap();
    assert!(doc.body.contains("改写"));
}

#[test]
fn delete_cleans_references_in_other_entries() {
    let (_dir, kb) = temp_kb();
    let target = created(
        kb.ops().create(Entity, "张三", "人物。", &[], None, &[]).unwrap(),
    );
    let other = created(
        kb.ops()
            .create(
                Concept,
                "条目",
                &format!("参见 [[{target}]] 的论述。"),
                &[(Predicate::related(), target.clone())],
                None,
                &[],
            )
            .unwrap(),
    );

    kb.ops().delete(&target).unwrap();
    assert!(kb.search().get_entry(&target).is_err());

    let doc = kb.search().get_entry(&other).unwrap();
    assert!(doc.relation_lines.is_empty());
    assert!(!doc.body.contains("[["));
    assert!(doc.body.contains("参见"), "正文句子必须保留：{}", doc.body);
}

#[test]
fn rename_moves_file_and_updates_index() {
    let (dir, kb) = temp_kb();
    let id = created(
        kb.ops().create(Concept, "旧标题", "正文。", &[], None, &[]).unwrap(),
    );
    kb.ops().rename(&id, "新标题").unwrap();

    let meta = kb.search().list_entries(None).unwrap().remove(0);
    assert_eq!(meta.title, "新标题");
    assert!(meta.file_path.contains("新标题"));
    assert!(!meta.file_path.contains("旧标题"));

    let files: Vec<_> = std::fs::read_dir(dir.path().join("wiki/concepts"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 1, "旧文件必须删除");
    assert!(files[0].contains("新标题"));
}

#[test]
fn operations_are_logged() {
    let (dir, kb) = temp_kb();
    let s = src("ref-1111111111111111");
    let id = created(
        kb.ops()
            .create(Summary, "日志", &format!("<{s}>内容</{s}>"), &[], Some(&s), &[])
            .unwrap(),
    );
    kb.ops().rename(&id, "日志二").unwrap();
    kb.ops().delete(&id).unwrap();

    let log = std::fs::read_to_string(dir.path().join("wiki/log.md")).unwrap();
    assert!(log.contains("] create | "));
    assert!(log.contains("] rename | "));
    assert!(log.contains("] delete | "));
}

#[test]
fn rebuild_survives_duplicate_id_files_and_lint_reports() {
    let (dir, kb) = temp_kb();
    let id = created(
        kb.ops().create(Concept, "旧标题", "正文。", &[], None, &[]).unwrap(),
    );
    // 模拟崩溃残留：同 id 再落一份新标题文件
    std::fs::write(
        dir.path().join("wiki/concepts").join(format!("{id}-新标题.md")),
        "---\ntitle: 新标题\ntype: concept\ncreated: 2020-01-01T00:00:00+00:00\nupdated: 2020-01-01T00:00:00+00:00\n---\n\n正文二。\n",
    )
    .unwrap();

    kb.index().rebuild().unwrap(); // 不因主键冲突失败
    let issues = kb.ops().lint().unwrap();
    assert!(issues
        .iter()
        .any(|i| i.message.contains("duplicate entry files")));
}

#[test]
fn concurrent_create_same_title_yields_single_entry() {
    use std::sync::{Arc, Barrier};
    let (_dir, kb) = temp_kb();
    let kb = Arc::new(kb);
    let barrier = Arc::new(Barrier::new(2));

    let handles: Vec<_> = (0..2)
        .map(|i| {
            let kb = kb.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                kb.ops()
                    .create(Concept, "Rust", &format!("线程{i}内容"), &[], None, &[])
                    .unwrap()
            })
        })
        .collect();
    let mut created_count = 0;
    for h in handles {
        match h.join().unwrap() {
            CreateOutcome::Created(_) => created_count += 1,
            CreateOutcome::MergedInto(_) => {}
        }
    }
    assert_eq!(created_count, 1, "并发同标题 create 恰好一个 Created，另一个合并");
}

#[test]
fn rename_rejects_title_already_used() {
    let (_dir, kb) = temp_kb();
    let _a = created(kb.ops().create(Concept, "Rust", "内容A", &[], None, &[]).unwrap());
    let b = created(kb.ops().create(Concept, "Rust2", "内容B", &[], None, &[]).unwrap());

    let err = kb.ops().rename(&b, "Rust").unwrap_err();
    assert!(matches!(err, KbError::Invalid(_)));
    kb.ops().rename(&b, "Rust3").unwrap(); // 不同名不受影响
}

#[test]
fn prune_fixes_hinted_dangling_link_and_spares_code_blocks() {
    let (_dir, kb) = temp_kb();
    let ghost = wiki("wiki-deadbeefdeadbeef");
    let body = format!(
        "参见 [[{ghost}-某页面]]。\n\n```markdown\n示例：- [[{ghost}]]\n```\n"
    );
    let id = created(kb.ops().create(Concept, "教程", &body, &[], None, &[]).unwrap());

    let report = kb.ops().prune().unwrap();
    assert_eq!(report.entries, vec![(id.clone(), 1)], "带标题提示的悬空链接应被清除");

    let doc = kb.search().get_entry(&id).unwrap();
    assert!(!doc.body.contains(&format!("[[{ghost}-某页面]]")));
    assert!(doc.body.contains("某页面"), "标题提示文本保留");
    assert!(
        doc.body.contains(&format!("[[{ghost}]]")),
        "代码块内的示例不得改写"
    );
}
