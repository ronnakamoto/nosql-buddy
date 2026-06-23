//! ZK audit log: tamper-evident Merkle tree of database operations.
//!
//! This module owns the audit state: a Poseidon Merkle tree that accumulates
//! audit events (inserts, updates, deletes) into a single root, plus the
//! ability to generate Groth16 inclusion proofs and commit roots to Soroban.
//!
//! ## Persistence
//!
//! The audit log is persisted as an append-only JSONL file at
//! `<app_data_dir>/audit/events.jsonl`. Every `record()` call appends one
//! line and fsyncs before returning, so a crash never loses a confirmed
//! event. On startup, `set_persistence_dir()` replays the file and rebuilds
//! the in-memory tree.
//!
//! ## Tamper-evidence
//!
//! Each persisted line stores the event payload, the leaf hash derived from
//! it, and the Merkle root computed *after* inserting that leaf. On replay:
//!
//! - The leaf is recomputed from the payload and asserted equal to the
//!   stored leaf. Mismatch ⇒ payload was edited.
//! - The root is recomputed after each insert and asserted equal to the
//!   stored `root_after`. Mismatch ⇒ events were reordered, inserted, or
//!   deleted.
//!
//! Truncation of the *tail* (deleting the last N events) is not detectable
//! from the file alone — that's what the on-chain Soroban root anchor is
//! for. The local file gives tamper-evidence for modification of any event
//! that remains in the file.
//!
//! ## Crash recovery
//!
//! If the process dies mid-append, the last line may be partial. On replay,
//! a line that fails to parse as JSON is truncated (the file is rewritten
//! up to the last good line) and a warning is logged. This is the standard
//! journaling approach for append-only logs.
//!
//! ## Architecture
//!
//! - [`AuditLog`] — the audit log: a Merkle tree + event metadata + optional
//!   on-disk persistence.
//! - [`commands`] — Tauri IPC commands for the frontend audit panel.
//! - [`interceptor`] — hooks into Mongo operations to auto-record audit events.

pub mod commands;
pub mod interceptor;

#[cfg(test)]
mod e2e_test;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use zk_audit::merkle::AuditMerkleTree;
use zk_audit::InclusionProof;

use crate::error::{AppError, AppResult};

/// An audit event recorded in the log.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    /// Sequence number (0-indexed position in the tree).
    pub index: u64,
    /// The leaf hash (Poseidon of the event payload).
    pub leaf_hex: String,
    /// Human-readable description of the operation.
    pub operation: String,
    /// Database/collection affected.
    pub database: String,
    pub collection: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
}

/// On-disk representation of one audit event, written as a single JSONL
/// line. Stores the payload so the leaf can be recomputed and verified
/// on replay, and `root_after` so the Merkle chain can be checked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedEvent {
    index: u64,
    operation: String,
    database: String,
    collection: String,
    /// The canonical payload string the leaf was derived from
    /// (`"{op}|{db}|{col}|{args}"`). Stored verbatim so the leaf can be
    /// recomputed and checked against `leaf_hex`.
    payload: String,
    leaf_hex: String,
    /// Merkle root hex computed *after* this leaf was inserted. Forms a
    /// chain: each event's `root_after` is a function of all prior leaves.
    root_after: String,
    timestamp: String,
}

/// Open file handle + path for the append-only JSONL log.
struct PersistenceState {
    /// Kept for diagnostics and future operations (rotation, explicit
    /// flush path, re-open after truncation). Currently only `file` is
    /// read after construction.
    #[allow(dead_code)]
    events_path: PathBuf,
    file: File,
}

/// The audit log state, protected by a mutex.
pub struct AuditLog {
    tree: Mutex<AuditMerkleTree>,
    events: Mutex<Vec<AuditEvent>>,
    /// When `Some`, every `record()` call appends a JSONL line and fsyncs.
    /// Set once via `set_persistence_dir()` from `setup()`.
    persistence: Mutex<Option<PersistenceState>>,
}

impl AuditLog {
    /// Create a new audit log with the default tree height (20 levels = 1M leaves).
    pub fn new() -> AppResult<Self> {
        let tree = AuditMerkleTree::with_height(20)?;
        Ok(Self {
            tree: Mutex::new(tree),
            events: Mutex::new(Vec::new()),
            persistence: Mutex::new(None),
        })
    }

