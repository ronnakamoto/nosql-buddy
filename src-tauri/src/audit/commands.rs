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

use crate::audit::audit_mode::{load_mode_config, AuditModeConfig, AuditNetwork};
use crate::audit::dev_setup::ChainConfig;
use crate::audit::stellar::{CommitResult, OnChainRoot, VerifyInclusionResult};
use crate::audit::stellar_native;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Resolve the active [`ChainConfig`] from persisted mode settings.
///
/// Testnet → the shared testnet defaults (RPC URL + bundled contract ID).
/// Mainnet → the user-configured RPC URL + contract ID from the settings store.
///
/// All commands that touch the Stellar network must call this instead of
/// hardcoding `ChainConfig::testnet()`.
fn chain_config(app: &tauri::AppHandle) -> AppResult<ChainConfig> {
    let mode = load_mode_config(app)?;
    Ok(chain_config_from_mode(&mode))
}

fn chain_config_from_mode(mode: &AuditModeConfig) -> ChainConfig {
    match mode.network {
        AuditNetwork::Testnet => testnet_chain_from_contract_id(&mode.testnet_contract_id),
        AuditNetwork::Mainnet => ChainConfig::mainnet(
            mode.mainnet_rpc_url.clone(),
            mode.mainnet_contract_id.clone(),
        ),
    }
}

fn testnet_chain_from_contract_id(contract_id: &str) -> ChainConfig {
    if contract_id.trim().is_empty() {
        ChainConfig::testnet()
    } else {
        ChainConfig::testnet_with_contract(contract_id.to_string())
    }
}

/// Status snapshot of the audit log.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditStatus {
    pub root_hex: String,
    pub leaf_count: usize,
    pub event_count: usize,
    pub tree_height: u32,
    /// Distinct audit domains `(deploymentId, database)` with their event
    /// counts, so the UI can populate per-deployment / per-database filters
    /// and grouping without scanning the full event list client-side.
    pub domains: Vec<AuditDomain>,
}

/// A single audit domain: the unique pairing of a deployment identity and a
/// database, plus how many events it holds.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuditDomain {
    pub deployment_id: String,
    pub database: String,
    pub event_count: usize,
}

/// Compute the distinct audit domains from an event list, sorted by
/// deployment id then database for a stable UI ordering.
fn summarize_domains(events: &[crate::audit::AuditEvent]) -> Vec<AuditDomain> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<(String, String), usize> = BTreeMap::new();
    for ev in events {
        *counts
            .entry((ev.deployment_id.clone(), ev.database.clone()))
            .or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|((deployment_id, database), event_count)| AuditDomain {
            deployment_id,
            database,
            event_count,
        })
        .collect()
}

/// Apply optional deployment_id / database filters to an event list.
/// `None` (or an empty string) means "no filter on that dimension".
fn filter_events(
    events: Vec<crate::audit::AuditEvent>,
    deployment_id: Option<&str>,
    database: Option<&str>,
) -> Vec<crate::audit::AuditEvent> {
    let dep = deployment_id.filter(|s| !s.is_empty());
    let db = database.filter(|s| !s.is_empty());
    if dep.is_none() && db.is_none() {
        return events;
    }
    events
        .into_iter()
        .filter(|ev| dep.map_or(true, |d| ev.deployment_id == d))
        .filter(|ev| db.map_or(true, |d| ev.database == d))
        .collect()
}

/// Get the current audit log status.
///
/// Async because `root_hex()` computes the Merkle root via recursive Poseidon
/// hashing, which is CPU-intensive and must not block the main thread.
#[tauri::command]
pub async fn audit_get_status(
    state: State<'_, AppState>,
    deployment_id: Option<String>,
    database: Option<String>,
) -> AppResult<AuditStatus> {
    let audit = &state.audit_log;
    let events = audit.list_events();
    let domains = summarize_domains(&events);
    let filtered_count = filter_events(events, deployment_id.as_deref(), database.as_deref()).len();
    Ok(AuditStatus {
        root_hex: audit.root_hex()?,
        leaf_count: audit.leaf_count(),
        event_count: filtered_count,
        tree_height: 20,
        domains,
    })
}

/// List recorded audit events, optionally filtered by deployment identity
/// and/or database. Filtering is done server-side so the UI never has to pull
/// the entire log just to show one domain.
#[tauri::command]
pub async fn audit_list_events(
    state: State<'_, AppState>,
    deployment_id: Option<String>,
    database: Option<String>,
) -> AppResult<Vec<crate::audit::AuditEvent>> {
    Ok(filter_events(
        state.audit_log.list_events(),
        deployment_id.as_deref(),
        database.as_deref(),
    ))
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
    let chain = chain_config_from_mode(&mode_config);
    let network = chain.network.clone();
    let contract_id = chain.contract_id.clone();

    // Find the epoch that contains this leaf and get its on-chain tx hash.
    let tx_hash = state
        .epoch_manager
        .list_epochs()
        .into_iter()
        .find(|e| e.start_index <= index && e.end_index.map_or(false, |end| end >= index))
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
    let chain = chain_config_from_mode(&mode_config);

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
                .resolve(
                    "resources/circuits/merkle_inclusion.r1cs",
                    tauri::path::BaseDirectory::Resource,
                )
                .map_err(|e| AppError::Validation(format!("resolve r1cs resource: {e}")))?;
            resource.to_string_lossy().to_string()
        }
    };
    let wasm = match wasm_path.as_deref() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => {
            let resource = app
                .path()
                .resolve(
                    "resources/circuits/merkle_inclusion.wasm",
                    tauri::path::BaseDirectory::Resource,
                )
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
    deployment_id: Option<String>,
    payload: String,
) -> AppResult<u64> {
    // The leaf is derived from the raw payload string. The same payload
    // is stored on disk so replay can recompute and verify the leaf.
    let leaf = crate::audit::leaf_from_payload(&operation, &database, &collection, &payload);

    let index = state.audit_log.record(
        deployment_id.as_deref().unwrap_or(""),
        &operation,
        &database,
        &collection,
        &payload,
        leaf,
    )?;

    // Advance the open batch (epoch) so the UI's "Batch · filling" counter
    // tracks recorded events and the on-disk epoch state stays in sync.
    if let Err(e) = state.epoch_manager.record_event(index, &state.audit_log) {
        tracing::warn!(error = %e, "failed to update epoch for recorded event");
    }

    Ok(index)
}

