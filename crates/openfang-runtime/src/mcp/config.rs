use serde::{Deserialize, Serialize};

/// Configuration for an MCP server connection.
#[derive(Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Display name for this server (used in tool namespacing).
    pub name: String,
    /// Transport configuration.
    pub transport: McpTransport,
    /// Request timeout in seconds (default: 60).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Environment variables to pass through to the subprocess (sandboxed).
    #[serde(default)]
    pub env: Vec<String>,
    /// Extra HTTP headers to send with every SSE / Streamable-HTTP request.
    /// Each entry is `"Header-Name: value"`.  Useful for authentication
    /// (`Authorization: Bearer <token>`), API keys (`X-Api-Key: ...`),
    /// or any custom headers required by a remote MCP server.
    #[serde(default)]
    pub headers: Vec<String>,
}

fn default_timeout() -> u64 {
    60
}

impl std::fmt::Debug for McpServerConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Sse { .. } => "sse",
            McpTransport::Http { .. } => "http",
        };
        formatter
            .debug_struct("McpServerConfig")
            .field("name", &self.name)
            .field("transport_kind", &kind)
            .field("timeout_secs", &self.timeout_secs)
            .field("headers", &"[REDACTED]")
            .field("env_count", &self.env.len())
            .finish_non_exhaustive()
    }
}

/// Transport type for MCP server connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    /// Subprocess with JSON-RPC over stdin/stdout.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Deprecated HTTP+SSE transport (protocol version 2024-11-05).
    /// Uses POST for sending and SSE for receiving.
    Sse { url: String },
    /// Streamable HTTP transport (MCP 2025-03-26+).
    /// Single endpoint, client MUST send Accept: application/json, text/event-stream.
    /// Server responds with either JSON or SSE stream.
    /// Supports Mcp-Session-Id for session management.
    Http { url: String },
}
