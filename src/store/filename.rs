//! 文件名规则（设计 §3.3）：`{WikiId}-{净化标题}.md`，标题为空时省略后半。

use crate::ids::WikiId;

const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
const TITLE_MAX_CHARS: usize = 50;

pub fn sanitize_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| !FORBIDDEN.contains(c))
        .take(TITLE_MAX_CHARS)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn entry_filename(id: &WikiId, title: &str) -> String {
    entry_filename_id_title(id.as_str(), title) + ".md"
}

/// 不含 `.md` 后缀的链接形式（index.md 使用）。
pub fn entry_filename_id_title(id: &str, title: &str) -> String {
    let sanitized = sanitize_title(title);
    if sanitized.is_empty() {
        id.to_string()
    } else {
        format!("{id}-{sanitized}")
    }
}

/// 从文件名（不含目录）提取 WikiId；切点落在多字节字符内部时返回 None。
pub fn id_from_filename(filename: &str) -> Option<WikiId> {
    WikiId::split_prefix(filename.strip_suffix(".md")?).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_forbidden_and_truncates() {
        assert_eq!(sanitize_title("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(sanitize_title(&"知".repeat(60)).chars().count(), 50);
        assert_eq!(sanitize_title("  padded  "), "padded");
        assert_eq!(sanitize_title("///"), "");
    }

    #[test]
    fn filename_formats() {
        let id = WikiId::new("wiki-91f2c43bf4de401e").unwrap();
        assert_eq!(entry_filename(&id, "BKT"), "wiki-91f2c43bf4de401e-BKT.md");
        assert_eq!(entry_filename(&id, ""), "wiki-91f2c43bf4de401e.md");
    }

    #[test]
    fn id_extraction() {
        assert_eq!(
            id_from_filename("wiki-91f2c43bf4de401e-BKT.md"),
            Some(WikiId::new("wiki-91f2c43bf4de401e").unwrap())
        );
        assert_eq!(
            id_from_filename("wiki-91f2c43bf4de401e.md"),
            Some(WikiId::new("wiki-91f2c43bf4de401e").unwrap())
        );
        assert_eq!(id_from_filename("notes.md"), None);
        assert_eq!(id_from_filename("wiki-91f2.md"), None);
        assert_eq!(id_from_filename("wiki-91f2c43bf4de40中.md"), None);
    }
}
