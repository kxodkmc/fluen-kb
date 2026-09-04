//! frontmatter：手写行级解析器（`key: value` 标量子集）。
//!
//! 未知键的值以原始字符串逐字节保留（MUST 级 round-trip 保证，设计 §3.1.2），
//! 因此不引入 YAML 库——其重排与转义行为会破坏该保证。

use crate::error::{KbError, KbResult};
use crate::ids::WikiType;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Frontmatter {
    pub title: String,
    pub wiki_type: WikiType,
    pub created: String,
    pub updated: String,
    pub extra: BTreeMap<String, String>,
}

/// 解析 `---\n…\n---\n` 块，返回 (frontmatter, 其后全部内容)。
pub fn parse(text: &str) -> KbResult<(Frontmatter, String)> {
    let body_start = closing_delim_end(text)
        .ok_or_else(|| KbError::invalid("entry file must start and close with `---` frontmatter"))?;

    let mut title = None;
    let mut wiki_type = None;
    let mut created = None;
    let mut updated = None;
    let mut extra = BTreeMap::new();

    for (i, line) in text[..body_start].split_inclusive('\n').enumerate() {
        let bare = line.trim_end_matches(['\n', '\r']);
        if i == 0 || bare == "---" || bare.trim().is_empty() {
            continue;
        }
        let colon = bare
            .find(':')
            .ok_or_else(|| KbError::invalid(format!("frontmatter line must be `key: value`: {bare:?}")))?;
        let key = bare[..colon].trim();
        if key.is_empty() {
            return Err(KbError::invalid("frontmatter key must not be empty"));
        }
        let raw = bare[colon + 1..].to_string();
        match key {
            "title" => title = Some(raw.trim().to_string()),
            "type" => wiki_type = Some(raw.trim().to_string()),
            "created" => created = Some(raw.trim().to_string()),
            "updated" => updated = Some(raw.trim().to_string()),
            _ => {
                extra.insert(key.to_string(), raw);
            }
        }
    }

    let title = title
        .filter(|t| !t.is_empty())
        .ok_or_else(|| KbError::invalid("frontmatter `title` is required and must be non-empty"))?;
    let wiki_type = wiki_type
        .as_deref()
        .and_then(WikiType::parse)
        .ok_or_else(|| KbError::invalid("frontmatter `type` must be one of summary|concept|entity"))?;
    let created = created
        .filter(|t| !t.is_empty())
        .ok_or_else(|| KbError::invalid("frontmatter `created` is required"))?;
    let updated = updated
        .filter(|t| !t.is_empty())
        .ok_or_else(|| KbError::invalid("frontmatter `updated` is required"))?;

    Ok((
        Frontmatter {
            title,
            wiki_type,
            created,
            updated,
            extra,
        },
        text[body_start..].to_string(),
    ))
}

pub fn serialize(fm: &Frontmatter) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("title: {}\n", fm.title));
    out.push_str(&format!("type: {}\n", fm.wiki_type.as_str()));
    out.push_str(&format!("created: {}\n", fm.created));
    out.push_str(&format!("updated: {}\n", fm.updated));
    for (k, v) in &fm.extra {
        out.push_str(&format!("{k}:{v}\n"));
    }
    out.push_str("---\n");
    out
}

/// 返回闭合 `---` 行之后的字节偏移；文件不以 frontmatter 开始时返回 None。
fn closing_delim_end(text: &str) -> Option<usize> {
    let mut pos = 0;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let bare = line.trim_end_matches(['\n', '\r']);
        if i == 0 {
            if bare != "---" {
                return None;
            }
        } else if bare == "---" {
            return Some(pos + line.len());
        }
        pos += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\ntitle: 知识追踪\ntype: concept\ncreated: 2026-09-03T12:00:00+00:00\nupdated: 2026-09-03T12:30:00+00:00\nlanguage: zh-Hans\nreviewers:  [\"a\", \"b\"]\n---\n\n正文\n";

    #[test]
    fn parse_and_serialize_round_trip() {
        let (fm, body) = parse(SAMPLE).unwrap();
        assert_eq!(fm.title, "知识追踪");
        assert_eq!(fm.wiki_type, WikiType::Concept);
        assert_eq!(fm.extra.get("language"), Some(&" zh-Hans".to_string()));
        assert_eq!(fm.extra.get("reviewers"), Some(&"  [\"a\", \"b\"]".to_string()));

        let mut text = serialize(&fm);
        text.push_str(&body);
        let (fm2, _) = parse(&text).unwrap();
        assert_eq!(fm, fm2);

        let round2 = serialize(&fm2);
        assert_eq!(text, round2 + &body);
    }

    #[test]
    fn unknown_keys_preserved_byte_exact() {
        let mut raw = String::from("---\ntitle: t\ntype: entity\ncreated: c\nupdated: u\n");
        raw.push_str("x: no-space-value\n");
        raw.push_str("y:   multiple spaces\n");
        raw.push_str("z:\"quoted\"\n");
        raw.push_str("---\nbody");
        let (fm, _) = parse(&raw).unwrap();
        assert_eq!(fm.extra.get("x"), Some(&" no-space-value".to_string()));
        assert_eq!(fm.extra.get("y"), Some(&"   multiple spaces".to_string()));
        assert_eq!(fm.extra.get("z"), Some(&"\"quoted\"".to_string()));

        let mut rewritten = serialize(&fm);
        rewritten.push_str("body");
        assert_eq!(rewritten, raw);
    }

    #[test]
    fn rejects_missing_frontmatter_and_bad_lines() {
        assert!(parse("no frontmatter").is_err());
        assert!(parse("---\ntitle: t\nno colon line\n---\n").is_err());
        assert!(parse("---\ntype: concept\ncreated: c\nupdated: u\n---\n").is_err());
        assert!(parse("---\ntitle: t\ntype: tag\ncreated: c\nupdated: u\n---\n").is_err());
        assert!(parse("---\ntitle: \ntype: concept\ncreated: c\nupdated: u\n---\n").is_err());
    }

    #[test]
    fn empty_extra_serializes_cleanly() {
        let fm = Frontmatter {
            title: "t".into(),
            wiki_type: WikiType::Summary,
            created: "c".into(),
            updated: "u".into(),
            extra: BTreeMap::new(),
        };
        assert_eq!(
            serialize(&fm),
            "---\ntitle: t\ntype: summary\ncreated: c\nupdated: u\n---\n"
        );
    }
}
