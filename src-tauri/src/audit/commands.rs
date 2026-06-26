//! Tauri IPC commands for the ZK audit log.
//!
//! These commands expose the audit log to the frontend:
//! - `audit_get_status` — current root, leaf count, event count.
//! - `audit_list_events` — list all recorded audit events.
//! - `audit_get_root` — get the current Merkle root as hex.
//! - `audit_generate_proof` — generate a Groth16 inclusion proof for a leaf.
//! - `audit_record_event` — manually record an audit event.
//! - `audit_commit_root` — commit the current root to Stellar testnet.
//! - `audit_get_onchain_root` — query the latest committed root from Stellar.

use serde::Serialize;
use tauri::State;

use crate::audit::audit_mode::{load_mode_config, AuditNetwork};
use crate::audit::dev_setup::ChainConfig;
use crate::audit::stellar::{CommitResult, OnChainRoot, VerifyInclusionResult};
use crate::audit::stellar_native;
use crate::error::{AppError, AppResult};
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
    Ok(state.audit_log.root_hex()?)
}

/// Generate a Groth16 inclusion proof for the event at the given index.
///
/// This requires the compiled circuit artifacts (R1CS + WASM) to be present.
/// If `r1cs_path` / `wasm_path` are empty, the bundled Tauri resources
/// are used (resolved via `tauri::Manager::path`).
#[tauri::command]
pub async fn audit_generate_proof(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    index: u64,
    r1cs_path: Option<String>,
    wasm_path: Option<String>,
    proving_key_path: Option<String>,
) -> AppResult<ProofResult> {
    use ark_ff::{BigInteger, PrimeField};

    let inclusion = state.audit_log.prove_inclusion(index)?;
    let root_for_hex = inclusion.root;

    // Resolve circuit artifact paths: use explicit paths if provided,
    // otherwise fall back to bundled Tauri resources.
    let (r1cs, wasm) = resolve_circuit_paths(&app, r1cs_path, wasm_path)?;

    // Generate the Groth16 proof on a blocking thread (CPU-heavy + wasmer
    // needs its own Tokio runtime for the WASM witness calculation).
    let prover = if let Some(pk) = proving_key_path.as_deref().filter(|s| !s.is_empty()) {
        zk_audit::AuditProver::with_proving_key(&r1cs, &wasm, pk)?
    } else {
        zk_audit::AuditProver::new(&r1cs, &wasm)?
    };
    let groth16_proof = tokio::task::spawn_blocking(move || prover.prove(&inclusion))
        .await
        .map_err(|e| crate::error::AppError::Internal(format!("proof task: {}", e)))?
        .map_err(crate::error::AppError::from)?;
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof)?;

    let root_bigint = root_for_hex.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let root_hex = hex::encode(&root_bytes);

    let mode_config = load_mode_config(&app)?;
    let (network, contract_id) = match mode_config.network {
        AuditNetwork::Testnet => ("testnet".to_string(), ChainConfig::testnet().contract_id),
        AuditNetwork::Mainnet => ("mainnet".to_string(), mode_config.mainnet_contract_id),
    };

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

    Ok(ProofResult {
        root_hex,
        leaf_index: index,
        proof: soroban_args.proof,
        vk: soroban_args.vk,
        pub_signals: soroban_args.pub_signals,
        network,
        contract_id,
        tx_hash,
    })
}

/// Submit a Groth16 inclusion proof to the Soroban contract for on-chain
/// verification. Returns the transaction hash and the boolean result.
#[tauri::command]
pub async fn audit_verify_proof_onchain(
    app: tauri::AppHandle,
    root_hex: String,
    proof_a: String,
    proof_b: String,
    proof_c: String,
    vk_alpha: String,
    vk_beta: String,
    vk_gamma: String,
    vk_delta: String,
    vk_ic: Vec<String>,
) -> AppResult<VerifyInclusionResult> {
    use crate::audit::audit_mode::load_production_keypair;

    let mode_config = load_mode_config(&app)?;
    let (network, contract_id) = match mode_config.network {
        AuditNetwork::Testnet => ("testnet".to_string(), ChainConfig::testnet().contract_id),
        AuditNetwork::Mainnet => ("mainnet".to_string(), mode_config.mainnet_contract_id),
    };

    let chain = match mode_config.network {
        AuditNetwork::Testnet => ChainConfig::testnet(),
        AuditNetwork::Mainnet => ChainConfig {
            contract_id,
            rpc_url: mode_config.mainnet_rpc_url,
            horizon_url: "https://horizon.stellar.org".to_string(),
            passphrase: stellar_native::MAINNET_PASSPHRASE.to_string(),
            network,
        },
    };

    let kp = load_production_keypair()?.ok_or_else(|| {
        AppError::Validation("no production keypair found — save it in Audit Settings".to_string())
    })?;

    stellar_native::verify_inclusion_native(
        &root_hex,
        &proof_a,
        &proof_b,
        &proof_c,
        &vk_alpha,
        &vk_beta,
        &vk_gamma,
        &vk_delta,
        &vk_ic,
        &kp,
        &chain.rpc_url,
        &chain.horizon_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await
    .map_err(AppError::from)
}

/// Resolve circuit artifact paths, falling back to bundled resources.
fn resolve_circuit_paths(
    app: &tauri::AppHandle,
    r1cs_path: Option<String>,
    wasm_path: Option<String>,
) -> AppResult<(String, String)> {
    use tauri::Manager;

    let r1cs = match r1cs_path.as_deref() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            let resource = app
                .path()
                .resolve("resources/circuits/merkle_inclusion.r1cs", tauri::path::BaseDirectory::Resource)
                .map_err(|e| AppError::Validation(format!("resolve r1cs resource: {e}")))?;
            resource.to_string_lossy().to_string()
        }
    };
    let wasm = match wasm_path.as_deref() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            let resource = app
                .path()
                .resolve("resources/circuits/merkle_inclusion.wasm", tauri::path::BaseDirectory::Resource)
                .map_err(|e| AppError::Validation(format!("resolve wasm resource: {e}")))?;
            resource.to_string_lossy().to_string()
        }
    };
    Ok((r1cs, wasm))
}

