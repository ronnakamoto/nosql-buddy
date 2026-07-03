//! Reader mode — verify the local audit log against on-chain roots.
//!
//! Reader mode is a read-only verification mode that checks the local
//! audit log's integrity against roots committed to the Stellar
//! blockchain. It does NOT record new events — it only reads and
//! verifies.
//!
//! ## Verification flow
//!
//! 1. Query the latest committed root from the Soroban contract.
//! 2. Search the local JSONL log for the event whose `root_after`
//!    matches the on-chain root. This is the "commitment point" —
//!    the event that was the last one included when the root was
//!    committed.
//! 3. Verify the root chain from the beginning of the log up to the
//!    commitment point: recompute each leaf and root, assert they
//!    match the stored values.
//! 4. Report the result: how many events are verified, how many
//!    events were added after the commitment, and whether any
//!    tamper was detected.
//!
//! ## Tamper detection
//!
//! - If the on-chain root is not found in the local log, either the
//!   log was truncated (events deleted after the commitment) or the
//!   on-chain root doesn't belong to this log.
//! - If the root chain verification fails at any event before the
//!   commitment point, an event was modified, reordered, or inserted.

use serde::{Deserialize, Serialize};

use crate::audit::stellar::OnChainRoot;
use crate::audit::stellar_native;
use crate::error::AuditResult;

/// The result of a reader-mode verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationReport {
    /// Whether the on-chain root was found in the local log.
    pub onchain_root_found: bool,
    /// The on-chain root that was checked (if any).
    pub onchain_root: Option<OnChainRoot>,
    /// The current local Merkle root (hex).
    pub local_root_hex: String,
    /// The event index at which the on-chain root was found
    /// (the "commitment point"). `None` if not found.
    pub commitment_event_index: Option<u64>,
    /// Total number of events in the local log.
    pub total_events: u64,
    /// Number of events verified up to the commitment point.
    pub verified_events: u64,
    /// Number of events added after the commitment point.
    pub events_after_commitment: u64,
    /// Whether the root chain is intact up to the commitment point.
    pub chain_intact: bool,
    /// Whether any tamper was detected.
    pub tamper_detected: bool,
    /// Human-readable summary of the verification result.
    pub summary: String,
}

/// Verify the local audit log against the latest on-chain root.
///
/// This is the main entry point for reader mode. It:
/// 1. Queries the latest committed root from Stellar via native contract simulation.
/// 2. Searches the local JSONL for the matching `root_after`.
/// 3. Verifies the root chain up to that point.
///
/// `events_jsonl` is the raw contents of the `events.jsonl` file.
/// `local_root_hex` is the current Merkle root from the audit log.
/// `rpc_url` and `contract_id` specify the Stellar network and contract.
pub async fn verify_against_onchain(
    events_jsonl: &str,
    local_root_hex: &str,
    rpc_url: &str,
    contract_id: &str,
) -> AuditResult<VerificationReport> {
    let kp = stellar_native::generate_keypair();
    let onchain_root = stellar_native::get_current_root_native(&kp, rpc_url, contract_id).await?;

    verify_with_onchain_root(onchain_root, events_jsonl, local_root_hex)
}

