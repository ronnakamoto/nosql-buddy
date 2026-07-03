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

use crate::error::{AuditError, AuditResult};

/// The deployed Soroban contract ID on Stellar testnet.
pub const CONTRACT_ID: &str = "CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5";

/// The Stellar identity name authorized to commit roots.
/// Override with the `STELLAR_IDENTITY` environment variable.
fn source_identity() -> String {
    std::env::var("STELLAR_IDENTITY").unwrap_or_else(|_| "spike".to_string())
}

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

/// The result of on-chain proof verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyInclusionResult {
    /// The transaction hash on Stellar.
    pub tx_hash: String,
    /// Whether the on-chain pairing check returned true.
    pub verified: bool,
}

/// The result of a read-only (simulated) on-chain proof verification.
///
/// Unlike [`VerifyInclusionResult`], this comes from a Soroban RPC
/// `simulateTransaction` call: no transaction is submitted, no fee is paid,
/// and no signing key is required. The pairing check still runs inside the
/// Soroban runtime against the contract's pinned verifying key and committed
/// root index, so the verdict is exactly as trustworthy as a submitted
/// transaction — anyone (an auditor, a judge, a script) can obtain it with
/// only the RPC URL and contract ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadonlyVerifyResult {
    /// Whether the on-chain pairing check returned true.
    pub verified: bool,
    /// Human-readable failure reason when `verified` is false (e.g. the
    /// root was never committed, the proof encoding was malformed, or the
    /// pairing check itself failed).
    pub reason: Option<String>,
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

/// The result of the contract's `verify_attestation(sequence)` query.
///
/// This is the **independent** attestation verdict: it counts how many
/// distinct, currently-authorized attesters signed the exact oplog root the
/// operator committed, and compares that against the on-chain K-of-N threshold.
/// The operator cannot fabricate it, because attestations are ed25519-verified
/// against keys the admin authorized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainAttestationVerification {
    /// The committed sequence this verdict is for.
    pub sequence: u64,
    /// The committed oplog root (hex), or empty if none was committed.
    pub oplog_root_hex: String,
    /// Total attestations on record for this sequence (authorized or not).
    pub attestation_count: u32,
    /// Attestations from currently-authorized attesters.
    pub authorized_count: u32,
    /// The on-chain K-of-N threshold required for a `verified` verdict.
    pub threshold: u32,
    /// True only when the threshold is met by distinct authorized attesters.
    pub all_match: bool,
    /// The contract's verdict string (verified / threshold_not_met /
    /// unauthorized_attester / no_attestations).
    pub verdict: String,
}

/// Commit a Merkle root to the Soroban contract on Stellar testnet.
///
/// This calls `commit_root(root, metadata)` on the contract. The `root_hex`
/// must be a 64-character hex string (32 bytes). The `metadata` is an
/// arbitrary string stored on-chain with the commitment.
pub fn commit_root(root_hex: &str, metadata: &str) -> AuditResult<CommitResult> {
    // The stellar CLI expects the root as a hex-encoded byte string.
    // The contract's `commit_root` takes `Bytes` which the CLI accepts
    // as a hex string.
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
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
    let sequence: u64 = parse_u64_output(stdout.trim())
        .map_err(|e| AuditError::Validation(format!("failed to parse commit_root sequence: {e} (stdout: {stdout})")))?;

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
pub fn get_current_root() -> AuditResult<Option<OnChainRoot>> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
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

/// Commit a Merkle root with an oplog completeness commitment.
///
/// This calls `commit_root_with_oplog` on the contract, storing both
/// the audit log root and the oplog Merkle root on-chain. The oplog
/// root binds the audit log to MongoDB's oplog, proving completeness.
///
/// Timestamps are packed as `(time << 32) | increment`.
pub fn commit_root_with_oplog(
    root_hex: &str,
    oplog_root_hex: &str,
    oplog_start_ts: u64,
    oplog_end_ts: u64,
    oplog_entry_count: u64,
    metadata: &str,
) -> AuditResult<CommitResult> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "commit_root_with_oplog",
        "--root",
        root_hex,
        "--oplog_root",
        oplog_root_hex,
        "--oplog_start_ts",
        &oplog_start_ts.to_string(),
        "--oplog_end_ts",
        &oplog_end_ts.to_string(),
        "--oplog_entry_count",
        &oplog_entry_count.to_string(),
        "--metadata",
        metadata,
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sequence: u64 = parse_u64_output(stdout.trim())
        .map_err(|e| AuditError::Validation(format!("failed to parse commit_root_with_oplog sequence: {e} (stdout: {stdout})")))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    let tx_hash = extract_tx_hash(&stderr).unwrap_or_default();

    Ok(CommitResult {
        sequence,
        tx_hash,
        root_hex: root_hex.to_string(),
    })
}

/// Authorize an attester address on the contract (admin only).
///
/// `public_key_hex` is the 32-byte ed25519 public key (64 hex chars) that the
/// attester will use to sign oplog attestations.
pub fn authorize_attester(attester_address: &str, public_key_hex: &str) -> AuditResult<()> {
    let _output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "authorize_attester",
        "--attester",
        attester_address,
        "--public_key",
        public_key_hex,
    ])?;
    Ok(())
}

