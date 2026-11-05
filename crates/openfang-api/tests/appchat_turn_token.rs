//! TIP-OF2-002 end-to-end API tests for `appchat_task_token`.
//!
//! Real kernel + real axum routes + a fake OpenAI-compatible LLM that asks
//! for the `turn_auth` MCP tool, + an in-process MCP peer recording the
//! `Authorization` header of every request. No network, no API key.
//!
//! Run: cargo +1.94.0 test --locked -p openfang-api --test appchat_turn_token

mod support;

use std::sync::atomic::Ordering;
use std::time::Duration;

use openfang_kernel::OpenFangKernel;
use openfang_types::config::McpTransportEntry;
use support::*;

const TOOL: &str = "mcp_appchat_probe_probe_result";

#[tokio::test]
async fn ac2_ac7_message_turn_sends_token_on_every_mcp_request_and_never_leaks() {
    let canary = format!("TOKEN-CANARY-{}", uuid::Uuid::new_v4());
    let (llm, llm_seen) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::ZERO).await;
    let (other, other_log) = start_mcp_peer(Duration::ZERO).await;
    let server = TestServer::start(&llm, &peer, Some(&other)).await;
    let agent = server.spawn_agent().await;

    let (status, body) = server
        .post(&format!("/api/agents/{agent}/message"), &msg(Some(&canary)))
        .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("TOOL_DONE"),
        "LLM saw the tool result: {body}"
    );

    let entries = log.lock().unwrap().clone();
    let methods: Vec<&str> = entries.iter().map(|(m, _)| m.as_str()).collect();
    for m in ["initialize", "tools/list", "tools/call"] {
        assert!(methods.contains(&m), "peer never saw {m}: {methods:?}");
    }
    let want = format!("Bearer {canary}");
    for (m, auth) in &entries {
        assert_eq!(
            auth.as_deref(),
            Some(want.as_str()),
            "{m} missing turn token"
        );
    }
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);

    // AC6: the static (non-turn_auth) server never receives the turn token.
    assert!(other_log
        .lock()
        .unwrap()
        .iter()
        .all(|(_, a)| a.as_deref() == Some("Bearer static-other")));

    // AC7: not in the LLM request, response body, nor the SQLite home.
    assert!(!llm_seen.lock().unwrap().iter().any(|r| r.contains(&canary)));
    assert!(!body.contains(&canary));
    drop(server.into_home_after_shutdown().assert_no_canary(&canary));
    assert_logs_free_of(&canary);
}

#[tokio::test]
async fn ac2_ac7_stream_turn_sends_token_and_sse_body_never_contains_it() {
    let canary = format!("TOKEN-CANARY-{}", uuid::Uuid::new_v4());
    let (llm, llm_seen) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::ZERO).await;
    let server = TestServer::start(&llm, &peer, None).await;
    let agent = server.spawn_agent().await;

    let (status, sse) = server
        .post(
            &format!("/api/agents/{agent}/message/stream"),
            &msg(Some(&canary)),
        )
        .await;
    assert_eq!(status, 200);
    assert!(
        sse.contains("TOOL_DONE"),
        "stream finished the tool turn: {sse}"
    );
    assert!(!sse.contains(&canary));
    assert_eq!(peer.calls.load(Ordering::SeqCst), 1);
    let want = format!("Bearer {canary}");
    let entries = log.lock().unwrap().clone();
    assert!(entries.iter().any(|(m, _)| m == "tools/call"));
    assert!(entries
        .iter()
        .all(|(_, a)| a.as_deref() == Some(want.as_str())));
    assert!(!llm_seen.lock().unwrap().iter().any(|r| r.contains(&canary)));
    drop(server.into_home_after_shutdown().assert_no_canary(&canary));
    assert_logs_free_of(&canary);
}

#[tokio::test]
async fn ac4_next_turn_opens_a_new_connection_with_only_its_own_token() {
    let (llm, _) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::ZERO).await;
    let server = TestServer::start(&llm, &peer, None).await;
    let agent = server.spawn_agent().await;
    let url = format!("/api/agents/{agent}/message");

    assert_eq!(server.post(&url, &msg(Some("T1"))).await.0, 200);
    let split = log.lock().unwrap().len();
    assert_eq!(server.post(&url, &msg(Some("T2"))).await.0, 200);

    let entries = log.lock().unwrap().clone();
    let (first, second) = entries.split_at(split);
    assert!(first.iter().all(|(_, a)| a.as_deref() == Some("Bearer T1")));
    assert!(
        second.iter().any(|(m, _)| m == "initialize"),
        "turn 2 re-initialized"
    );
    assert!(second
        .iter()
        .all(|(_, a)| a.as_deref() == Some("Bearer T2")));
}

