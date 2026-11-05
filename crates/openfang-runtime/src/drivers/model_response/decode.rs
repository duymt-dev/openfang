use crate::llm_driver::LlmError;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Decode a non-streaming provider response and record its actual model metadata.
/// Missing metadata is NOT evidence that the configured/requested model was used.
pub fn decode<T: DeserializeOwned>(body: &str, requested_model: &str) -> Result<T, LlmError> {
    let value: Value = serde_json::from_str(body).map_err(|e| LlmError::Parse(e.to_string()))?;
    let reported = identifier(value.get("model").and_then(Value::as_str)).to_owned();
    let result = serde_json::from_value(value).map_err(|e| LlmError::Parse(e.to_string()))?;
    tracing::info!(
        target: "openfang_model_response",
        driver = "openai-compatible",
        requested_model = identifier(Some(requested_model)),
        reported_model = reported,
        "Non-streaming provider response model"
    );
    Ok(result)
}

/// Record the model a streaming provider actually reported, once per completed
/// stream. `reported_model` is the last `model` seen in the SSE chunks (`None`
/// when the provider never sent one) and is sanitized exactly like `decode()`.
pub fn record_stream(requested_model: &str, reported_model: Option<&str>) {
    tracing::info!(
        target: "openfang_model_response",
        driver = "openai-compatible",
        requested_model = identifier(Some(requested_model)),
        reported_model = identifier(reported_model),
        "Streaming provider response model"
    );
}

fn identifier(value: Option<&str>) -> &str {
    match value {
        Some(value)
            if !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|b| {
                    b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b':')
                }) =>
        {
            value
        }
        _ => "<unavailable>",
    }
}
