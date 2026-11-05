//! Per-turn MCP bearer token, opaque to OpenFang (AppChat verifies it).
//!
//! OpenFang never parses or verifies `appchat_task_token` — it only carries
//! it to `turn_auth = true` MCP servers for the lifetime of a single turn.
//! `TurnMcpAuth` exists so the value can never accidentally reach `Debug`
//! output, logs, or serialization: no `Serialize`, and `Debug` is redacted.

use http::{HeaderName, HeaderValue};

/// Maximum accepted token length (bytes). Matches SPEC validation in
/// TIP-OF2-002: longer values are rejected before a turn is created.
pub const MAX_TOKEN_BYTES: usize = 4096;

/// A caller-supplied per-turn bearer token. Carries no business meaning
/// inside OpenFang; the token is authenticated by AppChat (`mcp_gateway::auth`).
#[derive(Clone)]
pub struct TurnMcpAuth(String);

impl std::fmt::Debug for TurnMcpAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TurnMcpAuth").field(&"[REDACTED]").finish()
    }
}

impl TurnMcpAuth {
    /// Build the `Authorization: Bearer <token>` header pair for this turn,
    /// marked sensitive so it never appears in transport-level debug logs.
    pub fn bearer_header(&self) -> Result<(HeaderName, HeaderValue), String> {
        let mut value = HeaderValue::from_str(&format!("Bearer {}", self.0))
            .map_err(|_| "Invalid appchat_task_token".to_string())?;
        value.set_sensitive(true);
        Ok((http::header::AUTHORIZATION, value))
    }
}

/// Validate a raw `appchat_task_token` from the API request body.
///
/// Empty or whitespace-only is treated as "no token" (`Ok(None)`), matching
/// SPEC validation. Too long, or containing bytes `HeaderValue` would reject
/// (CR/LF/control chars), is a hard error the API layer turns into 400.
pub fn validate_token(raw: Option<&str>) -> Result<Option<TurnMcpAuth>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > MAX_TOKEN_BYTES {
        return Err("Invalid appchat_task_token".to_string());
    }
    // HeaderValue rejects CR/LF and most control bytes; probe it here so the
    // API returns 400 before any turn/session state is created.
    HeaderValue::from_str(trimmed).map_err(|_| "Invalid appchat_task_token".to_string())?;
    Ok(Some(TurnMcpAuth(trimmed.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_and_empty_and_whitespace_are_no_token() {
        assert!(validate_token(None).unwrap().is_none());
        assert!(validate_token(Some("")).unwrap().is_none());
        assert!(validate_token(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn valid_token_round_trips_into_bearer_header() {
        let auth = validate_token(Some("T1-canary")).unwrap().unwrap();
        let (name, value) = auth.bearer_header().unwrap();
        assert_eq!(name, http::header::AUTHORIZATION);
        assert_eq!(value.to_str().unwrap(), "Bearer T1-canary");
        assert!(value.is_sensitive());
    }

    #[test]
    fn crlf_in_token_is_rejected() {
        let err = validate_token(Some("abc\r\nInjected: yes")).unwrap_err();
        assert_eq!(err, "Invalid appchat_task_token");
        assert!(!err.contains("abc"));
    }

    #[test]
    fn too_long_token_is_rejected() {
        let long = "a".repeat(MAX_TOKEN_BYTES + 1);
        let err = validate_token(Some(&long)).unwrap_err();
        assert_eq!(err, "Invalid appchat_task_token");
    }

    #[test]
    fn exactly_max_length_is_accepted() {
        let max = "a".repeat(MAX_TOKEN_BYTES);
        assert!(validate_token(Some(&max)).unwrap().is_some());
    }

    #[test]
    fn debug_never_prints_the_token() {
        let auth = validate_token(Some("TOKEN-CANARY-deadbeef"))
            .unwrap()
            .unwrap();
        let printed = format!("{auth:?}");
        assert!(!printed.contains("TOKEN-CANARY-deadbeef"));
        assert!(printed.contains("REDACTED"));
    }
}
