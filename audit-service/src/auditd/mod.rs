//! Standalone audit service (nosqlbuddy-audit).
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

pub mod attester;
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
use crate::audit::pinata::PinataConfig;
use crate::audit::stellar_native::{self, StellarKeypair, MAINNET_PASSPHRASE, TESTNET_HORIZON_URL, TESTNET_PASSPHRASE, TESTNET_RPC_URL};
use crate::audit::AuditLog;
use crate::error::AuditError;

/// Daemon mode: publisher, reader, or attester.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DaemonMode {
    Publish,
    Read,
    /// Independent attester mode: connects to an independent replica
    /// member, watches for new epoch commitments on-chain, independently
    /// computes the oplog hash, and submits attestations to the contract.
    Attest,
}

/// Chain configuration for native Stellar signing.
///
/// Replaces the old `STELLAR_IDENTITY` env var + `stellar` CLI approach.
/// The daemon loads a `StellarKeypair` from a secret key and uses these
/// endpoints to sign and submit transactions natively.
#[derive(Debug, Clone)]
pub struct DaemonChainConfig {
    pub network: String,
    pub rpc_url: String,
    pub horizon_url: String,
    pub passphrase: String,
    pub contract_id: String,
}

impl Default for DaemonChainConfig {
    fn default() -> Self {
        Self::testnet()
    }
}

impl DaemonChainConfig {
    pub fn testnet() -> Self {
        Self {
            network: "testnet".to_string(),
            rpc_url: TESTNET_RPC_URL.to_string(),
            horizon_url: TESTNET_HORIZON_URL.to_string(),
            passphrase: TESTNET_PASSPHRASE.to_string(),
            contract_id: std::env::var("CONTRACT_ID")
                .unwrap_or_else(|_| crate::audit::stellar::CONTRACT_ID.to_string()),
        }
    }

    pub fn mainnet(rpc_url: String, contract_id: String) -> Self {
        Self {
            network: "mainnet".to_string(),
            rpc_url,
            horizon_url: "https://horizon.stellar.org".to_string(),
            passphrase: MAINNET_PASSPHRASE.to_string(),
            contract_id,
        }
    }
}

/// Configuration for the daemon, parsed from CLI args.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub mode: DaemonMode,
    pub mongo_uri: Option<String>,
    pub data_dir: PathBuf,
    pub port: u16,
    pub circuit_dir: Option<PathBuf>,
    /// Path to a pre-generated proving key (from the ceremony tool).
    /// When set, proof generation uses this key instead of generating
    /// fresh parameters on every call.
    pub proving_key_path: Option<PathBuf>,
    pub ipfs_api_url: String,
    pub rpc_url: String,
    pub epoch_threshold: usize,
    pub epoch_time_secs: u64,
    /// Path to the ed25519 attester signing key file. Generated if missing.
    pub attester_key_file: Option<PathBuf>,
    /// If true, epoch close fails when oplog hash computation fails.
    /// If false, the epoch closes without the oplog hash (warns).
    pub oplog_hash_required: bool,
    /// Stellar secret key (S... strkey) for the publisher to sign transactions.
    /// Replaces the `STELLAR_IDENTITY` env var. When set, the daemon uses
    /// native signing instead of the `stellar` CLI.
    pub secret_key: Option<String>,
    /// Stellar secret key for the attester's Stellar account (separate from
    /// the ed25519 attester signing key). Required for native attestation.
    pub attester_secret_key: Option<String>,
    /// Pinata IPFS credentials. When present, the publisher pins epoch
    /// batches to Pinata instead of requiring a local IPFS daemon.
    pub pinata_config: Option<PinataConfig>,
    /// Chain configuration (network, RPC, Horizon, contract ID, passphrase).
    pub chain: DaemonChainConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            mode: DaemonMode::Publish,
            mongo_uri: None,
            data_dir: dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nosqlbuddy-audit"),
            port: 9173,
            circuit_dir: None,
            proving_key_path: None,
            ipfs_api_url: "http://127.0.0.1:5001".to_string(),
            rpc_url: crate::audit::stellar_rpc::TESTNET_RPC_URL.to_string(),
            epoch_threshold: 100,
            epoch_time_secs: 0,
            attester_key_file: None,
            oplog_hash_required: false,
            secret_key: None,
            attester_secret_key: None,
            pinata_config: None,
            chain: DaemonChainConfig::testnet(),
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
    /// Pre-generated proving key path (from ceremony). When set,
    /// proof generation is significantly faster.
    pub proving_key_path: Option<PathBuf>,
    pub ipfs_config: IpfsConfig,
    pub pinata_config: Option<PinataConfig>,
    pub rpc_url: String,
    /// MongoDB client for oplog hashing (publisher mode only).
    /// When present, epoch closes will compute and attach the oplog hash.
    pub mongo_client: Option<mongodb::Client>,
    /// Ed25519 signing key for the attester mode (oplog attestations).
    pub attester_key: Option<ed25519_dalek::SigningKey>,
    /// Stellar account address of the attester (derived from the keypair).
    pub attester_address: Option<String>,
    /// If true, epoch close fails when oplog hash computation fails.
    pub oplog_hash_required: bool,
    /// Stellar keypair for the publisher to sign transactions natively.
    /// When set, the publisher uses `stellar_native` instead of the CLI.
    pub signing_keypair: Option<StellarKeypair>,
    /// Stellar keypair for the attester's Stellar account (for native
    /// `attest_oplog` submission). Separate from `attester_key` (the ed25519
    /// oplog signing key).
    pub attester_stellar_keypair: Option<StellarKeypair>,
    /// Chain configuration for native signing (network, RPC, Horizon, contract).
    pub chain: DaemonChainConfig,
}

