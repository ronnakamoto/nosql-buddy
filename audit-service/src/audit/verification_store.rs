//! Persistent store for reader-mode verification runs (the "tamper timeline").
//!
//! Each time the user runs an integrity check (`audit_verify_reader_mode`),
//! the resulting [`VerificationReport`] is recorded here together with the
//! wall-clock time it ran. The store persists the run history to a JSON file
//! so the verification timeline survives app restarts. Previously, the
//! history lived only in the frontend's React state and was lost on relaunch.
//!
//! The design mirrors [`crate::audit::epoch::EpochManager`]'s persistence:
//! an in-memory `Vec` behind a `Mutex`, flushed to a pretty-printed JSON file
//! after every mutation. The file lives at
//! `<app_data_dir>/audit/verification_history.json`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::audit::reader::VerificationReport;
use crate::error::{AuditError, AuditResult};

/// The maximum number of verification runs to retain. Older runs are pruned
/// from the front so the file stays bounded for long-lived installs.
const MAX_RECORDS: usize = 500;

/// One recorded verification run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRecord {
    /// Unix epoch milliseconds when the verification ran.
    pub run_at: i64,
    /// The verification report produced by the run.
    pub report: VerificationReport,
}

/// Persistent history of verification runs.
pub struct VerificationStore {
    records: Mutex<Vec<VerificationRecord>>,
    /// Path to the JSON file where history is persisted. `None` keeps the
    /// history in memory only (e.g. in unit tests).
    persistence_path: Mutex<Option<PathBuf>>,
}

impl VerificationStore {
    /// Create an empty, in-memory-only store.
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            persistence_path: Mutex::new(None),
        }
    }

    /// Enable on-disk persistence and load any saved history.
    ///
    /// If `path` exists, its contents replace the in-memory history;
    /// otherwise the (empty) history is written so the file exists going
    /// forward. Corrupt files are logged and ignored rather than bricking
    /// startup; verification history is auxiliary, not authoritative.
    pub fn enable_persistence(&self, path: impl AsRef<Path>) -> AuditResult<()> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            match std::fs::read_to_string(&path)
                .map_err(|e| AuditError::Internal(format!("read verification history: {e}")))
                .and_then(|data| {
                    serde_json::from_str::<PersistedHistory>(&data).map_err(|e| {
                        AuditError::Internal(format!("parse verification history: {e}"))
                    })
                }) {
                Ok(state) => {
                    *self.records.lock().unwrap_or_else(|e| e.into_inner()) = state.records;
                }
                Err(e) => {
                    log::warn!(
                        "failed to load verification history from {}: {e}; starting fresh",
                        path.display()
                    );
                }
            }
        }
        *self
            .persistence_path
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(path);
        self.save()
    }

    /// Record a verification run and persist it. Returns the stored record.
    pub fn record(
        &self,
        run_at: i64,
        report: VerificationReport,
    ) -> AuditResult<VerificationRecord> {
        let record = VerificationRecord { run_at, report };
        {
            let mut records = self.records.lock().unwrap_or_else(|e| e.into_inner());
            records.push(record.clone());
            // Keep the history bounded: drop the oldest runs beyond the cap.
            let len = records.len();
            if len > MAX_RECORDS {
                records.drain(0..len - MAX_RECORDS);
            }
        }
        self.save()?;
        Ok(record)
    }

    /// List all recorded verification runs, oldest first.
    pub fn list(&self) -> Vec<VerificationRecord> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Remove all recorded verification runs and persist the empty history.
    pub fn clear(&self) -> AuditResult<()> {
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.save()
    }

    /// Persist the current history to disk, if a path is configured.
    fn save(&self) -> AuditResult<()> {
        let path = self
            .persistence_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path else {
            return Ok(());
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AuditError::Internal(format!("create verification history dir: {e}"))
            })?;
        }

        let state = PersistedHistory {
            records: self
                .records
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        };
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| AuditError::Internal(format!("serialize verification history: {e}")))?;
        std::fs::write(&path, json)
            .map_err(|e| AuditError::Internal(format!("write verification history: {e}")))?;
        Ok(())
    }
}

impl Default for VerificationStore {
    fn default() -> Self {
        Self::new()
    }
}

/// On-disk representation of the verification history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedHistory {
    records: Vec<VerificationRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reader::VerificationReport;

    fn sample_report(tamper: bool) -> VerificationReport {
        VerificationReport {
            onchain_root_found: true,
            onchain_root: None,
            local_root_hex: "abc".to_string(),
            commitment_event_index: Some(3),
            total_events: 10,
            verified_events: 4,
            events_after_commitment: 6,
            chain_intact: !tamper,
            tamper_detected: tamper,
            summary: if tamper { "tamper".into() } else { "ok".into() },
        }
    }

    #[test]
    fn record_and_list_in_memory() {
        let store = VerificationStore::new();
        assert!(store.list().is_empty());
        store.record(1000, sample_report(false)).unwrap();
        store.record(2000, sample_report(true)).unwrap();
        let history = store.list();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].run_at, 1000);
        assert!(!history[0].report.tamper_detected);
        assert_eq!(history[1].run_at, 2000);
        assert!(history[1].report.tamper_detected);
    }

    #[test]
    fn history_survives_restart() {
        let tmp = std::env::temp_dir().join(format!(
            "nosqlbuddy_verify_history_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let path = tmp.join("verification_history.json");

        {
            let store = VerificationStore::new();
            store.enable_persistence(&path).unwrap();
            store.record(1000, sample_report(false)).unwrap();
            store.record(2000, sample_report(true)).unwrap();
        }

        {
            let store = VerificationStore::new();
            store.enable_persistence(&path).unwrap();
            let history = store.list();
            assert_eq!(history.len(), 2);
            assert_eq!(history[0].run_at, 1000);
            assert_eq!(history[1].run_at, 2000);
            assert!(history[1].report.tamper_detected);
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn history_is_bounded() {
        let store = VerificationStore::new();
        for i in 0..(MAX_RECORDS + 50) {
            store.record(i as i64, sample_report(false)).unwrap();
        }
        let history = store.list();
        assert_eq!(history.len(), MAX_RECORDS);
        // Oldest 50 were pruned; the first retained run_at is 50.
        assert_eq!(history[0].run_at, 50);
    }
}
