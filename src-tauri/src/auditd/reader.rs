//! Reader mode HTTP handlers.
//!
//! Reader mode is read-only verification: it queries on-chain commitments
//! from Stellar via the native RPC client, fetches event batches from IPFS,
//! and verifies the local audit log against the on-chain roots.

use std::sync::Arc;

use axum::extract::State;
use axum::Json;

use crate::audit::reader::VerificationReport;
use crate::audit::stellar::OnChainRoot;
use crate::audit::stellar_rpc::StellarRpcClient;
use crate::error::AppError;

use super::{ApiError, ApiResult, DaemonState};

/// Verify the local audit log against the latest on-chain root.
///
/// Uses the native Rust RPC client (no `stellar` CLI dependency).
pub async fn verify(
    state: State<Arc<DaemonState>>,
) -> ApiResult<VerificationReport> {
    let local_root_hex = state.audit_log.root_hex().map_err(ApiError::from)?;

    // Read the JSONL log file.
    let events_path = state.data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path).map_err(AppError::from).map_err(ApiError::from)?
    } else {
        String::new()
    };

    // Query on-chain root via native RPC.
    let rpc_client = StellarRpcClient::with_url(&state.rpc_url);
    let onchain_root = rpc_client.get_current_root().await.map_err(ApiError::from)?;

    // Run the verification (sync, CPU-bound).
    let events_jsonl_clone = events_jsonl.clone();
    let report = tokio::task::spawn_blocking(move || {
        crate::audit::reader::verify_with_onchain_root(
            onchain_root,
            &events_jsonl_clone,
            &local_root_hex,
        )
    })
    .await
    .map_err(|e| AppError::Validation(format!("verify task join: {e}")))
    .map_err(ApiError::from)??;

    Ok(Json(report))
}

/// Get the latest on-chain root via native RPC.
pub async fn onchain_root(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Option<OnChainRoot>> {
    let client = StellarRpcClient::with_url(&state.rpc_url);
    let root = client.get_current_root().await.map_err(ApiError::from)?;
    Ok(Json(root))
}

/// Rebuild the local audit log from on-chain commitments + IPFS batches.
///
/// This is a placeholder for the full rebuild flow. The current implementation
/// verifies the local log against the on-chain root (same as `verify`), which
/// is the core of the reader mode. A full rebuild would:
/// 1. Read all committed roots from Stellar.
/// 2. For each root, fetch the IPFS batch by CID.
/// 3. Replay the batch into the local Merkle tree.
/// 4. Verify each batch's root matches the on-chain commitment.
pub async fn rebuild(
    state: State<Arc<DaemonState>>,
) -> ApiResult<VerificationReport> {
    // For now, rebuild = verify. The local log is already rebuilt from
    // JSONL + sled on startup. This endpoint confirms the local state
    // matches the on-chain anchor.
    verify(state).await
}
