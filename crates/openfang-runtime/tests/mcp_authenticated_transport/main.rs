mod guard;
mod peer;
mod policy;

use guard::ORIGIN;
use openfang_runtime::mcp::{McpConnection, McpServerConfig, McpTransport};
use peer::{Fixture, TOOL};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;

#[tokio::test]
async fn real_client_authenticates_entire_lifecycle_with_absent_or_exact_origin() {
    for origin in [None, Some(ORIGIN)] {
        let fixture = Fixture::start().await;
        let mut client = fixture.connect(origin).await.unwrap();
        assert_eq!(client.tools().len(), 1);
        assert_eq!(client.tools()[0].name, TOOL);
        let response = client.call_tool(TOOL, &json!({})).await.unwrap();
        let value: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["structuredContent"]["probe"], "protocol-only");
        let methods = fixture.policy.accepted.lock().unwrap();
        for method in [
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/call",
        ] {
            assert_eq!(
                methods.iter().filter(|m| *m == method).count(),
                1,
                "{methods:?}"
            );
        }
        assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn real_client_cannot_connect_with_disallowed_origin() {
    let fixture = Fixture::start().await;
    assert_eq!(
        fixture.connect(Some("null")).await.err().unwrap(),
        "MCP HTTP connection failed"
    );
    assert!(fixture.policy.accepted.lock().unwrap().is_empty());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn fixture_revocation_is_checked_after_discovery_without_replay() {
    let fixture = Fixture::start().await;
    let mut client = fixture.connect(None).await.unwrap();
    fixture.policy.revoked.store(true, Ordering::SeqCst);
    let before = fixture.policy.requests.load(Ordering::SeqCst);
    let error = client.call_tool(TOOL, &json!({})).await.unwrap_err();
    assert!(error.contains("outcome unknown"), "{error}");
    assert!(client
        .call_tool(TOOL, &json!({}))
        .await
        .unwrap_err()
        .contains("quarantined"));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.policy.requests.load(Ordering::SeqCst), before + 1);
    let accepted = fixture.policy.accepted.lock().unwrap();
    assert_eq!(accepted.iter().filter(|m| *m == "initialize").count(), 1);
    assert!(!accepted.iter().any(|m| m == "tools/call"));
}

#[tokio::test]
async fn real_client_rejects_missing_or_invalid_bearer_without_sensitive_error() {
    let fixture = Fixture::start().await;
    for headers in [vec![], vec!["Authorization: Bearer bad-fixture".into()]] {
        let error = McpConnection::connect(McpServerConfig {
            name: "guard".into(),
            transport: McpTransport::Http {
                url: fixture.url.clone(),
            },
            timeout_secs: 3,
            env: vec![],
            headers,
        })
        .await
        .err()
        .unwrap();
        assert_eq!(error, "MCP HTTP connection failed");
    }
    assert!(fixture.policy.accepted.lock().unwrap().is_empty());
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}