    /// Enable on-disk persistence and replay any existing log.
    ///
    /// Call once from `setup()` with the app's data directory. After this
    /// returns, every `record()` call appends to `<dir>/audit/events.jsonl`
    /// and fsyncs. If the file already exists, its events are replayed into
    /// the in-memory tree and integrity is verified (leaf recomputation +
    /// root chain). A partial last line (crash during append) is truncated.
    ///
    /// Calling this when events already exist in memory is an error —
    /// persistence must be wired before any audit events are recorded.
    pub fn set_persistence_dir(&self, dir: &Path) -> AppResult<()> {
        let audit_dir = dir.join("audit");
        std::fs::create_dir_all(&audit_dir)?;
        let events_path = audit_dir.join("events.jsonl");

        // Read existing content. We open for read first, replay, then
        // reopen for append. This keeps the replay logic simple and lets
        // us truncate a corrupt tail before the append handle is open.
        let replayed = if events_path.exists() {
            self.replay_file(&events_path)?
        } else {
            Vec::new()
        };

        // Open for append. create=true so a missing file is started fresh.
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(false)
            .open(&events_path)?;

        let mut persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        *persistence = Some(PersistenceState {
            events_path: events_path.clone(),
            file,
        });

        if !replayed.is_empty() {
            tracing::info!(
                "audit log replayed {} event(s) from {}",
                replayed.len(),
                events_path.display()
            );
        }

        Ok(())
    }

    /// Read a JSONL log file, replay events into the in-memory tree, and
    /// verify integrity. Returns the replayed events. A partial/corrupt
    /// last line is truncated from the file in place.
    fn replay_file(&self, path: &Path) -> AppResult<Vec<PersistedEvent>> {
        let raw = std::fs::read_to_string(path)?;
        let mut lines: Vec<&str> = raw.lines().collect();

        // Find the last parseable line. Any trailing lines that fail to
        // parse are treated as a partial write from a crash and dropped.
        let mut first_bad: Option<usize> = None;
        let mut parsed: Vec<PersistedEvent> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PersistedEvent>(line.trim()) {
                Ok(ev) => parsed.push(ev),
                Err(_) => {
                    first_bad = Some(i);
                    break;
                }
            }
        }

        if let Some(bad_idx) = first_bad {
            // Truncate the file at the last good line. This is the
            // journaling-style recovery: drop the partial tail.
            tracing::warn!(
                "audit log: truncating {} partial line(s) at {} (line {})",
                lines.len() - bad_idx,
                path.display(),
                bad_idx + 1,
            );
            lines.truncate(bad_idx);
            let clean: String = lines.join("\n");
            let clean = if clean.is_empty() {
                String::new()
            } else {
                format!("{clean}\n")
            };
            std::fs::write(path, clean)?;
        }

        if parsed.is_empty() {
            return Ok(Vec::new());
        }

