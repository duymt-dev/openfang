//! Test-only protocol peer, no AppChat tools or business effects.
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{http::StatusCode, response::Redirect, routing::post, Router};
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

use super::super::{McpServerConfig, McpTransport};

#[derive(Clone, Default)]
pub struct Peer {
    pub calls: Arc<AtomicUsize>,
    pub list_delay: Duration,
    pub call_delay: Duration,
    pub collisions: bool,
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
        tokio::time::sleep(self.list_delay).await;
        let mut result = ListToolsResult::default();
        let names = if self.collisions {
            vec!["probe-result", "probe_result"]
        } else {
            vec!["probe-result"]
        };
        for name in names {
            result.tools.push(
                serde_json::from_value(json!({"name":name,"description":"protocol fixture only",
                "inputSchema":{"type":"object","properties":{}}}))
                .unwrap(),
            );
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            request.name, "probe-result",
            "must preserve original hyphenated wire name"
        );
        tokio::time::sleep(self.call_delay).await;
        let error = request
            .arguments
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let payload = json!({"ok":!error,"source":"protocol-fixture"});
        Ok(
            serde_json::from_value(json!({"isError":error,"structuredContent":payload,
            "content":[{"type":"text","text":payload.to_string()}]}))
            .unwrap(),
        )
    }
}

pub struct Fixture {
    pub url: String,
    pub calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Fixture {
    pub async fn expired_session() -> (Self, Arc<AtomicUsize>) {
        use axum::{
            body::{to_bytes, Body},
            extract::Request,
            middleware::{from_fn, Next},
            response::IntoResponse,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let attempts = calls.clone();
        let initializations = Arc::new(AtomicUsize::new(0));
        let observed_init = initializations.clone();
        let service = StreamableHttpService::new(
            || Ok(Peer::default()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true),
        );
        let app = Router::new().route_service("/mcp", service).layer(from_fn(
            move |request: Request, next: Next| {
                let attempts = attempts.clone();
                let init = observed_init.clone();
                async move {
                    let (parts, body) = request.into_parts();
                    let bytes = to_bytes(body, 16 * 1024).await.unwrap();
                    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    if value["method"] == "initialize" {
                        init.fetch_add(1, Ordering::SeqCst);
                    }
                    if value["method"] == "tools/call" {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        return StatusCode::NOT_FOUND.into_response();
                    }
                    next.run(Request::from_parts(parts, Body::from(bytes)))
                        .await
                }
            },
        ));
        (Self::serve(app, calls).await, initializations)
    }

    pub async fn start(peer: Peer) -> Self {
        let calls = peer.calls.clone();
        let service = StreamableHttpService::new(
            move || Ok(peer.clone()),
            Arc::new(NeverSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(false)
                .with_json_response(true),
        );
        Self::serve(Router::new().route_service("/mcp", service), calls).await
    }

    pub async fn redirect() -> Self {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let app = Router::new()
            .route(
                "/mcp",
                post(|| async { Redirect::temporary("/credential-target") }),
            )
            .route(
                "/credential-target",
                post(move || {
                    let calls = observed.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        StatusCode::OK
                    }
                }),
            );
        Self::serve(app, calls).await
    }

    pub async fn stalled_handshake() -> Self {
        let app = Router::new().route(
            "/mcp",
            post(|| async {
                tokio::time::sleep(Duration::from_secs(10)).await;
                StatusCode::NO_CONTENT
            }),
        );
        Self::serve(app, Arc::new(AtomicUsize::new(0))).await
    }

    async fn serve(app: Router, calls: Arc<AtomicUsize>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { url, calls, task }
    }

    pub fn config(&self) -> McpServerConfig {
        McpServerConfig {
            name: "probe".into(),
            transport: McpTransport::Http {
                url: self.url.clone(),
            },
            timeout_secs: 1,
            env: vec![],
            headers: vec![],
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
