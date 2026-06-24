//! Stellar / Soroban on-chain commitment client.
//!
//! This module calls the deployed Soroban `ZkAuditCommitment` contract via
//! the `stellar` CLI as a subprocess. This is the MVP approach from the
//! architecture plan — production would use a Rust RPC client.
//!
//! The contract is deployed on Stellar testnet at:
//!   CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5
//!
//! The `spike` identity (stored in `~/.config/stellar/identity/spike.toml`)
//! is the admin authorized to call `commit_root`.

use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The deployed Soroban contract ID on Stellar testnet.
pub const CONTRACT_ID: &str = "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5";

/// The Stellar identity name authorized to commit roots.
pub const SOURCE_IDENTITY: &str = "spike";

/// The Stellar network to use.
pub const NETWORK: &str = "testnet";

/// The result of committing a root on-chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitResult {
    /// The on-chain sequence number returned by `commit_root`.
    pub sequence: u64,
    /// The transaction hash on Stellar.
    pub tx_hash: String,
    /// The root that was committed (hex).
    pub root_hex: String,
}

/// The result of querying the current on-chain root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainRoot {
    /// The sequence number of the latest committed root.
    pub sequence: u64,
    /// The root hash as a hex string (32 bytes = 64 hex chars).
    pub root_hex: String,
    /// The on-chain timestamp (Unix epoch seconds).
    pub timestamp: u64,
    /// Optional metadata associated with the commitment.
    pub metadata: String,
}

/// Commit a Merkle root to the Soroban contract on Stellar testnet.
///
/// This calls `commit_root(root, metadata)` on the contract. The `root_hex`
/// must be a 64-character hex string (32 bytes). The `metadata` is an
/// arbitrary string stored on-chain with the commitment.
pub fn commit_root(root_hex: &str, metadata: &str) -> AppResult<CommitResult> {
    // The stellar CLI expects the root as a hex-encoded byte string.
    // The contract's `commit_root` takes `Bytes` which the CLI accepts
    // as a hex string.
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        SOURCE_IDENTITY,
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "commit_root",
        "--root",
        root_hex,
        "--metadata",
        metadata,
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sequence: u64 = stdout
        .trim()
        .parse()
        .map_err(|e| AppError::Validation(format!("failed to parse commit_root sequence: {e} (stdout: {stdout})")))?;

    // Extract the tx hash from stderr (the CLI prints it to stderr).
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tx_hash = extract_tx_hash(&stderr).unwrap_or_default();

    Ok(CommitResult {
        sequence,
        tx_hash,
        root_hex: root_hex.to_string(),
    })
}

/// Get the current committed root from the Soroban contract.
pub fn get_current_root() -> AppResult<Option<OnChainRoot>> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        SOURCE_IDENTITY,
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "get_current_root",
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    // The contract returns Option<RootEntry>. When None, the CLI
    // prints something like "void" or an empty result.
    if trimmed.is_empty() || trimmed == "void" || trimmed == "null" {
        return Ok(None);
    }

    // The CLI prints the result as a Soroban value. For a struct,
    // it looks like: {"vec":[{"u64":"1"},{"bytes":"..."},{"u64":"..."},{"string":"..."}]}
    // We parse it loosely.
    let root_entry = parse_root_entry(trimmed)?;
    Ok(Some(root_entry))
}

/// Get the root history from the Soroban contract.
pub fn get_root_history(limit: u32) -> AppResult<Vec<OnChainRoot>> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        SOURCE_IDENTITY,
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "get_root_history",
        "--limit",
        &limit.to_string(),
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    // The CLI returns an array of RootEntry values.
    // For simplicity, we return an empty list if parsing fails.
    // A full parser would handle the Soroban CLI's XDR-based output format.
    if trimmed.is_empty() || trimmed == "void" || trimmed == "null" || trimmed == "[]" {
        return Ok(Vec::new());
    }

    // TODO: parse the full array format. For now, return empty.
    // The hackathon demo only needs get_current_root.
    Ok(Vec::new())
}

/// Run the `stellar` CLI with the given arguments.
fn run_stellar_cli(args: &[&str]) -> AppResult<std::process::Output> {
    Command::new("stellar")
        .args(args)
        .output()
        .map_err(|e| AppError::Validation(format!("failed to run stellar CLI: {e}")))
        .and_then(|output| {
            if output.status.success() {
                Ok(output)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(AppError::Validation(format!(
                    "stellar CLI failed: {stderr}"
                )))
            }
        })
}

