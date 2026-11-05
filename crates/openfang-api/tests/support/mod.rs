//! Test-only harness for `appchat_turn_token.rs`: a fake OpenAI-compatible
//! LLM, a recording MCP peer, and a kernel + real API router on loopback.
#![allow(dead_code)]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    extract::Request,
    middleware::{from_fn, Next},
    response::IntoResponse,
    routing::post,
    Router,
};
use openfang_api::routes::AppState;
use openfang_kernel::OpenFangKernel;
use openfang_types::config::{
    DefaultModelConfig, KernelConfig, McpServerConfigEntry, McpTransportEntry,
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
        ServerCapabilities, ServerInfo,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData, RoleServer, ServerHandler,
};
use serde_json::{json, Value};

/// Process-wide TRACE log capture for the AC7 leak checks. Installed once as
/// the global subscriber so spawned kernel tasks on any worker thread write
/// into it too (a thread-local `set_default` would miss them).
pub fn trace_log() -> &'static Mutex<Vec<u8>> {
    use std::sync::OnceLock;
    static BUF: OnceLock<&'static Mutex<Vec<u8>>> = OnceLock::new();
    BUF.get_or_init(|| {
        let buf: &'static Mutex<Vec<u8>> = Box::leak(Box::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || LogWriter(buf))
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
        buf
    })
}

struct LogWriter(&'static Mutex<Vec<u8>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// AC7: assert the canary is absent from everything logged so far at TRACE,
/// and that the capture actually recorded this run (non-vacuous).
pub fn assert_logs_free_of(canary: &str) {
    let text = String::from_utf8_lossy(&trace_log().lock().unwrap()).into_owned();
    assert!(
        text.contains("TRACE") || text.contains("DEBUG"),
        "trace capture is empty"
    );
    assert!(!text.contains(canary), "canary leaked into logs");
}

pub type HeaderLog = Arc<Mutex<Vec<(String, Option<String>)>>>;
pub type Seen = Arc<Mutex<Vec<String>>>;

pub fn msg(token: Option<&str>) -> Value {
    match token {
        Some(t) => json!({"message": "call the probe tool", "appchat_task_token": t}),
        None => json!({"message": "call the probe tool"}),
    }
}

pub async fn wait_until(cond: impl Fn() -> bool) {
    for _ in 0..400 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition not reached within 10s");
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // Same as the daemon: the auth middleware needs the peer address to
    // treat a loopback caller as trusted when no api_key is configured.
    let svc = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, svc).await.unwrap() });
    format!("http://{addr}")
}

// ---------------------------------------------------------------- fake LLM

/// OpenAI-compatible `/v1/chat/completions`: requests `tool` when it is
/// offered and no tool result exists yet, otherwise answers `TOOL_DONE`.
/// Records every raw request body (used for the AC5/AC7 assertions).
pub async fn start_fake_llm(tool: &'static str) -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let rec = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |body: String| {
            let rec = rec.clone();
            async move {
                rec.lock().unwrap().push(body.clone());
                let req: Value = serde_json::from_str(&body).unwrap_or_default();
                let offered = req["tools"]
                    .as_array()
                    .is_some_and(|ts| ts.iter().any(|t| t["function"]["name"] == tool));
                let answered = req["messages"].as_array().is_some_and(|ms| {
                    let last_user = ms.iter().rposition(|m| m["role"] == "user").unwrap_or(0);
                    ms[last_user..].iter().any(|m| m["role"] == "tool")
                });
                let stream = req["stream"].as_bool().unwrap_or(false);
                llm_reply(offered && !answered, tool, stream)
            }
        }),
    );
    (format!("{}/v1", serve(app).await), seen)
}

