//! Tauri IPC commands for the ZK audit log.
//!
//! These commands expose the audit log to the frontend:
//! - `audit_get_status` — current root, leaf count, event count.
//! - `audit_list_events` — list all recorded audit events.
//! - `audit_get_root` — get the current Merkle root as hex.
//! - `audit_generate_proof` — generate a Groth16 inclusion proof for a leaf.
//! - `audit_record_event` — manually record an audit event.

use serde::Serialize;
use tauri::State;

use crate::error::AppResult;
use crate::state::AppState;

/// Status snapshot of the audit log.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditStatus {
    pub root_hex: String,
    pub leaf_count: usize,
    pub event_count: usize,
    pub tree_height: u32,
}

/// Get the current audit log status.
///
/// Async because `root_hex()` computes the Merkle root via recursive Poseidon
/// hashing, which is CPU-intensive and must not block the main thread.
#[tauri::command]
pub async fn audit_get_status(state: State<'_, AppState>) -> AppResult<AuditStatus> {
    let audit = &state.audit_log;
    Ok(AuditStatus {
        root_hex: audit.root_hex()?,
        leaf_count: audit.leaf_count(),
        event_count: audit.event_count(),
        tree_height: 20,
    })
}

/// List all recorded audit events.
#[tauri::command]
pub async fn audit_list_events(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::audit::AuditEvent>> {
    Ok(state.audit_log.list_events())
}

/// Get the current Merkle root as a hex string.
///
/// Async because `root_hex()` computes the Merkle root via recursive Poseidon
/// hashing, which is CPU-intensive and must not block the main thread.
#[tauri::command]
pub async fn audit_get_root(state: State<'_, AppState>) -> AppResult<String> {
    state.audit_log.root_hex()
}

/// Generate a Groth16 inclusion proof for the event at the given index.
///
/// This requires the compiled circuit artifacts (R1CS + WASM) to be present.
/// The `r1cs_path` and `wasm_path` arguments point to the circuit build
/// output from `circom`.
#[tauri::command]
pub async fn audit_generate_proof(
    state: State<'_, AppState>,
    index: u64,
    r1cs_path: String,
    wasm_path: String,
) -> AppResult<ProofResult> {
    use ark_ff::{BigInteger, PrimeField};

    let inclusion = state.audit_log.prove_inclusion(index)?;

    // Generate the Groth16 proof.
    let prover = zk_audit::AuditProver::new(&r1cs_path, &wasm_path)?;
    let groth16_proof = prover.prove(&inclusion)?;
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof)?;

    let root_bigint = inclusion.root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let root_hex = hex::encode(&root_bytes);

    Ok(ProofResult {
        root_hex,
        leaf_index: index,
        proof: soroban_args.proof,
        vk: soroban_args.vk,
        pub_signals: soroban_args.pub_signals,
    })
}

/// The result of proof generation, ready for on-chain submission.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofResult {
    pub root_hex: String,
    pub leaf_index: u64,
    pub proof: zk_audit::serialize::SorobanProof,
    pub vk: zk_audit::serialize::SorobanVerifyingKey,
    pub pub_signals: Vec<String>,
}

/// Manually record an audit event (for testing or manual logging).
#[tauri::command]
pub async fn audit_record_event(
    state: State<'_, AppState>,
    operation: String,
    database: String,
    collection: String,
    payload: String,
) -> AppResult<u64> {
    use ark_bn254::Fr;
    use ark_ff::PrimeField;

    // Hash the payload into a field element. We use a simple hash-to-field
    // by taking the first 31 bytes of SHA-256 and interpreting as a field element.
    let hash = sha256_hash(&payload);
    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&hash[..31]);
    // Ensure it fits in the field (mask the top bit).
    bytes[31] &= 0x0F;
    let leaf = Fr::from_be_bytes_mod_order(&bytes);

    state
        .audit_log
        .record(&operation, &database, &collection, leaf)
}

fn sha256_hash(input: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_status_serializes_with_camel_case_fields() {
        let status = AuditStatus {
            root_hex: "abc".to_string(),
            leaf_count: 1,
            event_count: 2,
            tree_height: 20,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"rootHex\":\"abc\""), "rootHex must be camelCase: {json}");
        assert!(json.contains("\"leafCount\":1"), "leafCount must be camelCase: {json}");
        assert!(json.contains("\"eventCount\":2"), "eventCount must be camelCase: {json}");
        assert!(json.contains("\"treeHeight\":20"), "treeHeight must be camelCase: {json}");
    }

    #[test]
    fn proof_result_serializes_with_camel_case_fields() {
        let result = ProofResult {
            root_hex: "root".to_string(),
            leaf_index: 42,
            proof: zk_audit::serialize::SorobanProof {
                a: "a".to_string(),
                b: "b".to_string(),
                c: "c".to_string(),
            },
            vk: zk_audit::serialize::SorobanVerifyingKey {
                alpha: "alpha".to_string(),
                beta: "beta".to_string(),
                gamma: "gamma".to_string(),
                delta: "delta".to_string(),
                ic: vec!["ic".to_string()],
            },
            pub_signals: vec!["sig".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"rootHex\":\"root\""), "rootHex must be camelCase: {json}");
        assert!(json.contains("\"leafIndex\":42"), "leafIndex must be camelCase: {json}");
        assert!(json.contains("\"pubSignals\":[\"sig\"]"), "pubSignals must be camelCase: {json}");
    }
}
