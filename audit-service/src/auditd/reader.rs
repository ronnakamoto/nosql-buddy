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
use crate::audit::stellar_native;
use crate::error::AuditError;

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
        std::fs::read_to_string(&events_path).map_err(AuditError::from).map_err(ApiError::from)?
    } else {
        String::new()
    };

    // Query on-chain root via contract simulation.
    let kp = stellar_native::generate_keypair();
    let onchain_root = stellar_native::get_current_root_native(
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
    )
    .await
    .map_err(ApiError::from)?;

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
    .map_err(|e| AuditError::Validation(format!("verify task join: {e}")))
    .map_err(ApiError::from)??;

    Ok(Json(report))
}

/// Get the latest on-chain root via contract simulation.
pub async fn onchain_root(
    state: State<Arc<DaemonState>>,
) -> ApiResult<Option<OnChainRoot>> {
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

/// Rebuild the local audit log from on-chain commitments + IPFS batches.
///
/// 1. Queries the on-chain root history from Stellar.
/// 2. For each committed root, parses the metadata to extract the IPFS CID.
/// 3. Fetches the batch bytes from IPFS (Pinata preferred, local daemon fallback).
/// 4. Decrypts the batch with the auditor's age identity if available.
/// 5. Replays all batches into the local Merkle tree.
/// 6. Verifies the final root matches the latest on-chain commitment.
///
/// Returns a verification report showing how many events were rebuilt.
pub async fn rebuild(
    state: State<Arc<DaemonState>>,
) -> ApiResult<VerificationReport> {
    let local_root_hex = state.audit_log.root_hex().map_err(ApiError::from)?;

    // Query on-chain root history.
    let kp = stellar_native::generate_keypair();
    let history = stellar_native::get_root_history_native(
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
        100,
    )
    .await
    .map_err(ApiError::from)?;

    if history.is_empty() {
        return Ok(Json(VerificationReport {
            onchain_root_found: false,
            onchain_root: None,
            local_root_hex,
            commitment_event_index: None,
            total_events: 0,
            verified_events: 0,
            events_after_commitment: 0,
            chain_intact: true,
            tamper_detected: false,
            summary: "No on-chain roots committed yet. Nothing to rebuild.".to_string(),
        }));
    }

    // Gather all batch JSONL lines from IPFS.
    let mut all_lines: Vec<String> = Vec::new();
    let mut fetched_batches = 0usize;
    let mut failed_batches = 0usize;

    for root in &history {
        let Some(cid) = extract_cid_from_metadata(&root.metadata) else {
            continue;
        };

        let raw_bytes = match fetch_cid(&state, &cid).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("rebuild: failed to fetch CID {} for seq {}: {e}", cid, root.sequence);
                failed_batches += 1;
                continue;
            }
        };

        let decrypted = if let Some(identity) = &state.age_attester_identity {
            match crate::audit::crypto::decrypt_batch(&raw_bytes, identity) {
                Ok(plaintext) => {
                    log::info!("rebuild: decrypted batch {} (seq {})", cid, root.sequence);
                    plaintext.into_bytes()
                }
                Err(e) => {
                    log::warn!(
                        "rebuild: age decrypt failed for CID {} (seq {}): {e} — treating as plaintext",
                        cid,
                        root.sequence
                    );
                    raw_bytes
                }
            }
        } else {
            String::from_utf8(raw_bytes).unwrap_or_default().into_bytes()
        };

        let text = match String::from_utf8(decrypted) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("rebuild: CID {} (seq {}) is not valid UTF-8: {e}", cid, root.sequence);
                failed_batches += 1;
                continue;
            }
        };

        for line in text.lines() {
            let line = line.trim();
            if !line.is_empty() {
                all_lines.push(line.to_string());
            }
        }
        fetched_batches += 1;
    }

    if all_lines.is_empty() {
        return Ok(Json(VerificationReport {
            onchain_root_found: false,
            onchain_root: history.last().cloned(),
            local_root_hex,
            commitment_event_index: None,
            total_events: 0,
            verified_events: 0,
            events_after_commitment: 0,
            chain_intact: true,
            tamper_detected: false,
            summary: format!(
                "Found {} on-chain root(s), but no batches could be fetched/decrypted.",
                history.len()
            ),
        }));
    }

    // Write the reconstructed JSONL to disk and replay.
    let events_path = state.data_dir.join("audit").join("events.jsonl");
    let jsonl_content = all_lines.join("\n") + "\n";

    // Clear local state (tree, events, sled) before replay.
    state
        .audit_log
        .clear()
        .map_err(ApiError::from)?;

    std::fs::write(&events_path, &jsonl_content)
        .map_err(|e| ApiError(AuditError::Io(format!("write events.jsonl: {e}"))))?;

    state
        .audit_log
        .set_persistence_dir(&state.data_dir)
        .map_err(ApiError::from)?;

    let rebuilt_root_hex = state.audit_log.root_hex().map_err(ApiError::from)?;
    let total_events = state.audit_log.event_count() as u64;

    let latest_onchain = history.last().cloned();
    let root_matches = latest_onchain
        .as_ref()
        .map_or(false, |r| r.root_hex == rebuilt_root_hex);

    let summary = if root_matches {
        format!(
            "Rebuilt {} event(s) from {} batch(es). Final root matches on-chain commitment (seq {}).",
            total_events,
            fetched_batches,
            latest_onchain.as_ref().unwrap().sequence
        )
    } else {
        format!(
            "Rebuilt {} event(s) from {} batch(es). Root mismatch: local {} vs on-chain {} (seq {}). {} batch(es) failed to fetch.",
            total_events,
            fetched_batches,
            &rebuilt_root_hex[..rebuilt_root_hex.len().min(16)],
            latest_onchain.as_ref().map_or("?".to_string(), |r| r.root_hex.clone()),
            latest_onchain.as_ref().map_or(0, |r| r.sequence),
            failed_batches
        )
    };

    Ok(Json(VerificationReport {
        onchain_root_found: true,
        onchain_root: latest_onchain,
        local_root_hex: rebuilt_root_hex,
        commitment_event_index: Some(total_events.saturating_sub(1)),
        total_events,
        verified_events: total_events,
        events_after_commitment: 0,
        chain_intact: root_matches,
        tamper_detected: !root_matches,
        summary,
    }))
}

