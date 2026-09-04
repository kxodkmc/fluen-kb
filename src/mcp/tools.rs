//! MCP 工具面（设计 §9.4）：十个工具一一映射 SDK 方法。

use crate::error::{KbError, KbResult};
use crate::handle::Kb;
use crate::ids::{Predicate, SourceId, WikiId, WikiType};
use crate::model::EntryMeta;
use crate::ops::EditOp;
use crate::search::{QueryParams, SearchMethod};
use rmcp::model::Tool;
use serde_json::{Value, json};
use std::sync::Arc;

type ToolResult = Result<Value, String>;

fn invalid(message: impl Into<String>) -> String {
    message.into()
}

/// 工具定义（MCP 2026-07-28：确定性顺序，schema 用 serde_json 手工构建，不引 schemars）。
pub fn definitions() -> Vec<Tool> {
    vec![
        tool("knowledge_query", "Search the knowledge base (keyword / semantic / hybrid, with relation-neighborhood expansion).",
             json!({ "type": "object", "required": ["query"], "properties": {
                 "query": { "type": "string" },
                 "wiki_type": { "type": "string", "enum": ["summary", "concept", "entity"] },
                 "method": { "type": "string", "enum": ["keyword", "semantic", "hybrid"] },
                 "top_k": { "type": "integer", "minimum": 1 },
                 "include_content": { "type": "boolean" },
                 "expand": { "type": "integer", "minimum": 0, "maximum": 2 } } })),
        tool("knowledge_query_batch", "Run knowledge_query for a batch of query strings with default settings.",
             json!({ "type": "object", "required": ["queries"], "properties": {
                 "queries": { "type": "array", "items": { "type": "string" } } } })),
        tool("knowledge_create_entry", "Create an entry (deduplicated: summary by source, concept/entity by title+type). The body of a summary must be attributable to exactly one source: either wrap it in `<ref-…>` or pass `source`.",
             json!({ "type": "object", "required": ["type", "title", "body"], "properties": {
                 "type": { "type": "string", "enum": ["summary", "concept", "entity"] },
                 "title": { "type": "string" },
                 "body": { "type": "string", "description": "Markdown body; provenance via <ref-…> tags" },
                 "relations": { "type": "array", "items": { "type": "object", "required": ["to"], "properties": {
                     "predicate": { "type": "string" }, "to": { "type": "string" } } } },
                 "source": { "type": "string", "description": "ref-… ; summary only" },
                 "authors": { "type": "array", "items": { "type": "string" } } } })),
        tool("knowledge_edit_entry", "Apply edit primitives to an entry: search_replace / insert_after (first match in scope) / replace_source.",
             json!({ "type": "object", "required": ["id", "ops"], "properties": {
                 "id": { "type": "string" },
                 "ops": { "type": "array", "items": { "type": "object", "required": ["kind"], "properties": {
                     "kind": { "type": "string", "enum": ["search_replace", "insert_after", "replace_source"] },
                     "search": { "type": "string" }, "replace": { "type": "string" },
                     "anchor": { "type": "string" }, "content": { "type": "string" },
                     "src": { "type": "string" }, "scope": { "type": "string" } } } } } })),
        tool("knowledge_get_entry", "Load an entry document (frontmatter, provenance-tagged body, relation section).",
             json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } })),
        tool("knowledge_list_entries", "List indexed entries, optionally filtered by type.",
             json!({ "type": "object", "properties": {
                 "wiki_type": { "type": "string", "enum": ["summary", "concept", "entity"] } } })),
        tool("knowledge_meta", "Library overview (counts by type) or recent entries (latest updated).",
             json!({ "type": "object", "required": ["kind"], "properties": {
                 "kind": { "type": "string", "enum": ["overview", "recent"] } } })),
        tool("knowledge_delete_entry", "Delete an entry and clean references to it in other entries.",
             json!({ "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } })),
        tool("knowledge_lint", "Read-only health check; returns warnings and errors.",
             json!({ "type": "object", "properties": {} })),
        tool("knowledge_prune", "Remove dangling references (relation lines and body links) from entry files.",
             json!({ "type": "object", "properties": {} })),
    ]
}

fn tool(name: &str, description: &str, schema: Value) -> Tool {
    let schema_map = schema.as_object().cloned().unwrap_or_default();
    Tool::new(name.to_string(), description.to_string(), Arc::new(schema_map))
}

/// 分发工具调用：`name` 为工具名，`args` 为参数字段。成功返回结构化的 JSON 值。
pub fn dispatch(kb: &Kb, name: &str, args: &Value) -> ToolResult {
    match name {
        "knowledge_query" => query(kb, args),
        "knowledge_query_batch" => query_batch(kb, args),
        "knowledge_create_entry" => create_entry(kb, args),
        "knowledge_edit_entry" => edit_entry(kb, args),
        "knowledge_get_entry" => get_entry(kb, args),
        "knowledge_list_entries" => list_entries(kb, args),
        "knowledge_meta" => meta(kb, args),
        "knowledge_delete_entry" => delete_entry(kb, args),
        "knowledge_lint" => lint(kb),
        "knowledge_prune" => prune(kb),
        other => Err(invalid(format!("unknown tool {other:?}"))),
    }
}

fn query(kb: &Kb, args: &Value) -> ToolResult {
    let params = QueryParams {
        query: str_arg(args, "query")?,
        wiki_type: opt_enum(args, "wiki_type", WikiType::parse)?,
        method: opt_enum(args, "method", parse_method)?.unwrap_or_default(),
        top_k: args.get("top_k").and_then(Value::as_u64).map(|v| v as usize),
        include_content: args
            .get("include_content")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        expand: args.get("expand").and_then(Value::as_u64).unwrap_or(0) as u8,
    };
    let hits = kb.search().query(&params).map_err(kb_error)?;
    Ok(json!(hits.iter().map(hit_json).collect::<Vec<_>>()))
}

fn query_batch(kb: &Kb, args: &Value) -> ToolResult {
    let queries = args
        .get("queries")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("`queries` must be an array of strings"))?;
    let mut out = Vec::new();
    for q in queries {
        let params = QueryParams {
            query: q.as_str().ok_or_else(|| invalid("query must be a string"))?.to_string(),
            ..QueryParams::default()
        };
        let hits = kb.search().query(&params).map_err(kb_error)?;
        out.push(json!(hits.iter().map(hit_json).collect::<Vec<_>>()));
    }
    Ok(json!(out))
}

