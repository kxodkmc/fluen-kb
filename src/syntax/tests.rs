use crate::ids::SourceId;
use crate::syntax::{
    collect_sources, filter_by_source, normalize, parse, parse_detailed, remove_source,
    replace_source, strip,
};

fn src(v: &str) -> SourceId {
    SourceId::new(v).unwrap()
}

fn sources(body: &str) -> Vec<String> {
    collect_sources(body).iter().map(|s| s.to_string()).collect()
}

fn attr(body: &str, needle: &str) -> Vec<Vec<String>> {
    parse(body)
        .spans
        .iter()
        .filter(|s| s.text.contains(needle))
        .map(|s| s.sources.iter().map(|x| x.to_string()).collect())
        .collect()
}

#[test]
fn attribution_union_semantics() {
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let b = src("ref-bbbbbbbbbbbbbbbb");

    assert_eq!(attr("<ref-aaaaaaaaaaaaaaaa>甲</ref-aaaaaaaaaaaaaaaa>", "甲"), vec![vec![a.to_string()]]);

    let nested = "<ref-aaaaaaaaaaaaaaaa>甲<ref-bbbbbbbbbbbbbbbb>乙</ref-bbbbbbbbbbbbbbbb></ref-aaaaaaaaaaaaaaaa>";
    assert_eq!(attr(nested, "甲"), vec![vec![a.to_string()]]);
    assert_eq!(attr(nested, "乙"), vec![vec![a.to_string(), b.to_string()]]);

    let mixed = "<ref-aaaaaaaaaaaaaaaa>甲<ref-bbbbbbbbbbbbbbbb>乙</ref-bbbbbbbbbbbbbbbb>丙</ref-aaaaaaaaaaaaaaaa>";
    assert_eq!(attr(mixed, "丙"), vec![vec![a.to_string()]]);
    assert_eq!(attr("无包裹", "无包裹"), vec![Vec::<String>::new()]);
}

#[test]
fn t1_unclosed_open_auto_closes_at_block_end() {
    let body = "<ref-aaaaaaaaaaaaaaaa>第一段\n仍在第一段\n\n第二段";
    let (tree, stats) = parse_detailed(body);
    assert_eq!(stats.unmatched_opens, 1);
    let first = tree.spans.iter().map(|s| s.sources.len()).sum::<usize>();
    assert!(first > 0);
    assert_eq!(attr(body, "第二段"), vec![Vec::<String>::new()]);
}

#[test]
fn t1_unclosed_in_list_item_closes_at_item_end() {
    let body = "- <ref-aaaaaaaaaaaaaaaa>项一\n- 项二";
    assert_eq!(attr(body, "项二"), vec![Vec::<String>::new()]);
    assert_eq!(attr(body, "项一"), vec![vec!["ref-aaaaaaaaaaaaaaaa"]]);
}

#[test]
fn t2_orphan_close_ignored() {
    let body = "甲</ref-aaaaaaaaaaaaaaaa>乙";
    let (tree, stats) = parse_detailed(body);
    assert_eq!(stats.orphan_closes, 1);
    assert!(tree.spans.iter().all(|s| s.sources.is_empty()));
    assert_eq!(strip(body).0, "甲乙");
}

#[test]
fn t3_malformed_tag_is_literal_text() {
    let body = "<ref-short>甲</ref-short> <not-a-ref>乙</not-a-ref>";
    let (tree, stats) = parse_detailed(body);
    assert_eq!(stats.unmatched_opens, 0);
    assert!(tree.spans.iter().all(|s| s.sources.is_empty()));
    assert_eq!(strip(body).0, body);
}

#[test]
fn long_source_ids_up_to_32_hex_are_parsed() {
    let a = "ref-35d40cf66d054561955be8dc242b0413";
    let body = format!("<{a}>甲</{a}>");
    assert_eq!(attr(&body, "甲"), vec![vec![a]]);
    assert_eq!(normalize(&body), body);
    assert_eq!(sources(&body), vec![a]);

    let long = "ref-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let body = format!("<{long}>甲</{long}>");
    let (tree, stats) = parse_detailed(&body);
    assert_eq!(stats.unmatched_opens, 0);
    assert!(tree.spans.iter().all(|s| s.sources.is_empty()));
    assert_eq!(strip(&body).0, body);
}