#[tokio::test]
async fn ac5_without_token_turn_auth_tools_absent_and_peer_gets_zero_requests() {
    let (llm, llm_seen) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::ZERO).await;
    let server = TestServer::start(&llm, &peer, None).await;
    let agent = server.spawn_agent().await;

    let (status, _) = server
        .post(&format!("/api/agents/{agent}/message"), &msg(None))
        .await;
    assert_eq!(status, 200);
    assert!(log.lock().unwrap().is_empty());
    let first = llm_seen
        .lock()
        .unwrap()
        .first()
        .cloned()
        .unwrap_or_default();
    assert!(
        !first.contains(TOOL),
        "tool list sent to LLM must not include {TOOL}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac3_two_parallel_turns_do_not_wait_on_each_other() {
    let (llm, _) = start_fake_llm(TOOL).await;
    let (slow_peer, slow_log) = start_mcp_peer(Duration::from_secs(8)).await;
    let (fast_peer, fast_log) = start_mcp_peer(Duration::ZERO).await;
    let slow = TestServer::start(&llm, &slow_peer, None).await;
    let fast = TestServer::start_sharing(&slow, &fast_peer).await;
    let agent_slow = slow.spawn_agent().await;
    let agent_fast = fast.spawn_agent().await;

    // Warm the fast kernel (first turn pays one-off workspace/session setup)
    // so the timing below measures only contention, not cold start.
    let fast_url = format!("/api/agents/{agent_fast}/message");
    let warm = std::time::Instant::now();
    assert_eq!(fast.post(&fast_url, &msg(Some("T2"))).await.0, 200);
    let solo = warm.elapsed();

    let s = slow.clone_client();
    let slow_url = format!("{}/api/agents/{agent_slow}/message", slow.base_url);
    let slow_turn =
        tokio::spawn(async move { s.post(slow_url).json(&msg(Some("T1"))).send().await });
    wait_until(|| slow_peer.calls.load(Ordering::SeqCst) == 1).await;

    let started = std::time::Instant::now();
    let (status, _) = fast.post(&fast_url, &msg(Some("T2"))).await;
    let contended = started.elapsed();
    eprintln!("AC3 timing: fast solo={solo:?} fast while slow held={contended:?}");
    assert_eq!(status, 200);
    // The slow peer holds turn 1's tools/call for 8s. If turn 2 shared any
    // lock with turn 1 it would take ~8s longer than its solo baseline.
    assert!(
        contended < solo + Duration::from_secs(3),
        "turn 2 waited on turn 1: solo={solo:?} contended={contended:?}"
    );
    assert!(
        !slow_turn.is_finished(),
        "turn 1 must still be held by its peer"
    );
    assert_eq!(slow_turn.await.unwrap().unwrap().status(), 200);

    assert!(slow_log
        .lock()
        .unwrap()
        .iter()
        .all(|(_, a)| a.as_deref() == Some("Bearer T1")));
    assert!(fast_log
        .lock()
        .unwrap()
        .iter()
        .all(|(_, a)| a.as_deref() == Some("Bearer T2")));
}

#[tokio::test]
async fn ac10_stop_closes_the_turn_connection_and_no_new_tool_call_follows() {
    let (llm, _) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::from_secs(5)).await;
    let server = TestServer::start(&llm, &peer, None).await;
    let agent = server.spawn_agent().await;

    let c = server.clone_client();
    let url = format!("{}/api/agents/{agent}/message/stream", server.base_url);
    let turn = tokio::spawn(async move {
        let resp = c.post(url).json(&msg(Some("T-stop"))).send().await.unwrap();
        resp.text().await.unwrap_or_default()
    });
    wait_until(|| peer.calls.load(Ordering::SeqCst) == 1).await;

    let (status, body) = server
        .post(&format!("/api/agents/{agent}/stop"), &serde_json::json!({}))
        .await;
    assert_eq!(status, 200, "{body}");
    let _ = tokio::time::timeout(Duration::from_secs(5), turn).await;
    tokio::time::sleep(Duration::from_secs(6)).await;

    assert_eq!(
        peer.calls.load(Ordering::SeqCst),
        1,
        "no tools/call after stop"
    );
    let entries = log.lock().unwrap().clone();
    let last_call = entries
        .iter()
        .rposition(|(m, _)| m == "tools/call")
        .unwrap();
    assert!(entries[last_call + 1..]
        .iter()
        .all(|(m, _)| m != "tools/call"));
    assert!(
        peer.closed.load(Ordering::SeqCst) >= 1,
        "turn connection was closed"
    );
}

#[tokio::test]
async fn ac8_crlf_or_oversized_token_is_400_and_no_turn_is_created() {
    let (llm, llm_seen) = start_fake_llm(TOOL).await;
    let (peer, log) = start_mcp_peer(Duration::ZERO).await;
    let server = TestServer::start(&llm, &peer, None).await;
    let agent = server.spawn_agent().await;
    let long = "a".repeat(5000);

    for bad in ["bad\r\ntoken", long.as_str()] {
        for path in ["message", "message/stream"] {
            let (status, body) = server
                .post(&format!("/api/agents/{agent}/{path}"), &msg(Some(bad)))
                .await;
            assert_eq!(status, 400, "{path}");
            assert_eq!(body, r#"{"error":"Invalid appchat_task_token"}"#);
        }
    }
    assert!(log.lock().unwrap().is_empty());
    assert!(
        llm_seen.lock().unwrap().is_empty(),
        "no turn reached the LLM"
    );
}

#[test]
fn ac9_turn_auth_with_static_authorization_or_stdio_is_rejected_at_boot() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = kernel_config(
        &tmp,
        "http://127.0.0.1:1/v1",
        "http://127.0.0.1:1/mcp",
        None,
    );
    cfg.mcp_servers[0].headers = vec!["Authorization: Bearer static".into()];
    let err = OpenFangKernel::boot_with_config(cfg)
        .err()
        .expect("must reject")
        .to_string();
    assert!(
        err.contains("turn_auth=true cannot also have a static Authorization header"),
        "{err}"
    );

    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = kernel_config(
        &tmp,
        "http://127.0.0.1:1/v1",
        "http://127.0.0.1:1/mcp",
        None,
    );
    cfg.mcp_servers[0].transport = McpTransportEntry::Stdio {
        command: "echo".into(),
        args: vec![],
    };
    let err = OpenFangKernel::boot_with_config(cfg)
        .err()
        .expect("must reject")
        .to_string();
    assert!(
        err.contains("turn_auth=true requires an Http or Sse transport"),
        "{err}"
    );
}
