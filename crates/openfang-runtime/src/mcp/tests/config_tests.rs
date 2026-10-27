use super::super::*;

#[test]
fn config_debug_and_invalid_headers_do_not_leak_values() {
    let config = McpServerConfig {
        name: "probe".into(),
        transport: McpTransport::Http {
            url: "http://127.0.0.1/mcp?private=secret-marker".into(),
        },
        timeout_secs: 1,
        env: vec![],
        headers: vec!["Authorization: Bearer secret-marker".into()],
    };
    assert!(!format!("{config:?}").contains("secret-marker"));
    let valid = super::super::transport::parse_headers(&config.headers).unwrap();
    assert!(valid[&http::header::AUTHORIZATION].is_sensitive());
    for headers in [
        vec!["secret-marker-missing-colon".into()],
        vec!["Bad Header: secret-marker".into()],
        vec!["Authorization: secret-marker\r\nInjected: yes".into()],
        vec![
            "Authorization: secret-marker".into(),
            "authorization: duplicate".into(),
        ],
    ] {
        let error = super::super::transport::parse_headers(&headers).unwrap_err();
        assert!(!error.contains("secret-marker"));
    }
}

#[test]
fn mcp_transport_config_serde() {
    let config = McpServerConfig {
        name: "github".to_string(),
        transport: McpTransport::Stdio {
            command: "npx".to_string(),
            args: vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-github".to_string(),
            ],
        },
        timeout_secs: 30,
        env: vec!["GITHUB_PERSONAL_ACCESS_TOKEN".to_string()],
        headers: vec![],
    };

    let json = serde_json::to_string(&config).unwrap();
    let back: McpServerConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "github");
    assert_eq!(back.timeout_secs, 30);
    assert_eq!(back.env, vec!["GITHUB_PERSONAL_ACCESS_TOKEN"]);

    match back.transport {
        McpTransport::Stdio { command, args } => {
            assert_eq!(command, "npx");
            assert_eq!(args.len(), 2);
        }
        _ => panic!("Expected Stdio transport"),
    }

    // SSE variant
    let sse_config = McpServerConfig {
        name: "test".to_string(),
        transport: McpTransport::Sse {
            url: "https://example.com/mcp".to_string(),
        },
        timeout_secs: 60,
        env: vec![],
        headers: vec![],
    };
    let json = serde_json::to_string(&sse_config).unwrap();
    let back: McpServerConfig = serde_json::from_str(&json).unwrap();
    match back.transport {
        McpTransport::Sse { url } => assert_eq!(url, "https://example.com/mcp"),
        _ => panic!("Expected SSE transport"),
    }

    // HTTP (Streamable HTTP) variant
    let http_config = McpServerConfig {
        name: "atlassian".to_string(),
        transport: McpTransport::Http {
            url: "https://mcp.atlassian.com/v1/mcp".to_string(),
        },
        timeout_secs: 120,
        env: vec![],
        headers: vec!["Authorization: Bearer test-token-456".to_string()],
    };
    let json = serde_json::to_string(&http_config).unwrap();
    let back: McpServerConfig = serde_json::from_str(&json).unwrap();
    match back.transport {
        McpTransport::Http { url } => {
            assert_eq!(url, "https://mcp.atlassian.com/v1/mcp")
        }
        _ => panic!("Expected Http transport"),
    }
}
