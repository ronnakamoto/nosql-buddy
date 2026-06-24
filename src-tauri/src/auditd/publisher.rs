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
use crate::audit::stellar::{self, CommitResult, OnChainRoot};
use crate::audit::stellar_rpc::StellarRpcClient;
use crate::error::AppError;

use super::{ApiError, ApiResult, DaemonState};

// ─── Epoch management ─────────────────────────────────────────────────

pub async fn close_epoch(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Epoch> {
    let epoch = state
        .epoch_manager
        .close_current_epoch(&state.audit_log)
        .map_err(ApiError::from)?;
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
            ApiError(AppError::Validation(format!(
                "epoch {epoch_number} not found"
            )))
        })?;

    if epoch.is_open() {
        return Err(ApiError(AppError::Validation(format!(
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
    let metadata = match &cid {
        Some(c) => format!("epoch={epoch_number} cid={c}"),
        None => format!("epoch={epoch_number}"),
    };

    // Commit via stellar CLI (writes still use CLI).
    let root_hex_clone = root_hex.clone();
    let metadata_clone = metadata.clone();
    let result = tokio::task::spawn_blocking(move || {
        stellar::commit_root(&root_hex_clone, &metadata_clone)
    })
    .await
    .map_err(|e| AppError::Validation(format!("commit_root task join: {e}")))
    .map_err(ApiError::from)??;

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
    let client = StellarRpcClient::with_url(&state.rpc_url);
    let root = client.get_current_root().await.map_err(ApiError::from)?;
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
            ApiError(AppError::Validation(format!(
                "epoch {epoch_number} not found"
            )))
        })?;

    let end_index = epoch.end_index.ok_or_else(|| {
        ApiError(AppError::Validation(format!(
            "epoch {epoch_number} is still open — close it before publishing to IPFS"
        )))
    })?;

    // Read the JSONL log and extract events for this epoch.
    let events_path = state.data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path).map_err(AppError::from)?
    } else {
        String::new()
    };

    let batch_content = super::extract_epoch_batch(&events_jsonl, epoch.start_index, end_index);
    if batch_content.is_empty() {
        return Err(ApiError(AppError::Validation(format!(
            "no events found for epoch {epoch_number} (range {}-{})",
            epoch.start_index, end_index
        ))));
    }

    let config = state.ipfs_config.clone();
    let result =
        crate::audit::ipfs::publish_epoch_batch(&config, epoch_number, &batch_content)
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
    let config = &state.ipfs_config;
    let result = crate::audit::ipfs::check_daemon(config)
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
            ApiError(AppError::Validation(format!(
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