/// Fetch raw bytes for a CID, preferring Pinata when configured.
async fn fetch_cid(state: &DaemonState, cid: &str) -> Result<Vec<u8>, AuditError> {
    if let Some(pinata) = &state.pinata_config {
        if !pinata.api_key.is_empty() || !pinata.api_secret.is_empty() {
            return crate::audit::pinata::fetch_batch(pinata, cid).await;
        }
    }
    crate::audit::ipfs::fetch_batch(&state.ipfs_config, cid).await
}

/// Extract an IPFS CID from on-chain metadata string.
///
/// Metadata format: `epoch=N cid=CID ...` or `epoch=N` (no CID).
fn extract_cid_from_metadata(metadata: &str) -> Option<String> {
    for part in metadata.split_whitespace() {
        if let Some(val) = part.strip_prefix("cid=") {
            return Some(val.to_string());
        }
    }
    None
}

// ─── Oplog integrity verification (three-way compare) ─────────────────

/// Result of the three-way oplog integrity verification.
///
/// This is the auditor's verification tool. It compares three values:
/// 1. The on-chain oplog root (committed by the operator).
/// 2. The operator's oplog root (computed from the operator's replica).
/// 3. The auditor's oplog root (computed from the independent replica).
///
/// If all three match, the audit log is complete — no writes were omitted.
/// If (1) != (2), the operator committed a different hash than what their
/// own oplog contains (operator fraud).
/// If (1) != (3), the auditor's independent observation differs from the
/// on-chain commitment (omission or tampering).
/// If (2) != (3), the operator's and auditor's replicas disagree (replication
/// issue or the operator is serving a doctored oplog to the auditor).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogIntegrityReport {
    /// The on-chain sequence number being verified.
    pub sequence: u64,
    /// The on-chain oplog root (from the Soroban contract).
    pub on_chain_oplog_root: String,
    /// The oplog root computed from the operator's replica (if available).
    pub operator_oplog_root: Option<String>,
    /// The oplog root computed from the auditor's independent replica.
    pub auditor_oplog_root: Option<String>,
    /// Number of oplog entries in the range (from the auditor's computation).
    pub oplog_entry_count: Option<u64>,
    /// Whether every comparison actually performed passed. In reader mode
    /// the auditor connects to its own independent member and does NOT query
    /// the operator's live replica, so this reflects the on-chain (operator's
    /// committed) root vs the auditor's independent recomputation. It is not a
    /// three-way compare unless `operator_oplog_root` is populated.
    pub all_match: bool,
    /// Whether the on-chain root matches the auditor's root.
    pub on_chain_matches_auditor: bool,
    /// Whether the operator's *live* root matches the auditor's root. Only
    /// meaningful when `operator_oplog_root` is populated; in reader mode the
    /// operator's live root is not queried, so this is `false` (not computed).
    pub operator_matches_auditor: bool,
    /// Overall verdict: "complete", "mismatch", or "incomplete".
    pub verdict: String,
    /// Detailed explanation of the verdict.
    pub explanation: String,
    /// Any alerts raised during verification.
    pub alerts: Vec<String>,
}