        // Replay each event into the tree and verify integrity.
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());

        // The in-memory tree must be empty — set_persistence_dir is
        // called once at startup before any record() calls.
        if !events.is_empty() || tree.leaf_count() != 0 {
            return Err(AppError::ZkAudit(format!(
                "cannot replay into a non-empty audit log ({} event(s) already in memory)",
                events.len()
            )));
        }

        for ev in &parsed {
            // 1. Recompute the leaf from the stored payload and verify
            //    it matches the stored leaf_hex. Mismatch ⇒ payload was
            //    edited after the fact.
            let recomputed_leaf = leaf_from_payload(&ev.operation, &ev.database, &ev.collection, &ev.payload);
            let recomputed_hex = fr_to_hex(recomputed_leaf);
            if recomputed_hex != ev.leaf_hex {
                return Err(AppError::ZkAudit(format!(
                    "audit log tamper detected at index {}: leaf mismatch (stored {}, recomputed {})",
                    ev.index, ev.leaf_hex, recomputed_hex
                )));
            }

            // 2. Insert the leaf and verify the resulting root matches
            //    the stored root_after. Mismatch ⇒ events were reordered,
            //    inserted, or deleted.
            let inserted_idx = tree.insert(recomputed_leaf) as u64;
            if inserted_idx != ev.index {
                return Err(AppError::ZkAudit(format!(
                    "audit log tamper detected: index mismatch at line {} (expected {}, got {})",
                    ev.index, ev.index, inserted_idx
                )));
            }
            let recomputed_root = tree.root()?;
            let recomputed_root_hex = fr_to_hex(recomputed_root);
            if recomputed_root_hex != ev.root_after {
                return Err(AppError::ZkAudit(format!(
                    "audit log tamper detected at index {}: root_after mismatch (stored {}, recomputed {})",
                    ev.index, ev.root_after, recomputed_root_hex
                )));
            }

            events.push(AuditEvent {
                index: ev.index,
                leaf_hex: ev.leaf_hex.clone(),
                operation: ev.operation.clone(),
                database: ev.database.clone(),
                collection: ev.collection.clone(),
                timestamp: ev.timestamp.clone(),
            });
        }

        Ok(parsed)
    }

    /// Record an audit event. Returns the leaf index.
    ///
    /// `payload` is the canonical string the leaf was derived from
    /// (e.g. `"insert|db|col|{document_json}"`). It's persisted verbatim
    /// so the leaf can be recomputed and verified on replay.
    pub fn record(
        &self,
        operation: &str,
        database: &str,
        collection: &str,
        payload: &str,
        leaf: ark_bn254::Fr,
    ) -> AppResult<u64> {
        // Recover from poisoned mutex (a prior panic) rather than propagating
        // the panic — this prevents a single failure from bricking the entire
        // audit log for all subsequent commands.
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let index = tree.insert(leaf) as u64;

        let leaf_hex = fr_to_hex(leaf);

        // Compute the root *after* this insert so we can store it as
        // `root_after` — the chain link that lets replay detect
        // reordering / deletion.
        let root_after = fr_to_hex(tree.root()?);

        let timestamp = chrono::Utc::now().to_rfc3339();

        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.push(AuditEvent {
            index,
            leaf_hex: leaf_hex.clone(),
            operation: operation.to_string(),
            database: database.to_string(),
            collection: collection.to_string(),
            timestamp: timestamp.clone(),
        });

        // Persist atomically: append one JSONL line + fsync, all while
        // holding the persistence mutex. If persistence isn't wired yet
        // (e.g. in unit tests), this is a no-op.
        let mut persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = persistence.as_mut() {
            let persisted = PersistedEvent {
                index,
                operation: operation.to_string(),
                database: database.to_string(),
                collection: collection.to_string(),
                payload: payload.to_string(),
                leaf_hex: leaf_hex.clone(),
                root_after: root_after.clone(),
                timestamp: timestamp.clone(),
            };
            let line = serde_json::to_string(&persisted)?;
            writeln!(state.file, "{line}")?;
            state.file.sync_all()?;
        }

        Ok(index)
    }

    /// Get the current Merkle root as a hex string.
    pub fn root_hex(&self) -> AppResult<String> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let root = tree.root()?;
        Ok(fr_to_hex(root))
    }

    /// Get the current root as a field element.
    pub fn root(&self) -> AppResult<ark_bn254::Fr> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        Ok(tree.root()?)
    }

    /// Get the number of recorded events.
    pub fn event_count(&self) -> usize {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// List all recorded audit events.
    pub fn list_events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Generate an inclusion proof for the event at the given index.
    pub fn prove_inclusion(&self, index: u64) -> AppResult<InclusionProof> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let proof = tree
            .prove_inclusion(index as usize)
            .map_err(|e| crate::error::AppError::ZkAudit(e.to_string()))?;
        Ok(proof)
    }

    /// Get the current leaf count (same as event count).
    pub fn leaf_count(&self) -> usize {
        self.tree
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .leaf_count()
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new().expect("failed to create audit log")
    }
}

/// Encode a field element as a hex string (big-endian byte order, matching
/// the existing `root_hex` / `leaf_hex` format).
fn fr_to_hex(f: ark_bn254::Fr) -> String {
    use ark_ff::{BigInteger, PrimeField};
    let bigint = f.into_bigint();
    let bytes = bigint.to_bytes_be();
    hex::encode(&bytes)
}