fn create_entry(kb: &Kb, args: &Value) -> ToolResult {
    let wiki_type = enum_arg(args, "type", WikiType::parse)?;
    let title = str_arg(args, "title")?;
    let body = str_arg(args, "body")?;
    let relations = parse_relations(args.get("relations"))?;
    let source = opt_id(args, "source", SourceId::new)?;
    let authors = parse_id_list(args.get("authors"), WikiId::new)?;

    let outcome = kb
        .ops()
        .create(wiki_type, &title, &body, &relations, source.as_ref(), &authors)
        .map_err(kb_error)?;
    Ok(match outcome {
        crate::ops::CreateOutcome::Created(id) => json!({ "created": id.to_string() }),
        crate::ops::CreateOutcome::MergedInto(id) => json!({ "merged_into": id.to_string() }),
    })
}

fn edit_entry(kb: &Kb, args: &Value) -> ToolResult {
    let id = id_arg(args, "id", WikiId::new)?;
    let raw_ops = args
        .get("ops")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("`ops` must be an array"))?;
    let mut ops = Vec::new();
    for raw in raw_ops {
        let kind = raw
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("each op requires `kind`"))?;
        let scope = opt_id(raw, "scope", SourceId::new)?;
        ops.push(match kind {
            "search_replace" => EditOp::SearchReplace {
                search: raw
                    .get("search")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("search_replace requires `search`"))?
                    .to_string(),
                replace: raw
                    .get("replace")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("search_replace requires `replace`"))?
                    .to_string(),
                scope,
            },
            "insert_after" => EditOp::InsertAfter {
                anchor: raw
                    .get("anchor")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("insert_after requires `anchor`"))?
                    .to_string(),
                content: raw
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("insert_after requires `content`"))?
                    .to_string(),
                scope,
            },
            "replace_source" => EditOp::ReplaceSource {
                src: id_arg(raw, "src", SourceId::new)?,
                content: raw
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("replace_source requires `content`"))?
                    .to_string(),
            },
            other => return Err(invalid(format!("unknown op kind {other:?}"))),
        });
    }
    let results = kb.ops().edit(&id, ops).map_err(kb_error)?;
    Ok(json!(results
        .iter()
        .map(|r| r.is_ok())
        .collect::<Vec<_>>()))
}

fn get_entry(kb: &Kb, args: &Value) -> ToolResult {
    let id = id_arg(args, "id", WikiId::new)?;
    let doc = kb.search().get_entry(&id).map_err(kb_error)?;
    Ok(json!({
        "id": doc.id.to_string(),
        "type": doc.wiki_type.as_str(),
        "title": doc.title,
        "created": doc.created,
        "updated": doc.updated,
        "extra": doc.extra,
        "body": doc.body,
        "relations": doc.relations.iter()
            .map(|(p, t)| json!({ "predicate": p.as_str(), "to": t.to_string() }))
            .collect::<Vec<_>>(),
    }))
}

fn list_entries(kb: &Kb, args: &Value) -> ToolResult {
    let wiki_type = opt_enum(args, "wiki_type", WikiType::parse)?;
    let metas = kb.search().list_entries(wiki_type).map_err(kb_error)?;
    Ok(json!(metas.iter().map(meta_json).collect::<Vec<_>>()))
}

