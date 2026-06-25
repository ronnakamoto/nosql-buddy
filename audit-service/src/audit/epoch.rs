//! Epoch construction for deterministic batch commitment.
//!
//! An epoch is a batch of audit events that get committed to the
//! chain as a single Merkle root. This enables deterministic batch
//! commitments: instead of committing every event individually, we
//! group events into epochs and commit one root per epoch.
//!
//! ## Epoch triggers
//!
//! An epoch can be triggered by:
//! - **Count**: every N events, close the current epoch and start a new one.
//! - **Time**: every T seconds, close the current epoch.
//! - **Manual**: the user clicks "Commit Root" to close the current epoch.
//!
//! ## Epoch state
//!
//! Each epoch has:
//! - `epoch_number`: sequential counter (0, 1, 2, ...)
//! - `start_index`: the first leaf index in this epoch
//! - `end_index`: the last leaf index in this epoch (inclusive)
//! - `root_hex`: the Merkle root at `end_index` (audit log root)
//! - `committed`: whether this epoch's root has been committed on-chain
//! - `committed_at`: timestamp of on-chain commitment (if any)
//! - `tx_hash`: the on-chain transaction hash (if committed)
//!
//! ## Oplog completeness fields
//!
//! Each closed epoch also carries oplog completeness fields:
//! - `oplog_start_ts`: the oplog timestamp where this epoch's oplog range starts (inclusive)
//! - `oplog_end_ts`: the oplog timestamp where this epoch's oplog range ends (exclusive)
//! - `oplog_entry_count`: the number of oplog entries in the range
//! - `oplog_merkle_root_hex`: the SHA-256 Merkle root over canonicalized oplog entries
//! - `oplog_majority_commit_ts`: the majority-committed oplog timestamp at close time
//!
//! These fields are `None` when oplog completeness is not enabled. When
//! present, they bind the audit log to MongoDB's oplog — proving that
//! no writes were omitted from the audit log.
//!
//! The current (open) epoch accumulates events. When it closes, its
//! root is frozen and a new epoch opens.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::audit::AuditLog;
use crate::audit::oplog::OplogTimestamp;
use crate::error::{AuditError, AuditResult};

/// Configuration for when an epoch auto-closes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpochConfig {
    /// Close the epoch after this many events. 0 = disabled.
    pub event_threshold: usize,
    /// Close the epoch after this many seconds. 0 = disabled.
    pub time_threshold_secs: u64,
}

impl Default for EpochConfig {
    fn default() -> Self {
        Self {
            event_threshold: 100,
            time_threshold_secs: 0,
        }
    }
}

/// The state of one epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Epoch {
    pub epoch_number: u64,
    pub start_index: u64,
    pub end_index: Option<u64>,
    pub root_hex: Option<String>,
    pub event_count: usize,
    pub committed: bool,
    pub committed_at: Option<String>,
    pub tx_hash: Option<String>,
    // --- Oplog completeness fields (None when oplog hashing is not enabled) ---
    /// The oplog timestamp where this epoch's oplog range starts (inclusive).
    pub oplog_start_ts: Option<OplogTimestamp>,
    /// The oplog timestamp where this epoch's oplog range ends (exclusive).
    pub oplog_end_ts: Option<OplogTimestamp>,
    /// The number of oplog entries in this epoch's range.
    pub oplog_entry_count: Option<u64>,
    /// The SHA-256 Merkle root over canonicalized oplog entries.
    pub oplog_merkle_root_hex: Option<String>,
    /// The majority-committed oplog timestamp at the time of closing.
    pub oplog_majority_commit_ts: Option<OplogTimestamp>,
}

impl Epoch {
    /// Create a new open epoch starting at the given index.
    fn new(epoch_number: u64, start_index: u64) -> Self {
        Self {
            epoch_number,
            start_index,
            end_index: None,
            root_hex: None,
            event_count: 0,
            committed: false,
            committed_at: None,
            tx_hash: None,
            oplog_start_ts: None,
            oplog_end_ts: None,
            oplog_entry_count: None,
            oplog_merkle_root_hex: None,
            oplog_majority_commit_ts: None,
        }
    }