/// Recompute the leaf field element from the canonical payload string.
///
/// This must stay byte-for-byte identical to
/// [`interceptor::record_insert`] / [`record_update`] / [`record_delete`]
/// and [`commands::audit_record_event`]: SHA-256 of the payload, take the
/// first 31 bytes, mask the top nibble of byte 31, interpret as a field
/// element via `from_be_bytes_mod_order`. Any divergence here would make
/// replay-time verification flag every event as tampered.
pub(crate) fn leaf_from_payload(
    _operation: &str,
    _database: &str,
    _collection: &str,
    payload: &str,
) -> ark_bn254::Fr {
    use ark_bn254::Fr;
    use ark_ff::PrimeField;
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    let hash = hasher.finalize();
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash);
    bytes[..31].copy_from_slice(&hash[..31]);
    bytes[31] &= 0x0F;
    Fr::from_be_bytes_mod_order(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Round-trip: record events with persistence, reload from a fresh
    /// AuditLog, and verify the root + event list match.
    #[test]
    fn persistence_round_trip_preserves_root_and_events() {
        let dir = tempfile_dir();
        // First "session": enable persistence, record events.
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            interceptor::record_insert(&std::sync::Arc::new(audit), "db", "col", r#"{"a":1}"#).unwrap();
        }
        let dir2 = dir.clone();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir2).unwrap();
            // The replayed event must be present.
            assert_eq!(audit.event_count(), 1);
            assert_eq!(audit.leaf_count(), 1);
            let events = audit.list_events();
            assert_eq!(events[0].operation, "insert");
            assert_eq!(events[0].database, "db");
            assert_eq!(events[0].collection, "col");
            // Root must be non-trivial (not the empty-tree root).
            let empty_root = AuditLog::new().unwrap().root_hex().unwrap();
            assert_ne!(audit.root_hex().unwrap(), empty_root);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Two sessions: events from session 1 survive into session 2, and
    /// a new event in session 2 appends correctly.
    #[test]
    fn persistence_appends_across_sessions() {
        let dir = tempfile_dir();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "db", "col", r#"{"a":2}"#).unwrap();
            let root_after_session1 = a.root_hex().unwrap();
            drop(a);
            // Capture root by re-reading: we need it after the Arc drops.
            let _ = root_after_session1;
        }
        // Session 2: replay + append one more.
        let root_after_session2 = {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            assert_eq!(audit.event_count(), 2, "two events must survive from session 1");
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "db", "col", r#"{"a":3}"#).unwrap();
            assert_eq!(a.event_count(), 3);
            a.root_hex().unwrap()
        };
        // Session 3: replay all three.
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            assert_eq!(audit.event_count(), 3);
            assert_eq!(audit.root_hex().unwrap(), root_after_session2);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Tamper detection: editing a payload in the JSONL file must cause
    /// replay to fail with a leaf-mismatch error.
    #[test]
    fn replay_detects_payload_tamper() {
        let dir = tempfile_dir();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "db", "col", r#"{"a":1}"#).unwrap();
        }
        // Tamper: parse the JSONL line, mutate the payload field, rewrite.
        // (We can't do a literal string replace because the payload is
        // JSON-escaped inside the JSONL line.)
        let path = dir.join("audit").join("events.jsonl");
        let content = fs::read_to_string(&path).unwrap();
        let line = content.lines().next().unwrap();
        let mut ev: serde_json::Value = serde_json::from_str(line).unwrap();
        ev["payload"] = serde_json::json!("insert|db|col|{\"a\":999}");
        let tampered = serde_json::to_string(&ev).unwrap();
        fs::write(&path, format!("{tampered}\n")).unwrap();

        let audit = AuditLog::new().unwrap();
        let err = audit.set_persistence_dir(&dir).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("leaf mismatch") || msg.contains("tamper"),
            "expected tamper error, got: {msg}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Crash recovery: a partial last line (no trailing newline / corrupt
    /// JSON) is truncated and replay succeeds with the prior events.
    #[test]
    fn replay_truncates_partial_last_line() {
        let dir = tempfile_dir();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "db", "col", r#"{"a":2}"#).unwrap();
        }
        // Append a partial (corrupt) line — simulates a crash mid-write.
        let path = dir.join("audit").join("events.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{this is not valid json").unwrap();
        drop(f);

        let audit = AuditLog::new().unwrap();
        audit.set_persistence_dir(&dir).unwrap();
        assert_eq!(audit.event_count(), 2, "partial line must be dropped, prior 2 kept");
        let _ = fs::remove_dir_all(&dir);
    }

    /// Root equality after reload: the root computed in session 2 (after
    /// replay) must equal the root that was persisted as `root_after` of
    /// the last event in session 1.
    #[test]
    fn root_after_reload_matches_persisted_root_after() {
        let dir = tempfile_dir();
        let persisted_last_root;
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "db", "col", r#"{"a":2}"#).unwrap();
            // Read the last persisted line's root_after.
            let path = dir.join("audit").join("events.jsonl");
            let content = fs::read_to_string(&path).unwrap();
            let last_line = content.lines().last().unwrap();
            let ev: PersistedEvent = serde_json::from_str(last_line).unwrap();
            persisted_last_root = ev.root_after;
        }
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            assert_eq!(audit.root_hex().unwrap(), persisted_last_root);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "nosqlbuddy-audit-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        d
    }
}