fn meta(kb: &Kb, args: &Value) -> ToolResult {
    let kind = str_arg(args, "kind")?;
    let metas = kb.search().list_entries(None).map_err(kb_error)?;
    match kind.as_str() {
        "overview" => {
            let count = |t: WikiType| metas.iter().filter(|m| m.wiki_type == t).count();
            Ok(json!({
                "total": metas.len(),
                "summaries": count(WikiType::Summary),
                "concepts": count(WikiType::Concept),
                "entities": count(WikiType::Entity),
            }))
        }
        "recent" => {
            let mut recent: Vec<&EntryMeta> = metas.iter().collect();
            recent.sort_by(|a, b| b.updated.cmp(&a.updated));
            recent.truncate(10);
            Ok(json!(recent.iter().map(|m| meta_json(m)).collect::<Vec<_>>()))
        }
        other => Err(invalid(format!("unknown meta kind {other:?}"))),
    }
}

fn delete_entry(kb: &Kb, args: &Value) -> ToolResult {
    let id = id_arg(args, "id", WikiId::new)?;
    kb.ops().delete(&id).map_err(kb_error)?;
    Ok(json!({ "deleted": id.to_string() }))
}

fn lint(kb: &Kb) -> ToolResult {
    let issues = kb.ops().lint().map_err(kb_error)?;
    Ok(json!(issues
        .iter()
        .map(|i| json!({
            "level": if i.level == crate::lint::LintLevel::Error { "error" } else { "warning" },
            "entry": i.entry.as_ref().map(|e| e.to_string()),
            "message": i.message,
        }))
        .collect::<Vec<_>>()))
}

fn prune(kb: &Kb) -> ToolResult {
    let report = kb.ops().prune().map_err(kb_error)?;
    Ok(json!(report
        .entries
        .iter()
        .map(|(id, n)| json!({ "entry": id.to_string(), "removed": n }))
        .collect::<Vec<_>>()))
}

// ---- 序列化与参数提取 ----

fn meta_json(m: &EntryMeta) -> Value {
    json!({
        "id": m.id.to_string(),
        "type": m.wiki_type.as_str(),
        "title": m.title,
        "file_path": m.file_path,
        "sources": m.sources.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        "relations": m.relations.iter()
            .map(|(p, t)| json!({ "predicate": p.as_str(), "to": t.to_string() }))
            .collect::<Vec<_>>(),
        "created": m.created,
        "updated": m.updated,
    })
}

fn hit_json(h: &crate::search::SearchHit) -> Value {
    let mut v = meta_json(&h.meta);
    v["score"] = json!(h.score);
    if let Some(content) = &h.content {
        v["content"] = json!(content);
    }
    v
}

fn kb_error(e: KbError) -> String {
    invalid(format!("{e}"))
}

fn parse_method(s: &str) -> Option<SearchMethod> {
    match s {
        "keyword" => Some(SearchMethod::Keyword),
        "semantic" => Some(SearchMethod::Semantic),
        "hybrid" => Some(SearchMethod::Hybrid),
        _ => None,
    }
}

fn str_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("missing string argument `{key}`")))
}

fn id_arg<T>(args: &Value, key: &str, parse: impl Fn(&str) -> KbResult<T>) -> Result<T, String> {
    let raw = str_arg(args, key)?;
    parse(&raw).map_err(kb_error)
}

fn opt_id<T>(
    args: &Value,
    key: &str,
    parse: impl Fn(&str) -> KbResult<T>,
) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => id_arg(args, key, parse).map(Some),
    }
}

fn enum_arg<T>(args: &Value, key: &str, parse: impl Fn(&str) -> Option<T>) -> Result<T, String> {
    let raw = str_arg(args, key)?;
    parse(&raw).ok_or_else(|| invalid(format!("invalid `{key}` value {raw:?}")))
}

fn opt_enum<T>(
    args: &Value,
    key: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => enum_arg(args, key, parse).map(Some),
    }
}

fn parse_relations(value: Option<&Value>) -> Result<Vec<(Predicate, WikiId)>, String> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|item| {
            let to = item
                .get("to")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("relation requires `to`"))?;
            let predicate = match item.get("predicate") {
                Some(p) => Predicate::new(p.as_str().unwrap_or_default()).map_err(kb_error)?,
                None => Predicate::related(),
            };
            Ok((predicate, WikiId::new(to).map_err(kb_error)?))
        })
        .collect()
}

fn parse_id_list<T>(
    value: Option<&Value>,
    parse: impl Fn(&str) -> KbResult<T>,
) -> Result<Vec<T>, String> {
    let Some(list) = value.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    list.iter()
        .map(|item| {
            let raw = item
                .as_str()
                .ok_or_else(|| invalid("id list items must be strings"))?;
            parse(raw).map_err(kb_error)
        })
        .collect()
}
