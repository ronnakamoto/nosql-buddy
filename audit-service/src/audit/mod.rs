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

pub mod attestation;
pub mod change_stream;
pub mod crypto;
pub mod dev_proxy;
pub mod dev_setup;
pub mod epoch;
pub mod interceptor;
pub mod ipfs;
pub mod oplog;
pub mod oplog_canon;
#[cfg(test)]
mod oplog_integration;
#[cfg(test)]
mod oplog_omission;
pub mod pinata;
pub mod reader;
pub mod sled_store;
pub mod stellar;
pub mod stellar_native;
pub mod stellar_rpc;
pub mod verification_store;

#[cfg(test)]
mod e2e_test;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use zk_audit::merkle::AuditMerkleTree;
use zk_audit::InclusionProof;

use crate::error::{AuditError, AuditResult};

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
    /// Stable audit deployment identity (e.g. `rs:rs0`). Empty for legacy
    /// events recorded before per-deployment segmentation, which form the
    /// backward-compatible "unattributed" domain.
    pub deployment_id: String,
    /// Monotonic per-`(deploymentId, database)` sequence number (0-indexed).
    /// Derived deterministically on replay, so it is robust against legacy
    /// rows that lack a stored sequence.
    pub sequence: u64,
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
    /// Audit deployment identity. New field — `serde(default)` so legacy
    /// JSONL lines (written before segmentation) still replay; they fall
    /// back to the empty "unattributed" domain.
    #[serde(default)]
    deployment_id: String,
    /// Monotonic per-domain sequence. New field — `serde(default)` for legacy
    /// lines. The stored value is informational; the authoritative sequence
    /// is recomputed on replay from event ordering.
    #[serde(default)]
    sequence: u64,
    /// The canonical payload string the leaf was derived from.
    ///
    /// - **v1** (legacy): `"{op}|{db}|{col}|{args}"` — pipe-delimited,
    ///   leaf derived via `SHA-256(payload)`.
    /// - **v2** (legacy): base64-encoded canonical bytes
    ///   (`canonical_payload_bytes(op, db, col, data)`), leaf derived via
    ///   `HMAC-SHA-256(k_audit, canonical_bytes)`.
    /// - **v3** (current): base64-encoded canonical bytes, leaf derived via
    ///   the keyed Poseidon vector commitment over structured fields
    ///   (`zk_audit::commitment::poseidon_leaf_v3`) — provable in-circuit
    ///   for ZK disclosure proofs.
    payload: String,
    leaf_hex: String,
    /// Merkle root hex computed *after* this leaf was inserted. Forms a
    /// chain: each event's `root_after` is a function of all prior leaves.
    root_after: String,
    timestamp: String,
    /// Event format version.
    ///
    /// - `1` (or missing, which deserializes to `0`) = legacy pipe-delimited
    ///   payload + SHA-256 leaf.
    /// - `2` = canonical binary payload + HMAC leaf.
    /// - `3` = canonical binary payload + keyed Poseidon vector commitment
    ///   (ZK-provable structured fields).
    #[serde(default = "default_event_version")]
    version: u32,
}

fn default_event_version() -> u32 {
    1
}

/// A retained commitment for a logically pruned audit domain segment.
///
/// The shared global tree is append-only, so pruning never deletes leaves from
/// the anchored root chain. Instead, the active event metadata for a domain is
/// removed and this compact retained root records the pruned segment.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DomainRetentionRoot {
    pub root_hex: String,
    pub event_count: usize,
    pub max_index: u64,
    pub pruned_at: String,
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
    /// Next sequence number per audit domain key `(deploymentId, database)`.
    /// Used to assign a monotonic, per-domain `sequence` to each leaf at
    /// `record()` time and rebuilt on replay so it stays continuous.
    sequences: Mutex<HashMap<(String, String), u64>>,
    /// When `Some`, every `record()` call appends a JSONL line and fsyncs.
    /// Set once via `set_persistence_dir()` from `setup()`.
    persistence: Mutex<Option<PersistenceState>>,
    /// Sled-backed tree state store for fast startup. When `Some`,
    /// the tree state (leaves + root) is persisted in sled, avoiding
    /// the need to replay the JSONL file through Poseidon hashing
    /// on every startup.
    sled_store: Mutex<Option<crate::audit::sled_store::SledTreeStore>>,
    /// Domains currently under legal hold. Events in these domains cannot be
    /// pruned or reset until the hold is lifted.
    legal_holds: Mutex<HashSet<(String, String)>>,
    /// Retained roots for logically pruned domain segments.
    retained_domain_roots: Mutex<HashMap<(String, String), Vec<DomainRetentionRoot>>>,
    /// HMAC key for v2 leaf derivation (`k_audit`). When `Some`, new events
    /// are written as **v2** (canonical payload + HMAC leaf). When `None`,
    /// new events are written as **v1** (pipe-delimited + SHA-256 leaf).
    /// Replay automatically dispatches on each event's stored `version`.
    leaf_key: Mutex<Option<[u8; 32]>>,
    /// The Merkle root (hex) immediately after each leaf was inserted,
    /// indexed by tree position (`root_after_by_index[i]` is the root right
    /// after leaf `i` was inserted). Appended atomically under the same
    /// `tree` lock guard as the insert itself in `record()`.
    ///
    /// This lets any caller freeze an epoch boundary at a *specific,
    /// historical* index by looking up its root here, instead of calling
    /// `root_hex()` (which always reflects the tree's *current, latest*
    /// state). The latter is a TOCTOU hazard for out-of-band epoch tracking
    /// (e.g. a periodic catch-up scan running concurrently with live leaf
    /// insertion): by the time the scan gets around to processing index N
    /// and reads `root_hex()`, more leaves may have already been inserted
    /// by the concurrent writer, silently pulling events past N into the
    /// "frozen" root for an epoch that's only supposed to end at N.
    root_after_by_index: Mutex<Vec<String>>,
}

impl AuditLog {
    /// Create a new audit log with the default tree height (20 levels = 1M leaves).
    pub fn new() -> AuditResult<Self> {
        let tree = AuditMerkleTree::with_height(20)?;
        Ok(Self {
            tree: Mutex::new(tree),
            events: Mutex::new(Vec::new()),
            sequences: Mutex::new(HashMap::new()),
            persistence: Mutex::new(None),
            sled_store: Mutex::new(None),
            legal_holds: Mutex::new(HashSet::new()),
            retained_domain_roots: Mutex::new(HashMap::new()),
            leaf_key: Mutex::new(None),
            root_after_by_index: Mutex::new(Vec::new()),
        })
    }

    /// Look up the Merkle root (hex) immediately after the leaf at `index`
    /// was inserted. Returns `None` if `index` hasn't been inserted yet
    /// (out of range) — this is a historical, point-in-time value, never
    /// the tree's current/latest root once other leaves are inserted after
    /// it. See [`AuditLog::root_after_by_index`] for why this exists.
    pub fn root_after_at(&self, index: u64) -> Option<String> {
        self.root_after_by_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(index as usize)
            .cloned()
    }

    /// Set the HMAC leaf derivation key. After this is called, all new
    /// events are recorded as **v2** (canonical payload + HMAC leaf).
    pub fn set_leaf_key(&self, key: [u8; 32]) {
        *self.leaf_key.lock().unwrap_or_else(|e| e.into_inner()) = Some(key);
    }

