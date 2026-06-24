//! Standalone audit daemon (nosqlbuddy-auditd).
//!
//! One binary, two modes:
//! - **Publisher** (`--mode publish`): connects to MongoDB, runs the change
//!   stream listener, manages epochs, publishes to IPFS, commits roots to
//!   Stellar, and supports K-of-N threshold attestation.
//! - **Reader** (`--mode read`): reads commitments from Stellar via native
//!   RPC, fetches batches from IPFS, rebuilds the Merkle tree in local sled,
//!   and verifies roots match.
//!
//! Both modes expose an HTTP API on `localhost:9173` (configurable via
//! `--port`). The daemon reuses the same audit modules as the Tauri app —
//! no code duplication.

pub mod publisher;
pub mod reader;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::audit::attestation::AttestationManager;
use crate::audit::change_stream::ChangeStreamRegistry;
use crate::audit::epoch::EpochManager;
use crate::audit::ipfs::IpfsConfig;
use crate::audit::AuditLog;
use crate::error::AppError;

/// Daemon mode: publisher or reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DaemonMode {
    Publish,
    Read,
}

/// Configuration for the daemon, parsed from CLI args.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub mode: DaemonMode,
    pub mongo_uri: Option<String>,
    pub data_dir: PathBuf,
    pub port: u16,
    pub circuit_dir: Option<PathBuf>,
    pub ipfs_api_url: String,
    pub rpc_url: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            mode: DaemonMode::Publish,
            mongo_uri: None,
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nosqlbuddy-auditd"),
            port: 9173,
            circuit_dir: None,
            ipfs_api_url: "http://127.0.0.1:5001".to_string(),
            rpc_url: crate::audit::stellar_rpc::TESTNET_RPC_URL.to_string(),
        }
    }
}

/// Shared state for all HTTP handlers, wrapped in Arc for cheap cloning.
pub struct DaemonState {
    pub mode: DaemonMode,
    pub audit_log: Arc<AuditLog>,
    pub epoch_manager: EpochManager,
    pub attestation_manager: AttestationManager,
    pub change_streams: ChangeStreamRegistry,
    pub data_dir: PathBuf,
    pub circuit_dir: Option<PathBuf>,
    pub ipfs_config: IpfsConfig,
    pub rpc_url: String,
}

/// HTTP error wrapper for AppError. Converts domain errors into appropriate
/// HTTP status codes with a JSON body matching the AppError serialization.
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        use axum::http::StatusCode;

        let status = match &self.0 {
            AppError::NotFound(_) | AppError::ConnectionNotFound(_) | AppError::ProfileNotFound(_) => {
                StatusCode::NOT_FOUND
            }
            AppError::Validation(_) | AppError::InvalidBson(_) | AppError::SqlParse(_) => {
                StatusCode::BAD_REQUEST
            }
            AppError::Credential(_) | AppError::ProfileExists(_) => {
                StatusCode::CONFLICT
            }
            AppError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        let body = serde_json::to_string(&self.0).unwrap_or_else(|_| {
            format!(r#"{{"kind":"Internal","message":"{}"}}"#, self.0)
        });

        (status, body).into_response()
    }
}

/// Type alias for handler results.
pub type ApiResult<T> = Result<Json<T>, ApiError>;

/// Status response for the daemon itself.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub mode: DaemonMode,
    pub listening: bool,
    pub data_dir: String,
    pub audit: AuditStatusInfo,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditStatusInfo {
    pub root_hex: String,
    pub leaf_count: usize,
    pub event_count: usize,
    pub tree_height: u32,
}

