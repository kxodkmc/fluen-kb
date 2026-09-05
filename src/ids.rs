use crate::error::{KbError, KbResult};
use std::fmt;

macro_rules! hex_id {
    ($name:ident, $prefix:literal, $doc:literal, $min:literal..=$max:literal, $len_desc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn new(value: &str) -> KbResult<Self> {
                Self::parse(value).ok_or_else(|| {
                    KbError::invalid(concat!(
                        stringify!($name),
                        " malformed: expected `",
                        $prefix,
                        "` + ",
                        $len_desc
                    ))
                })
            }

            pub fn parse(value: &str) -> Option<Self> {
                let id = value.strip_prefix($prefix)?;
                if !($min..=$max).contains(&id.len())
                    || !id.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return None;
                }
                Some($name(value.to_ascii_lowercase()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

hex_id!(
    SourceId,
    "ref-",
    "来源标识：`ref-` + 16..=32 位小写 hex。指向宿主管理的原始来源。",
    16..=32,
    "16..=32 hex digits"
);
hex_id!(
    WikiId,
    "wiki-",
    "条目标识：`wiki-` + 16 位小写 hex。",
    16..=16,
    "16 hex digits"
);

impl WikiId {
    /// 从 `target` 开头拆出 id 与剩余部分；长度不足或切点落在多字节字符内部时返回 None。
    pub fn split_prefix(target: &str) -> Option<(Self, &str)> {
        let end = Self::PREFIX.len() + 16;
        let id = Self::parse(target.get(..end)?)?;
        Some((id, &target[end..]))
    }
}

/// 条目类型，固定三类（设计决策 D14）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WikiType {
    Summary,
    Concept,
    Entity,
}

impl WikiType {
    pub const ALL: [WikiType; 3] = [WikiType::Summary, WikiType::Concept, WikiType::Entity];

    pub fn as_str(self) -> &'static str {
        match self {
            WikiType::Summary => "summary",
            WikiType::Concept => "concept",
            WikiType::Entity => "entity",
        }
    }

    pub fn dir(self) -> &'static str {
        match self {
            WikiType::Summary => "summaries",
            WikiType::Concept => "concepts",
            WikiType::Entity => "entities",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "summary" => Some(WikiType::Summary),
            "concept" => Some(WikiType::Concept),
            "entity" => Some(WikiType::Entity),
            _ => None,
        }
    }
}

impl fmt::Display for WikiType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 类型化关系谓词。缺省 `related`。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Predicate(String);

impl Predicate {
    pub const RELATED: &str = "related";

    pub fn new(value: &str) -> KbResult<Self> {
        let len = value.chars().count();
        let valid = (1..=32).contains(&len)
            && value
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_');
        if !valid {
            return Err(KbError::invalid(
                "predicate must be 1..=32 alphanumeric/underscore characters",
            ));
        }
        Ok(Predicate(value.to_string()))
    }

    pub fn related() -> Self {
        Predicate(Self::RELATED.to_string())
    }

    pub fn is_related(&self) -> bool {
        self.0 == Self::RELATED
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(feature = "index")]
pub(crate) fn generate_wiki_id() -> String {
    format!("wiki-{}", &uuid::Uuid::new_v4().simple().to_string()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_id_accepts_valid_and_rejects_malformed() {
        assert!(SourceId::new("ref-3d09a8bb72c14e5f").is_ok());
        assert!(SourceId::new("ref-3D09A8BB72C14E5F").is_ok());
        assert!(SourceId::new("ref-3d09a8bb72c14e5f955be8dc242b0413").is_ok());
        assert_eq!(
            SourceId::new("ref-3D09A8BB72C14E5F").unwrap().as_str(),
            "ref-3d09a8bb72c14e5f"
        );
        assert!(SourceId::new("ref-3d09").is_err());
        assert!(SourceId::new("ref-3d09a8bb72c14e5").is_err());
        assert!(SourceId::new("ref-3d09a8bb72c14e5f955be8dc242b04133").is_err());
        assert!(SourceId::new("wiki-3d09a8bb72c14e5f").is_err());
        assert!(SourceId::new("ref-3d09a8bb72c14e5z").is_err());
    }

    #[test]
    fn wiki_id_parse() {
        assert!(WikiId::parse("wiki-91f2c43bf4de401e").is_some());
        assert!(WikiId::parse("wiki-91f2c43bf4de401e-extra").is_none());
        assert!(WikiId::parse("91f2c43bf4de401e").is_none());
    }

    #[test]
    fn wiki_type_roundtrip() {
        for t in WikiType::ALL {
            assert_eq!(WikiType::parse(t.as_str()), Some(t));
        }
        assert!(WikiType::parse("tag").is_none());
    }

    #[test]
    fn predicate_validation() {
        assert!(Predicate::new("应用了").is_ok());
        assert!(Predicate::new("uses_2").is_ok());
        assert!(Predicate::new("").is_err());
        assert!(Predicate::new(&"x".repeat(33)).is_err());
        assert!(Predicate::new("has space").is_err());
        assert!(Predicate::related().is_related());
    }
}