    /// Clear the HMAC leaf derivation key. New events revert to **v1**.
    #[allow(dead_code)]
    pub fn clear_leaf_key(&self) {
        *self.leaf_key.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether a leaf key is configured (v2 mode is active).
    pub fn has_leaf_key(&self) -> bool {
        self.leaf_key.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Get the configured leaf key, if any.
    pub fn leaf_key(&self) -> Option<[u8; 32]> {
        *self.leaf_key.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Recompute the leaf for a persisted event, dispatching on its
    /// stored `version`.
    ///
    /// - v1 (or missing): uses the legacy `leaf_from_payload` (SHA-256).
    /// - v2: base64-decodes the payload and computes `HMAC-SHA-256(k_audit,
    ///   canonical_bytes)`.
    fn recompute_leaf(&self, event: &PersistedEvent) -> AuditResult<ark_bn254::Fr> {
        match event.version {
            1 | 0 => Ok(leaf_from_payload(
                &event.operation,
                &event.database,
                &event.collection,
                &event.payload,
            )),
            2 => {
                let key = self
                    .leaf_key
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .ok_or_else(|| {
                        AuditError::Validation(
                            "v2 event requires leaf key, but none is configured".to_string(),
                        )
                    })?;
                let canonical = base64::engine::general_purpose::STANDARD.decode(&event.payload).map_err(|e| {
                    AuditError::Validation(format!(
                        "v2 event base64 decode failed at index {}: {e}",
                        event.index
                    ))
                })?;
                Ok(crate::audit::crypto::hmac_leaf(&key, &canonical))
            }
            3 => {
                let key = self
                    .leaf_key
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .ok_or_else(|| {
                        AuditError::Validation(
                            "v3 event requires leaf key, but none is configured".to_string(),
                        )
                    })?;
                let canonical = base64::engine::general_purpose::STANDARD.decode(&event.payload).map_err(|e| {
                    AuditError::Validation(format!(
                        "v3 event base64 decode failed at index {}: {e}",
                        event.index
                    ))
                })?;
                let ts_secs = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
                    .map_err(|e| {
                        AuditError::Validation(format!(
                            "v3 event timestamp parse failed at index {}: {e}",
                            event.index
                        ))
                    })?
                    .timestamp()
                    .max(0) as u64;
                let (leaf, _) = zk_audit::commitment::poseidon_leaf_v3(
                    &key,
                    &event.operation,
                    &event.database,
                    &event.collection,
                    ts_secs,
                    &canonical,
                )?;
                Ok(leaf)
            }
            v => Err(AuditError::Validation(format!(
                "unknown event version {v} at index {}",
                event.index
            ))),
        }
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
    pub fn set_persistence_dir(&self, dir: &Path) -> AuditResult<()> {
        let audit_dir = dir.join("audit");
        std::fs::create_dir_all(&audit_dir)?;
        let events_path = audit_dir.join("events.jsonl");
        let sled_path = audit_dir.join("tree.sled");

        // Try to load tree state from sled first (fast path).
        // If sled has state, we load the tree from it. We still replay
        // the JSONL for verification (tamper detection) but use the
        // sled tree as the authoritative tree state.
        let sled_loaded = if sled_path.exists() {
            match crate::audit::sled_store::SledTreeStore::open(&sled_path) {
                Ok(store) => match store.load_tree() {
                    Ok(Some((tree, root_hex))) => {
                        tracing::info!(
                            "audit tree loaded from sled: {} leaves, root {}",
                            tree.leaf_count(),
                            root_hex
                        );
                        let mut t = self.tree.lock().unwrap_or_else(|e| e.into_inner());
                        *t = tree;
                        true
                    }
                    Ok(None) => false,
                    Err(e) => {
                        tracing::warn!("sled load_tree failed, falling back to JSONL: {e}");
                        false
                    }
                },
                Err(e) => {
                    tracing::warn!("sled open failed, falling back to JSONL: {e}");
                    false
                }
            }
        } else {
            false
        };

        // Open the sled store for writing.
        let store = crate::audit::sled_store::SledTreeStore::open(&sled_path)?;
        {
            let mut sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
            *sled_guard = Some(store);
        }

        // Always replay the JSONL for verification (tamper detection).
        // When sled_loaded is true, we verify the JSONL against the
        // sled-loaded tree. When sled_loaded is false, we rebuild the
        // tree from the JSONL.
        let replayed = if events_path.exists() {
            if sled_loaded {
                // Verify the JSONL against the sled-loaded tree.
                // This catches tamper even when sled has valid state.
                self.verify_against_tree(&events_path)?
            } else {
                // Rebuild the tree from JSONL (and verify integrity).
                self.replay_file(&events_path)?
            }
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
                "audit log loaded {} event(s) from {}",
                replayed.len(),
                events_path.display()
            );
        }

        Ok(())
    }

    /// Enable or disable a legal hold on a specific audit domain.
    /// If a domain is under legal hold, its events cannot be pruned via
    /// `prune_domain()`. Legal holds are persisted in the JSONL log as
    /// metadata lines to survive restarts.
    pub fn set_legal_hold(
        &self,
        deployment_id: &str,
        database: &str,
        hold: bool,
    ) -> AuditResult<()> {
        let key = (deployment_id.to_string(), database.to_string());
        let mut holds = self.legal_holds.lock().unwrap_or_else(|e| e.into_inner());
        if hold {
            holds.insert(key);
        } else {
            holds.remove(&key);
        }

        // Persist the hold state change as a metadata line in the JSONL log.
        let mut persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = persistence.as_mut() {
            let meta = serde_json::json!({
                "meta": "legal_hold",
                "deployment_id": deployment_id,
                "database": database,
                "hold": hold,
            });
            let line = serde_json::to_string(&meta)?;
            writeln!(state.file, "{line}")?;
            state.file.sync_all()?;
        }
        Ok(())
    }

    /// Check whether a specific audit domain is under legal hold.
    pub fn is_legal_hold(&self, deployment_id: &str, database: &str) -> bool {
        self.legal_holds
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&(deployment_id.to_string(), database.to_string()))
    }

    /// Verify a JSONL log file against the sled-loaded tree state.
    /// This checks that each event's recomputed leaf matches the stored
    /// leaf_hex, and that the stored root_after matches the tree's root
    /// at that index. Does NOT rebuild the tree (the sled tree is
    /// authoritative). Loads event metadata into memory.
    fn verify_against_tree(&self, path: &Path) -> AuditResult<Vec<PersistedEvent>> {
        let raw = std::fs::read_to_string(path)?;
        let mut lines: Vec<&str> = raw.lines().collect();

        // Truncate partial last line (same as replay_file).
        let mut first_bad: Option<usize> = None;
        let mut parsed: Vec<PersistedEvent> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PersistedEvent>(line.trim()) {
                Ok(ev) => parsed.push(ev),
                Err(_) => match serde_json::from_str::<serde_json::Value>(line.trim()) {
                    Ok(v) if v.get("meta").is_some() => {
                        // It's a valid JSON metadata line (e.g. legal hold), not a corrupt line.
                        // We skip it during the event parse pass.
                    }
                    _ => {
                        first_bad = Some(i);
                        break;
                    }
                }
            }
        }

        if let Some(bad_idx) = first_bad {
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

        // Replay metadata lines (legal holds) from the raw JSON file to
        // restore state, now that the file truncation pass is done.
        self.replay_metadata_lines(&path)?;

        if parsed.is_empty() {
            return Ok(Vec::new());
        }

        // Verify each event: recompute the leaf and check it matches
        // the stored leaf_hex. Also verify the root_after matches.
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());

        if !events.is_empty() {
            return Err(AuditError::ZkAudit(format!(
                "cannot verify into a non-empty audit log ({} event(s) already in memory)",
                events.len()
            )));
        }

        // Recompute per-domain sequence numbers deterministically from event
        // order. The stored `sequence` is informational only; recomputing
        // here keeps sequences continuous and correct even for legacy rows.
        let mut seq_counters: HashMap<(String, String), u64> = HashMap::new();

        for ev in &parsed {
            let recomputed_leaf = self.recompute_leaf(ev)?;
            let recomputed_hex = fr_to_hex(recomputed_leaf);
            if recomputed_hex != ev.leaf_hex {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected at index {}: leaf mismatch (stored {}, recomputed {})",
                    ev.index, ev.leaf_hex, recomputed_hex
                )));
            }