/// Commit the current Merkle root to the Soroban contract on Stellar testnet.
///
/// This anchors the local audit log to an immutable on-chain commitment,
/// making truncation of the log's tail detectable. The root is submitted
/// along with optional metadata (e.g. event count, timestamp).
#[tauri::command]
pub async fn audit_commit_root(
    app: tauri::AppHandle,
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

    let chain = chain_config(&app)?;

    // Check if this root is already committed on-chain to avoid
    // RootAlreadyCommitted errors.
    let root_hex_check = root_hex.clone();
    let probe_kp = stellar_native::generate_keypair();
    let onchain =
        stellar_native::get_current_root_native(&probe_kp, &chain.rpc_url, &chain.contract_id)
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
    let kp = crate::audit::dev_setup::load_keypair_from_keychain()?.ok_or_else(|| {
        AppError::Validation("no Stellar keypair found — run onboarding first".to_string())
    })?;

    // Commit via native signing.
    let result = stellar_native::commit_root_native(
        &root_hex,
        &meta,
        &kp,
        &chain.rpc_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await?;

    Ok(result)
}

/// Get the latest committed root from the Soroban contract on the active network.
#[tauri::command]
pub async fn audit_get_onchain_root(app: tauri::AppHandle) -> AppResult<Option<OnChainRoot>> {
    let chain = chain_config(&app)?;
    let kp = stellar_native::generate_keypair();
    Ok(stellar_native::get_current_root_native(&kp, &chain.rpc_url, &chain.contract_id).await?)
}

// ─── Epoch management commands ────────────────────────────────────────

/// List all epochs (open and closed).
#[tauri::command]
pub async fn audit_list_epochs(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::audit::epoch::Epoch>> {
    Ok(state.epoch_manager.list_epochs())
}

/// Get the current (open) epoch.
#[tauri::command]
pub async fn audit_current_epoch(
    state: State<'_, AppState>,
) -> AppResult<crate::audit::epoch::Epoch> {
    Ok(state.epoch_manager.current_epoch())
}

/// Manually close the current epoch and freeze its root.
#[tauri::command]
pub async fn audit_close_epoch(
    state: State<'_, AppState>,
) -> AppResult<crate::audit::epoch::Epoch> {
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

/// Wipe all local audit data: the audit log (events + Merkle tree), the batch
/// (epoch) history, and the verification timeline. On-chain commitments are
/// NOT affected — this only clears local state so capture can start fresh.
#[tauri::command]
pub async fn audit_reset_data(state: State<'_, AppState>) -> AppResult<()> {
    state.audit_log.clear()?;
    state.epoch_manager.reset()?;
    state.verification_store.clear()?;
    // Re-align the open epoch's counter with the now-empty audit log.
    state
        .epoch_manager
        .sync_open_epoch_with_audit_log(&state.audit_log)?;
    Ok(())
}

// ─── Per-domain segmentation commands (Phase 2) ───────────────────────
//
// These expose per-`(deploymentId, database)` audit domains: their own
// Merkle roots, selective-disclosure inclusion proofs, legal holds, and
// logical retention (pruning). The shared global tree remains the anchored
// source of truth; per-domain roots are secondary commitments derived from
// the same leaves, so on-chain anchoring is untouched.
//
// `require_domain_access` is the RBAC hook point: today it only validates the
// domain key shape, but it is where an authorization policy (per-deployment /
// per-database roles) plugs in before any domain-scoped read or mutation.

/// RBAC hook point + input validation for domain-scoped commands.
///
/// Returns the normalized `(deployment_id, database)` pair. Authorization
/// enforcement is intentionally a single chokepoint so a future policy only
/// has to be wired here.
fn require_domain_access(deployment_id: &str, database: &str) -> AppResult<()> {
    if database.trim().is_empty() {
        return Err(AppError::Validation(
            "a database is required to address an audit domain".to_string(),
        ));
    }
    let _ = deployment_id; // deployment_id may be empty (the "unattributed" domain)
    Ok(())
}

/// One audit domain plus its secondary Merkle root and legal-hold flag.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainRootInfo {
    pub deployment_id: String,
    pub database: String,
    pub root_hex: String,
    pub event_count: usize,
    pub legal_hold: bool,
    pub retained_roots: Vec<crate::audit::DomainRetentionRoot>,
}

/// Aggregation super-root over all per-domain roots.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSuperRootResult {
    pub super_root_hex: String,
    pub domains: Vec<DomainRootInfo>,
}

/// Inclusion proof that one domain root is part of the aggregation super-root.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSuperProofResult {
    pub deployment_id: String,
    pub database: String,
    pub domain_root_hex: String,
    pub super_root_hex: String,
    pub position: usize,
    pub leaf_hex: String,
    pub path_elements: Vec<String>,
    pub path_indices: Vec<u64>,
}

/// List the distinct audit domains with their per-domain roots and status.
#[tauri::command]
pub async fn audit_list_domains(state: State<'_, AppState>) -> AppResult<Vec<DomainRootInfo>> {
    let audit = &state.audit_log;
    let mut out = Vec::new();
    for (deployment_id, database) in audit.list_domains() {
        let (root_hex, event_count) = audit.domain_root(&deployment_id, &database)?;
        let legal_hold = audit.is_legal_hold(&deployment_id, &database);
        let retained_roots = audit.retained_domain_roots(&deployment_id, &database);
        out.push(DomainRootInfo {
            deployment_id,
            database,
            root_hex,
            event_count,
            legal_hold,
            retained_roots,
        });
    }
    Ok(out)
}

