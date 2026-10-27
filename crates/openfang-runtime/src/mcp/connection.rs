use std::{collections::HashMap, time::Duration};

use openfang_types::tool::ToolDefinition;
use rmcp::{
    model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation},
    service::RunningService,
    RoleClient,
};
use tracing::{debug, info};

use super::{
    config::McpTransport, names::format_mcp_tool_name, result, transport, McpServerConfig,
};

/// An authenticated transport and its discovered tool mappings.
pub struct McpConnection {
    config: McpServerConfig,
    tools: Vec<ToolDefinition>,
    original_names: HashMap<String, String>,
    client: RunningService<RoleClient, ClientInfo>,
    // Set BEFORE awaiting a call. Dropping an interrupted future must not silently
    // make the same connection available for more tool effects or blind retries.
    call_interrupted: bool,
}

impl McpConnection {
    /// Connect and discover under a single configured deadline.
    pub async fn connect(config: McpServerConfig) -> Result<Self, String> {
        if config.timeout_secs == 0 {
            return Err("MCP timeout must be greater than zero".into());
        }
        tokio::time::timeout(
            Duration::from_secs(config.timeout_secs),
            Self::connect_inner(config),
        )
        .await
        .map_err(|_| "MCP connect/discovery deadline exceeded".to_string())?
    }

    async fn connect_inner(config: McpServerConfig) -> Result<Self, String> {
        let client_info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("openfang", env!("CARGO_PKG_VERSION")),
        );
        let client = match &config.transport {
            McpTransport::Stdio { command, args } => {
                transport::connect_stdio(command, args, &config.env, client_info).await?
            }
            McpTransport::Sse { url } | McpTransport::Http { url } => {
                transport::connect_http(url, &config.headers, client_info).await?
            }
        };
        let mut conn = Self {
            config,
            tools: Vec::new(),
            original_names: HashMap::new(),
            client,
            call_interrupted: false,
        };
        conn.discover_tools().await?;
        info!(server = %conn.config.name, tools = conn.tools.len(), "MCP server connected");
        Ok(conn)
    }

    async fn discover_tools(&mut self) -> Result<(), String> {
        let tools = self
            .client
            .list_all_tools()
            .await
            .map_err(|_| "MCP tool discovery failed".to_string())?;
        for tool in tools {
            let namespaced = format_mcp_tool_name(&self.config.name, &tool.name);
            if self.original_names.contains_key(&namespaced) {
                return Err("MCP discovery contains ambiguous normalized tool names".into());
            }
            let input_schema = serde_json::to_value(&tool.input_schema)
                .map_err(|_| "MCP tool schema serialization failed".to_string())?;
            self.original_names
                .insert(namespaced.clone(), tool.name.to_string());
            self.tools.push(ToolDefinition {
                name: namespaced,
                description: format!(
                    "[MCP:{}] {}",
                    self.config.name,
                    tool.description.as_deref().unwrap_or("")
                ),
                input_schema,
            });
        }
        Ok(())
    }

    /// Call a discovered namespaced tool. Err propagates into ToolResult.is_error.
    /// Timeout/transport interruption is unknown, not permission to retry.
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: &serde_json::Value,
    ) -> Result<String, String> {
        if self.call_interrupted {
            return Err(
                "MCP connection quarantined after interrupted call; reconcile before reconnecting"
                    .into(),
            );
        }
        let raw_name = self
            .original_names
            .get(name)
            .ok_or_else(|| "MCP tool was not discovered on this connection".to_string())?
            .clone();
        let args = arguments
            .as_object()
            .ok_or_else(|| "MCP tool arguments must be an object".to_string())?
            .clone();
        let params = CallToolRequestParams::new(raw_name.clone()).with_arguments(args);
        debug!(tool = %raw_name, server = %self.config.name, "MCP tool call");
        self.call_interrupted = true;
        let response = tokio::time::timeout(
            Duration::from_secs(self.config.timeout_secs),
            self.client.call_tool(params),
        )
        .await
        .map_err(|_| "MCP tool deadline exceeded; outcome unknown, do not retry".to_string())?
        .map_err(|_| {
            "MCP tool transport/protocol failure; outcome unknown, do not retry".to_string()
        })?;
        self.call_interrupted = false;
        result::render_result(response)
    }

    /// Discovered tool definitions (not a substitute for execution authorization).
    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    /// Configured server name, used for namespace resolution.
    pub fn name(&self) -> &str {
        &self.config.name
    }
}
