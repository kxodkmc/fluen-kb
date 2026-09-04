//! L1 存储层：条目文件 ↔ `EntryDocument`。零第三方依赖。

pub mod atomic;
pub mod filename;
pub mod frontmatter;
#[cfg(feature = "index")]
pub(crate) mod time;

use crate::error::{KbError, KbResult};
use crate::ids::{Predicate, WikiId, WikiType};
use crate::model::EntryDocument;
use std::fs;
use std::path::{Path, PathBuf};

pub const RELATIONS_HEADING: &str = "## 关联页面";

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn wiki_root(&self) -> PathBuf {
        self.root.join("wiki")
    }

    pub fn init(&self) -> KbResult<()> {
        for t in WikiType::ALL {
            fs::create_dir_all(self.dir_for(t))?;
        }
        Ok(())
    }

    pub(crate) fn dir_for(&self, wiki_type: WikiType) -> PathBuf {
        self.wiki_root().join(wiki_type.dir())
    }

    /// 返回 (绝对路径, 相对 root 的规范路径)。
    pub fn entry_path(&self, id: &WikiId, wiki_type: WikiType, title: &str) -> (PathBuf, String) {
        let filename = filename::entry_filename(id, title);
        let abs = self.dir_for(wiki_type).join(&filename);
        let rel = format!("wiki/{}/{}", wiki_type.dir(), filename);
        (abs, rel)
    }

    /// 全库扫描：三个类型目录下所有合法命名的条目文件。
    pub fn discover(&self) -> KbResult<Vec<(WikiId, PathBuf)>> {
        let mut out = Vec::new();
        for t in WikiType::ALL {
            let dir = self.dir_for(t);
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if let Some(id) = filename::id_from_filename(&name) {
                    out.push((id, entry.path()));
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    pub fn find_entry_file(&self, id: &WikiId) -> KbResult<Option<PathBuf>> {
        for t in WikiType::ALL {
            let dir = self.dir_for(t);
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == format!("{id}.md") || name.starts_with(&format!("{id}-")) {
                    return Ok(Some(entry.path()));
                }
            }
        }
        Ok(None)
    }

    pub fn load(&self, id: &WikiId) -> KbResult<EntryDocument> {
        let path = self
            .find_entry_file(id)?
            .ok_or_else(|| KbError::not_found(format!("entry file for {id}")))?;
        self.load_path(&path)
    }

    pub fn load_path(&self, path: &Path) -> KbResult<EntryDocument> {
        let text = fs::read_to_string(path)?;
        let id = path
            .file_name()
            .and_then(|n| filename::id_from_filename(&n.to_string_lossy()))
            .ok_or_else(|| KbError::invalid(format!("bad entry filename: {}", path.display())))?;

        let (fm, rest) = frontmatter::parse(&text)?;
        let (body, relation_lines, had_section) = split_relations_section(&rest);
        let mut relations = Vec::new();
        for line in &relation_lines {
            if let Some((pred, to)) = parse_relation_line(line) {
                relations.push((pred, to));
            }
        }

        Ok(EntryDocument {
            id,
            wiki_type: fm.wiki_type,
            title: fm.title,
            created: fm.created,
            updated: fm.updated,
            extra: fm.extra,
            body,
            relation_lines,
            relations,
            had_relations_section: had_section,
        })
    }

    /// 原子写入并返回相对路径。
    pub fn save(&self, doc: &EntryDocument) -> KbResult<String> {
        let fm = frontmatter::Frontmatter {
            title: doc.title.clone(),
            wiki_type: doc.wiki_type,
            created: doc.created.clone(),
            updated: doc.updated.clone(),
            extra: doc.extra.clone(),
        };
        let mut content = frontmatter::serialize(&fm);
        content.push('\n');
        let body = doc.body.trim_end_matches('\n');
        if !body.is_empty() {
            content.push_str(body);
            content.push('\n');
        }
        if doc.had_relations_section || !doc.relation_lines.is_empty() {
            content.push('\n');
            content.push_str(RELATIONS_HEADING);
            content.push('\n');
            for line in &doc.relation_lines {
                content.push_str(line.trim_end());
                content.push('\n');
            }
        }
        let (abs, rel) = self.entry_path(&doc.id, doc.wiki_type, &doc.title);
        atomic::atomic_write(&abs, &content)?;
        Ok(rel)
    }

    pub fn delete_file(&self, id: &WikiId) -> KbResult<()> {
        let path = self
            .find_entry_file(id)?
            .ok_or_else(|| KbError::not_found(format!("entry file for {id}")))?;
        fs::remove_file(path)?;
        Ok(())
    }

    pub fn rel_path(&self, abs: &Path) -> String {
        let rel = abs.strip_prefix(&self.root).unwrap_or(abs);
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// 拆出 `## 关联页面` 区：返回 (正文, 关联区原始行, 是否存在该区)。
/// 区内所有行（含语法无效行）原样保留在 `relation_lines` 中。
pub fn split_relations_section(body: &str) -> (String, Vec<String>, bool) {
    let Some(split) = find_relations_heading(body) else {
        return (body.trim_matches('\n').to_string(), Vec::new(), false);
    };
    let before = body[..split].trim_matches('\n').to_string();
    let after = &body[split..];
    let (_, tail) = after.split_once('\n').unwrap_or((after, ""));
    let lines = tail
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    (before, lines, true)
}

/// 最后一个 `## 关联页面` 标题行的起始偏移。
fn find_relations_heading(body: &str) -> Option<usize> {
    let mut found = None;
    let mut pos = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == RELATIONS_HEADING {
            found = Some(pos);
        }
        pos += line.len();
    }
    found
}

/// 解析关联行：`- [predicate ::] [[dir/]wikiId[-title]]`（设计 §3.1.3 EBNF）。
pub fn parse_relation_line(raw: &str) -> Option<(Predicate, WikiId)> {
    let line = raw.trim();
    let rest = line.strip_prefix("- ")?;
    let (pred, link) = match rest.find("::") {
        Some(i) => (
            Some(Predicate::new(rest[..i].trim()).ok()?),
            rest[i + 2..].trim(),
        ),
        None => (None, rest),
    };
    let inner = link.strip_prefix("[[")?.strip_suffix("]]")?;
    let target = inner.rsplit('/').next()?;
    if target.len() < WikiId::PREFIX.len() + 16 {
        return None;
    }
    let id = WikiId::parse(&target[..WikiId::PREFIX.len() + 16])?;
    Some((pred.unwrap_or_else(Predicate::related), id))
}

/// 生成规范关联行；`related` 谓词省略（与解析规则对称）。
pub fn relation_line(predicate: &Predicate, to: &WikiId) -> String {
    if predicate.is_related() {
        format!("- [[{to}]]")
    } else {
        format!("- {predicate} :: [[{to}]]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "wiki-91f2c43bf4de401e";
    const B: &str = "wiki-3d09a8bb72c14e5f";

    #[test]
    fn relation_line_parse_and_render() {
        assert_eq!(
            parse_relation_line(&format!("- 应用了 :: [[concepts/{A}-BKT]]")),
            Some((Predicate::new("应用了").unwrap(), WikiId::new(A).unwrap()))
        );
        assert_eq!(
            parse_relation_line(&format!("- [[{B}]]")),
            Some((Predicate::related(), WikiId::new(B).unwrap()))
        );
        assert_eq!(
            parse_relation_line(&format!("- uses ::[[{B}]]")),
            Some((Predicate::new("uses").unwrap(), WikiId::new(B).unwrap()))
        );
        assert_eq!(parse_relation_line("- 悬空 [[not-an-id]]"), None);
        assert_eq!(parse_relation_line("- 无链接"), None);
        assert_eq!(parse_relation_line("plain text"), None);
    }

    #[test]
    fn relation_line_render_omits_related_predicate() {
        let line = relation_line(&Predicate::related(), &WikiId::new(A).unwrap());
        assert_eq!(line, format!("- [[{A}]]"));
        let line = relation_line(&Predicate::new("作者").unwrap(), &WikiId::new(A).unwrap());
        assert_eq!(line, format!("- 作者 :: [[{A}]]"));
    }

    #[test]
    fn split_relations_section_variants() {
        let body = "正文。\n\n## 关联页面\n- [[wiki-91f2c43bf4de401e]]\n- 坏行\n";
        let (before, lines, had) = split_relations_section(body);
        assert_eq!(before, "正文。");
        assert!(had);
        assert_eq!(lines, vec!["- [[wiki-91f2c43bf4de401e]]", "- 坏行"]);

        let (before, lines, had) = split_relations_section("只有正文\n");
        assert_eq!(before, "只有正文");
        assert!(lines.is_empty());
        assert!(!had);
    }

    #[test]
    fn save_load_round_trip_preserves_unknown_keys_and_raw_lines() {
        let dir = tempfile_dir();
        let store = Store::new(&dir);
        store.init().unwrap();

        let mut extra = std::collections::BTreeMap::new();
        extra.insert("language".to_string(), " zh-Hans".to_string());
        let doc = EntryDocument {
            id: WikiId::new(A).unwrap(),
            wiki_type: WikiType::Concept,
            title: "知识追踪".into(),
            created: "2026-09-03T12:00:00+00:00".into(),
            updated: "2026-09-03T12:30:00+00:00".into(),
            extra,
            body: "<ref-aaaaaaaaaaaaaaaa>正文</ref-aaaaaaaaaaaaaaaa>".into(),
            relation_lines: vec![
                format!("- [[{B}]]"),
                "- 坏行 preserved".to_string(),
            ],
            relations: vec![(Predicate::related(), WikiId::new(B).unwrap())],
            had_relations_section: true,
        };

        let rel = store.save(&doc).unwrap();
        assert_eq!(rel, format!("wiki/concepts/{A}-知识追踪.md"));

        let loaded = store.load(&doc.id).unwrap();
        assert_eq!(loaded.title, doc.title);
        assert_eq!(loaded.extra, doc.extra);
        assert_eq!(loaded.body, doc.body);
        assert_eq!(loaded.relation_lines, doc.relation_lines);
        assert_eq!(loaded.relations, doc.relations);
        assert!(loaded.had_relations_section);

        // 再写一次：文件内容稳定（幂等）
        let first = fs::read_to_string(dir.join(&rel)).unwrap();
        store.save(&loaded).unwrap();
        let second = fs::read_to_string(dir.join(&rel)).unwrap();
        assert_eq!(first, second);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fluen-kb-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