/// Get the aggregation super-root over all per-domain roots.
#[tauri::command]
pub async fn audit_get_domain_super_root(
    state: State<'_, AppState>,
) -> AppResult<DomainSuperRootResult> {
    let audit = &state.audit_log;
    let (super_root_hex, entries) = audit.domain_super_root()?;
    let mut domains = Vec::with_capacity(entries.len());
    for (deployment_id, database, root_hex) in entries {
        let (_root_hex, event_count) = audit.domain_root(&deployment_id, &database)?;
        domains.push(DomainRootInfo {
            deployment_id: deployment_id.clone(),
            database: database.clone(),
            root_hex,
            event_count,
            legal_hold: audit.is_legal_hold(&deployment_id, &database),
            retained_roots: audit.retained_domain_roots(&deployment_id, &database),
        });
    }
    Ok(DomainSuperRootResult {
        super_root_hex,
        domains,
    })
}

/// Get the secondary Merkle root (and status) for a single audit domain.
#[tauri::command]
pub async fn audit_get_domain_root(
    state: State<'_, AppState>,
    deployment_id: String,
    database: String,
) -> AppResult<DomainRootInfo> {
    require_domain_access(&deployment_id, &database)?;
    let audit = &state.audit_log;
    let (root_hex, event_count) = audit.domain_root(&deployment_id, &database)?;
    Ok(DomainRootInfo {
        legal_hold: audit.is_legal_hold(&deployment_id, &database),
        retained_roots: audit.retained_domain_roots(&deployment_id, &database),
        deployment_id,
        database,
        root_hex,
        event_count,
    })
}

/// A selective-disclosure inclusion proof against a single domain's root.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainProofResult {
    pub deployment_id: String,
    pub database: String,
    pub position: usize,
    pub leaf_hex: String,
    pub root_hex: String,
    pub path_elements: Vec<String>,
    pub path_indices: Vec<u64>,
}

/// Generate an inclusion proof for the leaf at the given 0-indexed position
/// within a domain, against that domain's secondary Merkle tree. This proves
/// one domain's record without revealing any other domain's leaves.
#[tauri::command]
pub async fn audit_generate_domain_proof(
    state: State<'_, AppState>,
    deployment_id: String,
    database: String,
    position: usize,
) -> AppResult<DomainProofResult> {
    use ark_ff::{BigInteger, PrimeField};

    require_domain_access(&deployment_id, &database)?;
    let (proof, root_hex) =
        state
            .audit_log
            .prove_inclusion_in_domain(&deployment_id, &database, position)?;
    Ok(DomainProofResult {
        deployment_id,
        database,
        position,
        leaf_hex: hex::encode(proof.leaf.into_bigint().to_bytes_be()),
        root_hex,
        path_elements: proof
            .path_elements
            .into_iter()
            .map(|f| hex::encode(f.into_bigint().to_bytes_be()))
            .collect(),
        path_indices: proof.path_indices,
    })
}

/// Convert a super-root inclusion proof into the wire result, hex-encoding the
/// field elements (big-endian, modulus-reduced). Pure so it can be tested
/// against a real proof without a Tauri runtime.
fn super_proof_result_from(
    deployment_id: String,
    database: String,
    domain_root_hex: String,
    super_root_hex: String,
    proof: zk_audit::merkle::InclusionProof,
) -> DomainSuperProofResult {
    use ark_ff::{BigInteger, PrimeField};
    DomainSuperProofResult {
        deployment_id,
        database,
        domain_root_hex,
        super_root_hex,
        position: proof.leaf_index,
        leaf_hex: hex::encode(proof.leaf.into_bigint().to_bytes_be()),
        path_elements: proof
            .path_elements
            .into_iter()
            .map(|f| hex::encode(f.into_bigint().to_bytes_be()))
            .collect(),
        path_indices: proof.path_indices,
    }
}

/// Generate an inclusion proof that a domain root is part of the aggregation
/// super-root over all per-domain roots.
#[tauri::command]
pub async fn audit_generate_domain_super_proof(
    state: State<'_, AppState>,
    deployment_id: String,
    database: String,
) -> AppResult<DomainSuperProofResult> {
    require_domain_access(&deployment_id, &database)?;
    let (proof, super_root_hex, domain_root_hex) = state
        .audit_log
        .prove_domain_in_super(&deployment_id, &database)?;
    Ok(super_proof_result_from(
        deployment_id,
        database,
        domain_root_hex,
        super_root_hex,
        proof,
    ))
}

/// Enable or disable a legal hold on a single audit domain. While held, the
/// domain cannot be pruned/retained.
#[tauri::command]
pub async fn audit_set_legal_hold(
    state: State<'_, AppState>,
    deployment_id: String,
    database: String,
    hold: bool,
) -> AppResult<()> {
    require_domain_access(&deployment_id, &database)?;
    state
        .audit_log
        .set_legal_hold(&deployment_id, &database, hold)?;
    Ok(())
}

