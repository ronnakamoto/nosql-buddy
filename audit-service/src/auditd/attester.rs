//! Attester mode HTTP handlers.
//!
//! The attester daemon runs on the independent replica member (e.g.,
//! the auditor/regulator's mongo3). It:
//!
//! 1. Watches for new epoch commitments on-chain (via Stellar RPC).
//! 2. For each committed epoch with an oplog commitment, independently
//!    reads the oplog from the local replica member and computes the
//!    oplog hash.
//! 3. If the computed hash matches the on-chain oplog root, submits an
//!    attestation to the contract.
//! 4. If the hash doesn't match, raises an alert (omission detected).
//!
//! This is the trust anchor for the completeness guarantee (C1 fix).
//! The operator cannot forge the attester's observation because the
//! attester reads from its own independent replica member.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;

use crate::audit::oplog::{compute_oplog_range_hash, OplogTimestamp};
use crate::audit::stellar_native;
use crate::audit::stellar_rpc::StellarRpcClient;
use crate::error::AuditError;

use super::{ApiError, ApiResult, DaemonState};

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

/// Load or generate an ed25519 attester signing key.
///
/// If `key_file` exists, it is read as a 32-byte seed. Otherwise a new key is
/// generated, saved to the file (with restrictive permissions), and returned.
/// The public key is logged so the admin can authorize it on-chain.
pub fn load_or_generate_attester_key(key_file: &std::path::Path) -> Result<SigningKey, String> {
    if key_file.exists() {
        let bytes = std::fs::read(key_file)
            .map_err(|e| format!("failed to read attester key file {key_file:?}: {e}"))?;
        if bytes.len() != 32 {
            return Err(format!(
                "attester key file {key_file:?} must contain exactly 32 bytes, got {}",
                bytes.len()
            ));
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        return Ok(SigningKey::from_bytes(&seed));
    }

    use rand::rngs::OsRng;
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let seed = signing_key.to_bytes();

    if let Some(parent) = key_file.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create attester key directory {parent:?}: {e}"))?;
    }
    std::fs::write(key_file, &seed)
        .map_err(|e| format!("failed to write attester key file {key_file:?}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(key_file)
            .map_err(|e| format!("failed to read attester key file metadata: {e}"))?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(key_file, perms)
            .map_err(|e| format!("failed to set attester key file permissions: {e}"))?;
    }

    let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
    log::info!(
        "generated new attester key at {key_file:?}; public key: {public_key_hex} \
         (register this with authorize_attester)"
    );
    Ok(signing_key)
}

/// Sign the oplog attestation message: sha256(oplog_root || oplog_end_ts.to_be_bytes()).
pub fn sign_oplog_attestation(signing_key: &SigningKey, oplog_root_hex: &str, oplog_end_ts: u64) -> Result<String, String> {
    let oplog_root = hex::decode(oplog_root_hex)
        .map_err(|e| format!("invalid oplog root hex: {e}"))?;
    if oplog_root.len() != 32 {
        return Err(format!("oplog root must be 32 bytes, got {}", oplog_root.len()));
    }
    let mut oplog_root_bytes = [0u8; 32];
    oplog_root_bytes.copy_from_slice(&oplog_root);

    let mut message = [0u8; 40];
    message[0..32].copy_from_slice(&oplog_root_bytes);
    message[32..40].copy_from_slice(&oplog_end_ts.to_be_bytes());
    let message_hash = Sha256::digest(&message);
    let signature = signing_key.sign(&message_hash);
    Ok(hex::encode(signature.to_bytes()))
}

/// Status of the attester daemon.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttesterStatus {
    pub mode: String,
    pub connected: bool,
    pub last_scanned_sequence: Option<u64>,
    pub attestations_submitted: u64,
    pub alerts: Vec<AttesterAlert>,
}

/// An alert raised by the attester (e.g., oplog hash mismatch).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttesterAlert {
    pub sequence: u64,
    pub alert_type: String,
    pub message: String,
    pub on_chain_oplog_root: String,
    pub computed_oplog_root: String,
    pub timestamp: String,
}

/// Result of scanning and attesting for one epoch.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestResult {
    pub sequence: u64,
    pub on_chain_oplog_root: String,
    pub computed_oplog_root: String,
    pub matched: bool,
    pub attested: bool,
    pub alert: Option<AttesterAlert>,
}

/// GET /attest/status — return the attester daemon's status.
pub async fn get_status(
    state: State<Arc<DaemonState>>,
) -> ApiResult<AttesterStatus> {
    Ok(Json(AttesterStatus {
        mode: "attest".to_string(),
        connected: state.mongo_client.is_some(),
        last_scanned_sequence: None,
        attestations_submitted: 0,
        alerts: vec![],
    }))
}