#[test]
fn t4_code_regions_not_parsed() {
    let body = "```\n<ref-aaaaaaaaaaaaaaaa>块内</ref-aaaaaaaaaaaaaaaa>\n```\n`<ref-aaaaaaaaaaaaaaaa>行内</ref-aaaaaaaaaaaaaaaa>`";
    let (tree, stats) = parse_detailed(body);
    assert_eq!(stats.unmatched_opens, 0);
    assert!(tree.spans.iter().all(|s| s.sources.is_empty()));
    assert_eq!(strip(body).0, body);
}

#[test]
fn t5_cross_nesting_normalized_to_tree() {
    // <s1>甲<s2>乙</s1>丙</s2>：乙={s1,s2}，丙={s2}
    let a = "ref-1111111111111111";
    let b = "ref-2222222222222222";
    let body = format!("<{a}>甲<{b}>乙</{a}>丙</{b}>");
    assert_eq!(attr(&body, "乙"), vec![vec![a, b]]);
    assert_eq!(attr(&body, "丙"), vec![vec![b]]);
    let norm = normalize(&body);
    assert_eq!(norm, format!("<{a}>甲<{b}>乙</{b}></{a}><{b}>丙</{b}>"));
    assert_eq!(attr(&norm, "乙"), attr(&body, "乙"));
    assert_eq!(attr(&norm, "丙"), attr(&body, "丙"));
}

#[test]
fn t6_open_tag_attributes_accepted_and_ignored() {
    let body = r#"<ref-aaaaaaaaaaaaaaaa lang="zh" note="x y">甲</ref-aaaaaaaaaaaaaaaa>"#;
    assert_eq!(attr(body, "甲"), vec![vec!["ref-aaaaaaaaaaaaaaaa"]]);
    assert_eq!(normalize(body), "<ref-aaaaaaaaaaaaaaaa>甲</ref-aaaaaaaaaaaaaaaa>");
}

#[test]
fn t7_empty_tags_removed_by_normalize() {
    let a = "ref-aaaaaaaaaaaaaaaa";
    let body = format!("甲<{a}></{a}>乙");
    assert_eq!(normalize(&body), "甲乙");
}

#[test]
fn t8_tag_may_span_multiple_blocks() {
    let a = "ref-aaaaaaaaaaaaaaaa";
    let body = format!("<{a}>\n第一段\n\n第二段\n</{a}>\n\n第三段");
    assert_eq!(attr(&body, "第二段"), vec![vec![a]]);
    assert_eq!(attr(&body, "第三段"), vec![Vec::<String>::new()]);
    assert_eq!(normalize(&body), body);
}

#[test]
fn hex_case_insensitive_normalized_to_lowercase() {
    let body = "<ref-AAAAAAAAAAAAAAAA>甲</ref-AAAAAAAAAAAAAAAA>";
    assert_eq!(sources(body), vec!["ref-aaaaaaaaaaaaaaaa"]);
}

#[test]
fn strip_removes_tags_and_keeps_plain_text() {
    let body = "<ref-aaaaaaaaaaaaaaaa>知识追踪。<ref-bbbbbbbbbbbbbbbb>BKT。</ref-bbbbbbbbbbbbbbbb></ref-aaaaaaaaaaaaaaaa> 通用句。";
    let (plain, map) = strip(body);
    assert_eq!(plain, "知识追踪。BKT。 通用句。");
    assert_eq!(map.len(), 3);
    assert_eq!(map[0].1.len(), 1);
    assert_eq!(map[1].1.len(), 2);
    assert!(map[2].1.is_empty());
}

#[test]
fn collect_sources_first_appearance_order() {
    let body = "<ref-bbbbbbbbbbbbbbbb>甲</ref-bbbbbbbbbbbbbbbb><ref-aaaaaaaaaaaaaaaa>乙<ref-bbbbbbbbbbbbbbbb>丙</ref-bbbbbbbbbbbbbbbb></ref-aaaaaaaaaaaaaaaa>";
    assert_eq!(sources(body), vec!["ref-bbbbbbbbbbbbbbbb", "ref-aaaaaaaaaaaaaaaa"]);
}