/// Logically prune (retain-and-drop) a single audit domain's active events,
/// keeping a compact Merkle commitment to the pruned segment. Refused if the
/// domain is under legal hold. The global tree and on-chain anchor are
/// untouched. Returns the retained root, or `None` if the domain was empty.
#[tauri::command]
pub async fn audit_prune_domain(
    state: State<'_, AppState>,
    deployment_id: String,
    database: String,
) -> AppResult<Option<crate::audit::DomainRetentionRoot>> {
    require_domain_access(&deployment_id, &database)?;
    Ok(state.audit_log.prune_domain(&deployment_id, &database)?)
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

    let config = load_mode_config(&app)?;
    let chain = chain_config_from_mode(&config);

    let result = crate::audit::reader::verify_against_onchain(
        &events_jsonl,
        &local_root_hex,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await?;

    // Persist this verification run so the tamper timeline survives restarts.
    let run_at = chrono::Utc::now().timestamp_millis();
    state.verification_store.record(run_at, result.clone())?;

    Ok(result)
}

/// List all persisted verification runs (the tamper timeline), oldest first.
#[tauri::command]
pub async fn audit_list_verification_history(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::audit::verification_store::VerificationRecord>> {
    Ok(state.verification_store.list())
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
    let result =
        crate::audit::ipfs::publish_epoch_batch(&config, epoch_number, &batch_content).await?;

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
pub async fn audit_check_ipfs_daemon(api_url: Option<String>) -> AppResult<bool> {
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
    use crate::audit::stellar_native;

    let config = load_mode_config(&app)?;
    let chain = chain_config_from_mode(&config);

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
pub async fn audit_save_pinata_config(api_key: String, api_secret: String) -> AppResult<bool> {
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
pub async fn audit_test_pinata_connection(api_key: String, api_secret: String) -> AppResult<bool> {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditContractProvisionResult {
    pub account_id: String,
    pub contract_id: String,
    pub reused: bool,
    pub wasm_hash_hex: Option<String>,
    pub upload_tx_hash: Option<String>,
    pub create_tx_hash: Option<String>,
}

/// Provision a per-user testnet commitment contract owned by the in-app key.
#[tauri::command]
pub async fn audit_provision_testnet_contract(
    _state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<AuditContractProvisionResult> {
    use crate::audit::audit_mode::{
        load_mode_config, load_production_keypair, save_production_network, AuditMode, AuditNetwork,
    };
    use crate::audit::dev_setup::{
        generate_and_fund_account, load_keypair_from_keychain, save_keypair_to_keychain,
    };
    use tauri::Manager;

    let config = load_mode_config(&app)?;

    // The keypair that deploys and initializes the contract MUST be the same
    // one that later signs commits (`commit_root*` is admin-gated). Pick it by
    // mode so deploy, commit, and attest all use one identity:
    //   - Production: the user's imported `S…` key (funded on testnet so it can
    //     pay deploy fees; a no-op if already funded).
    //   - Dev: the app's local trial key (generated + funded on first use).
    let kp = match config.mode {
        AuditMode::Production => {
            let kp = load_production_keypair()?.ok_or_else(|| {
                AppError::Validation(
                    "no production keypair found — import your Stellar secret key in Audit \
                     Settings before provisioning a testnet contract"
                        .to_string(),
                )
            })?;
            if let Err(e) = stellar_native::fund_account(&kp.account_id()).await {
                // Friendbot rejects already-funded accounts; that is fine. A
                // genuinely unfunded account will surface a clear error at deploy.
                tracing::warn!("friendbot funding for {} skipped: {e}", kp.account_id());
            }
            kp
        }
        AuditMode::Dev => match load_keypair_from_keychain()? {
            Some(kp) => kp,
            None => {
                let kp = generate_and_fund_account().await?;
                save_keypair_to_keychain(&kp)?;
                kp
            }
        },
    };
    let account_id = kp.account_id();

    let existing_contract_id = config.testnet_contract_id.clone();
    if !existing_contract_id.trim().is_empty() {
        let chain = ChainConfig::testnet_with_contract(existing_contract_id.clone());
        match stellar_native::get_admin_native(&kp, &chain.rpc_url, &chain.contract_id).await {
            Ok(Some(admin)) if admin == account_id => {
                return Ok(AuditContractProvisionResult {
                    account_id,
                    contract_id: existing_contract_id,
                    reused: true,
                    wasm_hash_hex: None,
                    upload_tx_hash: None,
                    create_tx_hash: None,
                });
            }
            Ok(Some(admin)) => {
                return Err(AppError::Validation(format!(
                    "configured testnet contract {contract} is owned by {admin}, not this app key ({account}). Clear it or import the admin key.",
                    contract = existing_contract_id,
                    account = account_id,
                )));
            }
            Ok(None) => {
                tracing::warn!(
                    "configured testnet contract {} is not initialized; deploying a fresh contract",
                    existing_contract_id
                );
            }
            Err(e) => {
                tracing::warn!(
                    "could not validate configured testnet contract {}: {}; deploying a fresh contract",
                    existing_contract_id,
                    e
                );
            }
        }
    }

    let resource = app
        .path()
        .resolve(
            "resources/contract/zk_audit_commitment.wasm",
            tauri::path::BaseDirectory::Resource,
        )
        .map_err(|e| AppError::Validation(format!("resolve contract WASM resource: {e}")))?;
    let wasm = std::fs::read(&resource)
        .map_err(|e| AppError::Validation(format!("read contract WASM resource: {e}")))?;

    let chain = ChainConfig::testnet();
    let deployed = stellar_native::deploy_contract_native(
        &wasm,
        &kp,
        &chain.rpc_url,
        &chain.passphrase,
    )
    .await?;
    stellar_native::initialize_contract_native(
        &deployed.contract_id,
        &kp,
        &chain.rpc_url,
        &chain.passphrase,
    )
    .await?;

    save_production_network(
        &app,
        AuditNetwork::Testnet,
        deployed.contract_id.clone(),
        String::new(),
    )?;

    Ok(AuditContractProvisionResult {
        account_id,
        contract_id: deployed.contract_id,
        reused: false,
        wasm_hash_hex: Some(deployed.wasm_hash_hex),
        upload_tx_hash: Some(deployed.upload_tx_hash),
        create_tx_hash: Some(deployed.create_tx_hash),
    })
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

/// Shared commit path for the desktop app's dev and production modes.
///
/// This mirrors the daemon's commit flow (`auditd::mod::commit_epoch`): it
/// commits the most recent *sealed but uncommitted* epoch's frozen root, and
/// when a MongoDB connection is supplied it also computes and attaches the
/// epoch's oplog completeness hash, committing both on-chain via
/// `commit_root_with_oplog_native`. Without a MongoDB connection (or a sealed
/// epoch) it falls back to committing the root only.
///
/// Storing the oplog commitment here is what makes
/// `audit_verify_oplog_integrity` able to verify completeness — previously the
/// app never wrote an oplog commitment, so verification always reported
/// `no_oplog_commitment`.
fn compose_commit_metadata(
    metadata: Option<String>,
    event_count: usize,
    leaf_count: usize,
    network: &str,
    domain_super_root_hex: &str,
) -> String {
    let super_root_field = format!("domainSuperRoot={domain_super_root_hex}");
    match metadata.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()) {
        Some(meta) if meta.contains("domainSuperRoot=") => meta,
        Some(meta) => format!("{meta} {super_root_field}"),
        None => format!(
            "events={event_count} leaves={leaf_count} network={network} {super_root_field}"
        ),
    }
}

async fn commit_latest_epoch_root(
    state: &AppState,
    metadata: Option<String>,
    kp: &stellar_native::StellarKeypair,
    chain: &ChainConfig,
    connection_id: Option<String>,
) -> AppResult<CommitResult> {
    use crate::audit::oplog::{compute_oplog_range_hash, get_majority_commit_ts};

    let audit = &state.audit_log;

    // Preflight: `commit_root*` are admin-gated (`admin.require_auth()`). A
    // commit signed by a non-admin key passes simulation (Soroban records auth
    // without verifying signatures) but traps on-chain
    // (`INVOKE_HOST_FUNCTION_TRAPPED`), wasting a fee and surfacing an opaque
    // XDR. Check the contract admin up front and fail with an actionable
    // message. A read failure (e.g. RPC hiccup) must not block the commit, so
    // we only hard-fail on a confirmed mismatch.
    match stellar_native::get_admin_native(kp, &chain.rpc_url, &chain.contract_id).await {
        Ok(Some(admin)) if admin != kp.account_id() => {
            return Err(AppError::Validation(format!(
                "this Stellar key ({signer}) is not the admin of contract {contract} \
                 (admin is {admin}). Commits are owner-only, so the network would reject this \
                 on-chain. Deploy and initialize your own contract so this key becomes admin and \
                 set its contract ID, or import the admin key.",
                signer = kp.account_id(),
                contract = chain.contract_id,
            )));
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("commit preflight admin check could not complete (continuing): {e}");
        }
    }

    // Find the most recent sealed (closed), uncommitted epoch. The UI seals an
    // epoch before committing, so this is the epoch the user intends to commit.
    let target_epoch = state
        .epoch_manager
        .list_epochs()
        .into_iter()
        .filter(|e| !e.committed && e.end_index.is_some() && e.root_hex.is_some())
        .max_by_key(|e| e.epoch_number);

    // The root to commit: the frozen epoch root if available, otherwise the
    // live audit-log root (legacy behaviour when no epoch is sealed).
    let root_hex = match &target_epoch {
        Some(epoch) => epoch.root_hex.clone().expect("root_hex checked above"),
        None => {
            use ark_ff::{BigInteger, PrimeField};
            let root_bytes = audit.root()?.into_bigint().to_bytes_be();
            hex::encode(&root_bytes)
        }
    };

    // Guard against re-committing the same root.
    let rpc_client = crate::audit::stellar_rpc::StellarRpcClient::with_url(&chain.rpc_url);
    if let Some(entry) = rpc_client.get_current_root().await? {
        if entry.root_hex == root_hex {
            return Err(AppError::Validation(format!(
                "root 0x{}.. is already committed on-chain (seq #{}). New audit events are needed to produce a different root.",
                &root_hex[..root_hex.len().min(16)],
                entry.sequence
            )));
        }
    }

    let (domain_super_root_hex, _domains) = audit.domain_super_root()?;
    let meta = compose_commit_metadata(
        metadata,
        audit.event_count(),
        audit.leaf_count(),
        &chain.network,
        &domain_super_root_hex,
    );

    // If we have both a sealed epoch and a MongoDB connection, attach the
    // oplog completeness hash and commit it on-chain alongside the root.
    if let (Some(epoch), Some(conn_id)) = (target_epoch.as_ref(), connection_id.as_ref()) {
        let client = state.clients.get(conn_id).await?.client.clone();

        // Reuse an already-attached oplog hash if present; otherwise compute
        // the oplog range hash for this epoch and attach it.
        let epoch_with_oplog = if epoch.oplog_merkle_root_hex.is_some() {
            epoch.clone()
        } else {
            let start_ts = state.epoch_manager.next_oplog_start_ts();
            let majority_ts = get_majority_commit_ts(&client).await?;
            let range =
                compute_oplog_range_hash(&client, epoch.epoch_number, start_ts, majority_ts)
                    .await?;
            state.epoch_manager.attach_oplog_hash(
                epoch.epoch_number,
                range.start_ts,
                range.end_ts,
                range.entry_count,
                range.oplog_merkle_root_hex.clone(),
                range.majority_commit_ts,
            )?
        };

        let oplog_root_hex = epoch_with_oplog
            .oplog_merkle_root_hex
            .clone()
            .expect("oplog root attached above");
        let oplog_start = epoch_with_oplog
            .oplog_start_ts
            .map_or(0, |ts| ts.pack_u64());
        let oplog_end = epoch_with_oplog.oplog_end_ts.map_or(0, |ts| ts.pack_u64());
        let oplog_count = epoch_with_oplog.oplog_entry_count.unwrap_or(0);

        match stellar_native::commit_root_with_oplog_native(
            &root_hex,
            &oplog_root_hex,
            oplog_start,
            oplog_end,
            oplog_count,
            &meta,
            kp,
            &chain.rpc_url,
            &chain.contract_id,
            &chain.passphrase,
        )
        .await
        {
            Ok(result) => return Ok(result),
            // If the deployed contract is older than the oplog feature, don't
            // block the commit — fall back to a root-only commit so audit
            // anchoring keeps working. The verify path reports `contract_outdated`
            // to prompt a redeploy.
            Err(err) if stellar_native::is_contract_function_not_found_error(&err) => {
                tracing::warn!(
                    "commit: deployed contract lacks commit_root_with_oplog — committing root only (redeploy to enable oplog completeness)"
                );
            }
            Err(err) => return Err(AppError::from(err)),
        }
    }

    // No MongoDB connection (or no sealed epoch): commit the root only.
    let result = stellar_native::commit_root_native(
        &root_hex,
        &meta,
        kp,
        &chain.rpc_url,
        &chain.contract_id,
        &chain.passphrase,
    )
    .await?;
    Ok(result)
}

/// Commit a root to Stellar using native signing (no CLI subprocess).
///
/// This is the dev mode replacement for `audit_commit_root` that uses
/// the keypair from the OS keychain instead of the `stellar` CLI identity.
#[tauri::command]
pub async fn audit_commit_root_native(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    metadata: Option<String>,
    connection_id: Option<String>,
) -> AppResult<CommitResult> {
    let chain = chain_config(&app)?;

    // Load keypair from keychain.
    let kp = crate::audit::dev_setup::load_keypair_from_keychain()?.ok_or_else(|| {
        AppError::Validation("no Stellar keypair found — run onboarding first".to_string())
    })?;

    // Commit the sealed epoch root (with oplog completeness when a MongoDB
    // connection is available) via native signing.
    commit_latest_epoch_root(&state, metadata, &kp, &chain, connection_id).await
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
    connection_id: Option<String>,
) -> AppResult<CommitResult> {
    use crate::audit::audit_mode::{load_mode_config, load_production_keypair, AuditNetwork};

    // Load mode config to pick the network + contract/rpc.
    let config = load_mode_config(&app)?;

    if config.network == AuditNetwork::Mainnet && config.mainnet_contract_id.is_empty() {
        return Err(AppError::Validation(
            "mainnet contract ID is not configured — set it in Audit Settings".to_string(),
        ));
    }
    let chain = chain_config_from_mode(&config);

    // Load the production keypair from the keychain.
    let kp = load_production_keypair()?.ok_or_else(|| {
        AppError::Validation(
            "no production keypair found — import your Stellar secret key in Audit Settings"
                .to_string(),
        )
    })?;

    // Commit the sealed epoch root (with oplog completeness when a MongoDB
    // connection is available).
    commit_latest_epoch_root(&state, metadata, &kp, &chain, connection_id).await
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
    let pinata_config = crate::audit::dev_setup::load_pinata_from_keychain()?.ok_or_else(|| {
        AppError::Validation("no Pinata config found — run onboarding first".to_string())
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
pub async fn audit_get_attestation_threshold(state: State<'_, AppState>) -> AppResult<usize> {
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
    Ok(state
        .attestation_manager
        .get_status(epoch_number, &root_hex)?)
}

// ─── On-chain (independent) attestation commands ──────────────────────
//
// These wrap the Soroban contract directly. Unlike the off-chain
// `attestation_manager` (operator-held), the trust here comes from the
// contract: only keys the admin authorized can attest, signatures are
// ed25519-verified on-chain, and `verify_attestation` enforces the K-of-N
// threshold. The operator never holds an attester's secret key — auditors run
// their own attester and submit `attest_oplog` from their own account.

/// Load the mode-aware operator keypair (admin of the on-chain contract).
fn load_operator_keypair(
    app: &tauri::AppHandle,
) -> AppResult<stellar_native::StellarKeypair> {
    use crate::audit::audit_mode::{load_mode_config, load_production_keypair, AuditMode};
    use crate::audit::dev_setup::load_keypair_from_keychain;
    let config = load_mode_config(app)?;
    match config.mode {
        AuditMode::Production => load_production_keypair()?.ok_or_else(|| {
            AppError::Validation(
                "no production keypair found — import your Stellar secret key in Audit Settings"
                    .to_string(),
            )
        }),
        AuditMode::Dev => load_keypair_from_keychain()?.ok_or_else(|| {
            AppError::Validation("no dev keypair found — start the audit trial first".to_string())
        }),
    }
}

/// Authorize an external auditor's attester key on-chain (admin only).
///
/// `stellar_address` is the auditor's `G...` account; `ed25519_pubkey_hex` is
/// the 32-byte ed25519 public key (64 hex chars) that signs oplog attestations.
/// The auditor keeps both secrets; the operator only ever receives public
/// material. This is the F1 key-separation step.
#[tauri::command]
pub async fn audit_authorize_onchain_attester(
    app: tauri::AppHandle,
    stellar_address: String,
    ed25519_pubkey_hex: String,
) -> AppResult<()> {
    let chain = chain_config(&app)?;
    let kp = load_operator_keypair(&app)?;
    stellar_native::authorize_attester_native(
        &chain.contract_id,
        &kp,
        stellar_address.trim(),
        ed25519_pubkey_hex.trim(),
        &chain.rpc_url,
        &chain.passphrase,
    )
    .await?;
    Ok(())
}

/// Revoke a previously-authorized attester on-chain (admin only).
#[tauri::command]
pub async fn audit_revoke_onchain_attester(
    app: tauri::AppHandle,
    stellar_address: String,
) -> AppResult<()> {
    let chain = chain_config(&app)?;
    let kp = load_operator_keypair(&app)?;
    stellar_native::revoke_attester_native(
        &chain.contract_id,
        &kp,
        stellar_address.trim(),
        &chain.rpc_url,
        &chain.passphrase,
    )
    .await?;
    Ok(())
}

/// Set the on-chain K-of-N attestation threshold (admin only).
#[tauri::command]
pub async fn audit_set_onchain_threshold(
    app: tauri::AppHandle,
    threshold: u32,
) -> AppResult<()> {
    let chain = chain_config(&app)?;
    let kp = load_operator_keypair(&app)?;
    stellar_native::set_threshold_native(
        &chain.contract_id,
        &kp,
        threshold,
        &chain.rpc_url,
        &chain.passphrase,
    )
    .await?;
    Ok(())
}

/// Read the on-chain K-of-N attestation threshold.
#[tauri::command]
pub async fn audit_get_onchain_threshold(app: tauri::AppHandle) -> AppResult<u32> {
    let chain = chain_config(&app)?;
    let kp = load_operator_keypair(&app)?;
    Ok(stellar_native::get_threshold_native(&kp, &chain.rpc_url, &chain.contract_id).await?)
}

/// Query the contract's independent attestation verdict for a sequence.
///
/// This is the trust anchor for the UI: `verified` means K distinct authorized
/// attesters each signed the exact committed oplog root. The operator's own key
/// cannot produce this verdict.
#[tauri::command]
pub async fn audit_verify_onchain_attestation(
    app: tauri::AppHandle,
    sequence: u64,
) -> AppResult<crate::audit::stellar::OnChainAttestationVerification> {
    let chain = chain_config(&app)?;
    let kp = load_operator_keypair(&app)?;
    Ok(stellar_native::verify_attestation_native(
        &kp,
        &chain.rpc_url,
        &chain.contract_id,
        sequence,
    )
    .await?)
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
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<OplogIntegrityReport> {
    use crate::audit::oplog::{compute_oplog_range_hash, OplogTimestamp};

    let chain = chain_config(&app)?;

    // 1. Get the MongoDB client from the connection registry.
    let entry = state.clients.get(&connection_id).await?;
    let client = entry.client.clone();

    // 2. Get the latest on-chain root.
    let rpc_client = crate::audit::stellar_rpc::StellarRpcClient::with_url(&chain.rpc_url);
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

    // 3. Get the on-chain oplog commitment via native contract invocation.
    let probe_kp = stellar_native::generate_keypair();
    let on_chain_oplog = match stellar_native::get_oplog_commitment_native(
        sequence,
        &probe_kp,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await
    {
        Ok(commitment) => commitment,
        Err(err) if stellar_native::is_contract_function_not_found_error(&err) => {
            return Ok(OplogIntegrityReport {
                sequence,
                on_chain_oplog_root: "none".to_string(),
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                verdict: "contract_outdated".to_string(),
                explanation: format!(
                    "The deployed Soroban contract does not expose get_oplog_commitment, \
                    so oplog completeness cannot be verified for epoch {sequence}. \
                    Redeploy the current Soroban contract and update the contract ID in settings."
                ),
                alerts: vec![format!(
                    "Contract {} is missing get_oplog_commitment",
                    chain.contract_id
                )],
            });
        }
        Err(err) => return Err(AppError::from(err)),
    };

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
    app: tauri::AppHandle,
    sequence: u64,
) -> AppResult<Option<crate::audit::stellar::OnChainOplogCommitment>> {
    let chain = chain_config(&app)?;
    let kp = stellar_native::generate_keypair();
    Ok(stellar_native::get_oplog_commitment_native(
        sequence,
        &kp,
        &chain.rpc_url,
        &chain.contract_id,
    )
    .await?)
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
            domains: vec![AuditDomain {
                deployment_id: "rs:rs0".to_string(),
                database: "db".to_string(),
                event_count: 2,
            }],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(
            json.contains("\"rootHex\":\"abc\""),
            "rootHex must be camelCase: {json}"
        );
        assert!(
            json.contains("\"leafCount\":1"),
            "leafCount must be camelCase: {json}"
        );
        assert!(
            json.contains("\"eventCount\":2"),
            "eventCount must be camelCase: {json}"
        );
        assert!(
            json.contains("\"treeHeight\":20"),
            "treeHeight must be camelCase: {json}"
        );
        assert!(
            json.contains("\"deploymentId\":\"rs:rs0\""),
            "domain deploymentId must be camelCase: {json}"
        );
    }

    #[test]
    fn audit_event_filters_are_server_side_domain_aware() {
        let events = vec![
            audit_event(0, "rs:rs0", "sales"),
            audit_event(1, "rs:rs0", "billing"),
            audit_event(2, "rs:rs1", "sales"),
        ];

        let filtered = filter_events(events.clone(), Some("rs:rs0"), Some("sales"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].deployment_id, "rs:rs0");
        assert_eq!(filtered[0].database, "sales");

        let filtered = filter_events(events.clone(), None, Some("sales"));
        assert_eq!(filtered.len(), 2);

        let filtered = filter_events(events, Some("rs:rs0"), None);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn audit_domain_summary_counts_each_deployment_database_pair() {
        let domains = summarize_domains(&[
            audit_event(0, "rs:rs0", "sales"),
            audit_event(1, "rs:rs0", "sales"),
            audit_event(2, "rs:rs1", "sales"),
        ]);
        assert_eq!(
            domains,
            vec![
                AuditDomain {
                    deployment_id: "rs:rs0".to_string(),
                    database: "sales".to_string(),
                    event_count: 2,
                },
                AuditDomain {
                    deployment_id: "rs:rs1".to_string(),
                    database: "sales".to_string(),
                    event_count: 1,
                },
            ]
        );
    }

    fn audit_event(index: u64, deployment_id: &str, database: &str) -> crate::audit::AuditEvent {
        crate::audit::AuditEvent {
            index,
            leaf_hex: format!("leaf-{index}"),
            operation: "insert".to_string(),
            database: database.to_string(),
            collection: "orders".to_string(),
            deployment_id: deployment_id.to_string(),
            sequence: index,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        }
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
        assert!(
            json.contains("\"rootHex\":\"root\""),
            "rootHex must be camelCase: {json}"
        );
        assert!(
            json.contains("\"leafIndex\":42"),
            "leafIndex must be camelCase: {json}"
        );
        assert!(
            json.contains("\"pubSignals\":[\"sig\"]"),
            "pubSignals must be camelCase: {json}"
        );
        assert!(
            json.contains("\"network\":\"testnet\""),
            "network must be camelCase: {json}"
        );
        assert!(
            json.contains("\"contractId\":\"C123\""),
            "contractId must be camelCase: {json}"
        );
        assert!(
            json.contains("\"txHash\":\"txabc\""),
            "txHash must be camelCase: {json}"
        );
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
        assert_eq!(
            r1cs,
            "/tmp/fake-app/resources/circuits/merkle_inclusion.r1cs"
        );
        assert_eq!(
            wasm,
            "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm"
        );
    }

    #[test]
    fn resolve_circuit_paths_pure_ignores_empty_strings() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) = resolve_circuit_paths_pure(Some(""), Some(""), dir).unwrap();
        assert_eq!(
            r1cs,
            "/tmp/fake-app/resources/circuits/merkle_inclusion.r1cs"
        );
        assert_eq!(
            wasm,
            "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm"
        );
    }

    #[test]
    fn resolve_circuit_paths_pure_mixed_explicit_and_fallback() {
        let dir = std::path::Path::new("/tmp/fake-app");
        let (r1cs, wasm) =
            resolve_circuit_paths_pure(Some("/explicit/circuit.r1cs"), None, dir).unwrap();
        assert_eq!(r1cs, "/explicit/circuit.r1cs");
        assert_eq!(
            wasm,
            "/tmp/fake-app/resources/circuits/merkle_inclusion.wasm"
        );
    }

    #[test]
    fn bundled_circuit_artifacts_exist_on_disk() {
        // Verify the artifacts were actually copied to the resources dir.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let r1cs =
            std::path::Path::new(manifest_dir).join("resources/circuits/merkle_inclusion.r1cs");
        let wasm =
            std::path::Path::new(manifest_dir).join("resources/circuits/merkle_inclusion.wasm");
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

    #[test]
    fn require_domain_access_rejects_empty_database() {
        assert!(require_domain_access("rs:rs0", "db").is_ok());
        assert!(require_domain_access("", "db").is_ok(), "unattributed domain allowed");
        assert!(require_domain_access("rs:rs0", "").is_err(), "empty db refused");
        assert!(require_domain_access("rs:rs0", "   ").is_err(), "blank db refused");
    }

    #[test]
    fn domain_root_info_serializes_with_camel_case() {
        let info = DomainRootInfo {
            deployment_id: "rs:rs0".to_string(),
            database: "sales".to_string(),
            root_hex: "abc".to_string(),
            event_count: 5,
            legal_hold: true,
            retained_roots: vec![],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"deploymentId\":\"rs:rs0\""));
        assert!(json.contains("\"rootHex\":\"abc\""));
        assert!(json.contains("\"eventCount\":5"));
        assert!(json.contains("\"legalHold\":true"));
        assert!(json.contains("\"retainedRoots\":[]"));
    }

    #[test]
    fn domain_proof_result_serializes_with_camel_case() {
        let res = DomainProofResult {
            deployment_id: "rs:rs0".to_string(),
            database: "sales".to_string(),
            position: 1,
            leaf_hex: "leaf".to_string(),
            root_hex: "root".to_string(),
            path_elements: vec!["a".to_string()],
            path_indices: vec![0],
        };
        let json = serde_json::to_string(&res).unwrap();
        assert!(json.contains("\"deploymentId\":\"rs:rs0\""));
        assert!(json.contains("\"leafHex\":\"leaf\""));
        assert!(json.contains("\"rootHex\":\"root\""));
        assert!(json.contains("\"pathElements\":[\"a\"]"));
        assert!(json.contains("\"pathIndices\":[0]"));
    }

    #[test]
    fn domain_super_results_serialize_with_camel_case() {
        let domain = DomainRootInfo {
            deployment_id: "rs:rs0".to_string(),
            database: "sales".to_string(),
            root_hex: "domain-root".to_string(),
            event_count: 2,
            legal_hold: false,
            retained_roots: vec![],
        };
        let root = DomainSuperRootResult {
            super_root_hex: "super-root".to_string(),
            domains: vec![domain],
        };
        let root_json = serde_json::to_string(&root).unwrap();
        assert!(root_json.contains("\"superRootHex\":\"super-root\""));
        assert!(root_json.contains("\"deploymentId\":\"rs:rs0\""));
        assert!(root_json.contains("\"rootHex\":\"domain-root\""));

        let proof = DomainSuperProofResult {
            deployment_id: "rs:rs0".to_string(),
            database: "sales".to_string(),
            domain_root_hex: "domain-root".to_string(),
            super_root_hex: "super-root".to_string(),
            position: 0,
            leaf_hex: "leaf".to_string(),
            path_elements: vec!["a".to_string()],
            path_indices: vec![0],
        };
        let proof_json = serde_json::to_string(&proof).unwrap();
        assert!(proof_json.contains("\"domainRootHex\":\"domain-root\""));
        assert!(proof_json.contains("\"superRootHex\":\"super-root\""));
        assert!(proof_json.contains("\"leafHex\":\"leaf\""));
        assert!(proof_json.contains("\"pathElements\":[\"a\"]"));
    }

    #[test]
    fn compose_commit_metadata_anchors_domain_super_root() {
        let generated = compose_commit_metadata(None, 3, 4, "testnet", "abc123");
        assert_eq!(
            generated,
            "events=3 leaves=4 network=testnet domainSuperRoot=abc123"
        );

        let custom = compose_commit_metadata(Some("manual commit".to_string()), 3, 4, "testnet", "abc123");
        assert_eq!(custom, "manual commit domainSuperRoot=abc123");

        let already_tagged = compose_commit_metadata(
            Some("manual domainSuperRoot=existing".to_string()),
            3,
            4,
            "testnet",
            "abc123",
        );
        assert_eq!(already_tagged, "manual domainSuperRoot=existing");
    }

    /// The super-proof wire result must losslessly encode a real proof: the
    /// hex-encoded leaf/path round-trip back to field elements and the
    /// reconstructed proof verifies against the super-root the command reports.
    #[test]
    fn super_proof_result_round_trips_and_verifies() {
        use ark_ff::PrimeField;
        use std::sync::Arc;
        use zk_audit::prover::Fr;

        let audit = Arc::new(audit_service::audit::AuditLog::new().unwrap());
        audit_service::audit::interceptor::record_insert(
            &audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#,
        )
        .unwrap();
        audit_service::audit::interceptor::record_insert(
            &audit, "rs:rs0", "billing", "invoices", r#"{"a":1}"#,
        )
        .unwrap();
        audit_service::audit::interceptor::record_insert(
            &audit, "rs:rs1", "ops", "logs", r#"{"a":1}"#,
        )
        .unwrap();

        let (proof, super_root_hex, domain_root_hex) =
            audit.prove_domain_in_super("rs:rs0", "billing").unwrap();
        let expected_position = proof.leaf_index;

        let result = super_proof_result_from(
            "rs:rs0".to_string(),
            "billing".to_string(),
            domain_root_hex.clone(),
            super_root_hex.clone(),
            proof,
        );

        assert_eq!(result.deployment_id, "rs:rs0");
        assert_eq!(result.database, "billing");
        assert_eq!(result.domain_root_hex, domain_root_hex);
        assert_eq!(result.super_root_hex, super_root_hex);
        assert_eq!(result.position, expected_position);
        assert_eq!(result.path_elements.len(), result.path_indices.len());

        // Reconstruct the proof from the wire fields and verify it terminates
        // at the reported super-root — proving the encoding is lossless and the
        // proof is cryptographically sound through the command boundary.
        let to_fr = |h: &str| Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap());
        let rebuilt = zk_audit::merkle::InclusionProof {
            leaf: to_fr(&result.leaf_hex),
            leaf_index: result.position,
            path_elements: result.path_elements.iter().map(|h| to_fr(h)).collect(),
            path_indices: result.path_indices.clone(),
            root: to_fr(&result.super_root_hex),
        };
        assert!(
            rebuilt.verify().unwrap(),
            "reconstructed super-proof must verify against the super-root"
        );
    }
}