/// Pure path resolution logic — given explicit paths or a base resource
/// directory, return the R1CS and WASM paths. Testable without a Tauri
/// AppHandle.
#[cfg(test)]
fn resolve_circuit_paths_pure(
    r1cs_path: Option<&str>,
    wasm_path: Option<&str>,
    resource_dir: &std::path::Path,
) -> AppResult<(String, String)> {
    let r1cs = match r1cs_path.filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => resource_dir
            .join("resources/circuits/merkle_inclusion.r1cs")
            .to_string_lossy()
            .to_string(),
    };
    let wasm = match wasm_path.filter(|p| !p.is_empty()) {
        Some(p) => p.to_string(),
        None => resource_dir
            .join("resources/circuits/merkle_inclusion.wasm")
            .to_string_lossy()
            .to_string(),
    };
    Ok((r1cs, wasm))
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
    pub network: String,
    pub contract_id: String,
    pub tx_hash: String,
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
    // The leaf is derived from the raw payload string. The same payload
    // is stored on disk so replay can recompute and verify the leaf.
    let leaf = crate::audit::leaf_from_payload(&operation, &database, &collection, &payload);

    Ok(state
        .audit_log
        .record(&operation, &database, &collection, &payload, leaf)?)
}

/// Commit the current Merkle root to the Soroban contract on Stellar testnet.
///
/// This anchors the local audit log to an immutable on-chain commitment,
/// making truncation of the log's tail detectable. The root is submitted
/// along with optional metadata (e.g. event count, timestamp).
#[tauri::command]
pub async fn audit_commit_root(
    state: State<'_, AppState>,
    metadata: Option<String>,
) -> AppResult<CommitResult> {
    let audit = &state.audit_log;
    let root = audit.root()?;

    // Convert the field element to a 32-byte big-endian hex string.
    use ark_ff::{BigInteger, PrimeField};
    let root_bigint = root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let root_hex = hex::encode(&root_bytes);

    // Check if this root is already committed on-chain to avoid
    // RootAlreadyCommitted errors.
    let root_hex_check = root_hex.clone();
    let chain = crate::audit::dev_setup::ChainConfig::testnet();
    let probe_kp = stellar_native::generate_keypair();
    let onchain = stellar_native::get_current_root_native(
        &probe_kp,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await?;
    if let Some(ref entry) = onchain {
        if entry.root_hex == root_hex_check {
            return Err(AppError::Validation(format!(
                "root 0x{}.. is already committed on-chain (seq #{}). New audit events are needed to produce a different root.",
                &root_hex_check[..16],
                entry.sequence
            )));
        }
    }

    let meta = metadata.unwrap_or_else(|| {
        format!(
            "events={} leaves={}",
            audit.event_count(),
            audit.leaf_count()
        )
    });

    // Load keypair from keychain.
    let kp = crate::audit::dev_setup::load_keypair_from_keychain()?
        .ok_or_else(|| {
            AppError::Validation(
                "no Stellar keypair found — run onboarding first".to_string(),
            )
        })?;

    // Commit via native signing.
    let result = stellar_native::commit_root_native(
        &root_hex,
        &meta,
        &kp,
        &chain.rpc_url,
        &chain.horizon_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await?;

    Ok(result)
}

/// Get the latest committed root from the Soroban contract on Stellar testnet.
#[tauri::command]
pub async fn audit_get_onchain_root() -> AppResult<Option<OnChainRoot>> {
    let chain = crate::audit::dev_setup::ChainConfig::testnet();
    let kp = stellar_native::generate_keypair();
    Ok(stellar_native::get_current_root_native(&kp, &chain.rpc_url, &chain.contract_id).await?)
}

// ─── Epoch management commands ────────────────────────────────────────

/// List all epochs (open and closed).
#[tauri::command]
pub async fn audit_list_epochs(state: State<'_, AppState>) -> AppResult<Vec<crate::audit::epoch::Epoch>> {
    Ok(state.epoch_manager.list_epochs())
}

/// Get the current (open) epoch.
#[tauri::command]
pub async fn audit_current_epoch(state: State<'_, AppState>) -> AppResult<crate::audit::epoch::Epoch> {
    Ok(state.epoch_manager.current_epoch())
}

/// Manually close the current epoch and freeze its root.
#[tauri::command]
pub async fn audit_close_epoch(state: State<'_, AppState>) -> AppResult<crate::audit::epoch::Epoch> {
    Ok(state.epoch_manager.close_current_epoch(&state.audit_log)?)
}

/// Mark an epoch as committed on-chain with the given tx hash.
#[tauri::command]
pub async fn audit_mark_epoch_committed(
    epoch_number: u64,
    tx_hash: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    Ok(state.epoch_manager.mark_committed(epoch_number, tx_hash)?)
}

// ─── Reader mode commands ─────────────────────────────────────────────

/// Verify the local audit log against the latest on-chain root.
///
/// This is the main reader-mode command. It queries the on-chain root
/// from Stellar, searches the local JSONL log for the matching root,
/// and verifies the root chain up to that point. Returns a detailed
/// verification report.
#[tauri::command]
pub async fn audit_verify_reader_mode(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<crate::audit::reader::VerificationReport> {
    use tauri::Manager;

    // Get the local root.
    let local_root_hex = state.audit_log.root_hex()?;

    // Read the JSONL log file.
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Validation(format!("resolve app data dir: {e}")))?;
    let events_path = data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path)?
    } else {
        String::new()
    };

    // Resolve chain config for the on-chain root query.
    use crate::audit::audit_mode::load_mode_config;
    use crate::audit::dev_setup::ChainConfig;

    let config = load_mode_config(&app)?;
    let chain = match config.network {
        crate::audit::audit_mode::AuditNetwork::Testnet => ChainConfig::testnet(),
        crate::audit::audit_mode::AuditNetwork::Mainnet => {
            ChainConfig::mainnet(config.mainnet_rpc_url.clone(), config.mainnet_contract_id.clone())
        }
    };

    let result = crate::audit::reader::verify_against_onchain(
        &events_jsonl,
        &local_root_hex,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await?;

    Ok(result)
}

// ─── IPFS batch publishing commands ───────────────────────────────────

/// Publish an epoch's event batch to IPFS.
///
/// Collects all events belonging to the given epoch from the JSONL log,
/// publishes them to IPFS via the HTTP API, and stores the CID in sled.
/// The CID can optionally be committed on-chain as metadata for the
/// epoch's root commitment.
#[tauri::command]
pub async fn audit_publish_epoch_to_ipfs(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    epoch_number: u64,
    api_url: Option<String>,
) -> AppResult<crate::audit::ipfs::IpfsPublishResult> {
    use tauri::Manager;

    // Find the epoch and get its event range.
    let epochs = state.epoch_manager.list_epochs();
    let epoch = epochs
        .iter()
        .find(|e| e.epoch_number == epoch_number)
        .ok_or_else(|| AppError::Validation(format!("epoch {} not found", epoch_number)))?;

    let start_index = epoch.start_index;
    let end_index = epoch.end_index.ok_or_else(|| {
        AppError::Validation(format!(
            "epoch {} is still open — close it before publishing to IPFS",
            epoch_number
        ))
    })?;

    // Read the JSONL log and extract the events for this epoch.
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Validation(format!("resolve app data dir: {e}")))?;
    let events_path = data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path)?
    } else {
        String::new()
    };

    // Extract the lines for this epoch's event range.
    let batch_content = extract_epoch_batch(&events_jsonl, start_index, end_index);
    if batch_content.is_empty() {
        return Err(AppError::Validation(format!(
            "no events found for epoch {} (range {}-{})",
            epoch_number, start_index, end_index
        )));
    }

    // Configure the IPFS client.
    let config = crate::audit::ipfs::IpfsConfig {
        api_url: api_url.unwrap_or_else(|| "http://127.0.0.1:5001".to_string()),
        cid_version: 1,
    };

    // Publish to IPFS.
    let result = crate::audit::ipfs::publish_epoch_batch(&config, epoch_number, &batch_content)
        .await?;

    // Save the CID to sled.
    state.audit_log.save_ipfs_cid(epoch_number, &result.cid)?;

    Ok(result)
}

