use std::sync::{Arc, Mutex};

use axum::{routing::post, Json, Router};
use openfang_runtime::{
    drivers::openai::OpenAIDriver,
    llm_driver::{CompletionRequest, LlmDriver},
};
use openfang_types::message::{Message, MessageContent, Role};
use serde_json::{json, Value};

#[tokio::test]
async fn non_streaming_completion_explicitly_requests_json_instead_of_provider_default_sse() {
    let observed = Arc::new(Mutex::new(None));
    let capture = observed.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(body): Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                Json(json!({"model":"reported-model", "choices":[{
                "message":{"role":"assistant","content":"PROTOCOL_OK"},"finish_reason":"stop"
            }],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let driver = OpenAIDriver::new("fixture-key-not-a-secret".into(), base);
    let response = driver
        .complete(CompletionRequest {
            model: "requested-model".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text("protocol only".into()),
                ..Default::default()
            }],
            tools: vec![],
            max_tokens: 16,
            temperature: 0.0,
            system: None,
            thinking: None,
        })
        .await
        .unwrap();
    server.abort();
    assert_eq!(response.text(), "PROTOCOL_OK");
    let request = observed.lock().unwrap().take().unwrap();
    assert_eq!(request["model"], "requested-model");
    assert_eq!(
        request.get("stream"),
        Some(&Value::Bool(false)),
        "explicit false is mandatory for non-streaming endpoints"
    );
}
