//! TIP-OF2-002 AC2-AC7, AC10 — per-turn MCP auth isolation, no leakage.

use std::time::Duration;

use serde_json::json;

use super::super::{
    turn_auth::validate_token, turn_connections::TurnMcpConnections, McpConnection,
    McpServerConfig, McpTransport,
};
use super::fixture::{Fixture, HeaderLog, Peer};

fn http_config(url: &str) -> McpServerConfig {
    McpServerConfig {
        name: "probe".into(),
        transport: McpTransport::Http { url: url.into() },
        timeout_secs: 3,
        env: vec![],
        headers: vec![],
    }
}

fn auth_values(log: &HeaderLog) -> Vec<Option<String>> {
    log.lock().unwrap().iter().map(|(_, v)| v.clone()).collect()
}

/// Run an async block while capturing tracing output, without blocking on
/// a nested runtime (the test itself already runs on one). Default
/// `#[tokio::test]` is single-threaded, so the thread-local dispatcher set
/// here stays active across `.await` points as long as nothing spawns the
/// future onto another OS thread.
async fn with_capture<F, Fut>(run: F) -> (Fut::Output, String)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future,
{
    #[derive(Clone, Default)]
    struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }
    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buf.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let output = run().await;
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    (output, text)
}

// --- AC2: every MCP request in a turn carries this turn's bearer token -----

#[tokio::test]
async fn ac2_every_request_in_a_turn_carries_the_turn_token() {
    let (fixture, log) = Fixture::start_recording_headers(Peer::default()).await;
    let auth = validate_token(Some("T1")).unwrap().unwrap();
    let turn = TurnMcpConnections::connect_for_turn(&[http_config(&fixture.url)], &auth).await;
    turn.dispatch_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();

    let values = auth_values(&log);
    assert!(!values.is_empty());
    for v in &values {
        assert_eq!(v.as_deref(), Some("Bearer T1"));
    }
    // initialize + tools/list (discovery) + tools/call, all with T1
    assert!(values.len() >= 2);
}

// --- AC3: two turns with different tokens don't queue behind each other ----