/// Get the IPFS CID for a published epoch (if any).
#[tauri::command]
pub async fn audit_get_ipfs_cid(
    state: State<'_, AppState>,
    epoch_number: u64,
) -> AppResult<Option<String>> {
    Ok(state.audit_log.load_ipfs_cid(epoch_number)?)
}

/// Check if an IPFS daemon is reachable.
#[tauri::command]
pub async fn audit_check_ipfs_daemon(
    api_url: Option<String>,
) -> AppResult<bool> {
    let config = crate::audit::ipfs::IpfsConfig {
        api_url: api_url.unwrap_or_else(|| "http://127.0.0.1:5001".to_string()),
        cid_version: 1,
    };
    Ok(crate::audit::ipfs::check_daemon(&config).await?)
}

// ─── Stellar RPC client commands ──────────────────────────────────────

/// Get the latest committed root from Stellar using contract simulation
/// (read-only, no `stellar` CLI subprocess required).
///
/// Resolves the contract ID and RPC URL from the audit mode configuration
/// so the correct contract is queried for both testnet and mainnet.
#[tauri::command]
pub async fn audit_get_onchain_root_rpc(
    app: tauri::AppHandle,
    rpc_url: Option<String>,
) -> AppResult<Option<OnChainRoot>> {
    use crate::audit::audit_mode::load_mode_config;
    use crate::audit::dev_setup::ChainConfig;
    use crate::audit::stellar_native;

    let config = load_mode_config(&app)?;
    let chain = match config.network {
        crate::audit::audit_mode::AuditNetwork::Testnet => ChainConfig::testnet(),
        crate::audit::audit_mode::AuditNetwork::Mainnet => {
            ChainConfig::mainnet(config.mainnet_rpc_url.clone(), config.mainnet_contract_id.clone())
        }
    };

    let url = rpc_url.unwrap_or_else(|| chain.rpc_url.clone());
    let kp = stellar_native::generate_keypair();
    Ok(stellar_native::get_current_root_native(&kp, &url, &chain.contract_id).await?)
}