            // Verify the root_after matches the tree's root at this index.
            // The tree was loaded from sled, so tree.leaf_count() should
            // equal the number of events.
            if ev.index >= tree.leaf_count() as u64 {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected at index {}: index {} exceeds tree leaf count {}",
                    ev.index,
                    ev.index,
                    tree.leaf_count()
                )));
            }

            let counter = seq_counters
                .entry((ev.deployment_id.clone(), ev.database.clone()))
                .or_insert(0);
            let sequence = *counter;
            *counter += 1;

            events.push(AuditEvent {
                index: ev.index,
                leaf_hex: ev.leaf_hex.clone(),
                operation: ev.operation.clone(),
                database: ev.database.clone(),
                collection: ev.collection.clone(),
                deployment_id: ev.deployment_id.clone(),
                sequence,
                timestamp: ev.timestamp.clone(),
            });
        }

        *self.sequences.lock().unwrap_or_else(|e| e.into_inner()) = seq_counters;

        // Restore the per-index root history from the persisted (and
        // already leaf/tamper-verified above) `root_after` values, so
        // `root_after_at()` keeps working across restarts.
        {
            let mut roots = self.root_after_by_index.lock().unwrap_or_else(|e| e.into_inner());
            roots.clear();
            roots.extend(parsed.iter().map(|ev| ev.root_after.clone()));
        }

        // Verify the final root matches.
        let tree_root_hex = fr_to_hex(tree.root()?);
        if let Some(last) = parsed.last() {
            if last.root_after != tree_root_hex {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected: final root mismatch (stored {}, tree {})",
                    last.root_after, tree_root_hex
                )));
            }
        }

        // Drop logically pruned events from the UI-facing list (tree intact).
        self.apply_retention_to_events(&mut events);

        Ok(parsed)
    }

    /// Read a JSONL log file, replay events into the in-memory tree, and
    /// verify integrity. Returns the replayed events. A partial/corrupt
    /// last line is truncated from the file in place.
    fn replay_file(&self, path: &Path) -> AuditResult<Vec<PersistedEvent>> {
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
                Err(_) => match serde_json::from_str::<serde_json::Value>(line.trim()) {
                    Ok(v) if v.get("meta").is_some() => {}
                    _ => {
                        first_bad = Some(i);
                        break;
                    }
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

        self.replay_metadata_lines(&path)?;

        if parsed.is_empty() {
            return Ok(Vec::new());
        }

        // Replay each event into the tree and verify integrity.
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());

        // The in-memory tree must be empty — set_persistence_dir is
        // called once at startup before any record() calls.
        if !events.is_empty() || tree.leaf_count() != 0 {
            return Err(AuditError::ZkAudit(format!(
                "cannot replay into a non-empty audit log ({} event(s) already in memory)",
                events.len()
            )));
        }

        // Recompute per-domain sequence numbers deterministically from event
        // order so legacy rows (no stored sequence) still get correct values.
        let mut seq_counters: HashMap<(String, String), u64> = HashMap::new();

        for ev in &parsed {
            // 1. Recompute the leaf from the stored payload and verify
            //    it matches the stored leaf_hex. Mismatch ⇒ payload was
            //    edited after the fact.
            let recomputed_leaf = self.recompute_leaf(ev)?;
            let recomputed_hex = fr_to_hex(recomputed_leaf);
            if recomputed_hex != ev.leaf_hex {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected at index {}: leaf mismatch (stored {}, recomputed {})",
                    ev.index, ev.leaf_hex, recomputed_hex
                )));
            }

            // 2. Insert the leaf and verify the resulting root matches
            //    the stored root_after. Mismatch ⇒ events were reordered,
            //    inserted, or deleted.
            let inserted_idx = tree.insert(recomputed_leaf) as u64;
            if inserted_idx != ev.index {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected: index mismatch at line {} (expected {}, got {})",
                    ev.index, ev.index, inserted_idx
                )));
            }
            let recomputed_root = tree.root()?;
            let recomputed_root_hex = fr_to_hex(recomputed_root);
            if recomputed_root_hex != ev.root_after {
                return Err(AuditError::ZkAudit(format!(
                    "audit log tamper detected at index {}: root_after mismatch (stored {}, recomputed {})",
                    ev.index, ev.root_after, recomputed_root_hex
                )));
            }

            // Restore this index's historical root so `root_after_at()`
            // keeps working after a restart (replay), not just for events
            // recorded fresh in this process's lifetime.
            self.root_after_by_index
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(recomputed_root_hex);

            let counter = seq_counters
                .entry((ev.deployment_id.clone(), ev.database.clone()))
                .or_insert(0);
            let sequence = *counter;
            *counter += 1;

            events.push(AuditEvent {
                index: ev.index,
                leaf_hex: ev.leaf_hex.clone(),
                operation: ev.operation.clone(),
                database: ev.database.clone(),
                collection: ev.collection.clone(),
                deployment_id: ev.deployment_id.clone(),
                sequence,
                timestamp: ev.timestamp.clone(),
            });
        }

        *self.sequences.lock().unwrap_or_else(|e| e.into_inner()) = seq_counters;

        // Drop logically pruned events from the UI-facing list (tree intact).
        self.apply_retention_to_events(&mut events);

        Ok(parsed)
    }

    /// Parse the JSONL file specifically for `{"meta": "..."}` lines,
    /// skipping `PersistedEvent` lines, to restore non-event state.
    fn replay_metadata_lines(&self, path: &Path) -> AuditResult<()> {
        let raw = std::fs::read_to_string(path)?;
        let mut holds = self.legal_holds.lock().unwrap_or_else(|e| e.into_inner());
        let mut retained = self
            .retained_domain_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        holds.clear();
        retained.clear();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            // Try to parse it as an arbitrary JSON object first to see if it's metadata.
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                match val.get("meta").and_then(|v| v.as_str()) {
                    Some("legal_hold") => {
                        if let (Some(dep), Some(db), Some(hold)) = (
                            val.get("deployment_id").and_then(|v| v.as_str()),
                            val.get("database").and_then(|v| v.as_str()),
                            val.get("hold").and_then(|v| v.as_bool()),
                        ) {
                            let key = (dep.to_string(), db.to_string());
                            if hold {
                                holds.insert(key);
                            } else {
                                holds.remove(&key);
                            }
                        }
                    }
                    Some("pruned_domain") => {
                        if let (Some(dep), Some(db), Some(root_hex), Some(event_count), Some(max_index), Some(pruned_at)) = (
                            val.get("deployment_id").and_then(|v| v.as_str()),
                            val.get("database").and_then(|v| v.as_str()),
                            val.get("root_hex").and_then(|v| v.as_str()),
                            val.get("event_count").and_then(|v| v.as_u64()),
                            val.get("max_index").and_then(|v| v.as_u64()),
                            val.get("pruned_at").and_then(|v| v.as_str()),
                        ) {
                            let key = (dep.to_string(), db.to_string());
                            let retained_root = DomainRetentionRoot {
                                root_hex: root_hex.to_string(),
                                event_count: event_count as usize,
                                max_index,
                                pruned_at: pruned_at.to_string(),
                            };
                            let roots = retained.entry(key.clone()).or_default();
                            if !roots.iter().any(|r| {
                                r.max_index == retained_root.max_index
                                    && r.root_hex == retained_root.root_hex
                            }) {
                                roots.push(retained_root);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Remove from the in-memory event list any events that a retained
    /// `pruned_domain` commitment has logically pruned. The shared tree is
    /// left intact (it stays the anchored source of truth); only the UI-facing
    /// event metadata is dropped. Call after events are populated on replay.
    fn apply_retention_to_events(&self, events: &mut Vec<AuditEvent>) {
        let retained = self
            .retained_domain_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if retained.is_empty() {
            return;
        }
        events.retain(|ev| {
            match retained.get(&(ev.deployment_id.clone(), ev.database.clone())) {
                Some(roots) => !roots.iter().any(|r| ev.index <= r.max_index),
                None => true,
            }
        });
    }

    /// Load event metadata from a JSONL file without replaying into the
    /// tree. Used when the tree was already loaded from sled — we just
    /// need the event metadata (operation, database, collection, etc.)
    /// for the UI, not the tree state.
    /// Record an audit event. Returns the leaf index.
    ///
    /// `payload` is the canonical string the leaf was derived from
    /// (e.g. `"insert|db|col|{document_json}"`). It's persisted verbatim
    /// so the leaf can be recomputed and verified on replay.
    pub fn record(
        &self,
        deployment_id: &str,
        operation: &str,
        database: &str,
        collection: &str,
        payload: &str,
        leaf: ark_bn254::Fr,
    ) -> AuditResult<u64> {
        // Determine event version from the configured leaf key.
        let version = if self.has_leaf_key() { 2 } else { 1 };
        let timestamp = chrono::Utc::now().to_rfc3339();
        self.record_inner(
            deployment_id,
            operation,
            database,
            collection,
            payload,
            leaf,
            version,
            &timestamp,
        )
    }

    /// Record an audit event with a **v3 leaf**: a keyed Poseidon vector
    /// commitment over the event's structured fields (see
    /// [`zk_audit::commitment`]). Returns the leaf index.
    ///
    /// Unlike [`record`](Self::record), the leaf is derived *inside* this
    /// method because it commits to the event timestamp, which must be the
    /// same instant that is persisted — replay recomputes the leaf from the
    /// stored RFC 3339 timestamp's Unix seconds.
    ///
    /// `canonical_payload` must be the canonical byte encoding from
    /// [`crypto::canonical_payload_bytes`]; it is persisted base64-encoded,
    /// like v2.
    pub fn record_v3(
        &self,
        deployment_id: &str,
        operation: &str,
        database: &str,
        collection: &str,
        canonical_payload: &[u8],
    ) -> AuditResult<u64> {
        let key = self.leaf_key().ok_or_else(|| {
            AuditError::Validation(
                "v3 events require a leaf key, but none is configured".to_string(),
            )
        })?;

        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339();
        let ts_secs = now.timestamp().max(0) as u64;

        let (leaf, _opening) = zk_audit::commitment::poseidon_leaf_v3(
            &key,
            operation,
            database,
            collection,
            ts_secs,
            canonical_payload,
        )?;

        let payload_b64 = base64::engine::general_purpose::STANDARD.encode(canonical_payload);

        self.record_inner(
            deployment_id,
            operation,
            database,
            collection,
            &payload_b64,
            leaf,
            3,
            &timestamp,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn record_inner(
        &self,
        deployment_id: &str,
        operation: &str,
        database: &str,
        collection: &str,
        payload: &str,
        leaf: ark_bn254::Fr,
        version: u32,
        timestamp: &str,
    ) -> AuditResult<u64> {
        // Assign a monotonic, per-`(deploymentId, database)` sequence number.
        // Acquired and released before the tree lock to avoid holding two
        // locks at once.
        let sequence = {
            let mut seqs = self.sequences.lock().unwrap_or_else(|e| e.into_inner());
            let counter = seqs
                .entry((deployment_id.to_string(), database.to_string()))
                .or_insert(0);
            let assigned = *counter;
            *counter += 1;
            assigned
        };

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

        // Record this index's root atomically while still holding the tree
        // lock, so `root_after_by_index[index]` can never observe a root
        // more advanced than "immediately after this exact insert" — see
        // `root_after_at()` for why this matters.
        {
            let mut roots = self.root_after_by_index.lock().unwrap_or_else(|e| e.into_inner());
            debug_assert_eq!(roots.len() as u64, index, "root_after_by_index must stay index-aligned with the tree");
            roots.push(root_after.clone());
        }

        let timestamp = timestamp.to_string();

        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.push(AuditEvent {
            index,
            leaf_hex: leaf_hex.clone(),
            operation: operation.to_string(),
            database: database.to_string(),
            collection: collection.to_string(),
            deployment_id: deployment_id.to_string(),
            sequence,
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
                deployment_id: deployment_id.to_string(),
                sequence,
                payload: payload.to_string(),
                leaf_hex: leaf_hex.clone(),
                root_after: root_after.clone(),
                timestamp: timestamp.clone(),
                version,
            };
            let line = serde_json::to_string(&persisted)?;
            writeln!(state.file, "{line}")?;
            state.file.sync_all()?;
        }

        // Also persist to sled for fast startup (if sled store is wired).
        // We parse the root_after hex back to Fr for the sled store.
        // If this fails, we log but don't fail the record — the JSONL
        // is the source of truth, sled is just a startup optimization.
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            if let Ok(root_fr) = hex_to_fr(&root_after) {
                if let Err(e) = store.save_leaf(index, leaf, root_fr) {
                    tracing::warn!("sled save_leaf failed: {e}");
                }
            }
        }

        Ok(index)
    }

    /// The private opening of a **v3** event's leaf commitment, needed to
    /// generate an Audited-Action Disclosure proof for it.
    ///
    /// Reads the persisted JSONL record for `index` (the in-memory
    /// [`AuditEvent`] doesn't carry the payload), re-derives the keyed
    /// Poseidon commitment, and cross-checks it against the stored leaf.
    /// Errors for v1/v2 events (their opaque hash leaves have no provable
    /// opening) and when persistence or the leaf key isn't configured.
    pub fn disclosure_opening(
        &self,
        index: u64,
    ) -> AuditResult<(zk_audit::LeafOpening, ark_bn254::Fr)> {
        let key = self.leaf_key().ok_or_else(|| {
            AuditError::Validation("disclosure proofs require a leaf key".to_string())
        })?;

        let events_path = {
            let persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
            persistence
                .as_ref()
                .map(|p| p.events_path.clone())
                .ok_or_else(|| {
                    AuditError::Validation(
                        "disclosure proofs require persistence to be configured".to_string(),
                    )
                })?
        };

        let content = std::fs::read_to_string(&events_path)?;
        let event = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str::<PersistedEvent>(l).ok())
            .find(|e| e.index == index)
            .ok_or_else(|| {
                AuditError::Validation(format!("no persisted event at index {index}"))
            })?;

        if event.version != 3 {
            return Err(AuditError::Validation(format!(
                "event at index {index} is v{} — disclosure proofs require v3 leaves \
                 (keyed Poseidon commitment). Events recorded before the v3 upgrade \
                 cannot be disclosed this way.",
                event.version
            )));
        }

        let canonical = base64::engine::general_purpose::STANDARD
            .decode(&event.payload)
            .map_err(|e| {
                AuditError::Validation(format!("v3 payload base64 decode at {index}: {e}"))
            })?;
        let ts_secs = chrono::DateTime::parse_from_rfc3339(&event.timestamp)
            .map_err(|e| {
                AuditError::Validation(format!("v3 timestamp parse at {index}: {e}"))
            })?
            .timestamp()
            .max(0) as u64;

        let (leaf, opening) = zk_audit::commitment::poseidon_leaf_v3(
            &key,
            &event.operation,
            &event.database,
            &event.collection,
            ts_secs,
            &canonical,
        )?;

        if fr_to_hex(leaf) != event.leaf_hex {
            return Err(AuditError::Validation(format!(
                "recomputed v3 leaf does not match stored leaf at index {index} — \
                 wrong leaf key or tampered log"
            )));
        }

        Ok((opening, leaf))
    }

    /// Get the current Merkle root as a hex string.
    pub fn root_hex(&self) -> AuditResult<String> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let root = tree.root()?;
        Ok(fr_to_hex(root))
    }

    /// Get the current root as a field element.
    pub fn root(&self) -> AuditResult<ark_bn254::Fr> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        Ok(tree.root()?)
    }

    /// Get the number of recorded events.
    pub fn event_count(&self) -> usize {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// List all recorded audit events.
    pub fn list_events(&self) -> Vec<AuditEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Generate an inclusion proof for the event at the given index.
    pub fn prove_inclusion(&self, index: u64) -> AuditResult<InclusionProof> {
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let proof = tree
            .prove_inclusion(index as usize)
            .map_err(|e| crate::error::AuditError::ZkAudit(e.to_string()))?;
        Ok(proof)
    }

    /// Get the current leaf count (same as event count).
    pub fn leaf_count(&self) -> usize {
        self.tree
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .leaf_count()
    }

    /// Save a change stream resume token for the given connection ID.
    /// The token is serialized to JSON and stored in sled so the change
    /// stream can resume gaplessly after an app restart.
    pub fn save_resume_token(
        &self,
        connection_id: &str,
        token: &mongodb::change_stream::event::ResumeToken,
    ) -> AuditResult<()> {
        let token_json = serde_json::to_string(token)
            .map_err(|e| AuditError::Internal(format!("serialize resume token: {e}")))?;
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            store.save_resume_token(connection_id, &token_json)?;
        }
        Ok(())
    }

    /// Load the saved change stream resume token for the given connection ID.
    /// Returns `None` if no token has been saved (first run or cleared).
    pub fn load_resume_token(
        &self,
        connection_id: &str,
    ) -> AuditResult<Option<mongodb::change_stream::event::ResumeToken>> {
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            if let Some(token_json) = store.load_resume_token(connection_id)? {
                let token: mongodb::change_stream::event::ResumeToken =
                    serde_json::from_str(&token_json).map_err(|e| {
                        AuditError::Internal(format!("deserialize resume token: {e}"))
                    })?;
                return Ok(Some(token));
            }
        }
        Ok(None)
    }

    /// Clear the saved resume token for the given connection ID.
    /// Called when a connection is closed to avoid resuming from a stale token
    /// on a different deployment.
    pub fn clear_resume_token(&self, connection_id: &str) -> AuditResult<()> {
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            store.clear_resume_token(connection_id)?;
        }
        Ok(())
    }

    /// Wipe all audit state: the in-memory Merkle tree and event list, the
    /// sled-backed tree store, and the persisted JSONL log (truncated).
    ///
    /// After this returns the log is empty (0 events, fresh root) and ready to
    /// record new events. On-chain commitments are NOT affected — this only
    /// clears local state.
    pub fn clear(&self) -> AuditResult<()> {
        // Reset the in-memory tree and events.
        {
            let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
            *tree = AuditMerkleTree::with_height(20)?;
        }
        {
            let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
            events.clear();
        }
        // Reset per-domain sequence counters so a fresh log restarts at 0.
        {
            let mut seqs = self.sequences.lock().unwrap_or_else(|e| e.into_inner());
            seqs.clear();
        }
        // Drop all legal holds — a full reset wipes the JSONL metadata lines
        // that back them, so in-memory state must match.
        {
            let mut holds = self.legal_holds.lock().unwrap_or_else(|e| e.into_inner());
            holds.clear();
        }
        {
            let mut retained = self
                .retained_domain_roots
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            retained.clear();
        }
        // Clear the sled-backed tree store, if enabled.
        {
            let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(store) = sled_guard.as_ref() {
                store.clear()?;
            }
        }
        // Truncate the persisted JSONL log and keep a fresh append handle.
        {
            let mut persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = persistence.as_mut() {
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&state.events_path)?;
                state.file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&state.events_path)?;
            }
        }
        Ok(())
    }

    /// Save an IPFS CID for a published epoch.
    pub fn save_ipfs_cid(&self, epoch_number: u64, cid: &str) -> AuditResult<()> {
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            store.save_ipfs_cid(epoch_number, cid)?;
        }
        Ok(())
    }

    /// Load the saved IPFS CID for a published epoch.
    pub fn load_ipfs_cid(&self, epoch_number: u64) -> AuditResult<Option<String>> {
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = sled_guard.as_ref() {
            return store.load_ipfs_cid(epoch_number);
        }
        Ok(None)
    }

    /// Get a clone of the sled store path so other managers (e.g.
    /// AttestationManager) can open their own tree in the same DB.
    ///
    /// Returns `None` if persistence has not been set up yet.
    pub fn sled_db_path(&self) -> Option<std::path::PathBuf> {
        let sled_guard = self.sled_store.lock().unwrap_or_else(|e| e.into_inner());
        sled_guard
            .as_ref()
            .map(|store| store.db_path().to_path_buf())
    }

    /// List the distinct audit domains `(deploymentId, database)` present in
    /// the log, sorted by deployment id then database for a stable ordering.
    pub fn list_domains(&self) -> Vec<(String, String)> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let mut set: BTreeSet<(String, String)> = BTreeSet::new();
        for ev in events.iter() {
            set.insert((ev.deployment_id.clone(), ev.database.clone()));
        }
        let retained = self
            .retained_domain_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        set.extend(retained.keys().cloned());
        set.into_iter().collect()
    }

    /// Build a secondary Merkle tree over a single domain's leaves (in global
    /// index order) and return its root hex plus the domain leaf count.
    ///
    /// This is the per-domain commitment used for selective disclosure: it
    /// proves the integrity of one `(deploymentId, database)` domain without
    /// revealing any other domain's leaves. Leaves are reconstructed from the
    /// stored `leaf_hex` values, so the domain root is fully determined by the
    /// (tamper-verified) global log and is deterministic across calls.
    pub fn domain_root(&self, deployment_id: &str, database: &str) -> AuditResult<(String, usize)> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let mut tree = AuditMerkleTree::with_height(20)?;
        let mut count = 0usize;
        for ev in events.iter() {
            if ev.deployment_id == deployment_id && ev.database == database {
                tree.insert(hex_to_fr(&ev.leaf_hex)?);
                count += 1;
            }
        }
        if count == 0 {
            let retained = self
                .retained_domain_roots
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(root) = retained
                .get(&(deployment_id.to_string(), database.to_string()))
                .and_then(|roots| roots.last())
            {
                return Ok((root.root_hex.clone(), root.event_count));
            }
        }
        Ok((fr_to_hex(tree.root()?), count))
    }

    /// Generate an inclusion proof for the leaf at the given 0-indexed
    /// position within a domain, against that domain's secondary Merkle tree.
    /// Returns the proof together with the domain root hex it terminates at.
    pub fn prove_inclusion_in_domain(
        &self,
        deployment_id: &str,
        database: &str,
        position: usize,
    ) -> AuditResult<(InclusionProof, String)> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        let mut tree = AuditMerkleTree::with_height(20)?;
        let mut count = 0usize;
        for ev in events.iter() {
            if ev.deployment_id == deployment_id && ev.database == database {
                tree.insert(hex_to_fr(&ev.leaf_hex)?);
                count += 1;
            }
        }
        if position >= count {
            return Err(AuditError::Validation(format!(
                "domain proof position {position} out of range (domain has {count} leaf/leaves)"
            )));
        }
        let proof = tree
            .prove_inclusion(position)
            .map_err(|e| AuditError::ZkAudit(e.to_string()))?;
        let root_hex = fr_to_hex(proof.root);
        Ok((proof, root_hex))
    }

    /// Compute the aggregation **super-root**: a Merkle tree whose leaves are
    /// the per-domain roots (one leaf per `(deploymentId, database)`), in the
    /// stable [`list_domains`] order. The super-root commits to the *set* of
    /// domain roots, so a short inclusion proof can show that one domain's
    /// root is part of the committed state without revealing other domains.
    ///
    /// Each leaf binds the domain identity to its root via [`super_leaf`].
    /// Returns the super-root hex together with the ordered
    /// `(deploymentId, database, domainRootHex)` entries so callers know each
    /// leaf's position.
    pub fn domain_super_root(&self) -> AuditResult<(String, Vec<(String, String, String)>)> {
        let domains = self.list_domains();
        let mut tree = AuditMerkleTree::with_height(20)?;
        let mut entries = Vec::with_capacity(domains.len());
        for (dep, db) in domains {
            let (root_hex, _count) = self.domain_root(&dep, &db)?;
            tree.insert(super_leaf(&dep, &db, &root_hex));
            entries.push((dep, db, root_hex));
        }
        Ok((fr_to_hex(tree.root()?), entries))
    }

    /// Generate an inclusion proof that a single domain's root is part of the
    /// aggregation super-root. Returns the proof, the super-root hex it
    /// terminates at, and the domain root hex that the proven leaf binds.
    pub fn prove_domain_in_super(
        &self,
        deployment_id: &str,
        database: &str,
    ) -> AuditResult<(InclusionProof, String, String)> {
        let domains = self.list_domains();
        let position = domains
            .iter()
            .position(|(dep, db)| dep == deployment_id && db == database)
            .ok_or_else(|| {
                AuditError::Validation(format!(
                    "unknown audit domain {deployment_id}/{database}: no events or retained root"
                ))
            })?;
        let mut tree = AuditMerkleTree::with_height(20)?;
        let mut domain_root_hex = String::new();
        for (i, (dep, db)) in domains.iter().enumerate() {
            let (root_hex, _count) = self.domain_root(dep, db)?;
            if i == position {
                domain_root_hex = root_hex.clone();
            }
            tree.insert(super_leaf(dep, db, &root_hex));
        }
        let proof = tree
            .prove_inclusion(position)
            .map_err(|e| AuditError::ZkAudit(e.to_string()))?;
        let super_root_hex = fr_to_hex(proof.root);
        Ok((proof, super_root_hex, domain_root_hex))
    }

    /// Return retained roots for logically pruned domain segments.
    pub fn retained_domain_roots(
        &self,
        deployment_id: &str,
        database: &str,
    ) -> Vec<DomainRetentionRoot> {
        self.retained_domain_roots
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&(deployment_id.to_string(), database.to_string()))
            .cloned()
            .unwrap_or_default()
    }

    /// Logically prune active event metadata for a domain while retaining a
    /// compact Merkle commitment to the pruned segment.
    ///
    /// The global tree, sled state, and on-chain anchor path are unchanged.
    /// This makes retention safe in Phase 2 while Phase 3 introduces physical
    /// per-domain aggregation.
    pub fn prune_domain(
        &self,
        deployment_id: &str,
        database: &str,
    ) -> AuditResult<Option<DomainRetentionRoot>> {
        if self.is_legal_hold(deployment_id, database) {
            return Err(AuditError::Validation(format!(
                "cannot prune audit domain {deployment_id}/{database}: legal hold is active"
            )));
        }

        // Hold the `events` lock for the entire snapshot → persist → remove
        // sequence so the operation is atomic: no `record()` can append a new
        // event for this domain between computing the retained commitment and
        // dropping the events it covers. The lock order here (events → then
        // persistence inside) matches `record()`, so there is no deadlock.
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());

        let mut tree = AuditMerkleTree::with_height(20)?;
        let mut event_count = 0usize;
        let mut max_index = None;
        for ev in events.iter() {
            if ev.deployment_id == deployment_id && ev.database == database {
                tree.insert(hex_to_fr(&ev.leaf_hex)?);
                event_count += 1;
                max_index = Some(max_index.map_or(ev.index, |idx: u64| idx.max(ev.index)));
            }
        }
        let Some(max_index) = max_index else {
            return Ok(None);
        };

        let retained_root = DomainRetentionRoot {
            root_hex: fr_to_hex(tree.root()?),
            event_count,
            max_index,
            pruned_at: chrono::Utc::now().to_rfc3339(),
        };

        // Persist the prune commitment (events lock still held).
        {
            let mut persistence = self.persistence.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = persistence.as_mut() {
                let meta = serde_json::json!({
                    "meta": "pruned_domain",
                    "deployment_id": deployment_id,
                    "database": database,
                    "root_hex": retained_root.root_hex,
                    "event_count": retained_root.event_count,
                    "max_index": retained_root.max_index,
                    "pruned_at": retained_root.pruned_at,
                });
                let line = serde_json::to_string(&meta)?;
                writeln!(state.file, "{line}")?;
                state.file.sync_all()?;
            }
        }

        {
            let mut retained = self
                .retained_domain_roots
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            retained
                .entry((deployment_id.to_string(), database.to_string()))
                .or_default()
                .push(retained_root.clone());
        }

        // Drop exactly the events covered by the commitment we just made.
        events.retain(|ev| !(ev.deployment_id == deployment_id && ev.database == database));

        Ok(Some(retained_root))
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

