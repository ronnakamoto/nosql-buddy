//! Publisher mode HTTP handlers.
//!
//! These handlers expose the audit log's publisher functionality via HTTP:
//! epoch management, on-chain commitment, IPFS publishing, and K-of-N
//! threshold attestation.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;

use crate::audit::attestation::{Attestation, AttestationStatus, Publisher};
use crate::audit::epoch::Epoch;
use crate::audit::ipfs::IpfsPublishResult;
use crate::audit::stellar::{CommitResult, OnChainRoot};
use crate::audit::stellar_native;
use crate::error::AuditError;

use super::{ApiError, ApiResult, DaemonState};

// ─── Epoch management ─────────────────────────────────────────────────

pub async fn close_epoch(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Epoch> {
    let mut epoch = state
        .epoch_manager
        .close_current_epoch(&state.audit_log)
        .map_err(ApiError::from)?;

    // If we have a MongoDB client, compute and attach the oplog hash.
    // This binds the audit log epoch to the oplog, proving completeness.
    if let Some(client) = &state.mongo_client {
        match super::compute_and_attach_oplog_hash(&state, client, epoch.epoch_number).await {
            Ok((oplog_range, updated_epoch)) => {
                log::info!(
                    "close_epoch: oplog hash attached to epoch {}: root={}, entries={}",
                    epoch.epoch_number,
                    oplog_range.oplog_merkle_root_hex,
                    oplog_range.entry_count
                );
                // Use the updated epoch returned by attach_oplog_hash to avoid
                // the racy list/reload pattern.
                epoch = updated_epoch;
            }
            Err(e) => {
                if state.oplog_hash_required {
                    return Err(ApiError(AuditError::Validation(format!(
                        "close_epoch: oplog hash computation failed for epoch {}: {e}",
                        epoch.epoch_number
                    ))));
                }
                log::warn!(
                    "close_epoch: oplog hash computation failed for epoch {}: {e}",
                    epoch.epoch_number
                );
            }
        }
    }

    Ok(Json(epoch))
}

pub async fn list_epochs(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Vec<Epoch>> {
    Ok(Json(state.epoch_manager.list_epochs()))
}

pub async fn current_epoch(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Epoch> {
    Ok(Json(state.epoch_manager.current_epoch()))
}

// ─── On-chain commitment ──────────────────────────────────────────────

pub async fn commit_epoch(
    state: State<Arc<DaemonState>>,
    Path(epoch_number): Path<u64>,
) -> ApiResult<CommitResult> {
    // Find the epoch and get its root.
    let epochs = state.epoch_manager.list_epochs();
    let epoch = epochs
        .iter()
        .find(|e| e.epoch_number == epoch_number)
        .ok_or_else(|| {
            ApiError(AuditError::Validation(format!(
                "epoch {epoch_number} not found"
            )))
        })?;

    if epoch.is_open() {
        return Err(ApiError(AuditError::Validation(format!(
            "epoch {epoch_number} is still open — close it before committing"
        ))));
    }

    let root_hex = epoch.root_hex.clone().unwrap_or_else(|| {
        // If root_hex wasn't stored, compute it from the audit log.
        // This shouldn't happen if close_epoch was called, but handle it.
        state.audit_log.root_hex().unwrap_or_default()
    });

    // Get the IPFS CID if available, include it in metadata.
    let cid = state.audit_log.load_ipfs_cid(epoch_number).ok().flatten();

    // Include the oplog hash in the metadata if present.
    // This binds the on-chain commitment to the oplog completeness proof.
    let oplog_root = epoch.oplog_merkle_root_hex.as_deref();
    let metadata = match (&cid, oplog_root) {
        (Some(c), Some(oplog)) => format!(
            "epoch={epoch_number} cid={c} oplog_root={oplog} oplog_entries={}",
            epoch.oplog_entry_count.unwrap_or(0)
        ),
        (None, Some(oplog)) => format!(
            "epoch={epoch_number} oplog_root={oplog} oplog_entries={}",
            epoch.oplog_entry_count.unwrap_or(0)
        ),
        (Some(c), None) => format!("epoch={epoch_number} cid={c}"),
        (None, None) => format!("epoch={epoch_number}"),
    };

    // Commit on-chain using native signing (ed25519 + Soroban RPC).
    // A signing keypair is required — pass --secret-key or STELLAR_SECRET_KEY.
    let kp = state.signing_keypair.as_ref().ok_or_else(|| {
        ApiError(AuditError::Validation(
            "no signing keypair configured — use --secret-key or STELLAR_SECRET_KEY env var".to_string(),
        ))
    })?;
    let chain = &state.chain;
    let commit_result = if let Some(oplog_root_hex) = &epoch.oplog_merkle_root_hex {
        let oplog_start = epoch.oplog_start_ts.map(|t| t.pack_u64()).unwrap_or(0);
        let oplog_end = epoch.oplog_end_ts.map(|t| t.pack_u64()).unwrap_or(0);
        let oplog_count = epoch.oplog_entry_count.unwrap_or(0);

        stellar_native::commit_root_with_oplog_native(
            &root_hex,
            oplog_root_hex,
            oplog_start,
            oplog_end,
            oplog_count,
            &metadata,
            kp,
            &chain.rpc_url,
            &chain.horizon_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await
    } else {
        stellar_native::commit_root_native(
            &root_hex,
            &metadata,
            kp,
            &chain.rpc_url,
            &chain.horizon_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await
    };

    // If the contract rejected with RootAlreadyCommitted (error #2), this root
    // was committed by a previous publisher instance.  Query the live contract
    // history to find the real on-chain sequence for this root and recover.
    let result = match commit_result {
        Ok(r) => r,
        Err(ref e) if e.to_string().contains("#2") => {
            // Root is already on-chain — look it up from the contract directly.
            let probe_kp = stellar_native::generate_keypair();
            let history = stellar_native::get_root_history_native(
                &probe_kp,
                &state.rpc_url,
                &state.chain.contract_id,
                20,
            )
            .await
            .map_err(ApiError::from)?;

            let existing = history
                .into_iter()
                .find(|e| e.root_hex == root_hex)
                .ok_or_else(|| ApiError(AuditError::Validation(format!(
                    "root {root_hex} is already committed but could not be found in the on-chain history"
                ))))?;

            log::info!(
                "commit_epoch {epoch_number}: root already on-chain at sequence {} — marking committed",
                existing.sequence
            );

            // tx_hash is not stored in the contract; use empty string to
            // signal "recovered from duplicate" — it's still a real on-chain fact.
            CommitResult {
                sequence: existing.sequence,
                tx_hash: String::new(),
                root_hex: existing.root_hex,
            }
        }
        Err(e) => return Err(ApiError::from(e)),
    };

    // Mark the epoch as committed.
    state
        .epoch_manager
        .mark_committed(epoch_number, result.tx_hash.clone())
        .map_err(ApiError::from)?;

    Ok(Json(result))
}

pub async fn get_onchain_root(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Option<OnChainRoot>> {
    // Use contract simulation via the chain's contract ID for correct reads.
    let kp = stellar_native::generate_keypair();
    let root = stellar_native::get_current_root_native(
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
    )
    .await
    .map_err(ApiError::from)?;
    Ok(Json(root))
}

// ─── IPFS publishing ──────────────────────────────────────────────────

pub async fn publish_ipfs(
    state: State<Arc<DaemonState>>,
    Path(epoch_number): Path<u64>,
) -> ApiResult<IpfsPublishResult> {
    // Find the epoch and get its event range.
    let epochs = state.epoch_manager.list_epochs();
    let epoch = epochs
        .iter()
        .find(|e| e.epoch_number == epoch_number)
        .ok_or_else(|| {
            ApiError(AuditError::Validation(format!(
                "epoch {epoch_number} not found"
            )))
        })?;

    let end_index = epoch.end_index.ok_or_else(|| {
        ApiError(AuditError::Validation(format!(
            "epoch {epoch_number} is still open — close it before publishing to IPFS"
        )))
    })?;

    // Read the JSONL log and extract events for this epoch.
    let events_path = state.data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path).map_err(AuditError::from)?
    } else {
        String::new()
    };

    let batch_content = super::extract_epoch_batch(&events_jsonl, epoch.start_index, end_index);
    if batch_content.is_empty() {
        return Err(ApiError(AuditError::Validation(format!(
            "no events found for epoch {epoch_number} (range {}-{})",
            epoch.start_index, end_index
        ))));
    }

    let result = super::publish_epoch_batch_to_ipfs(&state, epoch_number, &batch_content)
        .await
        .map_err(ApiError::from)?;

    // Save the CID to sled.
    state
        .audit_log
        .save_ipfs_cid(epoch_number, &result.cid)
        .map_err(ApiError::from)?;

    Ok(Json(result))
}

pub async fn get_ipfs_cid(
    state: State<Arc<DaemonState>>,
    Path(epoch_number): Path<u64>,
) -> ApiResult<Option<String>> {
    let cid = state
        .audit_log
        .load_ipfs_cid(epoch_number)
        .map_err(ApiError::from)?;
    Ok(Json(cid))
}

pub async fn check_ipfs(
    state: State<Arc<DaemonState>>,
) -> ApiResult<bool> {
    let result = super::check_ipfs_backend(&state)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(result))
}

// ─── Multi-publisher threshold attestation ────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AddPublisherRequest {
    pub public_key: String,
    pub name: String,
}

pub async fn add_publisher(
    state: State<Arc<DaemonState>>,
    Json(req): Json<AddPublisherRequest>,
) -> ApiResult<Publisher> {
    let publisher = state
        .attestation_manager
        .add_publisher(req.public_key, req.name)
        .map_err(ApiError::from)?;
    Ok(Json(publisher))
}

pub async fn remove_publisher(
    state: State<Arc<DaemonState>>,
    Path(key): Path<String>,
) -> ApiResult<()> {
    state
        .attestation_manager
        .remove_publisher(&key)
        .map_err(ApiError::from)?;
    Ok(Json(()))
}

pub async fn list_publishers(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Vec<Publisher>> {
    let publishers = state
        .attestation_manager
        .list_publishers()
        .map_err(ApiError::from)?;
    Ok(Json(publishers))
}

#[derive(Debug, Deserialize)]
pub struct SubmitAttestationRequest {
    pub epoch_number: u64,
    pub root_hex: String,
    pub publisher_public_key: String,
    pub signature_hex: String,
}

pub async fn submit_attestation(
    state: State<Arc<DaemonState>>,
    Json(req): Json<SubmitAttestationRequest>,
) -> ApiResult<Attestation> {
    let attestation = state
        .attestation_manager
        .submit_attestation(
            req.epoch_number,
            &req.root_hex,
            &req.publisher_public_key,
            &req.signature_hex,
        )
        .map_err(ApiError::from)?;
    Ok(Json(attestation))
}

pub async fn list_attestations(
    state: State<Arc<DaemonState>>,
    Path(epoch_number): Path<u64>,
) -> ApiResult<Vec<Attestation>> {
    let attestations = state
        .attestation_manager
        .list_attestations(epoch_number)
        .map_err(ApiError::from)?;
    Ok(Json(attestations))
}

pub async fn attestation_status(
    state: State<Arc<DaemonState>>,
    Path(epoch_number): Path<u64>,
) -> ApiResult<AttestationStatus> {
    // Get the root hex for this epoch.
    let epochs = state.epoch_manager.list_epochs();
    let epoch = epochs
        .iter()
        .find(|e| e.epoch_number == epoch_number)
        .ok_or_else(|| {
            ApiError(AuditError::Validation(format!(
                "epoch {epoch_number} not found"
            )))
        })?;

    let root_hex = epoch.root_hex.clone().unwrap_or_default();
    let status = state
        .attestation_manager
        .get_status(epoch_number, &root_hex)
        .map_err(ApiError::from)?;
    Ok(Json(status))
}

#[derive(Debug, Deserialize)]
pub struct SetThresholdRequest {
    pub threshold: usize,
}

pub async fn set_threshold(
    state: State<Arc<DaemonState>>,
    Json(req): Json<SetThresholdRequest>,
) -> ApiResult<()> {
    state.attestation_manager.set_threshold(req.threshold);
    Ok(Json(()))
}

pub async fn get_threshold(
    state: State<Arc<DaemonState>>,
) -> ApiResult<usize> {
    Ok(Json(state.attestation_manager.threshold()))
}