#[test]
fn filter_by_source_keeps_only_matching_content() {
    let body = "<ref-aaaaaaaaaaaaaaaa>独属甲</ref-aaaaaaaaaaaaaaaa><ref-bbbbbbbbbbbbbbbb>乙</ref-bbbbbbbbbbbbbbbb>";
    let out = filter_by_source(body, &src("ref-aaaaaaaaaaaaaaaa"));
    assert!(out.contains("独属甲"));
    assert!(!out.contains("乙"));
}

#[test]
fn remove_source_prunes_by_attribution_set() {
    // 设计 §3.2.6 反例
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let b = src("ref-bbbbbbbbbbbbbbbb");
    let body = format!(
        "<{a}>结论X。<{b}>细节Y。</{b}></{a}> <{b}>补充Z。</{b}>"
    );
    let (out, touched) = remove_source(&body, &a);
    assert!(touched);
    assert!(!out.contains("结论X"));
    assert!(out.contains("细节Y。"));
    assert!(out.contains("补充Z。"));
    assert_eq!(sources(&out), vec![b.to_string()]);
}

#[test]
fn remove_source_keeps_content_of_other_sources_intact() {
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let body = format!("无归属句。<{a}>仅甲。</{a}>");
    let (out, _) = remove_source(&body, &a);
    assert_eq!(out, "无归属句。");
}

#[test]
fn remove_source_on_absent_source_is_noop() {
    let body = "<ref-aaaaaaaaaaaaaaaa>甲</ref-aaaaaaaaaaaaaaaa>";
    let (out, touched) = remove_source(body, &src("ref-cccccccccccccccc"));
    assert!(!touched);
    assert_eq!(out, body);
}

#[test]
fn replace_source_swaps_subtree() {
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let body = format!("<{a}>旧内容</{a}>通用");
    let out = replace_source(&body, &a, "新内容");
    assert!(out.contains("新内容"));
    assert!(!out.contains("旧内容"));
    assert!(out.contains("通用"));
    assert_eq!(sources(&out), vec![a.to_string()]);
}

#[test]
fn replace_source_appends_when_absent() {
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let out = replace_source("通用内容", &a, "新内容");
    assert_eq!(sources(&out), vec![a.to_string()]);
    assert!(out.contains("通用内容"));
}

#[test]
fn replace_source_accepts_pre_wrapped_content() {
    let a = src("ref-aaaaaaaaaaaaaaaa");
    let wrapped = format!("<{a}>已包裹</{a}>");
    let out = replace_source("通用", &a, &wrapped);
    assert_eq!(out.matches('<').count(), 2);
    assert!(out.contains("已包裹"));
}

#[test]
fn normalize_is_idempotent() {
    let body = "<ref-aaaaaaaaaaaaaaaa>甲<ref-bbbbbbbbbbbbbbbb>乙</ref-aaaaaaaaaaaaaaaa>丙</ref-bbbbbbbbbbbbbbbb>\n\n<ref-aaaaaaaaaaaaaaaa>未闭合";
    let once = normalize(body);
    let twice = normalize(&once);
    assert_eq!(once, twice);
}

#[test]
fn normalize_lands_t1_as_explicit_close() {
    let a = "ref-aaaaaaaaaaaaaaaa";
    let body = format!("<{a}>第一段\n\n第二段");
    let norm = normalize(&body);
    assert_eq!(norm, format!("<{a}>第一段</{a}>\n\n第二段"));
}

#[test]
fn parse_rebuild_preserves_attribution() {
    let body = "<ref-aaaaaaaaaaaaaaaa>甲<ref-bbbbbbbbbbbbbbbb>乙</ref-bbbbbbbbbbbbbbbb>丙</ref-aaaaaaaaaaaaaaaa>丁";
    let rebuilt = normalize(body);
    for needle in ["甲", "乙", "丙", "丁"] {
        assert_eq!(attr(body, needle), attr(&rebuilt, needle), "{needle}");
    }
}