// ─── Dev mode onboarding commands ─────────────────────────────────────

/// Check the onboarding status — whether a keypair and Pinata config
/// are already saved in the OS keychain.
#[tauri::command]
pub async fn audit_check_onboarding() -> AppResult<crate::audit::dev_setup::OnboardingStatus> {
    Ok(crate::audit::dev_setup::check_onboarding_status()?)
}

/// Save Pinata API credentials to the OS keychain and test the connection.
#[tauri::command]
pub async fn audit_save_pinata_config(
    api_key: String,
    api_secret: String,
) -> AppResult<bool> {
    let config = crate::audit::pinata::PinataConfig {
        api_key,
        api_secret,
        gateway_url: "https://gateway.pinata.cloud".to_string(),
    };

    // Test the connection before saving.
    let ok = crate::audit::pinata::check(&config).await?;
    if !ok {
        return Err(AppError::Validation(
            "Pinata authentication failed — check your API key and secret".to_string(),
        ));
    }

    crate::audit::dev_setup::save_pinata_to_keychain(&config)?;
    Ok(true)
}

/// Test a Pinata connection without saving credentials.
#[tauri::command]
pub async fn audit_test_pinata_connection(
    api_key: String,
    api_secret: String,
) -> AppResult<bool> {
    let config = crate::audit::pinata::PinataConfig {
        api_key,
        api_secret,
        gateway_url: "https://gateway.pinata.cloud".to_string(),
    };
    Ok(crate::audit::pinata::check(&config).await?)
}

/// Generate and fund a new Stellar testnet account, saving the keypair
/// to the OS keychain.
#[tauri::command]
pub async fn audit_generate_stellar_account() -> AppResult<String> {
    let kp = crate::audit::dev_setup::generate_and_fund_account().await?;
    let account_id = kp.account_id();
    crate::audit::dev_setup::save_keypair_to_keychain(&kp)?;
    Ok(account_id)
}

