#![cfg(feature = "mcp-server")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::temp_kb;
use fluen_kb::handle::Kb;
use fluen_kb::mcp::KbServer;
use rmcp::service::{ClientLifecycleMode, RunningService};
use rmcp::{ClientServiceExt, RoleClient, ServiceExt};
use rmcp::model::{CallToolRequestParams, ClientInfo, ProtocolVersion, ResultType};
use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;

type Conn = (RunningService<RoleClient, ClientInfo>, JoinHandle<()>);

/// 进程内 stdio 连接：服务端在 background task 运行，
/// 客户端以无状态 `server/discover`（MCP 2026-07-28）握手。
async fn connect(kb: Kb) -> Conn {
    // `duplex` 返回一对互相连接的流：写一端、另一端可读。
    let (client_write, server_read) = tokio::io::duplex(16 * 1024);
    let (server_write, client_read) = tokio::io::duplex(16 * 1024);

    let server = tokio::spawn(async move {
        KbServer::new(kb)
            .serve((server_read, server_write))
            .await
            .expect("server serve")
            .waiting()
            .await
            .ok();
    });

    let client = ClientInfo::default()
        .serve_with_lifecycle(
            (client_read, client_write),
            ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("client serve");

    (client, server)
}

fn call(name: &'static str, args: Map<String, Value>) -> CallToolRequestParams {
    let mut params = CallToolRequestParams::new(name);
    params.arguments = Some(args);
    params
}

#[tokio::test]
async fn discover_negotiates_2026_07_28_and_lists_tools() {
    let (_dir, kb) = temp_kb();
    let (client, _server) = connect(kb).await;

    let tools = client.list_tools(None).await.expect("list tools");
    let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
    for expected in [
        "knowledge_query",
        "knowledge_create_entry",
        "knowledge_edit_entry",
        "knowledge_get_entry",
        "knowledge_list_entries",
        "knowledge_meta",
        "knowledge_delete_entry",
        "knowledge_lint",
        "knowledge_prune",
    ] {
        assert!(names.iter().any(|n| n == expected), "missing tool {expected}");
    }
    // 顺序确定：同一请求连发两次，结果必须逐项一致（便于客户端缓存工具列表）。
    let again = {
        let r = client.list_tools(None).await.expect("list tools again");
        r.tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>()
    };
    assert_eq!(names, again, "tools must be in deterministic order");
}

#[tokio::test]
async fn create_query_lint_delete_end_to_end() {
    let (_dir, kb) = temp_kb();
    let (client, _server) = connect(kb).await;

    // create
    let mut args = Map::new();
    args.insert("type".into(), json!("concept"));
    args.insert("title".into(), json!("工具创建"));
    args.insert("body".into(), json!("<ref-1111111111111111>经由 MCP 工具创建的正文。</ref-1111111111111111>"));
    let created = client
        .call_tool(call("knowledge_create_entry", args))
        .await
        .expect("call create");
    assert_eq!(created.result_type, Some(ResultType::COMPLETE));
    let id = created.structured_content.expect("structured create")["created"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(id.starts_with("wiki-"));

    // query
    let mut args = Map::new();
    args.insert("query".into(), json!("MCP 工具"));
    args.insert("include_content".into(), json!(true));
    let res = client.call_tool(call("knowledge_query", args)).await.expect("call query");
    let hits = res.structured_content.expect("structured query");
    assert!(!hits.as_array().unwrap().is_empty());
    assert!(hits[0]["content"].as_str().unwrap().contains("经由 MCP 工具创建"));

    // lint（新建库应无问题）
    let res = client
        .call_tool(call("knowledge_lint", Map::new()))
        .await
        .expect("call lint");
    assert!(res.structured_content.expect("structured lint").as_array().unwrap().is_empty());

    // get
    let mut args = Map::new();
    args.insert("id".into(), json!(id));
    let res = client.call_tool(call("knowledge_get_entry", args)).await.expect("call get");
    let doc = res.structured_content.expect("structured get");
    assert_eq!(doc["title"], "工具创建");

    // delete
    let mut args = Map::new();
    args.insert("id".into(), json!(id));
    let res = client.call_tool(call("knowledge_delete_entry", args)).await.expect("call delete");
    assert!(res.structured_content.expect("structured delete")["deleted"]
        .as_str()
        .unwrap()
        .contains("wiki-"));
}

#[tokio::test]
async fn list_entries_paginates_and_meta_uses_aggregates() {
    let (_dir, kb) = temp_kb();
    let (client, _server) = connect(kb).await;

    for title in ["概念一", "概念二", "概念三"] {
        let mut args = Map::new();
        args.insert("type".into(), json!("concept"));
        args.insert("title".into(), json!(title));
        args.insert("body".into(), json!("<ref-1111111111111111>正文。</ref-1111111111111111>"));
        client
            .call_tool(call("knowledge_create_entry", args))
            .await
            .expect("call create");
    }

    // 第一页：limit=2，返回 {total, entries}
    let mut args = Map::new();
    args.insert("limit".into(), json!(2));
    let res = client
        .call_tool(call("knowledge_list_entries", args))
        .await
        .expect("call list page 1");
    let page = res.structured_content.expect("structured list");
    assert_eq!(page["total"], 3);
    assert_eq!(page["entries"].as_array().unwrap().len(), 2);

    // 第二页：offset=2，仅剩 1 条
    let mut args = Map::new();
    args.insert("limit".into(), json!(2));
    args.insert("offset".into(), json!(2));
    let res = client
        .call_tool(call("knowledge_list_entries", args))
        .await
        .expect("call list page 2");
    let page = res.structured_content.expect("structured list");
    assert_eq!(page["total"], 3);
    assert_eq!(page["entries"].as_array().unwrap().len(), 1);

    // meta overview：类型计数
    let mut args = Map::new();
    args.insert("kind".into(), json!("overview"));
    let res = client
        .call_tool(call("knowledge_meta", args))
        .await
        .expect("call meta overview");
    let overview = res.structured_content.expect("structured overview");
    assert_eq!(overview["total"], 3);
    assert_eq!(overview["concepts"], 3);

    // meta recent：最近更新
    let mut args = Map::new();
    args.insert("kind".into(), json!("recent"));
    let res = client
        .call_tool(call("knowledge_meta", args))
        .await
        .expect("call meta recent");
    let recent = res.structured_content.expect("structured recent");
    assert_eq!(recent.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn bad_arguments_return_protocol_error() {
    let (_dir, kb) = temp_kb();
    let (client, _server) = connect(kb).await;

    // 缺 title：应返回协议错误（invalid_params），而不是成功。
    let mut args = Map::new();
    args.insert("type".into(), json!("concept"));
    assert!(client.call_tool(call("knowledge_create_entry", args)).await.is_err());

    // 未知工具名：协议错误。
    assert!(client.call_tool(call("knowledge_nope", Map::new())).await.is_err());
}