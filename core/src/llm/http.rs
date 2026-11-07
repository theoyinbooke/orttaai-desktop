//! Shared HTTP plumbing for the local LLM clients (`ureq` 3, blocking).
//! Errors are worded so [`super::classify_error`] can tell timeouts,
//! unreachable servers and missing models apart.

use crate::error::{CoreError, Result};
use std::io::Read;
use std::time::Duration;

/// Connect quickly: a refused loopback connect is instant, a dead LAN host
/// shouldn't hold dictation for the OS default (~75 s).
const CONNECT_TIMEOUT_MS: u64 = 3_000;

/// Response status + body reader, read lazily (streaming endpoints).
pub(crate) struct Response {
    pub status: u16,
    pub body: Box<dyn Read + Send>,
}

impl Response {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Read the whole body (10 MB cap is plenty for JSON replies).
    pub fn text(mut self, ctx: &Ctx<'_>) -> Result<String> {
        let mut buf = Vec::new();
        (&mut self.body)
            .take(10 * 1024 * 1024)
            .read_to_end(&mut buf)
            .map_err(|e| ctx.io_error(e))?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

/// Who is being called, for error messages.
pub(crate) struct Ctx<'a> {
    pub provider: &'static str,
    pub endpoint: &'a str,
    pub timeout_ms: Option<u64>,
}

impl Ctx<'_> {
    fn timeout_error(&self) -> CoreError {
        match self.timeout_ms {
            Some(ms) => CoreError::Llm(format!("{} timed out after {ms} ms", self.provider)),
            None => CoreError::Llm(format!("{} timed out", self.provider)),
        }
    }

    fn unreachable_error(&self, detail: impl std::fmt::Display) -> CoreError {
        CoreError::Llm(format!(
            "{} is unreachable at {} ({detail}). Start {}, then try again.",
            self.provider, self.endpoint, self.provider
        ))
    }

    pub fn io_error(&self, e: std::io::Error) -> CoreError {
        use std::io::ErrorKind::*;
        match e.kind() {
            TimedOut | WouldBlock => self.timeout_error(),
            ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected
            | AddrNotAvailable => self.unreachable_error(e),
            _ => {
                // ureq wraps its own errors (incl. timeouts) in io::Error when
                // reading a body.
                let text = e.to_string();
                if text.to_ascii_lowercase().contains("timeout") {
                    self.timeout_error()
                } else {
                    CoreError::Llm(format!("{} request failed: {text}", self.provider))
                }
            }
        }
    }

    pub fn map_ureq(&self, e: ureq::Error) -> CoreError {
        match e {
            ureq::Error::Timeout(_) => self.timeout_error(),
            ureq::Error::ConnectionFailed => self.unreachable_error("connection refused"),
            ureq::Error::HostNotFound => self.unreachable_error("host not found"),
            ureq::Error::Io(io) => self.io_error(io),
            other => CoreError::Llm(format!("{} request failed: {other}", self.provider)),
        }
    }

    /// Turn a non-2xx response into `"<Provider> error <status>: <message>"`.
    pub fn status_error(&self, resp: Response) -> CoreError {
        let status = resp.status;
        let body = resp.text(self).unwrap_or_default();
        CoreError::Llm(format!(
            "{} error {status}: {}",
            self.provider,
            error_message_from_body(&body)
        ))
    }
}

/// `{"error":"…"}`, `{"error":{"message":"…"}}`, or the raw body.
pub(crate) fn error_message_from_body(body: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(m) = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .filter(|m| !m.is_empty())
        {
            return m.to_string();
        }
        if let Some(m) = v
            .get("error")
            .and_then(|e| e.as_str())
            .filter(|m| !m.is_empty())
        {
            return m.to_string();
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "Unknown error".to_string()
    } else {
        trimmed.chars().take(400).collect()
    }
}

fn agent(timeout_ms: Option<u64>) -> ureq::Agent {
    let connect = timeout_ms
        .map(|t| t.min(CONNECT_TIMEOUT_MS))
        .unwrap_or(CONNECT_TIMEOUT_MS)
        .max(50);
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(timeout_ms.map(|t| Duration::from_millis(t.max(50))))
        .timeout_connect(Some(Duration::from_millis(connect)))
        .build()
        .new_agent()
}

/// Join `base` (no trailing slash) and `path` (leading slash).
pub(crate) fn url(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

pub(crate) fn get(ctx: &Ctx<'_>, url: &str) -> Result<Response> {
    let resp = agent(ctx.timeout_ms)
        .get(url)
        .call()
        .map_err(|e| ctx.map_ureq(e))?;
    Ok(into_response(resp))
}

pub(crate) fn post_json(ctx: &Ctx<'_>, url: &str, body: &serde_json::Value) -> Result<Response> {
    let resp = agent(ctx.timeout_ms)
        .post(url)
        .header("Content-Type", "application/json")
        .send(serde_json::to_vec(body)?.as_slice())
        .map_err(|e| ctx.map_ureq(e))?;
    Ok(into_response(resp))
}

fn into_response(resp: ureq::http::Response<ureq::Body>) -> Response {
    let status = resp.status().as_u16();
    Response {
        status,
        body: Box::new(resp.into_body().into_reader()),
    }
}

/// GET → JSON value, mapping non-2xx to an error.
pub(crate) fn get_json(ctx: &Ctx<'_>, url: &str) -> Result<serde_json::Value> {
    let resp = get(ctx, url)?;
    if !resp.is_success() {
        return Err(ctx.status_error(resp));
    }
    parse_body(ctx, resp)
}

/// POST JSON → JSON value, mapping non-2xx to an error.
pub(crate) fn post_json_value(
    ctx: &Ctx<'_>,
    url: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let resp = post_json(ctx, url, body)?;
    if !resp.is_success() {
        return Err(ctx.status_error(resp));
    }
    parse_body(ctx, resp)
}

fn parse_body(ctx: &Ctx<'_>, resp: Response) -> Result<serde_json::Value> {
    let text = resp.text(ctx)?;
    serde_json::from_str(&text)
        .map_err(|_| CoreError::Llm(format!("Invalid response from {}.", ctx.provider)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_bodies() {
        assert_eq!(
            error_message_from_body(r#"{"error":"model 'x' not found"}"#),
            "model 'x' not found"
        );
        assert_eq!(
            error_message_from_body(r#"{"error":{"message":"bad schema"}}"#),
            "bad schema"
        );
        assert_eq!(error_message_from_body("  "), "Unknown error");
        assert_eq!(error_message_from_body("plain text"), "plain text");
        assert_eq!(url("http://x:1/", "/api/tags"), "http://x:1/api/tags");
    }

    #[test]
    fn unreachable_server_is_classified() {
        // Port 9 (discard) on loopback is closed on CI and dev machines.
        let ctx = Ctx {
            provider: "Ollama",
            endpoint: "http://127.0.0.1:9",
            timeout_ms: Some(1500),
        };
        let err = get(&ctx, "http://127.0.0.1:9/api/tags").err().unwrap();
        let kind = super::super::classify_error(&err);
        assert!(
            matches!(
                kind,
                super::super::LlmFailure::Unreachable | super::super::LlmFailure::Timeout
            ),
            "{err}"
        );
    }
}