/// Check if the given MongoDB connection is a replica set
/// (required for change streams).
#[tauri::command]
pub async fn audit_check_replica_set(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<bool> {
    let entry = state.clients.get(&connection_id).await?;
    Ok(crate::audit::dev_setup::check_mongodb_rs(&entry.client).await?)
}

/// Commit a root to Stellar using native signing (no CLI subprocess).
///
/// This is the dev mode replacement for `audit_commit_root` that uses
/// the keypair from the OS keychain instead of the `stellar` CLI identity.
#[tauri::command]
pub async fn audit_commit_root_native(
    state: State<'_, AppState>,
    metadata: Option<String>,
) -> AppResult<CommitResult> {
    let audit = &state.audit_log;
    let root = audit.root()?;

    use ark_ff::{BigInteger, PrimeField};
    let root_bigint = root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let root_hex = hex::encode(&root_bytes);

    // Check if this root is already committed on-chain to avoid
    // RootAlreadyCommitted errors (same guard as audit_commit_root).
    let root_hex_check = root_hex.clone();
    let rpc_client = crate::audit::stellar_rpc::StellarRpcClient::new();
    let onchain = rpc_client.get_current_root().await?;
    if let Some(ref entry) = onchain {
        if entry.root_hex == root_hex_check {
            return Err(AppError::Validation(format!(
                "root 0x{}.. is already committed on-chain (seq #{}). New audit events are needed to produce a different root.",
                &root_hex_check[..16],
                entry.sequence
            )));
        }
    }

    let meta = metadata.unwrap_or_else(|| {
        format!(
            "events={} leaves={}",
            audit.event_count(),
            audit.leaf_count()
        )
    });

    // Load keypair from keychain.
    let kp = crate::audit::dev_setup::load_keypair_from_keychain()?
        .ok_or_else(|| {
            AppError::Validation(
                "no Stellar keypair found — run onboarding first".to_string(),
            )
        })?;

    // Use testnet config (dev mode).
    let chain = crate::audit::dev_setup::ChainConfig::testnet();

    // Commit via native signing.
    let result = crate::audit::stellar_native::commit_root_native(
        &root_hex,
        &meta,
        &kp,
        &chain.rpc_url,
        &chain.horizon_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await?;

    Ok(result)
}

/// Commit a root to Stellar using the **production** keypair + chosen network.
///
/// Production mode: the user imports their own keypair and picks testnet or
/// mainnet. This is the "double check" — they run the in-app pipeline against
/// the same network their remote deployment uses, to verify it works.
///
/// - Testnet: uses the bundled testnet contract ID + testnet RPC/Horizon.
/// - Mainnet: uses the user's contract ID + mainnet RPC/Horizon.
#[tauri::command]
pub async fn audit_commit_root_production(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    metadata: Option<String>,
) -> AppResult<CommitResult> {
    use crate::audit::audit_mode::{load_mode_config, load_production_keypair, AuditNetwork};
    use crate::audit::dev_setup::ChainConfig;
    use crate::audit::stellar_native;

    let audit = &state.audit_log;
    let root = audit.root()?;

    use ark_ff::{BigInteger, PrimeField};
    let root_bigint = root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let root_hex = hex::encode(&root_bytes);

    // Load mode config to pick the network + contract/rpc.
    let config = load_mode_config(&app)?;

    let chain = match config.network {
        AuditNetwork::Testnet => ChainConfig::testnet(),
        AuditNetwork::Mainnet => {
            if config.mainnet_contract_id.is_empty() {
                return Err(AppError::Validation(
                    "mainnet contract ID is not configured — set it in Audit Settings".to_string(),
                ));
            }
            ChainConfig::mainnet(config.mainnet_rpc_url.clone(), config.mainnet_contract_id.clone())
        }
    };

    // Check if this root is already committed on-chain.
    let root_hex_check = root_hex.clone();
    let rpc_url = chain.rpc_url.clone();
    let rpc_client = crate::audit::stellar_rpc::StellarRpcClient::with_url(&rpc_url);
    let onchain = rpc_client.get_current_root().await?;
    if let Some(ref entry) = onchain {
        if entry.root_hex == root_hex_check {
            return Err(AppError::Validation(format!(
                "root 0x{}.. is already committed on-chain (seq #{}). New audit events are needed to produce a different root.",
                &root_hex_check[..16],
                entry.sequence
            )));
        }
    }

    let meta = metadata.unwrap_or_else(|| {
        format!(
            "events={} leaves={} network={}",
            audit.event_count(),
            audit.leaf_count(),
            chain.network
        )
    });

    // Load the production keypair from the keychain.
    let kp = load_production_keypair()?.ok_or_else(|| {
        AppError::Validation(
            "no production keypair found — import your Stellar secret key in Audit Settings"
                .to_string(),
        )
    })?;

    let result = stellar_native::commit_root_native(
        &root_hex,
        &meta,
        &kp,
        &chain.rpc_url,
        &chain.horizon_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await?;

    Ok(result)
}

/// Publish an epoch batch to IPFS via Pinata (no daemon required).
///
/// Uses the Pinata API key from the OS keychain.
#[tauri::command]
pub async fn audit_publish_epoch_to_pinata(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    epoch_number: u64,
) -> AppResult<crate::audit::ipfs::IpfsPublishResult> {
    use tauri::Manager;

    // Load Pinata config from keychain.
    let pinata_config = crate::audit::dev_setup::load_pinata_from_keychain()?
        .ok_or_else(|| {
            AppError::Validation(
                "no Pinata config found — run onboarding first".to_string(),
            )
        })?;

    // Find the epoch and get its event range.
    let epochs = state.epoch_manager.list_epochs();
    let epoch = epochs
        .iter()
        .find(|e| e.epoch_number == epoch_number)
        .ok_or_else(|| AppError::Validation(format!("epoch {} not found", epoch_number)))?;

    let start_index = epoch.start_index;
    let end_index = epoch.end_index.ok_or_else(|| {
        AppError::Validation(format!(
            "epoch {} is still open — close it before publishing to IPFS",
            epoch_number
        ))
    })?;

    // Read the JSONL log and extract the events for this epoch.
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Validation(format!("resolve app data dir: {e}")))?;
    let events_path = data_dir.join("audit").join("events.jsonl");
    let events_jsonl = if events_path.exists() {
        std::fs::read_to_string(&events_path)?
    } else {
        String::new()
    };

    let batch_content = extract_epoch_batch(&events_jsonl, start_index, end_index);
    if batch_content.is_empty() {
        return Err(AppError::Validation(format!(
            "no events found for epoch {} (range {}-{})",
            epoch_number, start_index, end_index
        )));
    }

    // Publish to Pinata.
    let result =
        crate::audit::pinata::publish_epoch_batch(&pinata_config, epoch_number, &batch_content)
            .await?;

    // Save the CID to sled.
    state.audit_log.save_ipfs_cid(epoch_number, &result.cid)?;

    Ok(result)
}

// ─── Multi-publisher threshold attestation commands ───────────────────

/// Register a new publisher for threshold attestation.
#[tauri::command]
pub async fn audit_add_publisher(
    state: State<'_, AppState>,
    public_key: String,
    name: String,
) -> AppResult<crate::audit::attestation::Publisher> {
    Ok(state.attestation_manager.add_publisher(public_key, name)?)
}

/// Remove a registered publisher.
#[tauri::command]
pub async fn audit_remove_publisher(
    state: State<'_, AppState>,
    public_key: String,
) -> AppResult<()> {
    Ok(state.attestation_manager.remove_publisher(&public_key)?)
}

/// List all registered publishers.
#[tauri::command]
pub async fn audit_list_publishers(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::audit::attestation::Publisher>> {
    Ok(state.attestation_manager.list_publishers()?)
}

/// Set the threshold K for K-of-N attestation.
#[tauri::command]
pub async fn audit_set_attestation_threshold(
    state: State<'_, AppState>,
    threshold: usize,
) -> AppResult<()> {
    state.attestation_manager.set_threshold(threshold);
    Ok(())
}

/// Get the current threshold K.
#[tauri::command]
pub async fn audit_get_attestation_threshold(
    state: State<'_, AppState>,
) -> AppResult<usize> {
    Ok(state.attestation_manager.threshold())
}

/// Submit an attestation for an epoch.
///
/// The caller provides:
/// - `epoch_number`: the epoch being attested
/// - `root_hex`: the root hash being attested
/// - `publisher_public_key`: the publisher's ed25519 public key (hex)
/// - `signature_hex`: the ed25519 signature of the root hash (hex)
///
/// The signature is verified against the publisher's registered public
/// key. If valid, the attestation is stored.
#[tauri::command]
pub async fn audit_submit_attestation(
    state: State<'_, AppState>,
    epoch_number: u64,
    root_hex: String,
    publisher_public_key: String,
    signature_hex: String,
) -> AppResult<crate::audit::attestation::Attestation> {
    Ok(state.attestation_manager.submit_attestation(
        epoch_number,
        &root_hex,
        &publisher_public_key,
        &signature_hex,
    )?)
}

/// List all attestations for an epoch.
#[tauri::command]
pub async fn audit_list_attestations(
    state: State<'_, AppState>,
    epoch_number: u64,
) -> AppResult<Vec<crate::audit::attestation::Attestation>> {
    Ok(state.attestation_manager.list_attestations(epoch_number)?)
}

/// Get the attestation status for an epoch (threshold met? who attested?).
#[tauri::command]
pub async fn audit_get_attestation_status(
    state: State<'_, AppState>,
    epoch_number: u64,
    root_hex: String,
) -> AppResult<crate::audit::attestation::AttestationStatus> {
    Ok(state.attestation_manager.get_status(epoch_number, &root_hex)?)
}

// ─── Oplog completeness verification commands ─────────────────────────

/// Result of the oplog integrity verification (three-way compare).
///
/// Mirrors the daemon's `OplogIntegrityReport` but is exposed via the Tauri
/// IPC so the desktop app can run "Verify Oplog Integrity" without the
/// standalone daemon.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogIntegrityReport {
    pub sequence: u64,
    pub on_chain_oplog_root: String,
    pub auditor_oplog_root: Option<String>,
    pub oplog_entry_count: Option<u64>,
    pub all_match: bool,
    pub on_chain_matches_auditor: bool,
    /// "complete", "mismatch", "stale", "no_commitment", "no_oplog_commitment", or "error"
    pub verdict: String,
    pub explanation: String,
    pub alerts: Vec<String>,
}

/// Verify oplog integrity via a three-way compare.
///
/// This is the desktop-app equivalent of the daemon's `/reader/verify-oplog`
/// endpoint. It requires a MongoDB connection (to the independent replica
/// member) identified by `connection_id`. It:
/// 1. Gets the latest on-chain root from Stellar.
/// 2. Gets the on-chain oplog commitment for that sequence.
/// 3. Independently computes the oplog hash from the connected replica.
/// 4. Compares the on-chain root with the auditor's computed root.
///
/// Verdicts:
/// - "complete" — on-chain root matches auditor's computation.
/// - "mismatch" — on-chain root differs from auditor's (omission detected).
/// - "stale" — oplog has rolled over; rely on on-chain attestation.
/// - "no_commitment" — no on-chain root has been committed.
/// - "no_oplog_commitment" — epoch was committed without an oplog hash.
/// - "error" — computation failed for an unexpected reason.
#[tauri::command]
pub async fn audit_verify_oplog_integrity(
    state: State<'_, AppState>,
    connection_id: String,
    rpc_url: Option<String>,
) -> AppResult<OplogIntegrityReport> {
    use crate::audit::oplog::{compute_oplog_range_hash, OplogTimestamp};

    // 1. Get the MongoDB client from the connection registry.
    let entry = state.clients.get(&connection_id).await?;
    let client = entry.client.clone();

    // 2. Get the latest on-chain root.
    let rpc_url = rpc_url.unwrap_or_else(|| {
        crate::audit::stellar_rpc::TESTNET_RPC_URL.to_string()
    });
    let rpc_client = crate::audit::stellar_rpc::StellarRpcClient::with_url(&rpc_url);
    let onchain_root = rpc_client.get_current_root().await?;

    let sequence = match onchain_root {
        Some(ref root) => root.sequence,
        None => {
            return Ok(OplogIntegrityReport {
                sequence: 0,
                on_chain_oplog_root: "none".to_string(),
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                verdict: "no_commitment".to_string(),
                explanation: "No on-chain root has been committed yet.".to_string(),
                alerts: vec![],
            });
        }
    };

    // 3. Get the on-chain oplog commitment via native contract simulation.
    let probe_kp = stellar_native::generate_keypair();
    let on_chain_oplog = stellar_native::get_oplog_commitment_native(
        sequence,
        &probe_kp,
        &rpc_url,
        &crate::audit::dev_setup::ChainConfig::testnet().contract_id,
    )
    .await?;

    let on_chain_oplog_root = match on_chain_oplog {
        Some(ref oc) => oc.oplog_root_hex.clone(),
        None => {
            return Ok(OplogIntegrityReport {
                sequence,
                on_chain_oplog_root: "none".to_string(),
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                verdict: "no_oplog_commitment".to_string(),
                explanation: format!(
                    "Epoch {sequence} was committed without an oplog hash. \
                    Completeness cannot be verified."
                ),
                alerts: vec![format!(
                    "Epoch {sequence} has no oplog commitment — completeness not guaranteed"
                )],
            });
        }
    };

    let on_chain_ref = on_chain_oplog.as_ref().unwrap();
    let start_ts = OplogTimestamp::unpack_u64(on_chain_ref.oplog_start_ts);
    let end_ts = OplogTimestamp::unpack_u64(on_chain_ref.oplog_end_ts);
    let on_chain_entry_count = on_chain_ref.oplog_entry_count;

    // 4. Independently compute the oplog hash.
    match compute_oplog_range_hash(&client, sequence, start_ts, end_ts).await {
        Ok(range) => {
            // Detect oplog rollover.
            if range.entry_count == 0 && on_chain_entry_count > 0 {
                return Ok(OplogIntegrityReport {
                    sequence,
                    on_chain_oplog_root,
                    auditor_oplog_root: None,
                    oplog_entry_count: Some(0),
                    all_match: false,
                    on_chain_matches_auditor: false,
                    verdict: "stale".to_string(),
                    explanation: format!(
                        "Oplog has rolled over — the {on_chain_entry_count} entries committed \
                        for this epoch are no longer in the oplog. Relying on the independent \
                        member's on-chain attestation (signed when fresh) as the durable guarantee."
                    ),
                    alerts: vec![format!(
                        "Oplog rolled over for epoch {sequence} — {on_chain_entry_count} entries \
                        were committed but 0 found. Verify via on-chain attestation instead."
                    )],
                });
            }

            let auditor_root = range.oplog_merkle_root_hex.clone();
            let matches = auditor_root == on_chain_oplog_root;

            let (verdict, explanation, alerts) = if matches {
                (
                    "complete".to_string(),
                    format!(
                        "Oplog integrity verified: on-chain root matches auditor's independent \
                        computation. {} oplog entries in the range.",
                        range.entry_count
                    ),
                    vec![],
                )
            } else {
                (
                    "mismatch".to_string(),
                    format!(
                        "OMISSION DETECTED: on-chain oplog root {} does not match auditor's \
                        independent computation {}. The operator may have omitted writes \
                        from the audit log.",
                        on_chain_oplog_root, auditor_root
                    ),
                    vec![format!(
                        "CRITICAL: oplog hash mismatch — on_chain={} auditor={} — possible omission",
                        on_chain_oplog_root, auditor_root
                    )],
                )
            };

            Ok(OplogIntegrityReport {
                sequence,
                on_chain_oplog_root,
                auditor_oplog_root: Some(auditor_root),
                oplog_entry_count: Some(range.entry_count),
                all_match: matches,
                on_chain_matches_auditor: matches,
                verdict,
                explanation,
                alerts,
            })
        }
        Err(e) => {
            let err_msg = format!("{e}");
            let is_stale = err_msg.contains("lastCommittedOpTime")
                || err_msg.contains("not found")
                || err_msg.contains("replica set");
            let verdict = if is_stale { "stale" } else { "error" };
            let explanation = if is_stale {
                format!(
                    "Oplog entries for this epoch may have rolled over. \
                    Relying on the independent member's on-chain attestation \
                    (signed when fresh) as the durable guarantee. Detail: {err_msg}"
                )
            } else {
                format!("Failed to compute oplog hash: {err_msg}")
            };
            Ok(OplogIntegrityReport {
                sequence,
                on_chain_oplog_root,
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                verdict: verdict.to_string(),
                explanation,
                alerts: vec![format!("Oplog verification: {err_msg}")],
            })
        }
    }
}

/// Get the on-chain oplog commitment for a specific epoch.
///
/// Returns the oplog root, start/end timestamps, and entry count committed
/// on-chain by the operator. This is the "operator's commitment" that the
/// auditor compares against their own independent computation.
#[tauri::command]
pub async fn audit_get_oplog_commitment(
    sequence: u64,
) -> AppResult<Option<crate::audit::stellar::OnChainOplogCommitment>> {
    let chain = crate::audit::dev_setup::ChainConfig::testnet();
    let kp = stellar_native::generate_keypair();
    Ok(stellar_native::get_oplog_commitment_native(sequence, &kp, &chain.rpc_url, &chain.contract_id).await?)
}

/// Extract the JSONL lines for events in the given index range
/// (inclusive on both ends).
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
            network: "testnet".to_string(),
            contract_id: "C123".to_string(),
            tx_hash: "txabc".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"rootHex\":\"root\""), "rootHex must be camelCase: {json}");
        assert!(json.contains("\"leafIndex\":42"), "leafIndex must be camelCase: {json}");
        assert!(json.contains("\"pubSignals\":[\"sig\"]"), "pubSignals must be camelCase: {json}");
        assert!(json.contains("\"network\":\"testnet\""), "network must be camelCase: {json}");
        assert!(json.contains("\"contractId\":\"C123\""), "contractId must be camelCase: {json}");
        assert!(json.contains("\"txHash\":\"txabc\""), "txHash must be camelCase: {json}");
    }

    #[test]
    fn resolve_circuit_paths_pure_uses_explicit_paths_when_provided() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) = resolve_circuit_paths_pure(
            Some("/custom/path/circuit.r1cs"),
            Some("/custom/path/circuit.wasm"),
            dir,
        )
        .unwrap();
        assert_eq!(r1cs, "/custom/path/circuit.r1cs");
        assert_eq!(wasm, "/custom/path/circuit.wasm");
    }

    #[test]
    fn resolve_circuit_paths_pure_falls_back_to_resource_dir() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) = resolve_circuit_paths_pure(None, None, dir).unwrap();
        assert_eq!(r1cs, "/tmp/fake-app/resources/circuits/merkle_inclusion.r1cs");
        assert_eq!(wasm, "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm");
    }

    #[test]
    fn resolve_circuit_paths_pure_ignores_empty_strings() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) = resolve_circuit_paths_pure(Some(""), Some(""), dir).unwrap();
        assert_eq!(r1cs, "/tmp/fake-app/resources/circuits/merkle_inclusion.r1cs");
        assert_eq!(wasm, "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm");
    }

    #[test]
    fn resolve_circuit_paths_pure_mixed_explicit_and_fallback() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) =
            resolve_circuit_paths_pure(Some("/explicit/circuit.r1cs"), None, dir).unwrap();
        assert_eq!(r1cs, "/explicit/circuit.r1cs");
        assert_eq!(wasm, "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm");
    }

    #[test]
    fn bundled_circuit_artifacts_exist_on_disk() {
        // Verify the artifacts were actually copied to the resources dir.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let r1cs = std::path::Path::new(manifest_dir)
            .join("resources/circuits/merkle_inclusion.r1cs");
        let wasm = std::path::Path::new(manifest_dir)
            .join("resources/circuits/merkle_inclusion.wasm");
        assert!(r1cs.exists(), "R1CS artifact missing at {}", r1cs.display());
        assert!(wasm.exists(), "WASM artifact missing at {}", wasm.display());
    }

    #[test]
    fn extract_epoch_batch_filters_by_index_range() {
        let jsonl = [
            r#"{"index":0,"operation":"insert"}"#,
            r#"{"index":1,"operation":"update"}"#,
            r#"{"index":2,"operation":"delete"}"#,
            r#"{"index":3,"operation":"insert"}"#,
            r#"{"index":4,"operation":"update"}"#,
        ]
        .join("\n");

        // Extract events 1-3 (inclusive).
        let batch = extract_epoch_batch(&jsonl, 1, 3);
        assert_eq!(batch.lines().count(), 3);
        assert!(batch.contains(r#""index":1"#));
        assert!(batch.contains(r#""index":2"#));
        assert!(batch.contains(r#""index":3"#));
        assert!(!batch.contains(r#""index":0"#));
        assert!(!batch.contains(r#""index":4"#));
    }

    #[test]
    fn extract_epoch_batch_empty_range_returns_empty() {
        let jsonl = r#"{"index":0,"operation":"insert"}"#;
        let batch = extract_epoch_batch(jsonl, 5, 10);
        assert!(batch.is_empty());
    }

    #[test]
    fn extract_epoch_batch_skips_invalid_lines() {
        let jsonl = [
            r#"{"index":0,"operation":"insert"}"#,
            "invalid json line",
            r#"{"index":1,"operation":"update"}"#,
        ]
        .join("\n");

        let batch = extract_epoch_batch(&jsonl, 0, 1);
        assert_eq!(batch.lines().count(), 2);
    }

    #[test]
    fn extract_epoch_batch_single_event() {
        let jsonl = [
            r#"{"index":0,"operation":"insert"}"#,
            r#"{"index":1,"operation":"update"}"#,
        ]
        .join("\n");

        let batch = extract_epoch_batch(&jsonl, 0, 0);
        assert_eq!(batch.lines().count(), 1);
        assert!(batch.contains(r#""index":0"#));
    }
}
