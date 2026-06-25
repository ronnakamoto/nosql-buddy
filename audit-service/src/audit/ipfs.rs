//! IPFS batch publishing for audit log epochs.
//!
//! Each epoch's events can be published to IPFS for decentralized
//! verification. The CID (Content Identifier) is stored in sled and
//! optionally committed on-chain as metadata, creating a three-layer
//! verifiability chain:
//!
//!   on-chain root → on-chain metadata (CID) → IPFS batch (events)
//!
//! ## Architecture
//!
//! The module uses the IPFS HTTP API (Kubo daemon) to add event
//! batches. The default endpoint is `http://127.0.0.1:5001/api/v0/add`,
//! configurable via `IpfsConfig`. The batch content is the JSONL
//! representation of the epoch's events.
//!
//! ## Fallback
//!
//! If no IPFS daemon is running, the `publish_epoch_batch` function
//! returns an error. The caller can choose to ignore the error and
//! continue without IPFS publishing — the on-chain root commitment
//! still provides tamper-evidence.

use serde::{Deserialize, Serialize};

use crate::error::{AuditError, AuditResult};

/// Configuration for the IPFS client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpfsConfig {
    /// The IPFS HTTP API base URL.
    pub api_url: String,
    /// Whether to use CIDv1 (base32) instead of CIDv0.
    pub cid_version: u8,
}

impl Default for IpfsConfig {
    fn default() -> Self {
        Self {
            api_url: "http://127.0.0.1:5001".to_string(),
            cid_version: 1,
        }
    }
}

/// The result of publishing an epoch batch to IPFS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpfsPublishResult {
    /// The CID (Content Identifier) of the published batch.
    pub cid: String,
    /// The epoch number that was published.
    pub epoch_number: u64,
    /// The number of events in the batch.
    pub event_count: u64,
    /// The size of the batch in bytes.
    pub batch_size_bytes: u64,
    /// The IPFS gateway URL for viewing the batch.
    pub gateway_url: String,
}

/// Publish an epoch's event batch to IPFS.
///
/// `batch_content` is the JSONL representation of the epoch's events.
/// The content is added to IPFS via the HTTP API, and the resulting
/// CID is returned.
pub async fn publish_epoch_batch(
    config: &IpfsConfig,
    epoch_number: u64,
    batch_content: &str,
) -> AuditResult<IpfsPublishResult> {
    if batch_content.is_empty() {
        return Err(AuditError::Validation(
            "cannot publish empty batch to IPFS".to_string(),
        ));
    }

    let event_count = batch_content.lines().filter(|l| !l.trim().is_empty()).count() as u64;
    let batch_size_bytes = batch_content.len() as u64;

    // Construct the IPFS API URL for adding a file.
    let url = format!(
        "{}/api/v0/add?cid-version={}&quiet=true&pin=true",
        config.api_url.trim_end_matches('/'),
        config.cid_version
    );

    // The IPFS /add endpoint expects multipart/form-data with the file
    // content. We construct a simple multipart body manually.
    let boundary = "nosqlbuddy-ipfs-boundary";
    let filename = format!("epoch-{}.jsonl", epoch_number);
    let body = format_multipart_body(boundary, &filename, batch_content);

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={}", boundary),
        )
        .body(body)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("IPFS API request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "IPFS API returned error: {} {}",
            status, text
        )));
    }

    // The IPFS /add API returns JSON with the CID.
    // Format: {"Name":"epoch-0.jsonl","Hash":"bafy...","Size":"123"}
    let ipfs_response: IpfsAddResponse = response
        .json()
        .await
        .map_err(|e| AuditError::Validation(format!("failed to parse IPFS response: {e}")))?;

    let cid = ipfs_response.hash;
    if cid.is_empty() {
        return Err(AuditError::Validation(
            "IPFS API returned empty CID".to_string(),
        ));
    }

    let gateway_url = format!("https://dweb.link/ipfs/{}", cid);

    Ok(IpfsPublishResult {
        cid,
        epoch_number,
        event_count,
        batch_size_bytes,
        gateway_url,
    })
}

/// Check if an IPFS daemon is reachable at the configured URL.
pub async fn check_daemon(config: &IpfsConfig) -> AuditResult<bool> {
    let url = format!("{}/api/v0/version", config.api_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    match client.post(&url).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

/// Construct a multipart/form-data body for the IPFS /add endpoint.
/// This is a minimal implementation — just one file field.
fn format_multipart_body(boundary: &str, filename: &str, content: &str) -> String {
    format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: application/octet-stream\r\n\r\n\
         {content}\r\n\
         --{boundary}--\r\n",
        boundary = boundary,
        filename = filename,
        content = content,
    )
}

/// The IPFS /add API response.
#[derive(Debug, Deserialize)]
struct IpfsAddResponse {
    #[serde(rename = "Hash")]
    hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipfs_config_default_is_localhost() {
        let config = IpfsConfig::default();
        assert_eq!(config.api_url, "http://127.0.0.1:5001");
        assert_eq!(config.cid_version, 1);
    }

    #[test]
    fn ipfs_publish_result_serializes_camel_case() {
        let result = IpfsPublishResult {
            cid: "bafy123".to_string(),
            epoch_number: 5,
            event_count: 100,
            batch_size_bytes: 4096,
            gateway_url: "https://dweb.link/ipfs/bafy123".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"epochNumber\":5"));
        assert!(json.contains("\"eventCount\":100"));
        assert!(json.contains("\"batchSizeBytes\":4096"));
        assert!(json.contains("\"gatewayUrl\""));
    }

    #[test]
    fn format_multipart_body_contains_boundary_and_content() {
        let body = format_multipart_body("test-boundary", "epoch-0.jsonl", "hello world");
        assert!(body.contains("--test-boundary"));
        assert!(body.contains("filename=\"epoch-0.jsonl\""));
        assert!(body.contains("hello world"));
        assert!(body.contains("--test-boundary--"));
    }

    #[tokio::test]
    async fn publish_empty_batch_returns_error() {
        let config = IpfsConfig::default();
        let result = publish_epoch_batch(&config, 0, "").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("empty batch"));
    }

    #[test]
    fn ipfs_config_serializes_camel_case() {
        let config = IpfsConfig {
            api_url: "http://example.com:5001".to_string(),
            cid_version: 0,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"apiUrl\""));
        assert!(json.contains("\"cidVersion\""));
    }
}