/// Submit an oplog attestation to the contract.
///
/// The attester's Stellar identity must sign the transaction. The
/// `signature_hex` is a 64-byte ed25519 signature (128 hex chars) over
/// `sha256(oplog_root || oplog_end_ts.to_be_bytes())`.
pub fn attest_oplog(
    attester_identity: &str,
    attester_address: &str,
    sequence: u64,
    signature_hex: &str,
) -> AuditResult<()> {
    let _output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        attester_identity,
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "attest_oplog",
        "--attester",
        attester_address,
        "--sequence",
        &sequence.to_string(),
        "--signature",
        signature_hex,
    ])?;
    Ok(())
}

/// Get the oplog commitment for a given sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnChainOplogCommitment {
    pub sequence: u64,
    pub oplog_root_hex: String,
    pub oplog_start_ts: u64,
    pub oplog_end_ts: u64,
    pub oplog_entry_count: u64,
}

/// Get the oplog commitment for a given sequence from the contract.
pub fn get_oplog_commitment(sequence: u64) -> AuditResult<Option<OnChainOplogCommitment>> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
        "--network",
        NETWORK,
        "--id",
        CONTRACT_ID,
        "--",
        "get_oplog_commitment",
        "--sequence",
        &sequence.to_string(),
    ])?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();

    if trimmed.is_empty() || trimmed == "void" || trimmed == "null" {
        return Ok(None);
    }

    // Parse the oplog commitment from the CLI output.
    // The output format is similar to RootEntry but with different fields.
    // For the hackathon, we do a loose parse.
    let oplog_root_hex = extract_typed_value(trimmed, "bytes", 0).unwrap_or_default();
    let oplog_start_ts = extract_typed_value(trimmed, "u64", 0)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let oplog_end_ts = extract_typed_value(trimmed, "u64", 1)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let oplog_entry_count = extract_typed_value(trimmed, "u64", 2)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    Ok(Some(OnChainOplogCommitment {
        sequence,
        oplog_root_hex,
        oplog_start_ts,
        oplog_end_ts,
        oplog_entry_count,
    }))
}

