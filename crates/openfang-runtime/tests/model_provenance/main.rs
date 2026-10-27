mod capture;
mod wire;

use capture::with_capture;
use openfang_runtime::drivers::model_response::decode;
use serde::Deserialize;
use serde_json::{json, Value};

#[test]
fn actual_response_metadata_is_logged_without_response_content_or_credentials() {
    let body = json!({"model":"gemini-3-flash", "choices":[{"content":"PRIVATE_RESPONSE"}],
        "api_key":"PRIVATE_KEY", "prompt":"PRIVATE_PROMPT"})
    .to_string();
    let log = with_capture(|| {
        let result: Value = decode(&body, "ag/gemini-3-flash").unwrap();
        assert_eq!(result["choices"][0]["content"], "PRIVATE_RESPONSE");
    });
    assert!(log.contains("ag/gemini-3-flash"));
    assert!(log.contains("reported_model=\"gemini-3-flash\""));
    for secret in ["PRIVATE_RESPONSE", "PRIVATE_KEY", "PRIVATE_PROMPT"] {
        assert!(!log.contains(secret));
    }
}

#[test]
fn missing_or_unsafe_reported_model_never_inherits_the_requested_model() {
    for value in [
        Value::Null,
        json!(""),
        json!("bad\nINJECTED"),
        json!("x".repeat(129)),
        json!(23),
    ] {
        let log = with_capture(|| {
            let _: Value = decode(&json!({"model":value}).to_string(), "requested-model").unwrap();
        });
        assert!(log.contains("reported_model=\"<unavailable>\""), "{log}");
        assert!(!log.contains("INJECTED"));
    }
}

#[test]
fn unsafe_requested_model_is_not_written_to_trace() {
    let log = with_capture(|| {
        let _: Value = decode("{}", "bad\r\nINJECTED").unwrap();
    });
    assert!(log.contains("requested_model=\"<unavailable>\""));
    assert!(!log.contains("INJECTED"));
}

#[derive(Deserialize)]
struct TypedResponse {
    choices: Vec<Value>,
}

#[test]
fn malformed_or_wrong_typed_response_cannot_produce_success_provenance() {
    for body in [
        "not-json",
        "{}",
        "{\"model\":\"misleading\",\"choices\":false}",
    ] {
        let log = with_capture(|| {
            assert!(decode::<TypedResponse>(body, "requested").is_err());
        });
        assert!(log.is_empty(), "{log}");
    }
    let valid = decode::<TypedResponse>("{\"choices\":[]}", "requested").unwrap();
    assert!(valid.choices.is_empty());
}