/// Build the axum router for the daemon. Routes are mode-specific.
pub fn build_router(state: Arc<DaemonState>) -> axum::Router {
    use axum::routing::{get, post};

    let common = axum::Router::new()
        .route("/status", get(get_status))
        .route("/events", get(list_events))
        .route("/root", get(get_root))
        .route("/proof/:index", post(generate_proof));

    match state.mode {
        DaemonMode::Publish => common
            .route("/epoch/close", post(publisher::close_epoch))
            .route("/epochs", get(publisher::list_epochs))
            .route("/epoch/current", get(publisher::current_epoch))
            .route("/epoch/:number/commit", post(publisher::commit_epoch))
            .route("/epoch/:number/publish-ipfs", post(publisher::publish_ipfs))
            .route("/epoch/:number/ipfs-cid", get(publisher::get_ipfs_cid))
            .route("/onchain-root", get(publisher::get_onchain_root))
            .route("/ipfs/check", get(publisher::check_ipfs))
            .route("/publishers", get(publisher::list_publishers).post(publisher::add_publisher))
            .route("/publishers/:key", axum::routing::delete(publisher::remove_publisher))
            .route("/attestations/:epoch", get(publisher::list_attestations))
            .route("/attestations/:epoch/status", get(publisher::attestation_status))
            .route("/attestations", post(publisher::submit_attestation))
            .route("/threshold", get(publisher::get_threshold).post(publisher::set_threshold))
            .with_state(state),
        DaemonMode::Read => common
            .route("/reader/verify", get(reader::verify))
            .route("/reader/onchain-root", get(reader::onchain_root))
            .route("/reader/rebuild", post(reader::rebuild))
            .with_state(state),
    }
}

/// Start the HTTP server on the configured port.
pub async fn run_server(state: Arc<DaemonState>, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("nosqlbuddy-auditd listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

// ─── Common handlers (shared by both modes) ───────────────────────────

async fn get_status(state: axum::extract::State<Arc<DaemonState>>) -> ApiResult<DaemonStatus> {
    let audit = &state.audit_log;
    let root_hex = audit.root_hex().map_err(ApiError::from)?;
    Ok(Json(DaemonStatus {
        mode: state.mode,
        listening: true,
        data_dir: state.data_dir.display().to_string(),
        audit: AuditStatusInfo {
            root_hex,
            leaf_count: audit.leaf_count(),
            event_count: audit.event_count(),
            tree_height: 20,
        },
    }))
}

async fn list_events(state: axum::extract::State<Arc<DaemonState>>) -> ApiResult<Vec<crate::audit::AuditEvent>> {
    Ok(Json(state.audit_log.list_events()))
}

async fn get_root(state: axum::extract::State<Arc<DaemonState>>) -> ApiResult<String> {
    let root = state.audit_log.root_hex().map_err(ApiError::from)?;
    Ok(Json(root))
}

async fn generate_proof(
    state: axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(index): axum::extract::Path<u64>,
) -> ApiResult<ProofResponse> {
    use ark_ff::{BigInteger, PrimeField};

    let inclusion = state.audit_log.prove_inclusion(index).map_err(ApiError::from)?;

    let circuit_dir = state.circuit_dir.as_deref().ok_or_else(|| {
        ApiError(AppError::Validation(
            "circuit directory not configured — use --circuit-dir".to_string(),
        ))
    })?;

    let r1cs = circuit_dir
        .join("merkle_inclusion.r1cs")
        .to_string_lossy()
        .to_string();
    let wasm = circuit_dir
        .join("merkle_inclusion.wasm")
        .to_string_lossy()
        .to_string();

    let prover = zk_audit::AuditProver::new(&r1cs, &wasm)
        .map_err(|e| ApiError(AppError::ZkAudit(e.to_string())))?;
    let groth16_proof = prover
        .prove(&inclusion)
        .map_err(|e| ApiError(AppError::ZkAudit(e.to_string())))?;
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof)
        .map_err(|e| ApiError(AppError::ZkAudit(e.to_string())))?;

    let root_bigint = inclusion.root.into_bigint();
    let root_hex = hex::encode(&root_bigint.to_bytes_be());

    Ok(Json(ProofResponse {
        root_hex,
        leaf_index: index,
        proof: soroban_args.proof,
        vk: soroban_args.vk,
        pub_signals: soroban_args.pub_signals,
    }))
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProofResponse {
    pub root_hex: String,
    pub leaf_index: u64,
    pub proof: zk_audit::serialize::SorobanProof,
    pub vk: zk_audit::serialize::SorobanVerifyingKey,
    pub pub_signals: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_default_is_publish_mode() {
        let config = DaemonConfig::default();
        assert_eq!(config.mode, DaemonMode::Publish);
        assert_eq!(config.port, 9173);
    }

    #[test]
    fn daemon_mode_serializes_as_lowercase() {
        let json = serde_json::to_string(&DaemonMode::Publish).unwrap();
        assert_eq!(json, "\"publish\"");
        let json = serde_json::to_string(&DaemonMode::Read).unwrap();
        assert_eq!(json, "\"read\"");
    }
}
