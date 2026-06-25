//! Dev mode proxy: forwards HTTP requests to the local audit services
//! running in Docker (publisher :9173, attester :9174, reader :9175).
//!
//! The Tauri webview cannot fetch localhost directly (CORS + same-origin),
//! so the Rust side proxies the calls and returns the JSON verbatim as
//! `serde_json::Value`. This lets the dev-mode live view query the real
//! docker publisher/reader daemons — the actual full system — without
//! redefining every response type.

use serde::Serialize;
use serde_json::Value;

use crate::error::{AuditError, AuditResult};

const DEFAULT_TIMEOUT_SECS: u64 = 15;

/// Build the full URL for a daemon port + path.
pub fn url(port: u16, path: &str) -> String {
    let path = path.trim_start_matches('/');
    format!("http://127.0.0.1:{port}/{path}")
}

/// Proxy a GET request to a local audit service.
pub async fn proxy_get(port: u16, path: &str) -> AuditResult<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| AuditError::Internal(format!("reqwest build: {e}")))?;

    let resp = client
        .get(url(port, path))
        .send()
        .await
        .map_err(|e| AuditError::Internal(format!("daemon GET {path} on :{port}: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AuditError::Internal(format!("daemon GET body: {e}")))?;

    if !status.is_success() {
        return Err(AuditError::Internal(format!(
            "daemon GET :{port}/{path} returned {status}: {text}"
        )));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|e| AuditError::Internal(format!("daemon GET :{port}/{path} parse: {e}")))
}

/// Proxy a POST request to a local audit service with a JSON body.
pub async fn proxy_post(port: u16, path: &str, body: Value) -> AuditResult<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| AuditError::Internal(format!("reqwest build: {e}")))?;

    let resp = client
        .post(url(port, path))
        .json(&body)
        .send()
        .await
        .map_err(|e| AuditError::Internal(format!("daemon POST {path} on :{port}: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| AuditError::Internal(format!("daemon POST body: {e}")))?;

    if !status.is_success() {
        return Err(AuditError::Internal(format!(
            "daemon POST :{port}/{path} returned {status}: {text}"
        )));
    }

    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text)
        .map_err(|e| AuditError::Internal(format!("daemon POST :{port}/{path} parse: {e}")))
}

/// A typed wrapper so the frontend gets a structured error vs a raw value.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyResponse {
    pub ok: bool,
    pub data: Value,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_builds_correctly() {
        assert_eq!(url(9173, "/status"), "http://127.0.0.1:9173/status");
        assert_eq!(url(9173, "status"), "http://127.0.0.1:9173/status");
        assert_eq!(url(9175, "/reader/verify-oplog"), "http://127.0.0.1:9175/reader/verify-oplog");
    }
}