/// POST /attest/scan — scan for unattested epochs and submit attestations.
///
/// This is the main attester loop. It:
/// 1. Gets the current on-chain root sequence.
/// 2. For each sequence that hasn't been attested yet:
///    a. Gets the on-chain oplog commitment.
///    b. Independently computes the oplog hash from the local replica.
///    c. If they match, submits an attestation.
///    d. If they don't match, raises an alert.
pub async fn scan_and_attest(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Vec<AttestResult>> {
    let client = state.mongo_client.as_ref().ok_or_else(|| {
        ApiError(AuditError::Validation(
            "attester mode requires a MongoDB connection — use --mongo-uri".to_string(),
        ))
    })?;

    let rpc = StellarRpcClient::with_url(&state.rpc_url);
    let current_root = rpc.get_current_root().await.map_err(ApiError::from)?;

    let current_seq = match current_root {
        Some(root) => root.sequence,
        None => {
            return Ok(Json(vec![]));
        }
    };

    let mut results = Vec::new();

    // Scan all committed epochs from 1 to current.
    // In production, we'd track which ones we've already attested and
    // only scan new ones. For the hackathon, we scan all and skip
    // duplicates (the contract rejects duplicate attestations).
    for seq in 1..=current_seq {
        let result = attest_epoch(&state, client, seq).await;
        match result {
            Ok(r) => results.push(r),
            Err(e) => {
                log::warn!("attest: failed for sequence {seq}: {e}");
            }
        }
    }

    Ok(Json(results))
}

/// Attest a single epoch.
async fn attest_epoch(
    state: &Arc<DaemonState>,
    client: &mongodb::Client,
    sequence: u64,
) -> Result<AttestResult, String> {
    // 1. Get the on-chain oplog commitment via native contract simulation.
    let stellar_kp = state.attester_stellar_keypair.as_ref().ok_or_else(|| {
        "no attester Stellar keypair configured — use --attester-secret-key or ATTESTER_SECRET_KEY env var".to_string()
    })?;
    let chain = &state.chain;
    let on_chain = stellar_native::get_oplog_commitment_native(
        sequence,
        stellar_kp,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await
    .map_err(|e| format!("get_oplog_commitment_native({sequence}): {e}"))?;

    let on_chain_oplog = match on_chain {
        Some(oc) => oc,
        None => {
            // No oplog commitment for this epoch — skip.
            return Ok(AttestResult {
                sequence,
                on_chain_oplog_root: "none".to_string(),
                computed_oplog_root: "n/a".to_string(),
                matched: false,
                attested: false,
                alert: None,
            });
        }
    };

    // 2. Unpack the oplog timestamps from the on-chain commitment.
    let oplog_start_ts = OplogTimestamp::unpack_u64(on_chain_oplog.oplog_start_ts);
    let oplog_end_ts = OplogTimestamp::unpack_u64(on_chain_oplog.oplog_end_ts);

    // 3. Independently compute the oplog hash from the local replica.
    let computed = compute_oplog_range_hash(client, sequence, oplog_start_ts, oplog_end_ts)
        .await
        .map_err(|e| format!("compute_oplog_range_hash: {e}"))?;

    let matched = computed.oplog_merkle_root_hex == on_chain_oplog.oplog_root_hex;

    // 4. If matched, submit attestation. If not, raise alert.
    let mut alert = None;
    let mut attested = false;

    if matched {
        let attester_key = state.attester_key.as_ref().ok_or_else(|| {
            "attester ed25519 key not configured — use --attester-key-file".to_string()
        })?;

        let signature_hex = sign_oplog_attestation(
            attester_key,
            &on_chain_oplog.oplog_root_hex,
            on_chain_oplog.oplog_end_ts,
        )?;

        let submit_result = stellar_native::attest_oplog_native(
            stellar_kp,
            sequence,
            &signature_hex,
            &chain.rpc_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await;

        match submit_result {
            Ok(()) => {
                attested = true;
                log::info!(
                    "attest: sequence {sequence} attested — oplog root matches: {}",
                    computed.oplog_merkle_root_hex
                );
            }
            Err(e) => {
                // Duplicate attestation is expected if we've already attested.
                log::warn!("attest: sequence {sequence} attestation failed: {e}");
            }
        }
    } else {
        // OMISSION DETECTED — the oplog hash doesn't match!
        alert = Some(AttesterAlert {
            sequence,
            alert_type: "oplog_hash_mismatch".to_string(),
            message: format!(
                "On-chain oplog root {} does not match independently computed root {} — possible omission detected!",
                on_chain_oplog.oplog_root_hex, computed.oplog_merkle_root_hex
            ),
            on_chain_oplog_root: on_chain_oplog.oplog_root_hex.clone(),
            computed_oplog_root: computed.oplog_merkle_root_hex.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });
        log::error!(
            "attest: ALERT — sequence {sequence} oplog hash mismatch! on_chain={} computed={}",
            on_chain_oplog.oplog_root_hex,
            computed.oplog_merkle_root_hex
        );
    }

    Ok(AttestResult {
        sequence,
        on_chain_oplog_root: on_chain_oplog.oplog_root_hex,
        computed_oplog_root: computed.oplog_merkle_root_hex,
        matched,
        attested,
        alert,
    })
}

/// GET /attest/attestations/:sequence — get attestations for an epoch.
pub async fn list_attestations(
    state: State<Arc<DaemonState>>,
    Path(sequence): Path<u64>,
) -> ApiResult<serde_json::Value> {
    use crate::audit::stellar_native;

    let kp = stellar_native::generate_keypair();
    let attesters = stellar_native::get_oplog_attestations_native(
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
        sequence,
    )
    .await
    .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "sequence": sequence,
        "count": attesters.len(),
        "attesters": attesters,
    })))
}

