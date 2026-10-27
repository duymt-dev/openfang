use serde_json::{json, Value};
use tokio::sync::Mutex;

#[tokio::test]
async fn expired_session_does_not_reinitialize_and_replay_tool_request() {
    let (peer, initializes) = Fixture::expired_session().await;
    let mut connection = McpConnection::connect(peer.config()).await.unwrap();
    assert!(connection
        .call_tool("mcp_probe_probe_result", &json!({}))
        .await
        .is_err());
    assert!(connection
        .call_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap_err()
        .contains("quarantined"));
    assert_eq!(
        peer.call_count(),
        1,
        "HTTP404 must not replay a possible write"
    );
    assert_eq!(initializes.load(std::sync::atomic::Ordering::SeqCst), 1);
}

use super::{
    super::{dispatch_tool, McpConnection},
    fixture::{Fixture, Peer},
};

#[tokio::test]
async fn actual_client_preserves_error_and_structured_content_through_dispatch() {
    let peer = Fixture::start(Peer::default()).await;
    let connection = McpConnection::connect(peer.config()).await.unwrap();
    assert_eq!(connection.tools().len(), 1);
    assert_eq!(connection.tools()[0].name, "mcp_probe_probe_result");
    let connections = Mutex::new(vec![connection]);
    for error in [false, true, false] {
        let result = dispatch_tool(
            &connections,
            "mcp_probe_probe_result",
            &json!({"error":error}),
        )
        .await;
        assert_eq!(result.is_err(), error);
        let value: Value = serde_json::from_str(&result.unwrap_or_else(|e| e)).unwrap();
        assert_eq!(value["isError"], error);
        assert_eq!(value["structuredContent"]["ok"], !error);
    }
    assert_eq!(
        peer.call_count(),
        3,
        "one network call per invocation, no hidden retry"
    );
}

#[tokio::test]
async fn unknown_tools_and_non_object_args_never_reach_peer() {
    let peer = Fixture::start(Peer::default()).await;
    let mut connection = McpConnection::connect(peer.config()).await.unwrap();
    for name in [
        "mcp_probe_not_discovered",
        "probe-result",
        "mcp_other_probe_result",
    ] {
        assert!(connection
            .call_tool(name, &json!({}))
            .await
            .unwrap_err()
            .contains("not discovered"));
    }
    assert!(connection
        .call_tool("mcp_probe_probe_result", &json!([]))
        .await
        .unwrap_err()
        .contains("object"));
    assert_eq!(peer.call_count(), 0);
}

#[tokio::test]
async fn normalized_name_collision_aborts_discovery() {
    let peer = Fixture::start(Peer {
        collisions: true,
        ..Default::default()
    })
    .await;
    let result = McpConnection::connect(peer.config()).await;
    assert!(result.err().unwrap().contains("ambiguous"));
    assert_eq!(peer.call_count(), 0);
}

#[tokio::test]
async fn authenticated_redirect_is_not_followed() {
    let peer = Fixture::redirect().await;
    let mut config = peer.config();
    config.headers = vec!["Authorization: Bearer fixture-secret-must-not-leak".into()];
    let error = McpConnection::connect(config).await.err().unwrap();
    assert!(!error.contains("fixture-secret"));
    assert_eq!(
        peer.call_count(),
        0,
        "redirect target must not receive any request"
    );
}
