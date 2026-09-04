//! L4 MCP server：基于官方 `rmcp` SDK（MCP 2026-07-28 无状态规范）。
//!
//! 实现 `ServerHandler`，只暴露 tools 能力；协议握手、`server/discover`、
//! `_meta`、结果 `resultType` 等由 rmcp 处理，符合原生规范。

mod tools;

use crate::error::{KbError, KbResult};
use crate::handle::Kb;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData, Implementation,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerInfo,
    ToolsCapability,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleServer};
use serde_json::Value;

/// 把 `Kb` 暴露为一个 MCP（tools-only）服务。
#[derive(Clone)]
pub struct KbServer {
    kb: Kb,
}

impl KbServer {
    pub fn new(kb: Kb) -> Self {
        KbServer { kb }
    }
}

impl ServerHandler for KbServer {
    fn get_info(&self) -> ServerInfo {
        let mut capabilities = ServerCapabilities::default();
        capabilities.tools = Some(ToolsCapability::default());
        ServerInfo::new(capabilities)
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(Implementation::new("fluen-kb", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(tools::definitions())))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + 'static {
        let kb = self.kb.clone();
        let name = request.name.to_string();
        let args: Value = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        async move {
            let result = tokio::task::spawn_blocking(move || tools::dispatch(&kb, &name, &args))
                .await
                .map_err(|e| ErrorData::internal_error(format!("kb task failed: {e}"), None))?;
            result.map_or_else(
                |message| Err(ErrorData::invalid_params(message, None)),
                |value| Ok(CallToolResponse::Complete(CallToolResult::structured(value))),
            )
        }
    }
}

/// 由 `kb` 构造 MCP 服务句柄。
pub fn server(kb: Kb) -> KbServer {
    KbServer::new(kb)
}

/// 在 **已有** tokio runtime 中运行 stdio MCP server（阻塞直到 stdin 关闭）。
///
/// 若从同步上下文调用，请使用 [`run_stdio`]。
pub async fn serve_stdio(kb: Kb) -> KbResult<()> {
    use rmcp::ServiceExt;
    let running = KbServer::new(kb.clone())
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| KbError::Cancelled(format!("mcp serve failed: {e}")))?;
    running
        .waiting()
        .await
        .map_err(|e| KbError::Cancelled(format!("mcp server task failed: {e}")))?;
    Ok(())
}

/// 同步阻塞运行 stdio MCP server，直到 stdin 关闭。
/// 内部自建 tokio runtime，不能在已有的 tokio runtime 中调用（见 [`serve_stdio`]）。
pub fn run_stdio(kb: Kb) -> KbResult<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .build()
        .map_err(KbError::Io)?;
    rt.block_on(serve_stdio(kb))
}