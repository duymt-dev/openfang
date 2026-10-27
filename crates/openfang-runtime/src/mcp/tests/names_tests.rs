use super::super::*;
use std::collections::HashMap;

#[test]
fn mcp_tool_namespacing() {
    assert_eq!(
        format_mcp_tool_name("github", "create_issue"),
        "mcp_github_create_issue"
    );
    assert_eq!(
        format_mcp_tool_name("my-server", "do_thing"),
        "mcp_my_server_do_thing"
    );
}

#[test]
fn detects_mcp_tool_namespace() {
    assert!(is_mcp_tool("mcp_github_create_issue"));
    assert!(!is_mcp_tool("file_read"));
    assert!(!is_mcp_tool(""));
}

#[test]
fn hyphenated_tool_name_preserved() {
    let namespaced = format_mcp_tool_name("sqlcl", "list-connections");
    assert_eq!(namespaced, "mcp_sqlcl_list_connections");

    let mut original_names = HashMap::new();
    original_names.insert(namespaced.clone(), "list-connections".to_string());

    let raw = original_names
        .get(&namespaced)
        .map(|s| s.as_str())
        .unwrap_or("list_connections");
    assert_eq!(raw, "list-connections");
}

#[test]
fn extracts_server_from_simple_namespace() {
    assert_eq!(
        extract_mcp_server("mcp_github_create_issue"),
        Some("github")
    );
    assert_eq!(extract_mcp_server("file_read"), None);
}

#[test]
fn extract_mcp_server_from_known_with_hyphens() {
    let servers = vec!["bocha-search", "github"];
    let tool = "mcp_bocha_search_bocha_web_search";
    assert_eq!(
        extract_mcp_server_from_known(tool, &servers),
        Some("bocha-search")
    );
    assert_eq!(
        extract_mcp_server_from_known("mcp_github_create_issue", &servers),
        Some("github")
    );
    assert_eq!(extract_mcp_server_from_known("file_read", &servers), None);
}

#[test]
fn extract_mcp_server_from_known_longest_match() {
    let servers = vec!["my-api", "my-api-v2"];
    assert_eq!(
        extract_mcp_server_from_known("mcp_my_api_v2_get_users", &servers),
        Some("my-api-v2")
    );
    assert_eq!(
        extract_mcp_server_from_known("mcp_my_api_list_items", &servers),
        Some("my-api")
    );
}