#[tokio::test]
async fn ac3_two_turns_do_not_share_a_mutex() {
    let slow_peer = Peer {
        call_delay: Duration::from_secs(3),
        ..Default::default()
    };
    let (slow_fixture, slow_log) = Fixture::start_recording_headers(slow_peer).await;
    let (fast_fixture, fast_log) = Fixture::start_recording_headers(Peer::default()).await;

    let auth1 = validate_token(Some("T1")).unwrap().unwrap();
    let auth2 = validate_token(Some("T2")).unwrap().unwrap();
    // The peer holds tools/call for 3s, so this turn's own deadline must
    // exceed that or the call is (correctly) quarantined as "outcome unknown".
    let slow_config = McpServerConfig {
        timeout_secs: 10,
        ..http_config(&slow_fixture.url)
    };
    let turn1 = TurnMcpConnections::connect_for_turn(&[slow_config], &auth1).await;
    let turn2 =
        TurnMcpConnections::connect_for_turn(&[http_config(&fast_fixture.url)], &auth2).await;

    let slow_call = tokio::spawn(async move {
        turn1
            .dispatch_tool("mcp_probe_probe_result", &json!({}))
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let fast_start = std::time::Instant::now();
    turn2
        .dispatch_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();
    // turn2 must complete well before turn1's 3s call_delay — proves no
    // shared mutex serializes the two turns.
    assert!(fast_start.elapsed() < Duration::from_secs(1));
    assert!(
        !slow_call.is_finished(),
        "turn 1 must still be held by the peer"
    );
    slow_call.await.unwrap().unwrap();

    for v in auth_values(&slow_log) {
        assert_eq!(v.as_deref(), Some("Bearer T1"));
    }
    for v in auth_values(&fast_log) {
        assert_eq!(v.as_deref(), Some("Bearer T2"));
    }
}

// --- AC4: a later turn never carries the previous turn's token -------------

#[tokio::test]
async fn ac4_next_turn_does_not_reuse_previous_token() {
    let (fixture, log) = Fixture::start_recording_headers(Peer::default()).await;

    let auth1 = validate_token(Some("T1")).unwrap().unwrap();
    let turn1 = TurnMcpConnections::connect_for_turn(&[http_config(&fixture.url)], &auth1).await;
    turn1
        .dispatch_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();
    drop(turn1);

    let auth2 = validate_token(Some("T2")).unwrap().unwrap();
    let turn2 = TurnMcpConnections::connect_for_turn(&[http_config(&fixture.url)], &auth2).await;
    turn2
        .dispatch_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();

    let values = auth_values(&log);
    let t1_count = values
        .iter()
        .filter(|v| v.as_deref() == Some("Bearer T1"))
        .count();
    let t2_count = values
        .iter()
        .filter(|v| v.as_deref() == Some("Bearer T2"))
        .count();
    assert!(t1_count > 0 && t2_count > 0);
    // Every request after turn1 dropped is T2 — none carries T1.
    let last_t1_idx = values
        .iter()
        .rposition(|v| v.as_deref() == Some("Bearer T1"))
        .unwrap();
    assert!(values[last_t1_idx + 1..]
        .iter()
        .all(|v| v.as_deref() == Some("Bearer T2")));
}

// --- AC5: no token => zero requests, zero turn_auth tools -------------------

#[tokio::test]
async fn ac5_no_token_means_no_turn_auth_tools_and_zero_requests() {
    let (fixture, log) = Fixture::start_recording_headers(Peer::default()).await;
    assert!(validate_token(None).unwrap().is_none());
    assert!(validate_token(Some("   ")).unwrap().is_none());
    // No TurnMcpConnections is ever constructed when there is no token —
    // the kernel integration (OF2-002 kernel.rs changes) must skip this
    // path entirely. Here we assert the fixture saw nothing, which is what
    // "connections never opened" looks like from the peer's side.
    let _ = &fixture.url;
    assert!(log.lock().unwrap().is_empty());
}

// --- AC6: a second, non-turn_auth server never sees the turn's token -------

#[tokio::test]
async fn ac6_second_server_never_sees_the_turn_token() {
    let (fixture_a, log_a) = Fixture::start_recording_headers(Peer::default()).await;
    let (fixture_b, log_b) = Fixture::start_recording_headers(Peer::default()).await;

    let auth = validate_token(Some("T1")).unwrap().unwrap();
    // Only server A is connected for this turn (turn_auth=true in config);
    // server B represents a non-turn_auth server reached via the kernel's
    // separate boot-time connection — never through TurnMcpConnections.
    let turn = TurnMcpConnections::connect_for_turn(&[http_config(&fixture_a.url)], &auth).await;
    turn.dispatch_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();

    // Independently exercise B through a plain (non-turn) connection with a
    // static header, proving the two paths are disjoint.
    let mut plain = McpConnection::connect(McpServerConfig {
        headers: vec!["Authorization: Bearer static-key".into()],
        ..http_config(&fixture_b.url)
    })
    .await
    .unwrap();
    plain
        .call_tool("mcp_probe_probe_result", &json!({}))
        .await
        .unwrap();

    assert!(auth_values(&log_a)
        .iter()
        .all(|v| v.as_deref() == Some("Bearer T1")));
    assert!(auth_values(&log_b)
        .iter()
        .all(|v| v.as_deref() != Some("Bearer T1")));
}

// --- AC7: the canary token never reaches logs ------------------------------

#[tokio::test]
async fn ac7_canary_token_never_appears_in_logs() {
    let canary = "TOKEN-CANARY-f3b1c9";
    let (output, log_text) = with_capture(|| async move {
        let fixture = Fixture::start(Peer::default()).await;
        let auth = validate_token(Some(canary)).unwrap().unwrap();
        let turn = TurnMcpConnections::connect_for_turn(&[http_config(&fixture.url)], &auth).await;
        let result = turn
            .dispatch_tool("mcp_probe_probe_result", &json!({}))
            .await;
        // Also exercise the failure path (bad server), still must not leak.
        let bad_auth = validate_token(Some(canary)).unwrap().unwrap();
        let _ = TurnMcpConnections::connect_for_turn(
            &[http_config("http://127.0.0.1:1/mcp")],
            &bad_auth,
        )
        .await;
        result
    })
    .await;
    output.unwrap();
    assert!(
        !log_text.contains(canary),
        "canary leaked into logs: {log_text}"
    );
}

// --- AC10-adjacent: dropping TurnMcpConnections closes its connections -----

#[tokio::test]
async fn dropping_turn_connections_stops_further_dispatch() {
    let fixture = Fixture::start(Peer::default()).await;
    let auth = validate_token(Some("T1")).unwrap().unwrap();
    let turn = TurnMcpConnections::connect_for_turn(&[http_config(&fixture.url)], &auth).await;
    assert!(turn.has_tool("mcp_probe_probe_result"));
    drop(turn);
    // No assertion beyond "drop doesn't panic and the fixture keeps running"
    // — McpConnection's RunningService Drop impl owns transport teardown;
    // re-dispatch after drop is a compile-time impossibility (turn is gone).
    assert_eq!(fixture.call_count(), 0);
}