/// GET /reader/verify-oplog — Verify oplog integrity via three-way compare.
///
/// This is the auditor's primary verification tool. It:
/// 1. Gets the latest on-chain oplog commitment from the contract.
/// 2. Independently reads the oplog from the connected replica member
///    (the auditor connects to their own independent member).
/// 3. Computes the oplog hash using the canonical serialization.
/// 4. Compares the on-chain root with the auditor's computed root.
///
/// If a MongoDB client is available in the daemon state, it uses that.
/// Otherwise, it only reports the on-chain root (degraded mode).
pub async fn verify_oplog(
    state: State<Arc<DaemonState>>,
) -> ApiResult<OplogIntegrityReport> {
    // 1. Get the on-chain root to find the latest sequence.
    let kp = stellar_native::generate_keypair();
    let onchain_root = stellar_native::get_current_root_native(
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
    )
    .await
    .map_err(ApiError::from)?;

    let sequence = match onchain_root {
        Some(ref root) => root.sequence,
        None => {
            return Ok(Json(OplogIntegrityReport {
                sequence: 0,
                on_chain_oplog_root: "none".to_string(),
                operator_oplog_root: None,
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                operator_matches_auditor: false,
                verdict: "no_commitment".to_string(),
                explanation: "No on-chain root has been committed yet.".to_string(),
                alerts: vec![],
            }));
        }
    };

    // 2. Get the on-chain oplog commitment via native simulation.
    //    Reuse the same ephemeral keypair (read-only simulation needs no funding).
    let on_chain_oplog = crate::audit::stellar_native::get_oplog_commitment_native(
        sequence,
        &kp,
        &state.rpc_url,
        &state.chain.contract_id,
    )
    .await
    .map_err(ApiError::from)?;

    let on_chain_oplog_root = match on_chain_oplog {
        Some(ref oc) => oc.oplog_root_hex.clone(),
        None => {
            return Ok(Json(OplogIntegrityReport {
                sequence,
                on_chain_oplog_root: "none".to_string(),
                operator_oplog_root: None,
                auditor_oplog_root: None,
                oplog_entry_count: None,
                all_match: false,
                on_chain_matches_auditor: false,
                operator_matches_auditor: false,
                verdict: "no_oplog_commitment".to_string(),
                explanation: format!(
                    "Epoch {sequence} was committed without an oplog hash. \
                    Completeness cannot be verified."
                ),
                alerts: vec![format!(
                    "Epoch {sequence} has no oplog commitment — completeness not guaranteed"
                )],
            }));
        }
    };

    // 3. If we have a MongoDB client, independently compute the oplog hash.
    let (auditor_oplog_root, oplog_entry_count) = if let Some(client) = &state.mongo_client {
        let on_chain_ref = on_chain_oplog.as_ref().unwrap();
        let start_ts = crate::audit::oplog::OplogTimestamp::unpack_u64(on_chain_ref.oplog_start_ts);
        let end_ts = crate::audit::oplog::OplogTimestamp::unpack_u64(on_chain_ref.oplog_end_ts);
        let on_chain_entry_count = on_chain_ref.oplog_entry_count;

        match crate::audit::oplog::compute_oplog_range_hash(
            client,
            sequence,
            start_ts,
            end_ts,
        ).await {
            Ok(range) => {
                // Detect oplog rollover: the on-chain commitment says there
                // were entries, but we found none. The oplog has rolled over
                // and the entries are no longer available. This is not an
                // error — the on-chain attestation is the durable guarantee.
                if range.entry_count == 0 && on_chain_entry_count > 0 {
                    return Ok(Json(OplogIntegrityReport {
                        sequence,
                        on_chain_oplog_root,
                        operator_oplog_root: None,
                        auditor_oplog_root: None,
                        oplog_entry_count: Some(0),
                        all_match: false,
                        on_chain_matches_auditor: false,
                        operator_matches_auditor: false,
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
                    }));
                }
                (Some(range.oplog_merkle_root_hex), Some(range.entry_count))
            }
            Err(e) => {
                let err_msg = format!("{e}");
                // Distinguish "stale" (oplog entries rolled over) from
                // generic errors. The most common stale signal is a MongoDB
                // query error when the oplog range is entirely beyond the
                // capped collection's current window.
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
                return Ok(Json(OplogIntegrityReport {
                    sequence,
                    on_chain_oplog_root,
                    operator_oplog_root: None,
                    auditor_oplog_root: None,
                    oplog_entry_count: None,
                    all_match: false,
                    on_chain_matches_auditor: false,
                    operator_matches_auditor: false,
                    verdict: verdict.to_string(),
                    explanation,
                    alerts: vec![format!("Oplog verification: {err_msg}")],
                }));
            }
        }
    } else {
        (None, None)
    };

    // 4. Three-way compare and build the verdict.
    let on_chain_matches_auditor = match &auditor_oplog_root {
        Some(auditor) => auditor == &on_chain_oplog_root,
        None => false,
    };

    // The operator's *live* root is not directly available in reader mode —
    // the reader connects to the auditor's independent member. The operator's
    // committed claim is the on-chain root, which we verify against the
    // auditor's independent recomputation. `operator_oplog_root` stays None to
    // make explicit that it was not independently recomputed here (a full
    // deployment would expose it via the publisher API for a true 3-way compare).
    let operator_oplog_root: Option<String> = None;
    let operator_matches_auditor = match (&operator_oplog_root, &auditor_oplog_root) {
        (Some(op), Some(aud)) => op == aud,
        _ => false,
    };

    // Report `all_match` as the comparison actually performed (on-chain vs
    // auditor). Reporting a three-way AND with an un-computed operator root
    // made `all_match` permanently false even on a verified `complete` verdict,
    // which misled anyone reading the raw report.
    let all_match = on_chain_matches_auditor;

    let (verdict, explanation, alerts) = if auditor_oplog_root.is_none() {
        (
            "incomplete".to_string(),
            "No MongoDB connection — could not independently verify the oplog hash. \
            Only the on-chain commitment is reported.".to_string(),
            vec!["No MongoDB connection available for independent verification".to_string()],
        )
    } else if on_chain_matches_auditor {
        (
            "complete".to_string(),
            format!(
                "Oplog integrity verified: on-chain root matches auditor's independent computation. \
                {} oplog entries in the range.",
                oplog_entry_count.unwrap_or(0)
            ),
            vec![],
        )
    } else {
        let auditor_root = auditor_oplog_root.as_ref().unwrap();
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

    Ok(Json(OplogIntegrityReport {
        sequence,
        on_chain_oplog_root,
        operator_oplog_root,
        auditor_oplog_root,
        oplog_entry_count,
        all_match,
        on_chain_matches_auditor,
        operator_matches_auditor,
        verdict,
        explanation,
        alerts,
    }))
}