/// Verify the local audit log against a specific on-chain root.
/// This is the testable inner function that doesn't call Stellar.
pub fn verify_with_onchain_root(
    onchain_root: Option<OnChainRoot>,
    events_jsonl: &str,
    local_root_hex: &str,
) -> AuditResult<VerificationReport> {
    let total_events = count_events(events_jsonl);

    match &onchain_root {
        Some(root) => {
            let onchain_root_hex = &root.root_hex;

            // Search for the on-chain root in the JSONL log.
            let commitment_index = find_root_in_log(events_jsonl, onchain_root_hex);

            match commitment_index {
                Some(index) => {
                    // Verify the root chain up to the commitment point.
                    let chain_ok = verify_root_chain(events_jsonl, index);

                    let events_after = total_events.saturating_sub(index + 1);
                    let tamper = !chain_ok;

                    let summary = if chain_ok {
                        if events_after == 0 {
                            format!(
                                "✅ Verified: on-chain root matches local log at event {}. \
                                 All {} events verified. Log is fully committed.",
                                index, total_events
                            )
                        } else {
                            format!(
                                "✅ Verified: on-chain root matches local log at event {}. \
                                 {} events verified, {} new event(s) since commitment.",
                                index,
                                index + 1,
                                events_after
                            )
                        }
                    } else {
                        format!(
                            "❌ Tamper detected: root chain broken before event {} \
                             (on-chain root match point). Events may have been modified \
                             or reordered.",
                            index
                        )
                    };

                    Ok(VerificationReport {
                        onchain_root_found: true,
                        onchain_root,
                        local_root_hex: local_root_hex.to_string(),
                        commitment_event_index: Some(index),
                        total_events,
                        verified_events: index + 1,
                        events_after_commitment: events_after,
                        chain_intact: chain_ok,
                        tamper_detected: tamper,
                        summary,
                    })
                }
                None => {
                    // The on-chain root was not found in the local log.
                    // This could mean:
                    // - The log was truncated (events deleted after commitment)
                    // - The on-chain root belongs to a different log
                    // - The log is fresh and hasn't been committed yet
                    let summary = if total_events == 0 {
                        "⚠️ Local log is empty. No events to verify against on-chain root.".to_string()
                    } else {
                        format!(
                            "❌ On-chain root {} not found in local log ({} events). \
                             The log may have been truncated, or the on-chain root \
                             belongs to a different audit log.",
                            &onchain_root_hex[..onchain_root_hex.len().min(16)],
                            total_events
                        )
                    };

                    Ok(VerificationReport {
                        onchain_root_found: false,
                        onchain_root,
                        local_root_hex: local_root_hex.to_string(),
                        commitment_event_index: None,
                        total_events,
                        verified_events: 0,
                        events_after_commitment: total_events,
                        chain_intact: false,
                        tamper_detected: total_events > 0,
                        summary,
                    })
                }
            }
        }
        None => {
            // No on-chain root has been committed yet.
            let summary = if total_events == 0 {
                "Local log is empty and no on-chain root has been committed.".to_string()
            } else {
                format!(
                    "No on-chain root has been committed yet. \
                     Local log has {} event(s) awaiting commitment.",
                    total_events
                )
            };

            Ok(VerificationReport {
                onchain_root_found: false,
                onchain_root: None,
                local_root_hex: local_root_hex.to_string(),
                commitment_event_index: None,
                total_events,
                verified_events: 0,
                events_after_commitment: total_events,
                chain_intact: true,
                tamper_detected: false,
                summary,
            })
        }
    }
}

/// Count the number of parseable event lines in the JSONL log.
fn count_events(jsonl: &str) -> u64 {
    jsonl
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| {
            serde_json::from_str::<serde_json::Value>(l.trim()).is_ok()
        })
        .count() as u64
}

/// Read a string field from a JSON event, tolerating both the snake_case
/// keys written to disk by `PersistedEvent` (`root_after`, `leaf_hex`) and
/// the camelCase keys used by the in-memory API types. The on-disk JSONL
/// log is snake_case, so `snake` must be tried first; trying only the
/// camelCase key (the original bug) made every lookup miss and produced
/// spurious "tamper detected" reports.
fn event_str<'a>(ev: &'a serde_json::Value, snake: &str, camel: &str) -> Option<&'a str> {
    ev.get(snake)
        .or_else(|| ev.get(camel))
        .and_then(|v| v.as_str())
}

/// Search the JSONL log for an event whose `root_after` matches the
/// given root hex. Returns the event index if found.
fn find_root_in_log(jsonl: &str, root_hex: &str) -> Option<u64> {
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(root_after) = event_str(&ev, "root_after", "rootAfter") {
                if root_after == root_hex {
                    return ev.get("index").and_then(|v| v.as_u64());
                }
            }
        }
    }
    None
}

