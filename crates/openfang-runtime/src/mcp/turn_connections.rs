//! Per-turn MCP connections for `turn_auth = true` servers (TIP-OF2-002).
//!
//! Each turn that supplies `appchat_task_token` gets its own connection set,
//! opened fresh and dropped at the end of the turn — never cached, never
//! shared with `kernel.mcp_connections`, never reused by a later turn even
//! on the same agent/session. The mutex here guards only *this* turn's
//! connections, so two turns with different tokens never queue behind
//! each other (unlike the boot-time shared `kernel.mcp_connections` mutex).

use openfang_types::agent::{AgentManifest, AgentMode};
use openfang_types::tool::ToolDefinition;
use tokio::sync::Mutex;

use super::{
    dispatch::dispatch_with_budget, names::normalize_name, turn_auth::TurnMcpAuth, McpConnection,
    McpServerConfig,
};

/// A turn-scoped set of authenticated MCP connections. Connections are
/// dropped (closing the transport) when this value is dropped — at the end
/// of the turn, on error, or on `stop_agent_run` aborting the owning task.
pub struct TurnMcpConnections {
    connections: Mutex<Vec<McpConnection>>,
    /// Tool definitions discovered at connect time, snapshotted here so
    /// callers can list them synchronously. Kept separate from
    /// `kernel.mcp_tools` so turn_auth tools are invisible to turns that
    /// supplied no token (SPEC business rule 3/6).
    tools: Vec<ToolDefinition>,
}

impl TurnMcpConnections {
    /// Open one connection per `turn_auth = true` server, each carrying this
    /// turn's `Authorization: Bearer <token>` header. A server that fails to
    /// connect is skipped (its tools are simply absent for this turn) rather
    /// than failing the whole turn — matching the Error Handling table.
    pub async fn connect_for_turn(configs: &[McpServerConfig], auth: &TurnMcpAuth) -> Self {
        let mut connections = Vec::with_capacity(configs.len());
        let mut tools = Vec::new();

        for base_config in configs {
            let (header_name, header_value) = match auth.bearer_header() {
                Ok(pair) => pair,
                Err(_) => continue, // malformed token already rejected at the API layer
            };
            let header_line = format!(
                "{}: {}",
                header_name.as_str(),
                header_value.to_str().unwrap_or_default()
            );
            let turn_config = McpServerConfig {
                name: base_config.name.clone(),
                transport: base_config.transport.clone(),
                timeout_secs: base_config.timeout_secs,
                env: base_config.env.clone(),
                headers: vec![header_line],
            };
            match McpConnection::connect(turn_config).await {
                Ok(conn) => {
                    tools.extend(conn.tools().iter().cloned());
                    connections.push(conn);
                }
                Err(_) => {
                    // "MCP turn connection failed" per SPEC business rule 7 —
                    // never include the server/URL/token in this message.
                    tracing::warn!(
                        server = %base_config.name,
                        "MCP turn connection failed"
                    );
                }
            }
        }

        Self {
            connections: Mutex::new(connections),
            tools,
        }
    }

    /// Tool definitions discovered on this turn's connections only — never
    /// merged into the kernel-wide MCP tool cache.
    pub fn tool_definitions(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Whether a namespaced tool name was discovered on this turn's
    /// connections (used to decide whether `dispatch_tool` should be tried
    /// before falling back to the kernel-wide connections).
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }

    /// Dispatch a tool call against this turn's connections. Uses the same
    /// budget/quarantine semantics as the kernel-wide dispatcher — no
    /// retries, "outcome unknown" on timeout/transport failure.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<String, String> {
        dispatch_with_budget(
            &self.connections,
            name,
            input,
            std::time::Duration::from_secs(15),
        )
        .await
    }
}

/// Which `turn_auth` servers this agent may reach: the manifest's
/// `mcp_servers` allowlist, same rule as kernel-wide MCP (empty = all).
pub fn servers_for_agent(
    configs: Vec<McpServerConfig>,
    manifest: &AgentManifest,
) -> Vec<McpServerConfig> {
    if manifest.mcp_servers.is_empty() {
        return configs;
    }
    let allowed: Vec<String> = manifest
        .mcp_servers
        .iter()
        .map(|s| normalize_name(s))
        .collect();
    configs
        .into_iter()
        .filter(|c| allowed.contains(&normalize_name(&c.name)))
        .collect()
}

/// Apply the same per-agent tool filters the kernel applies to kernel-wide
/// MCP tools (declared `capabilities.tools`, `tool_allowlist`/`tool_blocklist`,
/// agent mode), so a token never widens what an agent may call.
pub fn tools_for_agent(
    turn_tools: &[ToolDefinition],
    manifest: &AgentManifest,
    mode: &AgentMode,
) -> Vec<ToolDefinition> {
    let declared = &manifest.capabilities.tools;
    let unrestricted = declared.is_empty() || declared.iter().any(|t| t == "*");
    let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
    let tools: Vec<ToolDefinition> = turn_tools
        .iter()
        .filter(|t| unrestricted || declared.iter().any(|d| d == &t.name))
        .filter(|t| {
            manifest.tool_allowlist.is_empty()
                || manifest.tool_allowlist.iter().any(|a| eq(a, &t.name))
        })
        .filter(|t| !manifest.tool_blocklist.iter().any(|b| eq(b, &t.name)))
        .cloned()
        .collect();
    mode.filter_tools(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tests::fixture::{Fixture, Peer};

    fn turn_config(url: &str) -> McpServerConfig {
        McpServerConfig {
            name: "probe".into(),
            transport: super::super::McpTransport::Http { url: url.into() },
            timeout_secs: 3,
            env: vec![],
            headers: vec![],
        }
    }

    #[tokio::test]
    async fn opens_connection_with_bearer_header_and_discovers_tools() {
        let fixture = Fixture::start(Peer::default()).await;
        let auth = super::super::turn_auth::validate_token(Some("T1-canary"))
            .unwrap()
            .unwrap();
        let turn = TurnMcpConnections::connect_for_turn(&[turn_config(&fixture.url)], &auth).await;
        let tools = turn.tool_definitions();
        assert!(!tools.is_empty());
        assert!(turn.has_tool("mcp_probe_probe_result"));
    }

    #[tokio::test]
    async fn no_tools_when_connect_fails() {
        // Deliberately unroutable local port — connect_for_turn must not
        // fail the turn, only skip that server's tools.
        let bad = turn_config("http://127.0.0.1:1/mcp");
        let auth = super::super::turn_auth::validate_token(Some("T1"))
            .unwrap()
            .unwrap();
        let turn = TurnMcpConnections::connect_for_turn(&[bad], &auth).await;
        assert!(turn.tool_definitions().is_empty());
        assert!(!turn.has_tool("mcp_probe_probe_result"));
    }

    #[tokio::test]
    async fn dispatch_tool_reaches_the_turn_connection() {
        let fixture = Fixture::start(Peer::default()).await;
        let auth = super::super::turn_auth::validate_token(Some("T1-canary"))
            .unwrap()
            .unwrap();
        let turn = TurnMcpConnections::connect_for_turn(&[turn_config(&fixture.url)], &auth).await;
        let result = turn
            .dispatch_tool("mcp_probe_probe_result", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
        assert_eq!(fixture.call_count(), 1);
    }
}