/// Get the root history from the Soroban contract.
pub fn get_root_history(limit: u32) -> AuditResult<Vec<OnChainRoot>> {
    let output = run_stellar_cli(&[
        "contract",
        "invoke",
        "--source",
        &source_identity(),
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
fn run_stellar_cli(args: &[&str]) -> AuditResult<std::process::Output> {
    let identity = source_identity();
    Command::new("stellar")
        .args(args)
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AuditError::Validation(
                    "stellar CLI not found in PATH. Install it: https://docs.stellar.org/tools/developer-tools/cli/install".to_string(),
                )
            } else {
                AuditError::Validation(format!("failed to run stellar CLI: {e}"))
            }
        })
        .and_then(|output| {
            if output.status.success() {
                Ok(output)
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut msg = format!("stellar CLI failed: {stderr}");
                if stderr.contains("identity") || stderr.contains("signing key") {
                    msg = format!("{msg}\n\nHINT: identity '{identity}' not found. Create it with:\n  stellar keys generate --global {identity} --network testnet\n\nOr set another identity via STELLAR_IDENTITY env var.");
                } else if stderr.contains("network") {
                    msg = format!("{msg}\n\nHINT: add testnet with:\n  stellar network add testnet --rpc-url https://soroban-testnet.stellar.org:443 --global");
                } else if !stdout.is_empty() {
                    msg = format!("{msg}\nstdout: {stdout}");
                }
                Err(AuditError::Validation(msg))
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
///
/// The stellar CLI's output format has changed across versions:
///
/// - **New (>=22, flat JSON)**: the CLI prints struct results as a flat JSON
///   object with field names matching the contract struct:
///   `{"metadata":"...","root":"2e1a...","sequence":2,"timestamp":1782276789}`
///
/// - **Old (<=21, typed vec)**: the CLI printed struct results as a JSON-like
///   structure with typed values:
///   `{"vec":[{"u64":"1"},{"bytes":"0000..."},{"u64":"1782..."},{"string":"metadata"}]}`
///
/// We try the new flat-JSON format first (via serde_json), then fall back to
/// the legacy typed-vec parser so both CLI versions keep working.
fn parse_root_entry(s: &str) -> AuditResult<OnChainRoot> {
    let s = s.trim();

    // Try the new flat-JSON format first.
    if let Ok(entry) = parse_root_entry_flat(s) {
        return Ok(entry);
    }

    // Fall back to the legacy typed-vec format.
    parse_root_entry_legacy(s)
}

/// Parse the new (stellar CLI >=22) flat JSON output format.
fn parse_root_entry_flat(s: &str) -> AuditResult<OnChainRoot> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FlatRoot {
        sequence: serde_json::Number,
        root: String,
        timestamp: serde_json::Number,
        #[serde(default)]
        metadata: String,
    }

    let flat: FlatRoot = serde_json::from_str(s)
        .map_err(|e| AuditError::Validation(format!("parse flat root entry: {e}")))?;

    let sequence = flat
        .sequence
        .as_u64()
        .ok_or_else(|| AuditError::Validation(format!("sequence is not a u64: {}", flat.sequence)))?;
    let timestamp = flat
        .timestamp
        .as_u64()
        .ok_or_else(|| AuditError::Validation(format!("timestamp is not a u64: {}", flat.timestamp)))?;

    // The CLI may print the root as a hex string (with or without a "0x"
    // prefix). Normalize to bare lowercase hex.
    let root_hex = flat.root.trim().trim_start_matches("0x").to_lowercase();

    Ok(OnChainRoot {
        sequence,
        root_hex,
        timestamp,
        metadata: flat.metadata,
    })
}

/// Parse the legacy (stellar CLI <=21) typed-vec output format:
///   {"vec":[{"u64":"1"},{"bytes":"0000..."},{"u64":"1782..."},{"string":"metadata"}]}
fn parse_root_entry_legacy(s: &str) -> AuditResult<OnChainRoot> {
    // Extract the sequence (first u64)
    let sequence = extract_typed_value(s, "u64", 0)?
        .parse::<u64>()
        .map_err(|e| AuditError::Validation(format!("parse sequence: {e}")))?;

    // Extract the root bytes (first bytes field)
    let root_hex = extract_typed_value(s, "bytes", 0)?;

    // Extract the timestamp (second u64)
    let timestamp = extract_typed_value(s, "u64", 1)?
        .parse::<u64>()
        .map_err(|e| AuditError::Validation(format!("parse timestamp: {e}")))?;

    // Extract the metadata (first string field)
    let metadata = extract_typed_value(s, "string", 0).unwrap_or_default();

    Ok(OnChainRoot {
        sequence,
        root_hex,
        timestamp,
        metadata,
    })
}

/// Parse a `u64` from the stellar CLI's stdout for a `u64`-returning invocation.
///
/// The stellar CLI prints a bare `u64` return value differently across
/// versions:
/// - New (>=22): a bare number, e.g. `3`
/// - Old (<=21): a typed JSON value, e.g. `"3"` or `{"u64":"3"}`
///
/// This helper handles all of these.
fn parse_u64_output(s: &str) -> Result<u64, String> {
    let s = s.trim();
    // Bare number (new CLI).
    if let Ok(n) = s.parse::<u64>() {
        return Ok(n);
    }
    // Quoted number, e.g. `"3"`.
    let unquoted = s.trim_matches('"');
    if let Ok(n) = unquoted.parse::<u64>() {
        return Ok(n);
    }
    // Typed JSON value, e.g. `{"u64":"3"}` (legacy CLI).
    if let Ok(v) = extract_typed_value(s, "u64", 0) {
        if let Ok(n) = v.parse::<u64>() {
            return Ok(n);
        }
    }
    Err(format!("unrecognized u64 output: {s:?}"))
}


/// e.g., extract_typed_value(s, "u64", 0) extracts the first {"u64":"..."} value.
fn extract_typed_value(s: &str, type_name: &str, n: usize) -> AuditResult<String> {
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
    Err(AuditError::Validation(format!(
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
    fn parse_root_entry_parses_new_flat_format() {
        // Output from stellar CLI 27.0.0 (real captured output).
        let s = r#"{"metadata":"events=2 leaves=2","root":"2e1a1c70812d9e9c445800c9e63c7f13b0b9bd2c57a528121d7911e0f7b1a18b","sequence":2,"timestamp":1782276789}"#;
        let entry = parse_root_entry(s).unwrap();
        assert_eq!(entry.sequence, 2);
        assert_eq!(
            entry.root_hex,
            "2e1a1c70812d9e9c445800c9e63c7f13b0b9bd2c57a528121d7911e0f7b1a18b"
        );
        assert_eq!(entry.timestamp, 1782276789);
        assert_eq!(entry.metadata, "events=2 leaves=2");
    }

    #[test]
    fn parse_root_entry_flat_strips_0x_prefix() {
        let s = r#"{"metadata":"","root":"0x2e1a1c","sequence":5,"timestamp":1}"#;
        let entry = parse_root_entry(s).unwrap();
        assert_eq!(entry.root_hex, "2e1a1c");
        assert_eq!(entry.sequence, 5);
    }

    #[test]
    fn parse_u64_output_handles_bare_number() {
        assert_eq!(parse_u64_output("3").unwrap(), 3);
        assert_eq!(parse_u64_output("  42 \n").unwrap(), 42);
    }

    #[test]
    fn parse_u64_output_handles_quoted_number() {
        assert_eq!(parse_u64_output("\"7\"").unwrap(), 7);
    }

    #[test]
    fn parse_u64_output_handles_typed_legacy() {
        assert_eq!(parse_u64_output(r#"{"u64":"9"}"#).unwrap(), 9);
    }

    #[test]
    fn parse_u64_output_rejects_garbage() {
        assert!(parse_u64_output("not a number").is_err());
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
