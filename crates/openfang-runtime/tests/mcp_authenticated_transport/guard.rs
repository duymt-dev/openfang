//! Test-only policy peer. Not production credential validation or tenant authorization.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};

use axum_compat::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

pub const AUTH: &str = "Bearer protocol-fixture-only";
pub const ORIGIN: &str = "http://127.0.0.1:4200";
pub const LIMIT: usize = 64 * 1024;

#[derive(Clone, Default)]
pub struct Policy {
    pub revoked: Arc<AtomicBool>,
    pub requests: Arc<AtomicUsize>,
    pub accepted: Arc<Mutex<Vec<String>>>,
}

pub async fn enforce(State(policy): State<Policy>, request: Request, next: Next) -> Response {
    policy.requests.fetch_add(1, Ordering::SeqCst);
    let auth = request.headers().get_all("authorization");
    if auth.iter().count() != 1
        || auth.iter().next().unwrap() != AUTH
        || policy.revoked.load(Ordering::SeqCst)
    {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    let origin = request.headers().get_all("origin");
    if origin.iter().count() > 1 || origin.iter().any(|value| value != ORIGIN) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, LIMIT).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body limit exceeded").into_response();
    };
    let method = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("method").and_then(|m| m.as_str()).map(str::to_owned))
        .unwrap_or_else(|| parts.method.to_string());
    // Capture method only: never credentials, body, tenant or customer payload.
    policy.accepted.lock().unwrap().push(method);
    next.run(Request::from_parts(parts, Body::from(body))).await
}