    /// Whether this epoch is still open (accepting events).
    pub fn is_open(&self) -> bool {
        self.end_index.is_none()
    }

    /// Whether this epoch has oplog completeness data attached.
    pub fn has_oplog_hash(&self) -> bool {
        self.oplog_merkle_root_hex.is_some()
    }
}

/// Manages epoch construction for an audit log.
pub struct EpochManager {
    config: Mutex<EpochConfig>,
    epochs: Mutex<Vec<Epoch>>,
    /// The oplog timestamp where the next epoch's oplog range should start.
    /// This is updated when an epoch's oplog hash is attached. If None,
    /// oplog completeness is not enabled and the next epoch should start
    /// from the beginning of the oplog (timestamp 0).
    next_oplog_start_ts: Mutex<Option<OplogTimestamp>>,
}

impl EpochManager {
    /// Create a new epoch manager with the given config.
    pub fn new(config: EpochConfig) -> Self {
        let initial = Epoch::new(0, 0);
        Self {
            config: Mutex::new(config),
            epochs: Mutex::new(vec![initial]),
            next_oplog_start_ts: Mutex::new(None),
        }
    }

    /// Get the oplog start timestamp for the next epoch to close.
    /// Returns `OplogTimestamp::zero()` if oplog hashing hasn't been
    /// initialized yet (first epoch starts from the beginning of the oplog).
    pub fn next_oplog_start_ts(&self) -> OplogTimestamp {
        self.next_oplog_start_ts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .unwrap_or_else(OplogTimestamp::zero)
    }

    /// Attach oplog completeness data to a closed epoch.
    ///
    /// This is called after the epoch is closed and the oplog hash has
    /// been computed externally (using the `oplog` module with a MongoDB
    /// client). The `oplog_end_ts` from the attached data becomes the
    /// `next_oplog_start_ts` for the subsequent epoch.
    ///
    /// Returns the updated epoch, or an error if the epoch is not found
    /// or is still open.
    pub fn attach_oplog_hash(
        &self,
        epoch_number: u64,
        oplog_start_ts: OplogTimestamp,
        oplog_end_ts: OplogTimestamp,
        oplog_entry_count: u64,
        oplog_merkle_root_hex: String,
        oplog_majority_commit_ts: OplogTimestamp,
    ) -> AuditResult<Epoch> {
        let mut epochs = self.epochs.lock().unwrap_or_else(|e| e.into_inner());

        for epoch in epochs.iter_mut() {
            if epoch.epoch_number == epoch_number {
                if epoch.is_open() {
                    return Err(AuditError::Validation(format!(
                        "epoch {epoch_number} is still open — close it before attaching oplog hash"
                    )));
                }
                epoch.oplog_start_ts = Some(oplog_start_ts);
                epoch.oplog_end_ts = Some(oplog_end_ts);
                epoch.oplog_entry_count = Some(oplog_entry_count);
                epoch.oplog_merkle_root_hex = Some(oplog_merkle_root_hex);
                epoch.oplog_majority_commit_ts = Some(oplog_majority_commit_ts);

                // Update the next epoch's oplog start timestamp.
                let mut next_ts = self
                    .next_oplog_start_ts
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                *next_ts = Some(oplog_end_ts);

                return Ok(epoch.clone());
            }
        }

        Err(AuditError::Validation(format!(
            "epoch {epoch_number} not found"
        )))
    }

    /// Record that an event was added to the audit log at the given
    /// index. This updates the current epoch's event count. If the
    /// event threshold is reached, the epoch is auto-closed.
    /// Returns `Some(closed_epoch)` if an epoch was closed.
    pub fn record_event(&self, index: u64, audit_log: &AuditLog) -> AuditResult<Option<Epoch>> {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        let mut epochs = self.epochs.lock().unwrap_or_else(|e| e.into_inner());

        let current = epochs.last_mut().expect("always at least one epoch");
        if !current.is_open() {
            // Shouldn't happen — we always keep an open epoch.
            return Err(AuditError::Validation(
                "no open epoch to record event into".to_string(),
            ));
        }

        current.event_count += 1;

        // Check if we should auto-close the epoch.
        let should_close = config.event_threshold > 0
            && current.event_count >= config.event_threshold;

        if should_close {
            current.end_index = Some(index);
            current.root_hex = Some(audit_log.root_hex()?);
            let closed = current.clone();

            // Start a new epoch.
            let next_number = closed.epoch_number + 1;
            let next_start = index + 1;
            epochs.push(Epoch::new(next_number, next_start));

            return Ok(Some(closed));
        }

        Ok(None)
    }