/// Verify the root chain in the JSONL log up to (and including) the given
/// event index by REBUILDING the Poseidon Merkle tree.
///
/// For each event it (a) recomputes the leaf from the stored payload and
/// checks it against `leaf_hex`, (b) inserts it into a fresh
/// `AuditMerkleTree` and checks the insertion index matches the stored
/// `index`, and (c) checks the tree root after the insert matches the
/// stored `root_after`. This mirrors the authoritative `replay_file`
/// verification, so it detects payload edits, reordering, insertion, and
/// deletion — not just leaf mismatches. Returns `true` only when the chain
/// is fully intact up to `up_to_index`.
fn verify_root_chain(jsonl: &str, up_to_index: u64) -> bool {
    use crate::audit::leaf_from_payload;
    use ark_ff::{BigInteger, PrimeField};
    use zk_audit::merkle::AuditMerkleTree;

    let fr_to_hex = |f: ark_bn254::Fr| hex::encode(f.into_bigint().to_bytes_be());

    let mut tree = match AuditMerkleTree::new() {
        Ok(t) => t,
        Err(_) => return false,
    };

    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let ev: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return false,
        };

        let index = match ev.get("index").and_then(|v| v.as_u64()) {
            Some(i) => i,
            None => {
                // Metadata lines (e.g. legal holds) carry no `index` — skip
                // them rather than treating them as corruption.
                if ev.get("meta").is_some() {
                    continue;
                }
                return false;
            }
        };

        if index > up_to_index {
            break;
        }

        let operation = ev
            .get("operation")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let database = ev.get("database").and_then(|v| v.as_str()).unwrap_or("");
        let collection = ev
            .get("collection")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let payload = ev.get("payload").and_then(|v| v.as_str()).unwrap_or("");
        let version = ev.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let stored_leaf = event_str(&ev, "leaf_hex", "leafHex").unwrap_or("");
        let stored_root = event_str(&ev, "root_after", "rootAfter").unwrap_or("");

        // (a) Recompute the leaf and check it matches the stored leaf_hex.
        let recomputed_leaf = match version {
            1 | 0 => leaf_from_payload(operation, database, collection, payload),
            2 => {
                // v2 events require the leaf key to recompute the HMAC leaf.
                // If no key is available, we cannot verify v2 events.
                return false;
            }
            _ => return false,
        };
        if fr_to_hex(recomputed_leaf) != stored_leaf {
            return false;
        }

        // (b) Insert into the tree; the insertion index must match `index`
        //     (catches reordering / gaps / deletions).
        let inserted_idx = tree.insert(recomputed_leaf) as u64;
        if inserted_idx != index {
            return false;
        }

        // (c) The tree root after this insert must equal the stored
        //     root_after (catches root forgery / silent tampering).
        let recomputed_root = match tree.root() {
            Ok(r) => r,
            Err(_) => return false,
        };
        if fr_to_hex(recomputed_root) != stored_root {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::leaf_from_payload;

    fn make_jsonl(events: &[(u64, &str, &str, &str, &str, &str)]) -> String {
        // (index, operation, database, collection, payload, root_after)
        // leaf_hex is computed from the payload using leaf_from_payload.
        events
            .iter()
            .map(|(i, op, db, col, payload, root)| {
                let leaf = leaf_from_payload(op, db, col, payload);
                let leaf_hex = {
                    use ark_ff::{BigInteger, PrimeField};
                    let bigint = leaf.into_bigint();
                    hex::encode(&bigint.to_bytes_be())
                };
                serde_json::json!({
                    "index": i,
                    "operation": op,
                    "database": db,
                    "collection": col,
                    "payload": payload,
                    "leaf_hex": leaf_hex,
                    "root_after": root,
                    "timestamp": "2026-06-23T00:00:00Z"
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn fr_hex(f: ark_bn254::Fr) -> String {
        use ark_ff::{BigInteger, PrimeField};
        hex::encode(f.into_bigint().to_bytes_be())
    }

    /// Build a JSONL log with REAL `root_after` values by replaying the
    /// events through an `AuditMerkleTree`, exactly as the audit log is
    /// written in production. Returns `(jsonl, roots)` where `roots[i]` is
    /// the tree root after inserting event `i`.
    fn make_jsonl_real(events: &[(u64, &str, &str, &str, &str)]) -> (String, Vec<String>) {
        use zk_audit::merkle::AuditMerkleTree;
        let mut tree = AuditMerkleTree::new().unwrap();
        let mut roots = Vec::new();
        let lines: Vec<String> = events
            .iter()
            .map(|(i, op, db, col, payload)| {
                let leaf = leaf_from_payload(op, db, col, payload);
                let leaf_hex = fr_hex(leaf);
                tree.insert(leaf);
                let root = fr_hex(tree.root().unwrap());
                roots.push(root.clone());
                serde_json::json!({
                    "index": i,
                    "operation": op,
                    "database": db,
                    "collection": col,
                    "payload": payload,
                    "leaf_hex": leaf_hex,
                    "root_after": root,
                    "timestamp": "2026-06-23T00:00:00Z"
                })
                .to_string()
            })
            .collect();
        (lines.join("\n"), roots)
    }

    #[test]
    fn reader_mode_no_onchain_root() {
        let jsonl = make_jsonl(&[
            (0, "insert", "db", "col", r#"{"a":1}"#, "root0"),
        ]);
        let report = verify_with_onchain_root(None, &jsonl, "root0").unwrap();

        assert!(!report.onchain_root_found);
        assert!(report.onchain_root.is_none());
        assert_eq!(report.total_events, 1);
        assert!(!report.tamper_detected);
        assert!(report.chain_intact);
        assert!(report.summary.contains("awaiting commitment"));
    }

    #[test]
    fn reader_mode_empty_log_no_onchain() {
        let report = verify_with_onchain_root(None, "", "empty").unwrap();

        assert_eq!(report.total_events, 0);
        assert!(!report.tamper_detected);
        assert!(report.summary.contains("empty"));
    }

    #[test]
    fn reader_mode_onchain_root_found() {
        let (jsonl, roots) = make_jsonl_real(&[
            (0, "insert", "db", "col", r#"{"a":1}"#),
            (1, "insert", "db", "col", r#"{"a":2}"#),
            (2, "insert", "db", "col", r#"{"a":3}"#),
        ]);

        let onchain = OnChainRoot {
            sequence: 1,
            root_hex: roots[1].clone(),
            timestamp: 1000,
            metadata: "events=2".to_string(),
        };

        let report = verify_with_onchain_root(Some(onchain), &jsonl, &roots[2]).unwrap();

        assert!(report.onchain_root_found);
        assert_eq!(report.commitment_event_index, Some(1));
        assert_eq!(report.verified_events, 2);
        assert_eq!(report.events_after_commitment, 1);
        assert_eq!(report.total_events, 3);
        assert!(report.chain_intact);
        assert!(!report.tamper_detected);
        assert!(report.summary.contains("Verified"));
    }

    #[test]
    fn reader_mode_onchain_root_not_found() {
        let jsonl = make_jsonl(&[
            (0, "insert", "db", "col", r#"{"a":1}"#, "root0"),
            (1, "insert", "db", "col", r#"{"a":2}"#, "root1"),
        ]);

        let onchain = OnChainRoot {
            sequence: 1,
            root_hex: "nonexistent".to_string(),
            timestamp: 1000,
            metadata: "".to_string(),
        };

        let report = verify_with_onchain_root(Some(onchain), &jsonl, "root1").unwrap();

        assert!(!report.onchain_root_found);
        assert_eq!(report.commitment_event_index, None);
        assert!(report.tamper_detected);
        assert!(report.summary.contains("not found"));
    }

    #[test]
    fn reader_mode_onchain_root_found_fully_committed() {
        let (jsonl, roots) = make_jsonl_real(&[
            (0, "insert", "db", "col", r#"{"a":1}"#),
            (1, "insert", "db", "col", r#"{"a":2}"#),
        ]);

        let onchain = OnChainRoot {
            sequence: 1,
            root_hex: roots[1].clone(),
            timestamp: 1000,
            metadata: "events=2".to_string(),
        };

        let report = verify_with_onchain_root(Some(onchain), &jsonl, &roots[1]).unwrap();

        assert!(report.onchain_root_found);
        assert_eq!(report.events_after_commitment, 0);
        assert!(report.summary.contains("fully committed"));
    }

    #[test]
    fn reader_mode_count_events_skips_invalid_lines() {
        let jsonl = "valid line 1\n\ninvalid json\n";
        // Only lines that parse as JSON are counted.
        // "valid line 1" is not valid JSON, so count is 0.
        assert_eq!(count_events(jsonl), 0);

        let jsonl2 = r#"{"index":0}"#;
        assert_eq!(count_events(jsonl2), 1);
    }

    #[test]
    fn reader_mode_find_root_in_log() {
        let jsonl = make_jsonl(&[
            (0, "insert", "db", "col", r#"{"a":1}"#, "root0"),
            (1, "insert", "db", "col", r#"{"a":2}"#, "root1"),
            (2, "insert", "db", "col", r#"{"a":3}"#, "root2"),
        ]);

        assert_eq!(find_root_in_log(&jsonl, "root1"), Some(1));
        assert_eq!(find_root_in_log(&jsonl, "root0"), Some(0));
        assert_eq!(find_root_in_log(&jsonl, "nonexistent"), None);
    }

    #[test]
    fn reader_mode_accepts_legacy_camel_case_events() {
        use zk_audit::merkle::AuditMerkleTree;
        let leaf = leaf_from_payload("insert", "db", "col", r#"{"a":1}"#);
        let leaf_hex = fr_hex(leaf);
        let mut tree = AuditMerkleTree::new().unwrap();
        tree.insert(leaf);
        let root = fr_hex(tree.root().unwrap());
        let jsonl = serde_json::json!({
            "index": 0,
            "operation": "insert",
            "database": "db",
            "collection": "col",
            "payload": r#"{"a":1}"#,
            "leafHex": leaf_hex,
            "rootAfter": root,
            "timestamp": "2026-06-23T00:00:00Z"
        })
        .to_string();

        assert_eq!(find_root_in_log(&jsonl, &root), Some(0));
        assert!(verify_root_chain(&jsonl, 0));
    }

    #[test]
    fn reader_mode_detects_forged_root_after() {
        // The old leaf-only check would pass a forged root_after as long as
        // the payload (and thus the leaf) was untouched. Rebuilding the tree
        // catches it: the recomputed root no longer matches the stored one.
        let (jsonl, roots) = make_jsonl_real(&[
            (0, "insert", "db", "col", r#"{"a":1}"#),
            (1, "insert", "db", "col", r#"{"a":2}"#),
        ]);
        // Forge the committed root_after at index 1 with a well-formed but
        // wrong value. Leaves are unchanged.
        let forged = jsonl.replace(&roots[1], &"f".repeat(64));
        assert_ne!(forged, jsonl, "forge replacement must change the log");
        assert!(
            !verify_root_chain(&forged, 1),
            "a forged root_after must be detected by rebuilding the tree"
        );
        // The honest log still verifies.
        assert!(verify_root_chain(&jsonl, 1));
    }

    #[test]
    fn reader_mode_verification_report_serializes_camel_case() {
        let report = VerificationReport {
            onchain_root_found: true,
            onchain_root: None,
            local_root_hex: "abc".to_string(),
            commitment_event_index: Some(5),
            total_events: 10,
            verified_events: 6,
            events_after_commitment: 4,
            chain_intact: true,
            tamper_detected: false,
            summary: "ok".to_string(),
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"onchainRootFound\":true"));
        assert!(json.contains("\"localRootHex\":\"abc\""));
        assert!(json.contains("\"commitmentEventIndex\":5"));
        assert!(json.contains("\"eventsAfterCommitment\":4"));
        assert!(json.contains("\"chainIntact\":true"));
        assert!(json.contains("\"tamperDetected\":false"));
    }
}