/// HTTP error wrapper for AuditError. Converts domain errors into appropriate
/// HTTP status codes with a JSON body matching the AuditError serialization.
pub struct ApiError(pub AuditError);

impl From<AuditError> for ApiError {
    fn from(e: AuditError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        use axum::http::StatusCode;

        let status = match &self.0 {
            AuditError::NotFound(_) => StatusCode::NOT_FOUND,
            AuditError::Validation(_) => StatusCode::BAD_REQUEST,
            AuditError::Credential(_) => StatusCode::CONFLICT,
            AuditError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
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

/// Load a `StellarKeypair` from a Stellar secret key strkey (S... format).
///
/// Decodes the base32 strkey, verifies the version byte and checksum, and
/// returns the keypair. Used by the daemon bin to load keypairs from
/// `--secret-key` / `--attester-secret-key` CLI args or env vars.
pub fn load_keypair_from_secret_key(secret_key: &str) -> Result<StellarKeypair, String> {
    let secret_bytes = decode_secret_key_strkey(secret_key)
        .ok_or_else(|| format!("invalid Stellar secret key (expected S... strkey, got {} chars)", secret_key.len()))?;
    Ok(StellarKeypair::from_secret_bytes(&secret_bytes))
}

/// Decode a Stellar secret key strkey (S...) to 32 raw bytes.
fn decode_secret_key_strkey(s: &str) -> Option<[u8; 32]> {
    if !s.starts_with('S') {
        return None;
    }
    let decoded = stellar_native::base32_decode(s)?;
    // Strkey format: version (1) + payload (32) + checksum (2) = 35 bytes
    if decoded.len() != 35 {
        return None;
    }
    // Verify version byte: 18 << 3 = 0x90 (ED25519 secret key)
    if decoded[0] != 18 << 3 {
        return None;
    }
    // Verify checksum (little-endian)
    let payload = &decoded[..33];
    let checksum = &decoded[33..];
    let expected = stellar_native::crc16_xmodem(payload);
    let expected_le = [(expected & 0xff) as u8, (expected >> 8) as u8];
    if checksum != expected_le {
        return None;
    }
    let mut result = [0u8; 32];
    result.copy_from_slice(&decoded[1..33]);
    Some(result)
}

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
        .route("/proof/:index", post(generate_proof))
        .route("/verify-onchain", post(verify_onchain));

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
            .route("/reader/verify-oplog", get(reader::verify_oplog))
            .route("/reader/onchain-root", get(reader::onchain_root))
            .route("/reader/rebuild", post(reader::rebuild))
            .with_state(state),
        DaemonMode::Attest => common
            .route("/attest/status", get(attester::get_status))
            .route("/attest/scan", post(attester::scan_and_attest))
            .route("/attest/attestations/:sequence", get(attester::list_attestations))
            .with_state(state),
    }
}

/// Start the HTTP server on the configured port.
/// In publisher mode, also spawns the auto-commit background task.
pub async fn run_server(state: Arc<DaemonState>, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    // Spawn auto-commit loop in publisher mode.
    if state.mode == DaemonMode::Publish {
        let auto_state = state.clone();
        tokio::spawn(async move {
            auto_commit_loop(auto_state).await;
        });
        log::info!("auto-commit background task started (epoch threshold: {} events)", state.epoch_manager.config().event_threshold);
    }

    // Spawn auto-attest loop in attester mode.
    // This is the "fresh attestation" mechanism (C2 fix): the attester
    // continuously scans for new on-chain commitments and signs each
    // epoch's oplog hash while the entries are still in the oplog.
    if state.mode == DaemonMode::Attest {
        let attest_state = state.clone();
        tokio::spawn(async move {
            auto_attest_loop(attest_state).await;
        });
        log::info!("auto-attest background task started (scans every 10s for new commitments to attest)");
    }

    let router = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    log::info!("nosqlbuddy-audit listening on http://{addr}");
    axum::serve(listener, router).await?;
    Ok(())
}

/// Publish an epoch batch to IPFS, preferring Pinata when credentials are
/// configured, otherwise falling back to the local Kubo daemon.
async fn publish_epoch_batch_to_ipfs(
    state: &DaemonState,
    epoch_number: u64,
    batch_content: &str,
) -> Result<crate::audit::ipfs::IpfsPublishResult, crate::error::AuditError> {
    if let Some(pinata) = &state.pinata_config {
        if !pinata.api_key.is_empty() || !pinata.api_secret.is_empty() {
            return crate::audit::pinata::publish_epoch_batch(pinata, epoch_number, batch_content).await;
        }
    }
    crate::audit::ipfs::publish_epoch_batch(&state.ipfs_config, epoch_number, batch_content).await
}

/// Check whether the configured IPFS backend is reachable.
async fn check_ipfs_backend(state: &DaemonState) -> Result<bool, crate::error::AuditError> {
    if let Some(pinata) = &state.pinata_config {
        if !pinata.api_key.is_empty() || !pinata.api_secret.is_empty() {
            return crate::audit::pinata::check(pinata).await;
        }
    }
    crate::audit::ipfs::check_daemon(&state.ipfs_config).await
}

/// Background task that monitors the audit log for new events, feeds them
/// into the epoch manager, and when an epoch auto-closes, publishes the
/// batch to IPFS and commits the root to Stellar.
async fn auto_commit_loop(state: Arc<DaemonState>) {
    let mut last_event_count: usize = state.audit_log.event_count();

    // At startup, bring the open epoch in sync with the audit log. If events
    // accumulated while the daemon was down, the count is restored. If the
    // auto-close threshold was already crossed, the epoch is closed now.
    if let Ok(Some(closed)) = state.epoch_manager.sync_open_epoch_with_audit_log(&state.audit_log) {
        log::info!(
            "auto-commit: restored epoch {} and auto-closed it ({} events)",
            closed.epoch_number,
            closed.event_count
        );
        if let Err(e) = publish_and_commit(&state, &closed).await {
            log::error!("auto-commit: failed for restored epoch {}: {e}", closed.epoch_number);
        }
    } else {
        log::info!("auto-commit: restored epoch manager to {last_event_count} events");
    }

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        let current_count = state.audit_log.event_count();
        if current_count == last_event_count {
            continue;
        }

        // Feed new events into the epoch manager.
        for i in last_event_count as u64..current_count as u64 {
            match state.epoch_manager.record_event(i, &state.audit_log) {
                Ok(Some(closed_epoch)) => {
                    log::info!(
                        "auto-commit: epoch {} closed ({} events), publishing + committing...",
                        closed_epoch.epoch_number,
                        closed_epoch.event_count
                    );
                    if let Err(e) = publish_and_commit(&state, &closed_epoch).await {
                        log::error!(
                            "auto-commit: failed for epoch {}: {e}",
                            closed_epoch.epoch_number
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("auto-commit: record_event error for index {i}: {e}");
                }
            }
        }

        last_event_count = current_count;
    }
}

/// Background task for attester mode: continuously scans for new on-chain
/// commitments and independently attests each epoch's oplog hash.
///
/// This is the "fresh attestation" mechanism (C2 fix). The attester runs on
/// the independent replica member and signs each epoch's oplog hash while
/// the entries are still in the oplog. The on-chain signature is durable
/// even after the oplog rolls over.
///
/// The loop polls the Stellar RPC for the latest committed sequence every
/// 10 seconds. For each new sequence that hasn't been attested yet, it:
/// 1. Gets the on-chain oplog commitment.
/// 2. Independently computes the oplog hash from the local replica.
/// 3. If they match, submits an attestation to the contract.
/// 4. If they don't match, logs an alert (omission detected).
async fn auto_attest_loop(state: Arc<DaemonState>) {
    use crate::audit::stellar_rpc::StellarRpcClient;

    let mut last_attested_seq: u64 = 0;

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;

        let client = match &state.mongo_client {
            Some(c) => c,
            None => continue,
        };

        let rpc = StellarRpcClient::with_url_and_contract(&state.rpc_url, &state.chain.contract_id);
        let current_root = match rpc.get_current_root().await {
            Ok(Some(root)) => root,
            Ok(None) => continue,
            Err(e) => {
                log::warn!("auto-attest: failed to get current root: {e}");
                continue;
            }
        };

        let current_seq = current_root.sequence;
        if current_seq <= last_attested_seq {
            continue;
        }

        // Scan new sequences from last_attested_seq+1 to current_seq.
        for seq in (last_attested_seq + 1)..=current_seq {
            match attest_epoch_internal(&state, client, seq).await {
                Ok(true) => {
                    log::info!("auto-attest: sequence {seq} attested successfully");
                    last_attested_seq = seq;
                }
                Ok(false) => {
                    // No oplog commitment for this epoch, or hash mismatch.
                    // Still advance so we don't keep retrying.
                    last_attested_seq = seq;
                }
                Err(e) => {
                    log::warn!("auto-attest: failed for sequence {seq}: {e}");
                }
            }
        }
    }
}

/// Internal helper: attest a single epoch (used by both the auto-attest loop
/// and the manual /attest/scan endpoint).
async fn attest_epoch_internal(
    state: &Arc<DaemonState>,
    client: &mongodb::Client,
    sequence: u64,
) -> Result<bool, String> {
    use crate::audit::oplog::{compute_oplog_range_hash, OplogTimestamp};
    use crate::audit::stellar_native;
    use crate::auditd::attester::sign_oplog_attestation;

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
        None => return Ok(false),
    };

