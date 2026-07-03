//! IPFS batch publishing for audit log epochs via Pinata cloud pinning.
//!
//! This module is a drop-in replacement for the local Kubo daemon approach
//! in [`crate::audit::ipfs`]. Instead of requiring a running IPFS node,
//! it pins epoch batches to Pinata's cloud pinning API. The interface
//! mirrors `ipfs.rs` so callers can switch backends with minimal change.
//!
//! ## Architecture
//!
//! Epoch event batches (JSONL) are uploaded to Pinata's
//! `pinFileToIPFS` endpoint using multipart/form-data. Pinata pins the
//! content and returns the resulting CID (`IpfsHash`). The CID is then
//! retrievable via the configured Pinata gateway.
//!
//! ## Authentication
//!
//! Pinata uses an API key / API secret pair, sent as the
//! `pinata_api_key` and `pinata_secret_api_key` headers on every
//! request. The [`check`] function validates these credentials against
//! Pinata's `testAuthentication` endpoint.

use serde::{Deserialize, Serialize};

use crate::audit::ipfs::IpfsPublishResult;
use crate::error::{AuditError, AuditResult};

/// Configuration for the Pinata cloud pinning client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinataConfig {
    /// The Pinata API key.
    pub api_key: String,
    /// The Pinata API secret.
    pub api_secret: String,
    /// The IPFS gateway URL used to fetch pinned content.
    pub gateway_url: String,
}

impl Default for PinataConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            api_secret: String::new(),
            gateway_url: "https://gateway.pinata.cloud".to_string(),
        }
    }
}

/// The Pinata `pinFileToIPFS` response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PinataPinResponse {
    ipfs_hash: String,
    #[allow(dead_code)]
    pin_size: u64,
    #[allow(dead_code)]
    timestamp: String,
}

/// Publish an epoch's event batch to IPFS via Pinata cloud pinning.
///
/// `batch_content` is the raw bytes of the epoch batch (plaintext JSONL or
/// encrypted ciphertext). The content is uploaded to Pinata's
/// `pinFileToIPFS` endpoint, and the resulting CID is returned.
pub async fn publish_epoch_batch(
    config: &PinataConfig,
    epoch_number: u64,
    batch_content: &[u8],
) -> AuditResult<IpfsPublishResult> {
    if batch_content.is_empty() {
        return Err(AuditError::Validation(
            "cannot publish empty batch to Pinata".to_string(),
        ));
    }

    let event_count = std::str::from_utf8(batch_content)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u64)
        .unwrap_or(0);
    let batch_size_bytes = batch_content.len() as u64;

    let filename = format!("epoch-{}", epoch_number);
    let form = reqwest::multipart::Form::new().part(
        "file",
        reqwest::multipart::Part::bytes(batch_content.to_vec())
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| AuditError::Validation(format!("multipart mime: {e}")))?,
    );

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.pinata.cloud/pinning/pinFileToIPFS")
        .header("pinata_api_key", &config.api_key)
        .header("pinata_secret_api_key", &config.api_secret)
        .multipart(form)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("Pinata API request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "Pinata API returned error: {} {}",
            status, text
        )));
    }

    let pinata_response: PinataPinResponse = response
        .json()
        .await
        .map_err(|e| AuditError::Validation(format!("failed to parse Pinata response: {e}")))?;

    let cid = pinata_response.ipfs_hash;
    if cid.is_empty() {
        return Err(AuditError::Validation(
            "Pinata API returned empty CID".to_string(),
        ));
    }

    let gateway_url = format!(
        "{}/ipfs/{}",
        config.gateway_url.trim_end_matches('/'),
        cid
    );

    Ok(IpfsPublishResult {
        cid,
        epoch_number,
        event_count,
        batch_size_bytes,
        gateway_url,
        encrypted: false,
    })
}

/// Fetch a pinned batch from the Pinata gateway.
///
/// Returns the raw bytes stored at the given CID.
pub async fn fetch_batch(config: &PinataConfig, cid: &str) -> AuditResult<Vec<u8>> {
    let url = format!(
        "{}/ipfs/{}",
        config.gateway_url.trim_end_matches('/'),
        cid
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| AuditError::Validation(format!("Pinata gateway request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AuditError::Validation(format!(
            "Pinata gateway returned error: {} {}",
            status, text
        )));
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| AuditError::Validation(format!("failed to read Pinata gateway response: {e}")))
}

/// Check whether the Pinata API credentials are valid.
///
/// Returns `true` if authentication succeeds, `false` otherwise.
pub async fn check(config: &PinataConfig) -> AuditResult<bool> {
    let client = reqwest::Client::new();
    match client
        .get("https://api.pinata.cloud/data/testAuthentication")
        .header("pinata_api_key", &config.api_key)
        .header("pinata_secret_api_key", &config.api_secret)
        .send()
        .await
    {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinata_config_default() {
        let config = PinataConfig::default();
        assert_eq!(config.gateway_url, "https://gateway.pinata.cloud");
        assert_eq!(config.api_key, "");
        assert_eq!(config.api_secret, "");
    }

    #[test]
    fn pinata_config_serializes_camel_case() {
        let config = PinataConfig {
            api_key: "key123".to_string(),
            api_secret: "secret456".to_string(),
            gateway_url: "https://gateway.pinata.cloud".to_string(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"apiKey\""));
        assert!(json.contains("\"apiSecret\""));
        assert!(json.contains("\"gatewayUrl\""));
    }

    #[tokio::test]
    async fn publish_empty_batch_returns_error() {
        let config = PinataConfig::default();
        let result = publish_epoch_batch(&config, 0, b"").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("empty batch"));
    }
}
