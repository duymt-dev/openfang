//! Wire-level negative probes for the test peer, not production grant evidence.
use super::{
    guard::{AUTH, LIMIT, ORIGIN},
    peer::Fixture,
};
use reqwest::{Client, Method, RequestBuilder, StatusCode};
use serde_json::{json, Value};

fn request(fixture: &Fixture, method: Method, body: String) -> RequestBuilder {
    Client::new()
        .request(method, &fixture.url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .body(body)
}

fn initialize() -> String {
    json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
        "protocolVersion":"2025-11-25", "capabilities":{},
        "clientInfo":{"name":"policy-probe", "version":"1"}
    }})
    .to_string()
}

#[tokio::test]
async fn credentials_checked_for_all_methods_before_sdk_and_never_echoed() {
    let fixture = Fixture::start().await;
    let bodies = [
        initialize(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"transport-probe"}})
            .to_string(),
    ];
    for method in [Method::POST, Method::GET, Method::DELETE] {
        for body in &bodies {
            for auth in [vec![], vec!["Bearer bad-fixture"], vec![AUTH, AUTH]] {
                let mut req = request(&fixture, method.clone(), body.clone());
                for value in auth {
                    req = req.header("authorization", value);
                }
                let response = req.send().await.unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert_eq!(response.text().await.unwrap(), "unauthorized");
            }
        }
    }
    assert!(fixture.policy.accepted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn supplied_origin_requires_single_exact_value_before_sdk() {
    let fixture = Fixture::start().await;
    for origins in [
        vec!["null"],
        vec!["https://127.0.0.1:4200"],
        vec!["http://localhost:4200"],
        vec!["http://127.0.0.1:4200/"],
        vec!["http://127.0.0.1:4200.evil"],
        vec![ORIGIN, ORIGIN],
    ] {
        let mut req = request(&fixture, Method::POST, initialize()).header("authorization", AUTH);
        for origin in origins {
            req = req.header("origin", origin);
        }
        let response = req.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.text().await.unwrap(), "forbidden");
    }
    assert!(fixture.policy.accepted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn authenticated_sdk_negotiation_notifications_and_unsupported_methods() {
    let fixture = Fixture::start().await;
    let response = request(&fixture, Method::POST, initialize())
        .header("authorization", AUTH)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get("mcp-session-id").is_none());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["result"]["protocolVersion"], "2025-06-18");
    println!(
        "rmcp1.3 / axum0.7 negotiated protocol: {}",
        body["result"]["protocolVersion"]
    );
    let notification = json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string();
    assert_eq!(
        request(&fixture, Method::POST, notification)
            .header("authorization", AUTH)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    for method in [Method::GET, Method::DELETE] {
        assert_eq!(
            request(&fixture, method, String::new())
                .header("authorization", AUTH)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}

#[tokio::test]
async fn body_limit_checks_actual_bytes_including_chunked_transfer() {
    let fixture = Fixture::start().await;
    let mut body = initialize();
    body.extend(std::iter::repeat_n(' ', LIMIT - body.len()));
    assert_eq!(body.len(), LIMIT);
    let response = request(&fixture, Method::POST, body.clone())
        .header("authorization", AUTH)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let accepted_before = fixture.policy.accepted.lock().unwrap().len();
    body.push(' ');
    let response = request(&fixture, Method::POST, body.clone())
        .header("authorization", AUTH)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let chunks = futures::stream::iter(vec![Ok::<_, std::io::Error>(body.into_bytes())]);
    let response = request(&fixture, Method::POST, String::new())
        .header("authorization", AUTH)
        .body(reqwest::Body::wrap_stream(chunks))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        fixture.policy.accepted.lock().unwrap().len(),
        accepted_before
    );
}