    /// Manually close the current epoch and commit its root.
    /// Returns the closed epoch with the frozen root.
    pub fn close_current_epoch(&self, audit_log: &AuditLog) -> AuditResult<Epoch> {
        let mut epochs = self.epochs.lock().unwrap_or_else(|e| e.into_inner());

        let current = epochs.last_mut().expect("always at least one epoch");
        if !current.is_open() {
            return Err(AuditError::Validation(
                "current epoch is already closed".to_string(),
            ));
        }

        if current.event_count == 0 {
            return Err(AuditError::Validation(
                "cannot close an empty epoch".to_string(),
            ));
        }

        let end_index = current.start_index + current.event_count as u64 - 1;
        current.end_index = Some(end_index);
        current.root_hex = Some(audit_log.root_hex()?);
        let closed = current.clone();

        // Start a new epoch.
        let next_number = closed.epoch_number + 1;
        let next_start = end_index + 1;
        epochs.push(Epoch::new(next_number, next_start));

        Ok(closed)
    }

    /// Mark an epoch as committed on-chain.
    pub fn mark_committed(
        &self,
        epoch_number: u64,
        tx_hash: String,
    ) -> AuditResult<()> {
        let mut epochs = self.epochs.lock().unwrap_or_else(|e| e.into_inner());
        for epoch in epochs.iter_mut() {
            if epoch.epoch_number == epoch_number {
                epoch.committed = true;
                epoch.committed_at = Some(chrono::Utc::now().to_rfc3339());
                epoch.tx_hash = Some(tx_hash);
                return Ok(());
            }
        }
        Err(AuditError::Validation(format!(
            "epoch {epoch_number} not found"
        )))
    }

    /// Get all epochs (open and closed).
    pub fn list_epochs(&self) -> Vec<Epoch> {
        self.epochs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Get the current (open) epoch.
    pub fn current_epoch(&self) -> Epoch {
        self.epochs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .expect("always at least one epoch")
            .clone()
    }

    /// Get all closed (committed or uncommitted) epochs.
    pub fn closed_epochs(&self) -> Vec<Epoch> {
        self.epochs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|e| !e.is_open())
            .cloned()
            .collect()
    }

    /// Get the current epoch configuration.
    pub fn config(&self) -> EpochConfig {
        self.config.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Update the epoch configuration.
    pub fn set_config(&self, config: EpochConfig) {
        let mut guard = self.config.lock().unwrap_or_else(|e| e.into_inner());
        *guard = config;
    }
}

impl Default for EpochManager {
    fn default() -> Self {
        Self::new(EpochConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_audit_with_events(n: usize) -> Arc<AuditLog> {
        let audit = Arc::new(AuditLog::new().unwrap());
        for i in 0..n {
            crate::audit::interceptor::record_insert(
                &audit,
                "db",
                "col",
                &format!(r#"{{"a":{}}}"#, i),
            )
            .unwrap();
        }
        audit
    }

    #[test]
    fn epoch_manager_starts_with_one_open_epoch() {
        let mgr = EpochManager::new(EpochConfig::default());
        let epochs = mgr.list_epochs();
        assert_eq!(epochs.len(), 1);
        assert!(epochs[0].is_open());
        assert_eq!(epochs[0].epoch_number, 0);
        assert_eq!(epochs[0].event_count, 0);
    }

    #[test]
    fn record_event_increments_count() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 10,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(1);

        let closed = mgr.record_event(0, &audit).unwrap();
        assert!(closed.is_none(), "epoch should not close yet");

        let current = mgr.current_epoch();
        assert_eq!(current.event_count, 1);
    }

    #[test]
    fn epoch_auto_closes_at_threshold() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 3,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(3);

        // First two events: no close.
        assert!(mgr.record_event(0, &audit).unwrap().is_none());
        assert!(mgr.record_event(1, &audit).unwrap().is_none());

        // Third event: auto-close.
        let closed = mgr.record_event(2, &audit).unwrap().unwrap();
        assert!(!closed.is_open());
        assert_eq!(closed.epoch_number, 0);
        assert_eq!(closed.start_index, 0);
        assert_eq!(closed.end_index, Some(2));
        assert_eq!(closed.event_count, 3);
        assert!(closed.root_hex.is_some());

        // A new epoch should be open.
        let current = mgr.current_epoch();
        assert!(current.is_open());
        assert_eq!(current.epoch_number, 1);
        assert_eq!(current.start_index, 3);
    }

    #[test]
    fn manual_close_freezes_root() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 100,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(5);

        for i in 0..5 {
            mgr.record_event(i as u64, &audit).unwrap();
        }

        let closed = mgr.close_current_epoch(&audit).unwrap();
        assert!(!closed.is_open());
        assert_eq!(closed.event_count, 5);
        assert_eq!(closed.end_index, Some(4));
        assert_eq!(
            closed.root_hex.as_ref().unwrap(),
            &audit.root_hex().unwrap()
        );
    }