fn llm_reply(call_tool: bool, tool: &str, stream: bool) -> axum::response::Response {
    let usage = json!({"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15});
    let call = json!([{"index": 0, "id": "call_1", "type": "function",
        "function": {"name": tool, "arguments": "{}"}}]);
    let finish = if call_tool { "tool_calls" } else { "stop" };
    if !stream {
        let message = if call_tool {
            json!({"role": "assistant", "content": null, "tool_calls": call})
        } else {
            json!({"role": "assistant", "content": "TOOL_DONE"})
        };
        return axum::Json(
            json!({"id": "x", "object": "chat.completion", "model": "fake",
            "choices": [{"index": 0, "message": message, "finish_reason": finish}],
            "usage": usage}),
        )
        .into_response();
    }
    let delta = if call_tool {
        json!({"role": "assistant", "tool_calls": call})
    } else {
        json!({"role": "assistant", "content": "TOOL_DONE"})
    };
    let chunk = |d: Value, f: Value| {
        json!({"id": "x", "object": "chat.completion.chunk", "model": "fake",
            "choices": [{"index": 0, "delta": d, "finish_reason": f}]})
    };
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk(delta, Value::Null),
        chunk(json!({}), json!(finish)),
        json!({"id": "x", "object": "chat.completion.chunk", "choices": [], "usage": usage}),
    );
    ([("content-type", "text/event-stream")], body).into_response()
}

// ---------------------------------------------------------------- MCP peer

#[derive(Clone, Default)]
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
            serde_json::from_value(json!({"name": "probe-result",
                "description": "turn-token fixture only",
                "inputSchema": {"type": "object", "properties": {}}}))
            .unwrap(),
        );
        Ok(result)
    }

    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        let payload = json!({"ok": true, "source": "turn-token-fixture"});
        Ok(
            serde_json::from_value(json!({"isError": false, "structuredContent": payload,
            "content": [{"type": "text", "text": payload.to_string()}]}))
            .unwrap(),
        )
    }
}

pub struct McpPeer {
    pub url: String,
    pub calls: Arc<AtomicUsize>,
    /// HTTP `DELETE` (session close) requests received.
    pub closed: Arc<AtomicUsize>,
}

/// Stateful (session-tracking) peer so a client closing its connection shows
/// up as an HTTP `DELETE`. Records `(json-rpc method | HTTP verb, Authorization)`.
pub async fn start_mcp_peer(call_delay: Duration) -> (McpPeer, HeaderLog) {
    let peer = Peer {
        delay: call_delay,
        ..Default::default()
    };
    let calls = peer.calls.clone();
    let closed = Arc::new(AtomicUsize::new(0));
    let log: HeaderLog = Arc::new(Mutex::new(Vec::new()));
    let (captured, closes) = (log.clone(), closed.clone());
    let service = StreamableHttpService::new(
        move || Ok(peer.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_json_response(true),
    );
    let app = Router::new().route_service("/mcp", service).layer(from_fn(
        move |request: Request, next: Next| {
            let (captured, closes) = (captured.clone(), closes.clone());
            async move {
                let auth = request
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                if request.method() == axum::http::Method::DELETE {
                    closes.fetch_add(1, Ordering::SeqCst);
                    captured.lock().unwrap().push(("DELETE".into(), auth));
                    return next.run(request).await;
                }
                let (parts, body) = request.into_parts();
                let bytes = to_bytes(body, 64 * 1024).await.unwrap();
                let value: Value = serde_json::from_slice(&bytes).unwrap_or_default();
                let method = value["method"]
                    .as_str()
                    .unwrap_or(parts.method.as_str())
                    .to_string();
                captured.lock().unwrap().push((method, auth));
                next.run(Request::from_parts(parts, Body::from(bytes)))
                    .await
            }
        },
    ));
    let url = format!("{}/mcp", serve(app).await);
    (McpPeer { url, calls, closed }, log)
}

// ---------------------------------------------------------------- kernel/API

pub fn kernel_config(
    tmp: &tempfile::TempDir,
    llm_url: &str,
    mcp_url: &str,
    other_mcp: Option<&str>,
) -> KernelConfig {
    let mut servers = vec![McpServerConfigEntry {
        name: "appchat_probe".into(),
        transport: McpTransportEntry::Http {
            url: mcp_url.into(),
        },
        timeout_secs: 10,
        env: vec![],
        headers: vec![],
        turn_auth: true,
    }];
    if let Some(url) = other_mcp {
        servers.push(McpServerConfigEntry {
            name: "other".into(),
            transport: McpTransportEntry::Http { url: url.into() },
            timeout_secs: 10,
            env: vec![],
            headers: vec!["Authorization: Bearer static-other".into()],
            turn_auth: false,
        });
    }
    KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "openai".into(),
            model: "fake-model".into(),
            api_key_env: "OF2_002_FAKE_LLM_KEY".into(),
            base_url: Some(llm_url.into()),
            subprocess_timeout_secs: None,
        },
        mcp_servers: servers,
        ..KernelConfig::default()
    }
}