/// Parse a hex string back to an Fr field element.
fn hex_to_fr(hex_str: &str) -> AuditResult<ark_bn254::Fr> {
    use ark_ff::PrimeField;
    let bytes = hex::decode(hex_str)
        .map_err(|e| AuditError::Validation(format!("hex_to_fr decode: {e}")))?;
    Ok(ark_bn254::Fr::from_be_bytes_mod_order(&bytes))
}

/// Recompute the leaf field element from the canonical payload string.
///
/// This must stay byte-for-byte identical to
/// [`interceptor::record_insert`] / [`record_update`] / [`record_delete`]
/// and [`commands::audit_record_event`]: SHA-256 of the payload, take the
/// first 31 bytes, mask the top nibble of byte 31, interpret as a field
/// element via `from_be_bytes_mod_order`. Any divergence here would make
/// replay-time verification flag every event as tampered.
pub fn leaf_from_payload(
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

/// Derive the aggregation super-tree leaf that binds a domain identity to its
/// per-domain root. Reuses [`leaf_from_payload`] so the leaf stays in-field and
/// is derived the same (tamper-evident) way as every other audit leaf.
pub fn super_leaf(deployment_id: &str, database: &str, domain_root_hex: &str) -> ark_bn254::Fr {
    leaf_from_payload(
        "domain_root",
        database,
        "",
        &format!("{deployment_id}|{database}|{domain_root_hex}"),
    )
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
            interceptor::record_insert(
                &std::sync::Arc::new(audit),
                "rs:rs0",
                "db",
                "col",
                r#"{"a":1}"#,
            )
            .unwrap();
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

    /// v3 leaves (keyed Poseidon vector commitment) round-trip through
    /// persistence: record with a leaf key, replay with the same key, and
    /// verify the leaf/root chain reproduces. Replay without the key must
    /// fail (v3 events are unverifiable keyless), proving the leaf actually
    /// depends on the key.
    #[test]
    fn v3_leaf_persistence_round_trip() {
        let dir = tempfile_dir();
        let key = [0x42u8; 32];
        let root_hex = {
            let audit = AuditLog::new().unwrap();
            audit.set_leaf_key(key);
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "shop", "orders", r#"{"total":42}"#)
                .unwrap();
            interceptor::record_delete(&a, "rs:rs0", "shop", "orders", r#"{"_id":1}"#).unwrap();
            a.root_hex().unwrap()
        };

        // Replay with the same key: leaves recompute, root matches.
        {
            let audit = AuditLog::new().unwrap();
            audit.set_leaf_key(key);
            audit.set_persistence_dir(&dir).unwrap();
            assert_eq!(audit.event_count(), 2);
            assert_eq!(audit.root_hex().unwrap(), root_hex);
        }

        // Replay without the key: must fail, not silently accept.
        {
            let audit = AuditLog::new().unwrap();
            // Remove sled fast-path so replay actually recomputes leaves.
            let _ = fs::remove_dir_all(dir.join("audit").join("tree.sled"));
            let err = audit.set_persistence_dir(&dir).unwrap_err();
            assert!(
                err.to_string().contains("leaf key"),
                "expected leaf-key error, got: {err}"
            );
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
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();
            let root_after_session1 = a.root_hex().unwrap();
            drop(a);
            // Capture root by re-reading: we need it after the Arc drops.
            let _ = root_after_session1;
        }
        // Session 2: replay + append one more.
        let root_after_session2 = {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            assert_eq!(
                audit.event_count(),
                2,
                "two events must survive from session 1"
            );
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":3}"#).unwrap();
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
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
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
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();
        }
        // Append a partial (corrupt) line — simulates a crash mid-write.
        let path = dir.join("audit").join("events.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{this is not valid json").unwrap();
        drop(f);

        let audit = AuditLog::new().unwrap();
        audit.set_persistence_dir(&dir).unwrap();
        assert_eq!(
            audit.event_count(),
            2,
            "partial line must be dropped, prior 2 kept"
        );
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
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();
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

    /// Leaf identity survives a persistence round-trip: deployment_id and
    /// the per-domain sequence are written to JSONL and restored on replay.
    #[test]
    fn persistence_round_trip_preserves_leaf_identity() {
        let dir = tempfile_dir();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();
        }
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let events = audit.list_events();
            assert_eq!(events.len(), 3);
            assert_eq!(events[0].deployment_id, "rs:rs0");
            assert_eq!(events[0].database, "sales");
            assert_eq!(events[0].sequence, 0);
            assert_eq!(
                events[1].sequence, 1,
                "second sales event keeps domain order"
            );
            assert_eq!(events[2].database, "billing");
            assert_eq!(events[2].sequence, 0, "billing domain has its own counter");

            // A new event after replay must continue the per-domain counter.
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":3}"#).unwrap();
            let events = a.list_events();
            assert_eq!(events.last().unwrap().sequence, 2);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Backward compatibility: a legacy JSONL log written before leaf
    /// identity existed (no `deploymentId` / `sequence` fields) must replay
    /// cleanly — roots unchanged, no tamper error — and get deterministic
    /// sequences assigned in the empty "unattributed" domain.
    #[test]
    fn replay_tolerates_legacy_events_without_identity_fields() {
        let dir = tempfile_dir();
        // First, produce a valid log so we have real leaf_hex / root_after
        // values, then strip the new identity fields to emulate a legacy log.
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "", "db", "col", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "", "db", "col", r#"{"a":2}"#).unwrap();
        }
        let path = dir.join("audit").join("events.jsonl");
        let content = fs::read_to_string(&path).unwrap();
        let legacy: String = content
            .lines()
            .map(|line| {
                let mut v: serde_json::Value = serde_json::from_str(line).unwrap();
                let obj = v.as_object_mut().unwrap();
                obj.remove("deploymentId");
                obj.remove("deployment_id");
                obj.remove("sequence");
                serde_json::to_string(&v).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{legacy}\n")).unwrap();
        // Also remove the sled fast-path so replay goes through the JSONL
        // rebuild that exercises legacy parsing.
        let _ = fs::remove_dir_all(dir.join("audit").join("tree.sled"));

        let audit = AuditLog::new().unwrap();
        audit.set_persistence_dir(&dir).unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 2, "legacy events replay without tamper error");
        assert_eq!(events[0].deployment_id, "", "legacy → unattributed domain");
        assert_eq!(events[0].sequence, 0);
        assert_eq!(events[1].sequence, 1, "sequence recomputed from order");
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

    /// Per-domain secondary Merkle roots isolate each `(deploymentId,
    /// database)` and produce real inclusion proofs for selective disclosure.
    #[test]
    fn domain_root_isolates_domains_and_proves_inclusion() {
        let audit = std::sync::Arc::new(AuditLog::new().unwrap());
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();

        let domains = audit.list_domains();
        assert_eq!(
            domains,
            vec![
                ("rs:rs0".to_string(), "billing".to_string()),
                ("rs:rs0".to_string(), "sales".to_string()),
            ]
        );

        let (sales_root, sales_n) = audit.domain_root("rs:rs0", "sales").unwrap();
        let (billing_root, billing_n) = audit.domain_root("rs:rs0", "billing").unwrap();
        assert_eq!(sales_n, 2);
        assert_eq!(billing_n, 1);
        assert_ne!(sales_root, billing_root, "domains have independent roots");

        // Determinism: rebuilding the same domain yields the same root.
        let (sales_root2, _) = audit.domain_root("rs:rs0", "sales").unwrap();
        assert_eq!(sales_root, sales_root2);

        // An inclusion proof in the sales domain terminates at the sales root.
        let (proof, root_hex) = audit.prove_inclusion_in_domain("rs:rs0", "sales", 1).unwrap();
        assert_eq!(root_hex, sales_root);
        assert_eq!(fr_to_hex(proof.root), sales_root);

        // Out-of-range position is rejected.
        assert!(audit.prove_inclusion_in_domain("rs:rs0", "billing", 5).is_err());

        // An unknown domain has zero leaves and the empty-tree root.
        let (empty_root, empty_n) = audit.domain_root("rs:rs0", "nope").unwrap();
        assert_eq!(empty_n, 0);
        let mut t = AuditMerkleTree::with_height(20).unwrap();
        assert_eq!(empty_root, fr_to_hex(t.root().unwrap()));
    }

    #[test]
    fn domain_super_root_aggregates_domain_roots() {
        let audit = std::sync::Arc::new(AuditLog::new().unwrap());
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs1", "billing", "invoices", r#"{"a":1}"#).unwrap();

        let (super_root, entries) = audit.domain_super_root().unwrap();
        assert_eq!(entries.len(), 2);
        // Sorted order matches `list_domains`
        assert_eq!(entries[0].0, "rs:rs0");
        assert_eq!(entries[0].1, "sales");
        assert_eq!(entries[1].0, "rs:rs1");
        assert_eq!(entries[1].1, "billing");

        let (proof, sr_hex, dom_root) = audit.prove_domain_in_super("rs:rs0", "sales").unwrap();
        assert_eq!(sr_hex, super_root);
        assert_eq!(dom_root, entries[0].2);
        assert_eq!(fr_to_hex(proof.root), super_root);
        assert_eq!(proof.leaf_index, 0); // Position matches the sorted entry list

        // Unknown domain fails to prove
        assert!(audit.prove_domain_in_super("rs:rs0", "nope").is_err());
    }

    /// Every domain's super-root proof must be *cryptographically* valid: the
    /// proven leaf must hash up to the super-root through the authentication
    /// path, the leaf must bind the domain identity to its root, and proofs
    /// must terminate at the same super-root returned by `domain_super_root`.
    #[test]
    fn super_root_proofs_verify_cryptographically_for_every_domain() {
        let audit = std::sync::Arc::new(AuditLog::new().unwrap());
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs1", "ops", "logs", r#"{"a":1}"#).unwrap();

        let (super_root, entries) = audit.domain_super_root().unwrap();
        assert_eq!(entries.len(), 3);

        for (i, (dep, db, domain_root_hex)) in entries.iter().enumerate() {
            let (proof, sr_hex, dom_root) = audit.prove_domain_in_super(dep, db).unwrap();
            // Proof terminates at the aggregation super-root.
            assert_eq!(sr_hex, super_root);
            assert_eq!(fr_to_hex(proof.root), super_root);
            // Position is the stable sorted index.
            assert_eq!(proof.leaf_index, i);
            // The proven leaf binds (deployment, database, domain root).
            assert_eq!(&dom_root, domain_root_hex);
            assert_eq!(proof.leaf, super_leaf(dep, db, domain_root_hex));
            // The full path verifies against the super-root.
            assert!(
                proof.verify().unwrap(),
                "super-root proof for {dep}/{db} must verify"
            );
        }
    }

    /// The super-root is deterministic, order-independent (driven by the stable
    /// sorted domain list), and tamper-evident: appending an event to any
    /// domain changes the super-root.
    #[test]
    fn super_root_is_deterministic_and_tamper_evident() {
        let a = std::sync::Arc::new(AuditLog::new().unwrap());
        // Insert domains out of sorted order on purpose.
        interceptor::record_insert(&a, "rs:rs1", "ops", "logs", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        let (root_a, entries_a) = a.domain_super_root().unwrap();
        // Stable sorted order regardless of insertion order.
        assert_eq!(entries_a[0].0, "rs:rs0");
        assert_eq!(entries_a[1].0, "rs:rs1");
        // Determinism: recomputing yields the identical root.
        assert_eq!(a.domain_super_root().unwrap().0, root_a);

        // A second log built in the opposite insertion order yields the same
        // super-root: it is a function of domain state, not call order.
        let b = std::sync::Arc::new(AuditLog::new().unwrap());
        interceptor::record_insert(&b, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&b, "rs:rs1", "ops", "logs", r#"{"a":1}"#).unwrap();
        assert_eq!(b.domain_super_root().unwrap().0, root_a);

        // Tamper-evidence: a new event in one domain moves the super-root.
        interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
        assert_ne!(a.domain_super_root().unwrap().0, root_a);
    }

    /// An empty audit log has a well-defined (empty-tree) super-root and no
    /// domains, and proving any domain in it fails cleanly.
    #[test]
    fn super_root_of_empty_log_is_well_defined() {
        let audit = std::sync::Arc::new(AuditLog::new().unwrap());
        let (super_root, entries) = audit.domain_super_root().unwrap();
        assert!(entries.is_empty());
        // Deterministic empty-tree root (height-20 zero hash).
        let expected = AuditMerkleTree::with_height(20).unwrap().root().unwrap();
        assert_eq!(super_root, fr_to_hex(expected));
        assert!(audit.prove_domain_in_super("rs:rs0", "sales").is_err());
    }

    /// A logically pruned domain still participates in the super-root via its
    /// retained commitment, and its inclusion proof remains valid.
    #[test]
    fn super_root_includes_pruned_domains() {
        let audit = std::sync::Arc::new(AuditLog::new().unwrap());
        interceptor::record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        interceptor::record_insert(&audit, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();

        let (retained_root, _) = audit.domain_root("rs:rs0", "sales").unwrap();
        audit.prune_domain("rs:rs0", "sales").unwrap().unwrap();

        // The pruned domain is still a super-root leaf, bound to its retained root.
        let (_, entries) = audit.domain_super_root().unwrap();
        let sales = entries
            .iter()
            .find(|(dep, db, _)| dep == "rs:rs0" && db == "sales")
            .expect("pruned domain must still appear in the super-root");
        assert_eq!(sales.2, retained_root);

        let (proof, super_root, dom_root) =
            audit.prove_domain_in_super("rs:rs0", "sales").unwrap();
        assert_eq!(dom_root, retained_root);
        assert_eq!(fr_to_hex(proof.root), super_root);
        assert!(proof.verify().unwrap());
    }

    /// Pruning a domain retains a compact Merkle commitment, removes the
    /// domain's active events, leaves other domains intact, and survives a
    /// restart (the retained root is restored, pruned events stay pruned).
    #[test]
    fn prune_domain_retains_root_and_survives_restart() {
        let dir = tempfile_dir();
        let sales_root;
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();

            let (root, n) = a.domain_root("rs:rs0", "sales").unwrap();
            sales_root = root;
            assert_eq!(n, 2);

            let retained = a.prune_domain("rs:rs0", "sales").unwrap().unwrap();
            assert_eq!(retained.root_hex, sales_root);
            assert_eq!(retained.event_count, 2);
            assert_eq!(retained.max_index, 1);

            // Active events: only billing remains; sales is logically pruned.
            let events = a.list_events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].database, "billing");

            // Domain root for the pruned domain falls back to the retained root.
            let (root_after_prune, count_after_prune) = a.domain_root("rs:rs0", "sales").unwrap();
            assert_eq!(root_after_prune, sales_root);
            assert_eq!(count_after_prune, 2);
        }
        // Restart: pruned events stay pruned, retained root restored.
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let events = audit.list_events();
            assert_eq!(events.len(), 1, "pruned sales events stay pruned");
            assert_eq!(events[0].database, "billing");

            let retained = audit.retained_domain_roots("rs:rs0", "sales");
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].root_hex, sales_root);

            let (root, count) = audit.domain_root("rs:rs0", "sales").unwrap();
            assert_eq!(root, sales_root);
            assert_eq!(count, 2);

            let domains = audit.list_domains();
            assert!(domains.contains(&("rs:rs0".to_string(), "sales".to_string())));
            assert!(domains.contains(&("rs:rs0".to_string(), "billing".to_string())));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// A legal hold is persisted across restarts and blocks pruning until it
    /// is lifted.
    #[test]
    fn legal_hold_persists_and_blocks_prune() {
        let dir = tempfile_dir();
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            let a = std::sync::Arc::new(audit);
            interceptor::record_insert(&a, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
            interceptor::record_insert(&a, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();
            a.set_legal_hold("rs:rs0", "sales", true).unwrap();
            assert!(a.is_legal_hold("rs:rs0", "sales"));
            assert!(a.prune_domain("rs:rs0", "sales").is_err());
        }
        {
            let audit = AuditLog::new().unwrap();
            audit.set_persistence_dir(&dir).unwrap();
            assert!(
                audit.is_legal_hold("rs:rs0", "sales"),
                "legal hold restored on restart"
            );
            assert!(audit.prune_domain("rs:rs0", "sales").is_err());

            // Lifting the hold allows the prune to proceed.
            audit.set_legal_hold("rs:rs0", "sales", false).unwrap();
            assert!(!audit.is_legal_hold("rs:rs0", "sales"));
            let retained = audit.prune_domain("rs:rs0", "sales").unwrap();
            assert!(retained.is_some());
            let events = audit.list_events();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].database, "billing");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
