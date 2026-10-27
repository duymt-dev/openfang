use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;

use super::{extract_mcp_server_from_known, McpConnection};

/// Pilot execution cap includes queueing for the shared MCP mutex, not just HTTP.
pub async fn dispatch_tool(
    connections: &Mutex<Vec<McpConnection>>,
    name: &str,
    input: &Value,
) -> Result<String, String> {
    dispatch_with_budget(connections, name, input, Duration::from_secs(15)).await
}

pub(super) async fn dispatch_with_budget(
    connections: &Mutex<Vec<McpConnection>>,
    name: &str,
    input: &Value,
    budget: Duration,
) -> Result<String, String> {
    tokio::time::timeout(budget, async {
        let mut connections = connections.lock().await;
        let names: Vec<&str> = connections
            .iter()
            .map(|connection| connection.name())
            .collect();
        let server = extract_mcp_server_from_known(name, &names)
            .ok_or_else(|| "MCP server not connected for this tool".to_string())?
            .to_owned();
        let connection = connections
            .iter_mut()
            .find(|connection| connection.name() == server)
            .ok_or_else(|| "MCP server not connected for this tool".to_string())?;
        connection.call_tool(name, input).await
    })
    .await
    .map_err(|_| {
        "MCP dispatch deadline exceeded (lock wait + call); outcome unknown, do not retry"
            .to_string()
    })?
}