    #[test]
    fn cannot_close_empty_epoch() {
        let mgr = EpochManager::new(EpochConfig::default());
        let audit = make_audit_with_events(0);

        let err = mgr.close_current_epoch(&audit).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn mark_committed_sets_tx_hash() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(2);

        mgr.record_event(0, &audit).unwrap();
        let closed = mgr.record_event(1, &audit).unwrap().unwrap();
        assert!(!closed.committed);

        mgr.mark_committed(0, "tx123abc".to_string()).unwrap();

        let epochs = mgr.list_epochs();
        let epoch0 = epochs.iter().find(|e| e.epoch_number == 0).unwrap();
        assert!(epoch0.committed);
        assert_eq!(epoch0.tx_hash.as_ref().unwrap(), "tx123abc");
        assert!(epoch0.committed_at.is_some());
    }

    #[test]
    fn closed_epochs_filters_out_open() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(3);

        mgr.record_event(0, &audit).unwrap();
        mgr.record_event(1, &audit).unwrap();
        mgr.record_event(2, &audit).unwrap();

        let closed = mgr.closed_epochs();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].epoch_number, 0);
    }

    #[test]
    fn multiple_epochs_chain_correctly() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(6);

        for i in 0..6 {
            mgr.record_event(i as u64, &audit).unwrap();
        }

        let epochs = mgr.list_epochs();
        // 3 closed epochs + 1 open = 4 total
        assert_eq!(epochs.len(), 4);
        assert_eq!(epochs[0].epoch_number, 0);
        assert_eq!(epochs[0].start_index, 0);
        assert_eq!(epochs[0].end_index, Some(1));
        assert_eq!(epochs[1].epoch_number, 1);
        assert_eq!(epochs[1].start_index, 2);
        assert_eq!(epochs[1].end_index, Some(3));
        assert_eq!(epochs[2].epoch_number, 2);
        assert_eq!(epochs[2].start_index, 4);
        assert_eq!(epochs[2].end_index, Some(5));
        assert!(epochs[3].is_open());
        assert_eq!(epochs[3].start_index, 6);
    }

    // --- Oplog hash attachment tests ---

    #[test]
    fn next_oplog_start_ts_defaults_to_zero() {
        let mgr = EpochManager::new(EpochConfig::default());
        let ts = mgr.next_oplog_start_ts();
        assert_eq!(ts.time, 0);
        assert_eq!(ts.increment, 0);
    }

    #[test]
    fn attach_oplog_hash_to_closed_epoch() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(2);

        mgr.record_event(0, &audit).unwrap();
        let closed = mgr.record_event(1, &audit).unwrap().unwrap();
        assert!(!closed.has_oplog_hash());

        let start_ts = OplogTimestamp { time: 1000, increment: 0 };
        let end_ts = OplogTimestamp { time: 2000, increment: 5 };
        let majority_ts = OplogTimestamp { time: 2000, increment: 10 };

        let updated = mgr.attach_oplog_hash(
            0,
            start_ts,
            end_ts,
            42,
            "abc123".to_string(),
            majority_ts,
        ).unwrap();

        assert!(updated.has_oplog_hash());
        assert_eq!(updated.oplog_start_ts, Some(start_ts));
        assert_eq!(updated.oplog_end_ts, Some(end_ts));
        assert_eq!(updated.oplog_entry_count, Some(42));
        assert_eq!(updated.oplog_merkle_root_hex.as_deref(), Some("abc123"));
        assert_eq!(updated.oplog_majority_commit_ts, Some(majority_ts));
    }

    #[test]
    fn attach_oplog_hash_updates_next_oplog_start() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(2);

        mgr.record_event(0, &audit).unwrap();
        mgr.record_event(1, &audit).unwrap();

        let start_ts = OplogTimestamp { time: 1000, increment: 0 };
        let end_ts = OplogTimestamp { time: 2000, increment: 5 };
        let majority_ts = OplogTimestamp { time: 2000, increment: 10 };

        mgr.attach_oplog_hash(0, start_ts, end_ts, 42, "abc".to_string(), majority_ts)
            .unwrap();

        // The next epoch's oplog start should be the previous epoch's end_ts.
        let next_start = mgr.next_oplog_start_ts();
        assert_eq!(next_start, end_ts);
    }

    #[test]
    fn attach_oplog_hash_rejects_open_epoch() {
        let mgr = EpochManager::new(EpochConfig::default());

        let start_ts = OplogTimestamp { time: 1000, increment: 0 };
        let end_ts = OplogTimestamp { time: 2000, increment: 5 };
        let majority_ts = OplogTimestamp { time: 2000, increment: 10 };

        let err = mgr
            .attach_oplog_hash(0, start_ts, end_ts, 1, "abc".to_string(), majority_ts)
            .unwrap_err();
        assert!(err.to_string().contains("still open"));
    }

    #[test]
    fn attach_oplog_hash_rejects_unknown_epoch() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(2);
        mgr.record_event(0, &audit).unwrap();
        mgr.record_event(1, &audit).unwrap();

        let start_ts = OplogTimestamp { time: 1000, increment: 0 };
        let end_ts = OplogTimestamp { time: 2000, increment: 5 };
        let majority_ts = OplogTimestamp { time: 2000, increment: 10 };

        let err = mgr
            .attach_oplog_hash(99, start_ts, end_ts, 1, "abc".to_string(), majority_ts)
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn oplog_hash_chains_across_epochs() {
        let mgr = EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        });
        let audit = make_audit_with_events(4);

        // Close epoch 0.
        mgr.record_event(0, &audit).unwrap();
        mgr.record_event(1, &audit).unwrap();

        let start0 = OplogTimestamp::zero();
        let end0 = OplogTimestamp { time: 1000, increment: 5 };
        let majority0 = OplogTimestamp { time: 1000, increment: 10 };

        mgr.attach_oplog_hash(0, start0, end0, 10, "root0".to_string(), majority0)
            .unwrap();

        // Close epoch 1.
        mgr.record_event(2, &audit).unwrap();
        mgr.record_event(3, &audit).unwrap();

        // Epoch 1's oplog start should be epoch 0's end.
        let start1 = mgr.next_oplog_start_ts();
        assert_eq!(start1, end0);

        let end1 = OplogTimestamp { time: 2000, increment: 3 };
        let majority1 = OplogTimestamp { time: 2000, increment: 8 };

        let updated1 = mgr
            .attach_oplog_hash(1, start1, end1, 15, "root1".to_string(), majority1)
            .unwrap();

        assert_eq!(updated1.oplog_start_ts, Some(end0));
        assert_eq!(updated1.oplog_end_ts, Some(end1));
        assert_eq!(updated1.oplog_merkle_root_hex.as_deref(), Some("root1"));

        // Next epoch's start should be epoch 1's end.
        let start2 = mgr.next_oplog_start_ts();
        assert_eq!(start2, end1);
    }
}
