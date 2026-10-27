mod fixture;
mod runner;

use std::time::Duration;

use fixture::{Fixture, TOOL};
use openfang_runtime::tool_runner::builtin_tool_definitions;
use runner::{assert_denied, invoke};
use serde_json::{json, Value};
use tokio::sync::Mutex;

#[tokio::test]
async fn every_builtin_is_denied_at_execution_with_mcp_only_capabilities() {
    let allowed = vec![TOOL.to_owned()];
    let builtins = builtin_tool_definitions();
    assert!(!builtins.is_empty());
    for tool in &builtins {
        assert_denied(invoke(&tool.name, json!({}), &allowed, None).await);
    }
    println!("execution-denied builtins: {}", builtins.len());
}

#[tokio::test]
async fn aliases_and_undiscovered_tools_cannot_bypass_the_execution_allowlist() {
    let fixture = Fixture::start(Duration::ZERO).await;
    let allowed = vec![TOOL.to_owned()];
    for name in [
        "Bash",
        "fs-exec",
        "fs-write",
        "Read",
        "WebFetch",
        "WebSearch",
        "sessions_spawn",
        "sessions_send",
        "mcp_peer_not_granted",
        "mcp_other_probe_result",
    ] {
        assert_denied(invoke(name, json!({}), &allowed, Some(&fixture.connections)).await);
    }
    assert_eq!(fixture.call_count(), 0);
}

#[tokio::test]
async fn empty_allowlist_denies_even_discovered_tools() {
    let fixture = Fixture::start(Duration::ZERO).await;
    assert_denied(invoke(TOOL, json!({}), &[], Some(&fixture.connections)).await);
    assert_eq!(fixture.call_count(), 0);
}

#[tokio::test]
async fn runner_preserves_mcp_errors_structured_content_and_call_identity() {
    let fixture = Fixture::start(Duration::ZERO).await;
    let allowed = vec![TOOL.to_owned()];
    for (index, error) in [false, true, false].into_iter().enumerate() {
        let result = invoke(
            TOOL,
            json!({"error":error}),
            &allowed,
            Some(&fixture.connections),
        )
        .await;
        assert_eq!(result.tool_use_id, "policy-call");
        assert_eq!(result.is_error, error);
        let body = if error {
            result.content.strip_prefix("Error: ").unwrap()
        } else {
            &result.content
        };
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(value["isError"], error);
        assert_eq!(value["structuredContent"]["ok"], !error);
        assert_eq!(value["structuredContent"]["receipt"], "protocol-only");
        assert_eq!(fixture.call_count(), index + 1);
    }
}

#[tokio::test]
async fn runner_propagates_unknown_timeout_and_quarantines_without_second_call() {
    let fixture = Fixture::start(Duration::from_secs(10)).await;
    let allowed = vec![TOOL.to_owned()];
    let first = tokio::time::timeout(
        Duration::from_secs(3),
        invoke(TOOL, json!({}), &allowed, Some(&fixture.connections)),
    )
    .await
    .unwrap();
    assert!(first.is_error);
    assert!(
        first.content.contains("outcome unknown"),
        "{}",
        first.content
    );
    let second = invoke(TOOL, json!({}), &allowed, Some(&fixture.connections)).await;
    assert!(second.is_error);
    assert!(second.content.contains("quarantined"));
    assert_eq!(fixture.call_count(), 1);
}

#[tokio::test(start_paused = true)]
async fn runner_bounds_mutex_wait_without_owning_or_mutating_connection() {
    let connections = Mutex::new(vec![]);
    let guard = connections.lock().await;
    let allowed = vec![TOOL.to_owned()];
    let started = tokio::time::Instant::now();
    let result = invoke(TOOL, json!({}), &allowed, Some(&connections)).await;
    assert!(result.is_error);
    assert!(result.content.contains("dispatch deadline exceeded"));
    assert_eq!(started.elapsed(), Duration::from_secs(15));
    assert!(guard.is_empty());
}
