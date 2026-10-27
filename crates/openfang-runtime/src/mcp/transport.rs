use http::{HeaderName, HeaderValue};
use rmcp::{model::ClientInfo, service::RunningService, RoleClient, ServiceExt};
use std::collections::HashMap;

/// Connect using stdio transport (subprocess).
pub(super) async fn connect_stdio(
    command: &str,
    args: &[String],
    env_whitelist: &[String],
    client_info: ClientInfo,
) -> Result<RunningService<RoleClient, ClientInfo>, String> {
    use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
    use tokio::process::Command;

    // Validate command path (no path traversal)
    if command.contains("..") {
        return Err("MCP command path contains '..': rejected".to_string());
    }

    let cmd_str = command.to_string();
    let args_vec: Vec<String> = args.to_vec();
    let env_list: Vec<String> = env_whitelist.to_vec();

    let transport = TokioChildProcess::new(Command::new(&cmd_str).configure(move |cmd| {
        for arg in &args_vec {
            cmd.arg(arg);
        }
        // Sandbox: clear environment, only pass whitelisted vars
        cmd.env_clear();
        for var_name in &env_list {
            if let Ok(val) = std::env::var(var_name) {
                cmd.env(var_name, val);
            }
        }
        // Always pass PATH for binary resolution
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        // Some stdio MCP servers launched via node/npx require a usable home
        // directory even when they do not declare any explicit secret env vars.
        for var in &["HOME", "TMP", "TEMP"] {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }
        // On Windows, npm/node need extra vars
        if cfg!(windows) {
            for var in &[
                "APPDATA",
                "LOCALAPPDATA",
                "USERPROFILE",
                "SystemRoot",
                "HOMEDRIVE",
                "HOMEPATH",
            ] {
                if let Ok(val) = std::env::var(var) {
                    cmd.env(var, val);
                }
            }
        }
    }))
    .map_err(|e| format!("Failed to spawn MCP server '{cmd_str}': {e}"))?;

    let client = client_info
        .serve(transport)
        .await
        .map_err(|e| format!("MCP stdio handshake failed: {e}"))?;

    Ok(client)
}

/// Connect using Streamable HTTP transport (or SSE fallback via the same endpoint).
///
/// The `rmcp` SDK's `StreamableHttpClientTransport` handles the full
/// Streamable HTTP protocol: Accept headers, Mcp-Session-Id tracking,
/// SSE stream parsing, and content-type negotiation.
pub(super) async fn connect_http(
    url: &str,
    headers: &[String],
    client_info: ClientInfo,
) -> Result<RunningService<RoleClient, ClientInfo>, String> {
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use rmcp::transport::StreamableHttpClientTransport;

    check_ssrf(url)?;

    let custom_headers = parse_headers(headers)?;

    // rmcp 1.3+ marks StreamableHttpClientTransportConfig as #[non_exhaustive].
    // Use the official builder API (credit: @jefflower, PR #986).
    let config = StreamableHttpClientTransportConfig::with_uri(url)
        .custom_headers(custom_headers)
        .reinit_on_expired_session(false);

    let http_client = reqwest_mcp::Client::builder()
        .redirect(reqwest_mcp::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|_| "MCP HTTP client initialization failed".to_string())?;
    let transport = StreamableHttpClientTransport::with_client(http_client, config);

    let client = client_info
        .serve(transport)
        .await
        .map_err(|_| "MCP HTTP connection failed".to_string())?;

    Ok(client)
}

pub(super) fn parse_headers(
    headers: &[String],
) -> Result<HashMap<HeaderName, HeaderValue>, String> {
    let mut parsed = HashMap::new();
    for header in headers {
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| "Invalid MCP HTTP header configuration".to_string())?;
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| "Invalid MCP HTTP header configuration".to_string())?;
        let mut value = HeaderValue::from_str(value.trim())
            .map_err(|_| "Invalid MCP HTTP header configuration".to_string())?;
        value.set_sensitive(true);
        if parsed.insert(name, value).is_some() {
            return Err("Duplicate MCP HTTP header configuration".into());
        }
    }
    Ok(parsed)
}

/// Basic SSRF check: reject obviously private/metadata URLs.
fn check_ssrf(url: &str) -> Result<(), String> {
    let lower = url.to_lowercase();
    if lower.contains("169.254.169.254") || lower.contains("metadata.google") {
        return Err("SSRF: MCP URL targets metadata endpoint".to_string());
    }
    Ok(())
}