    // On-chain timestamps are packed as (time << 32) | increment via pack_u64().
    let oplog_start_ts = OplogTimestamp::unpack_u64(on_chain_oplog.oplog_start_ts);
    let oplog_end_ts = OplogTimestamp::unpack_u64(on_chain_oplog.oplog_end_ts);

    let computed = compute_oplog_range_hash(client, sequence, oplog_start_ts, oplog_end_ts)
        .await
        .map_err(|e| format!("compute_oplog_range_hash: {e}"))?;

    let matched = computed.oplog_merkle_root_hex == on_chain_oplog.oplog_root_hex;

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
            Ok(()) => Ok(true),
            Err(e) => {
                // Duplicate attestation is expected if we've already attested.
                log::warn!("auto-attest: sequence {sequence} attestation submission: {e}");
                Ok(false)
            }
        }
    } else {
        log::error!(
            "auto-attest: ALERT — sequence {sequence} oplog hash mismatch! \
             on_chain={} computed={}",
            on_chain_oplog.oplog_root_hex,
            computed.oplog_merkle_root_hex
        );
        Ok(false)
    }
}

/// Publish an epoch's events to IPFS and commit the root to Stellar.
///
/// If a MongoDB client is available, this also computes the oplog hash
/// for the epoch's time range and attaches it to the epoch before
/// committing. The oplog hash provides the completeness guarantee:
/// it proves that no writes were omitted from the audit log.
async fn publish_and_commit(
    state: &DaemonState,
    epoch: &crate::audit::epoch::Epoch,
) -> Result<(), String> {
    let epoch_number = epoch.epoch_number;
    let end_index = epoch.end_index.ok_or("epoch has no end_index")?;

    // 0. Compute oplog hash and attach to epoch (if MongoDB client is available).
    if let Some(client) = &state.mongo_client {
        match compute_and_attach_oplog_hash(state, client, epoch_number).await {
            Ok((oplog_range, _epoch)) => {
                log::info!(
                    "auto-commit: epoch {epoch_number} oplog hash attached: root={}, entries={}",
                    oplog_range.oplog_merkle_root_hex,
                    oplog_range.entry_count
                );
            }
            Err(e) => {
                log::warn!(
                    "auto-commit: oplog hash computation failed for epoch {epoch_number}: {e} — committing without oplog hash"
                );
            }
        }
    }

    // Reload the epoch to get the updated oplog fields (if attached above).
    let epoch = {
        let epochs = state.epoch_manager.list_epochs();
        epochs
            .iter()
            .find(|e| e.epoch_number == epoch_number)
            .cloned()
            .unwrap_or_else(|| epoch.clone())
    };

    // 1. Publish to IPFS.
    let events_path = state.data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path).map_err(|e| format!("read events.jsonl: {e}"))?
    } else {
        String::new()
    };

    let batch_content = extract_epoch_batch(&events_jsonl, epoch.start_index, end_index);
    if !batch_content.is_empty() {
        match publish_epoch_batch_to_ipfs(state, epoch_number, &batch_content).await {
            Ok(result) => {
                log::info!("auto-commit: epoch {epoch_number} published to IPFS, CID: {}", result.cid);
                let _ = state.audit_log.save_ipfs_cid(epoch_number, &result.cid);
            }
            Err(e) => {
                log::warn!("auto-commit: IPFS publish failed for epoch {epoch_number}: {e} — committing without CID");
            }
        }
    }

    // 2. Commit root to Stellar.
    //    If the epoch has an oplog hash, use commit_root_with_oplog to
    //    store both the audit root and the oplog root on-chain.
    let root_hex = epoch.root_hex.clone().unwrap_or_else(|| {
        state.audit_log.root_hex().unwrap_or_default()
    });

    let cid = state.audit_log.load_ipfs_cid(epoch_number).ok().flatten();
    let metadata = match &cid {
        Some(c) => format!("epoch={epoch_number} cid={c}"),
        None => format!("epoch={epoch_number}"),
    };

    let kp = state.signing_keypair.as_ref().ok_or_else(|| {
        "no signing keypair configured — use --secret-key or STELLAR_SECRET_KEY env var".to_string()
    })?;
    let chain = &state.chain;
    let commit_result = if let Some(oplog_root_hex) = &epoch.oplog_merkle_root_hex {
        let oplog_start = epoch
            .oplog_start_ts
            .map_or(0, |ts| ts.pack_u64());
        let oplog_end = epoch
            .oplog_end_ts
            .map_or(0, |ts| ts.pack_u64());
        let oplog_count = epoch.oplog_entry_count.unwrap_or(0);

        log::info!(
            "auto-commit: epoch {epoch_number} committing with oplog hash: root={}, entries={}",
            oplog_root_hex, oplog_count
        );

        stellar_native::commit_root_with_oplog_native(
            &root_hex,
            oplog_root_hex,
            oplog_start,
            oplog_end,
            oplog_count,
            &metadata,
            kp,
            &chain.rpc_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await
        .map_err(|e| format!("commit_root_with_oplog_native: {e}"))?
    } else {
        stellar_native::commit_root_native(
            &root_hex,
            &metadata,
            kp,
            &chain.rpc_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await
        .map_err(|e| format!("commit_root_native: {e}"))?
    };

    log::info!(
        "auto-commit: epoch {epoch_number} committed on-chain, tx: {}",
        commit_result.tx_hash
    );

    // 3. Mark epoch as committed.
    state
        .epoch_manager
        .mark_committed(epoch_number, commit_result.tx_hash)
        .map_err(|e| format!("mark_committed: {e}"))?;

    Ok(())
}

/// Extract JSONL lines for events in the given index range (inclusive).
fn extract_epoch_batch(jsonl: &str, start_index: u64, end_index: u64) -> String {
    jsonl
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let ev: serde_json::Value = serde_json::from_str(line).ok()?;
            let index = ev.get("index")?.as_u64()?;
            if index >= start_index && index <= end_index {
                Some(line.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Compute the oplog hash for an epoch's time range and attach it to
/// the epoch in the EpochManager.
///
/// The oplog range is `[next_oplog_start_ts, majority_commit_ts)`. The
/// start timestamp is tracked by the EpochManager (chained from the
/// previous epoch's end). The end timestamp is the current majority-
/// committed oplog timestamp, ensuring we only hash durable entries.
async fn compute_and_attach_oplog_hash(
    state: &DaemonState,
    client: &mongodb::Client,
    epoch_number: u64,
) -> Result<(crate::audit::oplog::OplogRange, crate::audit::epoch::Epoch), String> {
    use crate::audit::oplog::{compute_oplog_range_hash, get_majority_commit_ts};

    let start_ts = state.epoch_manager.next_oplog_start_ts();
    let majority_ts = get_majority_commit_ts(client)
        .await
        .map_err(|e| format!("get_majority_commit_ts: {e}"))?;

    // If the majority commit point hasn't advanced past the start,
    // there are no new oplog entries to hash for this epoch.
    if majority_ts <= start_ts {
        log::warn!(
            "oplog hash: majority commit ts {} not past start ts {} for epoch {epoch_number} — attaching empty range",
            majority_ts, start_ts
        );
    }

    let oplog_range = compute_oplog_range_hash(client, epoch_number, start_ts, majority_ts)
        .await
        .map_err(|e| format!("compute_oplog_range_hash: {e}"))?;

    let epoch = state
        .epoch_manager
        .attach_oplog_hash(
            epoch_number,
            oplog_range.start_ts,
            oplog_range.end_ts,
            oplog_range.entry_count,
            oplog_range.oplog_merkle_root_hex.clone(),
            oplog_range.majority_commit_ts,
        )
        .map_err(|e| format!("attach_oplog_hash: {e}"))?;

    Ok((oplog_range, epoch))
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
    let root_for_hex = inclusion.root;

    let circuit_dir = state.circuit_dir.as_deref().ok_or_else(|| {
        ApiError(AuditError::Validation(
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

    let prover = if let Some(pk_path) = &state.proving_key_path {
        zk_audit::AuditProver::with_proving_key(&r1cs, &wasm, pk_path.to_str().unwrap())
    } else {
        zk_audit::AuditProver::new(&r1cs, &wasm)
    }
    .map_err(|e| ApiError(AuditError::ZkAudit(e.to_string())))?;
    let groth16_proof = tokio::task::spawn_blocking(move || prover.prove(&inclusion))
        .await
        .map_err(|e| ApiError(AuditError::ZkAudit(format!("proof task: {}", e))))?
        .map_err(|e| ApiError(AuditError::ZkAudit(e.to_string())))?;
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof)
        .map_err(|e| ApiError(AuditError::ZkAudit(e.to_string())))?;

    let root_bigint = root_for_hex.into_bigint();
    let root_hex = hex::encode(&root_bigint.to_bytes_be());

    // Find the epoch that contains this leaf and get its on-chain tx hash.
    let tx_hash = state
        .epoch_manager
        .list_epochs()
        .into_iter()
        .find(|e| {
            e.start_index <= index
                && e.end_index.map_or(false, |end| end >= index)
        })
        .and_then(|e| e.tx_hash)
        .unwrap_or_default();

    Ok(Json(ProofResponse {
        root_hex,
        leaf_index: index,
        proof: soroban_args.proof,
        vk: soroban_args.vk,
        pub_signals: soroban_args.pub_signals,
        network: state.chain.network.clone(),
        contract_id: state.chain.contract_id.clone(),
        tx_hash,
    }))
}

// ─── On-chain verification ────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyOnchainRequest {
    pub root_hex: String,
    pub proof_a: String,
    pub proof_b: String,
    pub proof_c: String,
    pub vk_alpha: String,
    pub vk_beta: String,
    pub vk_gamma: String,
    pub vk_delta: String,
    pub vk_ic: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyOnchainResponse {
    pub tx_hash: String,
    pub verified: bool,
}

/// Submit a Groth16 inclusion proof to the Soroban contract for on-chain
/// verification. Returns the transaction hash and the boolean result.
async fn verify_onchain(
    state: axum::extract::State<Arc<DaemonState>>,
    axum::Json(req): axum::Json<VerifyOnchainRequest>,
) -> ApiResult<VerifyOnchainResponse> {
    let kp = state.signing_keypair.as_ref().ok_or_else(|| {
        ApiError(AuditError::Validation(
            "no signing keypair configured — use --secret-key or STELLAR_SECRET_KEY env var".to_string(),
        ))
    })?;
    let chain = &state.chain;

    let result = stellar_native::verify_inclusion_native(
        &req.root_hex,
        &req.proof_a,
        &req.proof_b,
        &req.proof_c,
        &req.vk_alpha,
        &req.vk_beta,
        &req.vk_gamma,
        &req.vk_delta,
        &req.vk_ic,
        kp,
        &chain.rpc_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await
    .map_err(ApiError::from)?;

    Ok(Json(VerifyOnchainResponse {
        tx_hash: result.tx_hash,
        verified: result.verified,
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
    pub network: String,
    pub contract_id: String,
    pub tx_hash: String,
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

    #[test]
    fn extract_epoch_batch_filters_by_range() {
        let jsonl = [
            r#"{"index":0,"operation":"insert"}"#,
            r#"{"index":1,"operation":"update"}"#,
            r#"{"index":2,"operation":"delete"}"#,
            r#"{"index":3,"operation":"insert"}"#,
        ]
        .join("\n");

        let batch = extract_epoch_batch(&jsonl, 1, 2);
        assert_eq!(batch.lines().count(), 2);
        assert!(batch.contains(r#""index":1"#));
        assert!(batch.contains(r#""index":2"#));
        assert!(!batch.contains(r#""index":0"#));
        assert!(!batch.contains(r#""index":3"#));
    }

    #[test]
    fn extract_epoch_batch_empty_range() {
        let jsonl = r#"{"index":0,"operation":"insert"}"#;
        let batch = extract_epoch_batch(jsonl, 5, 10);
        assert!(batch.is_empty());
    }

    #[test]
    fn extract_epoch_batch_skips_invalid_lines() {
        let jsonl = [
            r#"{"index":0,"operation":"insert"}"#,
            "not valid json",
            r#"{"index":1,"operation":"update"}"#,
        ]
        .join("\n");

        let batch = extract_epoch_batch(&jsonl, 0, 1);
        assert_eq!(batch.lines().count(), 2);
    }

    #[test]
    fn test_load_keypair_from_secret_key_roundtrip() {
        let kp = stellar_native::generate_keypair();
        let secret_str = kp.secret_key_str();
        let loaded = load_keypair_from_secret_key(&secret_str);
        assert!(loaded.is_ok());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.public_bytes(), kp.public_bytes());
        assert_eq!(loaded.account_id(), kp.account_id());
    }

    #[test]
    fn test_load_keypair_rejects_garbage() {
        assert!(load_keypair_from_secret_key("not-a-key").is_err());
        let bad = format!("S{}", "x".repeat(55));
        assert!(load_keypair_from_secret_key(&bad).is_err());
        assert!(load_keypair_from_secret_key("").is_err());
    }

    #[test]
    fn test_daemon_chain_config_testnet_defaults() {
        let c = DaemonChainConfig::testnet();
        assert_eq!(c.network, "testnet");
        assert!(!c.contract_id.is_empty());
        assert!(!c.rpc_url.is_empty());
        assert!(!c.passphrase.is_empty());
    }
}
