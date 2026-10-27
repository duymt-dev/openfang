use rmcp::model::CallToolResult;

/// Preserve structured output and the error channel without interpreting business data.
pub(super) fn render_result(result: CallToolResult) -> Result<String, String> {
    let is_error = result.is_error.unwrap_or(false);
    let text: Vec<&str> = result
        .content
        .iter()
        .filter_map(|item| item.as_text().map(|tc| tc.text.as_str()))
        .collect();
    let rendered = if result.structured_content.is_some() || text.is_empty() {
        // Keep BOTH content and structuredContent; never silently discard either.
        serde_json::to_string(&result).map_err(|_| "MCP result serialization failed".to_string())?
    } else {
        text.join("\n")
    };
    if is_error {
        Err(rendered)
    } else {
        Ok(rendered)
    }
}
