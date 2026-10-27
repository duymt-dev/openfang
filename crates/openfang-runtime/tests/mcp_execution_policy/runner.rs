use openfang_runtime::{mcp::McpConnection, tool_runner::execute_tool};
use openfang_types::tool::ToolResult;
use serde_json::Value;
use tokio::sync::Mutex;

pub async fn invoke(
    name: &str,
    input: Value,
    allowed: &[String],
    connections: Option<&Mutex<Vec<McpConnection>>>,
) -> ToolResult {
    execute_tool(
        "policy-call",
        name,
        &input,
        None,
        Some(allowed),
        None,
        None,
        connections,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
}

pub fn assert_denied(result: ToolResult) {
    assert!(result.is_error, "unexpected success: {}", result.content);
    assert!(
        result.content.starts_with("Permission denied:"),
        "{}",
        result.content
    );
    assert_eq!(result.tool_use_id, "policy-call");
}
