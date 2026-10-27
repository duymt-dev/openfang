/// Format a namespaced MCP tool name: `mcp_{server}_{tool}`.
pub fn format_mcp_tool_name(server: &str, tool: &str) -> String {
    format!("mcp_{}_{}", normalize_name(server), normalize_name(tool))
}

/// Check if a tool name is an MCP-namespaced tool.
pub fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_")
}

/// Extract server name from an MCP tool name.
///
/// Falls back to first-underscore heuristic, but prefer
/// `extract_mcp_server_from_known()` which handles server names containing
/// hyphens (normalized to underscores) correctly.
pub fn extract_mcp_server(tool_name: &str) -> Option<&str> {
    if !tool_name.starts_with("mcp_") {
        return None;
    }
    let rest = &tool_name[4..];
    rest.find('_').map(|pos| &rest[..pos])
}

/// Extract the original server name by matching against known server names.
///
/// This handles server names with hyphens (e.g. "bocha-search") correctly —
/// the normalized prefix "mcp_bocha_search_" is matched against each known
/// server's normalized name, returning the original (unhyphenated) name.
pub fn extract_mcp_server_from_known<'a>(
    tool_name: &str,
    server_names: &[&'a str],
) -> Option<&'a str> {
    if !tool_name.starts_with("mcp_") {
        return None;
    }
    // Sort by length descending so longer (more specific) names match first
    let mut sorted: Vec<&&str> = server_names.iter().collect();
    sorted.sort_by_key(|a| std::cmp::Reverse(a.len()));
    for name in sorted {
        let prefix = format!("mcp_{}_", normalize_name(name));
        if tool_name.starts_with(&prefix) {
            return Some(name);
        }
    }
    None
}

/// Normalize a name for use in tool namespacing (lowercase, replace hyphens).
pub fn normalize_name(name: &str) -> String {
    name.to_lowercase().replace('-', "_")
}
