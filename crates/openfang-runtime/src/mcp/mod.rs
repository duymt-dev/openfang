//! MCP client transport, discovery and bounded execution. No business rules.

mod config;
mod connection;
mod dispatch;
mod names;
mod result;
mod transport;

pub use config::{McpServerConfig, McpTransport};
pub use connection::McpConnection;
pub use dispatch::dispatch_tool;
pub use names::{
    extract_mcp_server, extract_mcp_server_from_known, format_mcp_tool_name, is_mcp_tool,
    normalize_name,
};

#[cfg(test)]
mod tests;
