use rmcp::model::CallToolResult;
use serde_json::{json, Value};

use super::super::result::render_result;

fn decode(value: Value) -> CallToolResult {
    serde_json::from_value(value).unwrap()
}

#[test]
fn text_error_is_not_a_success_string() {
    let result = decode(json!({"isError":true,"content":[{"type":"text","text":"denied"}]}));
    assert_eq!(render_result(result), Err("denied".into()));
}

#[test]
fn structured_result_preserves_text_even_if_they_disagree() {
    for error in [false, true] {
        let result = decode(json!({"isError":error,"structuredContent":{"ok":!error},
            "content":[{"type":"text","text":"do not silently discard me"}]}));
        let rendered = render_result(result);
        assert_eq!(rendered.is_err(), error);
        let content = rendered.unwrap_or_else(|e| e);
        let value: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["isError"], error);
        assert_eq!(value["structuredContent"]["ok"], !error);
        assert_eq!(value["content"][0]["text"], "do not silently discard me");
    }
}

#[test]
fn legacy_text_success_keeps_its_public_api() {
    let result =
        decode(json!({"content":[{"type":"text","text":"first"},{"type":"text","text":"second"}]}));
    assert_eq!(render_result(result), Ok("first\nsecond".into()));
}

#[test]
fn empty_error_content_still_returns_err() {
    let value = render_result(decode(json!({"content":[],"isError":true}))).unwrap_err();
    assert_eq!(
        serde_json::from_str::<Value>(&value).unwrap()["isError"],
        true
    );
}