/// Extract the transaction hash from the stellar CLI's stderr output.
/// The CLI prints lines like:
///   "🔗 https://stellar.expert/explorer/testnet/tx/<HASH>"
fn extract_tx_hash(stderr: &str) -> Option<String> {
    for line in stderr.lines() {
        if line.contains("stellar.expert/explorer/testnet/tx/") {
            let hash = line
                .rsplit('/')
                .next()?
                .trim();
            return Some(hash.to_string());
        }
    }
    None
}

/// Parse a RootEntry from the stellar CLI's output format.
/// The CLI prints struct results as:
///   {"vec":[{"u64":"1"},{"bytes":"0000..."},{"u64":"1782..."},{"string":"metadata"}]}
fn parse_root_entry(s: &str) -> AppResult<OnChainRoot> {
    // Try to parse as a Soroban CLI value. The format is a JSON-like
    // structure with typed values. We use a loose parser.
    let s = s.trim();

    // Extract the sequence (first u64)
    let sequence = extract_typed_value(s, "u64", 0)?
        .parse::<u64>()
        .map_err(|e| AppError::Validation(format!("parse sequence: {e}")))?;

    // Extract the root bytes (first bytes field)
    let root_hex = extract_typed_value(s, "bytes", 0)?;

    // Extract the timestamp (second u64)
    let timestamp = extract_typed_value(s, "u64", 1)?
        .parse::<u64>()
        .map_err(|e| AppError::Validation(format!("parse timestamp: {e}")))?;

    // Extract the metadata (first string field)
    let metadata = extract_typed_value(s, "string", 0).unwrap_or_default();

    Ok(OnChainRoot {
        sequence,
        root_hex,
        timestamp,
        metadata,
    })
}

/// Extract the Nth occurrence of a typed value from the CLI output.
/// e.g., extract_typed_value(s, "u64", 0) extracts the first {"u64":"..."} value.
fn extract_typed_value(s: &str, type_name: &str, n: usize) -> AppResult<String> {
    let pattern = format!("\"{type_name}\":\"");
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = s[start..].find(&pattern) {
        let abs_pos = start + pos + pattern.len();
        if let Some(end) = s[abs_pos..].find('"') {
            if count == n {
                return Ok(s[abs_pos..abs_pos + end].to_string());
            }
            count += 1;
            start = abs_pos + end + 1;
        } else {
            break;
        }
    }
    Err(AppError::Validation(format!(
        "could not find {type_name} #{n} in CLI output"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_typed_value_finds_first_u64() {
        let s = r#"{"vec":[{"u64":"1"},{"bytes":"abc"},{"u64":"999"}]}"#;
        assert_eq!(extract_typed_value(s, "u64", 0).unwrap(), "1");
        assert_eq!(extract_typed_value(s, "u64", 1).unwrap(), "999");
    }

    #[test]
    fn extract_typed_value_finds_bytes() {
        let s = r#"{"vec":[{"u64":"1"},{"bytes":"deadbeef"},{"u64":"999"}]}"#;
        assert_eq!(extract_typed_value(s, "bytes", 0).unwrap(), "deadbeef");
    }

    #[test]
    fn extract_typed_value_finds_string() {
        let s = r#"{"vec":[{"u64":"1"},{"bytes":"abc"},{"u64":"999"},{"string":"hello"}]}"#;
        assert_eq!(extract_typed_value(s, "string", 0).unwrap(), "hello");
    }

    #[test]
    fn extract_typed_value_returns_error_when_not_found() {
        let s = r#"{"vec":[{"u64":"1"}]}"#;
        assert!(extract_typed_value(s, "string", 0).is_err());
    }

    #[test]
    fn parse_root_entry_parses_full_output() {
        let s = r#"{"vec":[{"u64":"1"},{"bytes":"0000000000000000000000000000000000000000000000000000000000000001"},{"u64":"1782230809"},{"string":"test commit"}]}"#;
        let entry = parse_root_entry(s).unwrap();
        assert_eq!(entry.sequence, 1);
        assert_eq!(
            entry.root_hex,
            "0000000000000000000000000000000000000000000000000000000000000001"
        );
        assert_eq!(entry.timestamp, 1782230809);
        assert_eq!(entry.metadata, "test commit");
    }

    #[test]
    fn extract_tx_hash_finds_hash_in_url() {
        let stderr = "ℹ️  Simulating transaction...\n🔗 https://stellar.expert/explorer/testnet/tx/abc123def456\n✅ Transaction submitted";
        assert_eq!(
            extract_tx_hash(stderr),
            Some("abc123def456".to_string())
        );
    }

    #[test]
    fn extract_tx_hash_returns_none_when_no_url() {
        let stderr = "no url here";
        assert_eq!(extract_tx_hash(stderr), None);
    }

    #[test]
    fn contract_id_is_not_empty() {
        assert!(!CONTRACT_ID.is_empty());
        assert!(CONTRACT_ID.starts_with('C'));
    }
}