pub struct TestServer {
    pub base_url: String,
    pub state: Arc<AppState>,
    client: reqwest::Client,
    llm_url: String,
    tmp: Option<tempfile::TempDir>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

pub struct ShutdownHome(tempfile::TempDir);

impl ShutdownHome {
    /// AC7: the canary must not appear in any file of the kernel home
    /// (SQLite session/memory DB, JSONL mirrors, daily logs).
    pub fn assert_no_canary(self, canary: &str) -> Self {
        let mut stack = vec![self.0.path().to_path_buf()];
        let mut scanned = 0;
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Ok(bytes) = std::fs::read(&p) {
                    scanned += 1;
                    let hit = bytes.windows(canary.len()).any(|w| w == canary.as_bytes());
                    assert!(!hit, "canary found in {}", p.display());
                }
            }
        }
        assert!(scanned > 0, "home had no files to scan");
        self
    }
}

impl TestServer {
    pub async fn start(llm_url: &str, peer: &McpPeer, other: Option<&McpPeer>) -> Self {
        trace_log();
        std::env::set_var("OF2_002_FAKE_LLM_KEY", "fake-llm-key");
        let tmp = tempfile::tempdir().unwrap();
        let config = kernel_config(&tmp, llm_url, &peer.url, other.map(|o| o.url.as_str()));
        let kernel = Arc::new(OpenFangKernel::boot_with_config(config).expect("kernel boots"));
        kernel.set_self_handle();
        kernel.start_background_agents();
        let addr = "127.0.0.1:0".parse().unwrap();
        let (app, state) = openfang_api::server::build_router(kernel, addr).await;
        let server = Self {
            base_url: serve(app).await,
            state,
            client: reqwest::Client::new(),
            llm_url: llm_url.into(),
            tmp: Some(tmp),
        };
        if other.is_some() {
            let st = server.state.clone();
            wait_until(move || st.kernel.mcp_tools.lock().is_ok_and(|t| !t.is_empty())).await;
        }
        server
    }

    /// Second kernel (own home + own peer) hitting the same fake LLM.
    pub async fn start_sharing(first: &TestServer, peer: &McpPeer) -> Self {
        Self::start(&first.llm_url, peer, None).await
    }

    pub fn clone_client(&self) -> reqwest::Client {
        self.client.clone()
    }

    pub async fn post(&self, path: &str, body: &Value) -> (u16, String) {
        let resp = self
            .client
            .post(format!("{}{path}", self.base_url))
            .json(body)
            .send()
            .await
            .unwrap();
        (resp.status().as_u16(), resp.text().await.unwrap())
    }

    pub async fn spawn_agent(&self) -> String {
        let manifest = format!(
            r#"name = "turn-token-{}"
version = "0.1.0"
description = "t"
author = "t"
module = "builtin:chat"

[model]
provider = "openai"
model = "fake-model"
base_url = "{}"
api_key_env = "OF2_002_FAKE_LLM_KEY"
system_prompt = "test"

[capabilities]
tools = ["*"]
memory_read = ["*"]
memory_write = ["self.*"]
"#,
            uuid::Uuid::new_v4().simple(),
            self.llm_url
        );
        let (status, body) = self
            .post("/api/agents", &json!({"manifest_toml": manifest}))
            .await;
        assert_eq!(status, 201, "{body}");
        serde_json::from_str::<Value>(&body).unwrap()["agent_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    pub fn into_home_after_shutdown(mut self) -> ShutdownHome {
        self.state.kernel.shutdown();
        ShutdownHome(self.tmp.take().unwrap())
    }
}
