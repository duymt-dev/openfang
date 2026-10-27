//! Ephemeral SDK interoperability peer. No AppChat operation or business receipt.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum_compat::{middleware, Router};
use openfang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::json;
use tokio::{net::TcpListener, task::JoinHandle};

use super::guard::{enforce, Policy, AUTH};

pub const TOOL: &str = "mcp_guard_transport_probe";

#[derive(Clone)]
struct Peer(Arc<AtomicUsize>);

impl ServerHandler for Peer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = ListToolsResult::default();
        result.tools.push(
            serde_json::from_value(json!({
                "name":"transport-probe", "description":"protocol-only test",
                "inputSchema":{"type":"object","properties":{}}
            }))
            .unwrap(),
        );
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        assert_eq!(request.name, "transport-probe");
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::from_value(json!({
            "isError":false, "content":[], "structuredContent":{"probe":"protocol-only"}
        }))
        .unwrap())
    }
}

pub struct Fixture {
    pub url: String,
    pub policy: Policy,
    pub calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn start() -> Self {
        let policy = Policy::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let peer = Peer(calls.clone());
        let service = StreamableHttpService::new(
            move || Ok(peer.clone()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true),
        );
        let app = Router::new()
            .route_service("/mcp", service)
            .layer(middleware::from_fn_with_state(policy.clone(), enforce));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum_compat::serve(listener, app).await.unwrap() });
        Self {
            url,
            policy,
            calls,
            task,
        }
    }

    pub async fn connect(&self, origin: Option<&str>) -> Result<McpConnection, String> {
        let mut headers = vec![format!("Authorization: {AUTH}")];
        if let Some(origin) = origin {
            headers.push(format!("Origin: {origin}"));
        }
        McpConnection::connect(McpServerConfig {
            name: "guard".into(),
            transport: McpTransport::Http {
                url: self.url.clone(),
            },
            timeout_secs: 3,
            env: vec![],
            headers,
        })
        .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
