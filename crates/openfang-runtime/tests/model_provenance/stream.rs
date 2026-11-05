use axum::{response::IntoResponse, routing::post, Router};
use openfang_runtime::{
    drivers::{model_response::record_stream, openai::OpenAIDriver},
    llm_driver::{CompletionRequest, LlmDriver},
};
use openfang_types::message::{Message, MessageContent, Role};

use crate::capture::{with_capture, Capture};

#[test]
fn stream_trace_reports_sanitized_model_and_never_inherits_requested() {
    let log = with_capture(|| record_stream("ag/gemini-3-flash", Some("gemini-3-flash")));
    assert!(log.contains("openfang_model_response"), "{log}");
    assert!(
        log.contains("requested_model=\"ag/gemini-3-flash\""),
        "{log}"
    );
    assert!(log.contains("reported_model=\"gemini-3-flash\""), "{log}");

    for reported in [None, Some(""), Some("bad\nINJECTED")] {
        let log = with_capture(|| record_stream("requested-model", reported));
        assert!(log.contains("reported_model=\"<unavailable>\""), "{log}");
        assert!(!log.contains("INJECTED"));
    }
}

/// End-to-end through `OpenAIDriver::stream()`: the trace carries the model
/// from the provider's SSE chunks, not the requested one, and no content.
#[tokio::test]
async fn openai_stream_records_model_from_sse_chunks_without_content() {
    let sse = concat!(
        "data: {\"model\":\"gemini-3-flash\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"PRIVATE_STREAM_TEXT\"},\"finish_reason\":null}]}\n\n",
        "data: {\"model\":\"gemini-3-flash\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || async move { ([("content-type", "text/event-stream")], sse).into_response() }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let driver = OpenAIDriver::new("fixture-key-not-a-secret".into(), base);
    let response = driver
        .stream(
            CompletionRequest {
                model: "ag/gemini-3-flash".into(),
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
            },
            tx,
        )
        .await
        .unwrap();
    drain.await.unwrap();
    server.abort();

    assert_eq!(response.text(), "PRIVATE_STREAM_TEXT");
    let log = capture.text();
    let trace: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("openfang_model_response"))
        .collect();
    assert_eq!(trace.len(), 1, "exactly one trace per stream: {log}");
    assert!(trace[0].contains("requested_model=\"ag/gemini-3-flash\""));
    assert!(trace[0].contains("reported_model=\"gemini-3-flash\""));
    assert!(!log.contains("PRIVATE_STREAM_TEXT"));
    assert!(!log.contains("fixture-key-not-a-secret"));
}
