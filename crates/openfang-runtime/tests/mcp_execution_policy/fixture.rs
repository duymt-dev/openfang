//! SDK protocol peer only: never a public business tool or live acceptance fixture.
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::Router;
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
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

pub const TOOL: &str = "mcp_peer_probe_result";

#[derive(Clone)]
struct Peer {
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

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
                "name":"probe-result", "description":"protocol result fixture",
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
        assert_eq!(request.name, "probe-result");
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        let error = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("error"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(serde_json::from_value(json!({
            "isError":error, "structuredContent":{"ok":!error,"receipt":"protocol-only"},
            "content":[{"type":"text","text":"protocol result"}]
        }))
        .unwrap())
    }
}

pub struct Fixture {
    pub connections: Mutex<Vec<McpConnection>>,
    calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn start(delay: Duration) -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let peer = Peer {
            calls: calls.clone(),
            delay,
        };
        let service = StreamableHttpService::new(
            move || Ok(peer.clone()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, Router::new().route_service("/mcp", service))
                .await
                .unwrap();
        });
        let connection = McpConnection::connect(McpServerConfig {
            name: "peer".into(),
            transport: McpTransport::Http { url },
            timeout_secs: 1,
            env: vec![],
            headers: vec![],
        })
        .await
        .unwrap();
        assert_eq!(connection.tools()[0].name, TOOL);
        Self {
            connections: Mutex::new(vec![connection]),
            calls,
            task,
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
