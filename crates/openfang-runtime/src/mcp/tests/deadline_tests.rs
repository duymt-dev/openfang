use serde_json::json;
use std::time::Duration;
use tokio::sync::Mutex;

use super::{
    super::{dispatch::dispatch_with_budget, McpConnection},
    fixture::{Fixture, Peer},
};

#[tokio::test(start_paused = true)]
async fn mutex_wait_is_inside_dispatch_deadline() {
    let connections = Mutex::new(Vec::<McpConnection>::new());
    let guard = connections.lock().await;
    let start = tokio::time::Instant::now();
    let result = dispatch_with_budget(
        &connections,
        "mcp_probe_probe_result",
        &json!({}),
        Duration::from_secs(1),
    )
    .await;
    assert!(result.unwrap_err().contains("lock wait + call"));
    assert_eq!(start.elapsed(), Duration::from_secs(1));
    drop(guard);
    assert!(connections.try_lock().is_ok());
}

#[tokio::test]
async fn direct_call_deadline_quarantines_connection_without_retry() {
    let peer = Fixture::start(Peer {
        call_delay: Duration::from_secs(10),
        ..Default::default()
    })
    .await;
    let mut connection = McpConnection::connect(peer.config()).await.unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(3),
        connection.call_tool("mcp_probe_probe_result", &json!({})),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.contains("deadline exceeded"));
    assert!(connection
        .call_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap_err()
        .contains("quarantined"));
    assert_eq!(peer.call_count(), 1);
}

#[tokio::test]
async fn outer_dispatch_cancellation_also_quarantines_and_releases_mutex() {
    let peer = Fixture::start(Peer {
        call_delay: Duration::from_secs(10),
        ..Default::default()
    })
    .await;
    let mut config = peer.config();
    config.timeout_secs = 10;
    let connection = McpConnection::connect(config).await.unwrap();
    let connections = Mutex::new(vec![connection]);
    let error = dispatch_with_budget(
        &connections,
        "mcp_probe_probe_result",
        &json!({}),
        Duration::from_millis(250),
    )
    .await
    .unwrap_err();
    assert!(error.contains("lock wait + call"));
    let mut guard = connections
        .try_lock()
        .expect("cancelled dispatch releases mutex");
    assert!(guard[0]
        .call_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap_err()
        .contains("quarantined"));
    assert_eq!(peer.call_count(), 1);
}

#[tokio::test]
async fn handshake_and_discovery_are_bounded() {
    for peer in [
        Fixture::stalled_handshake().await,
        Fixture::start(Peer {
            list_delay: Duration::from_secs(10),
            ..Default::default()
        })
        .await,
    ] {
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            McpConnection::connect(peer.config()),
        )
        .await
        .unwrap()
        .err()
        .unwrap();
        assert!(error.contains("connect/discovery deadline"));
        assert_eq!(peer.call_count(), 0);
    }
}

#[tokio::test]
async fn zero_timeout_is_rejected_before_connect() {
    let peer = Fixture::start(Peer::default()).await;
    let mut config = peer.config();
    config.timeout_secs = 0;
    assert!(McpConnection::connect(config)
        .await
        .err()
        .unwrap()
        .contains("greater than zero"));
    assert_eq!(peer.call_count(), 0);
}
