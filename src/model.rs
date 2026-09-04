use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use std::collections::BTreeMap;

/// 索引视图：由 DB 派生，字段永远完整。
#[derive(Debug, Clone, PartialEq)]
pub struct EntryMeta {
    pub id: WikiId,
    pub wiki_type: WikiType,
    pub title: String,
    pub file_path: String,
    pub relations: Vec<(Predicate, WikiId)>,
    pub sources: Vec<SourceId>,
    pub created: String,
    pub updated: String,
}

/// 文件视图：frontmatter + 原始正文 + 关联区原始行。
///
/// `relation_lines` 保留关联区的原始文本（含语法无效的行），
/// 使任何"读 → 改 → 写"路径都不丢失宿主内容（设计 §3.1.3）。
#[derive(Debug, Clone, PartialEq)]
pub struct EntryDocument {
    pub id: WikiId,
    pub wiki_type: WikiType,
    pub title: String,
    pub created: String,
    pub updated: String,
    pub extra: BTreeMap<String, String>,
    pub body: String,
    pub relation_lines: Vec<String>,
    pub relations: Vec<(Predicate, WikiId)>,
    pub had_relations_section: bool,
}

impl EntryDocument {
    pub fn to_meta(&self, file_path: &str, sources: Vec<SourceId>) -> EntryMeta {
        EntryMeta {
            id: self.id.clone(),
            wiki_type: self.wiki_type,
            title: self.title.clone(),
            file_path: file_path.to_string(),
            relations: self.relations.clone(),
            sources,
            created: self.created.clone(),
            updated: self.updated.clone(),
        }
    }
}
