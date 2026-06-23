//! ZK audit log: tamper-evident Merkle tree of database operations.
//!
//! This module owns the audit state: a Poseidon Merkle tree that accumulates
//! audit events (inserts, updates, deletes) into a single root, plus the
//! ability to generate Groth16 inclusion proofs and commit roots to Soroban.
//!
//! ## Architecture
//!
//! - [`AuditLog`] — the in-memory audit log: a Merkle tree + event metadata.
//! - [`commands`] — Tauri IPC commands for the frontend audit panel.
//! - [`interceptor`] — hooks into Mongo operations to auto-record audit events.

pub mod commands;
pub mod interceptor;

#[cfg(test)]
mod e2e_test;

use std::sync::Mutex;

use zk_audit::merkle::AuditMerkleTree;
use zk_audit::InclusionProof;

use crate::error::AppResult;

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

/// The audit log state, protected by a mutex.
pub struct AuditLog {
    tree: Mutex<AuditMerkleTree>,
    events: Mutex<Vec<AuditEvent>>,
}

impl AuditLog {
    /// Create a new audit log with the default tree height (20 levels = 1M leaves).
    pub fn new() -> AppResult<Self> {
        let tree = AuditMerkleTree::with_height(20)?;
        Ok(Self {
            tree: Mutex::new(tree),
            events: Mutex::new(Vec::new()),
        })
    }

    /// Record an audit event. Returns the leaf index.
    pub fn record(
        &self,
        operation: &str,
        database: &str,
        collection: &str,
        leaf: ark_bn254::Fr,
    ) -> AppResult<u64> {
        use ark_ff::{BigInteger, PrimeField};

        // Recover from poisoned mutex (a prior panic) rather than propagating
        // the panic — this prevents a single failure from bricking the entire
        // audit log for all subsequent commands.
        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let index = tree.insert(leaf) as u64;

        let leaf_bigint = leaf.into_bigint();
        let leaf_bytes = leaf_bigint.to_bytes_be();
        let leaf_hex = hex::encode(&leaf_bytes);

        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.push(AuditEvent {
            index,
            leaf_hex,
            operation: operation.to_string(),
            database: database.to_string(),
            collection: collection.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        });

        Ok(index)
    }

    /// Get the current Merkle root as a hex string.
    pub fn root_hex(&self) -> AppResult<String> {
        use ark_ff::{BigInteger, PrimeField};

        let mut tree = self.tree.lock().unwrap_or_else(|e| e.into_inner());
        let root = tree.root()?;
        let root_bigint = root.into_bigint();
        let root_bytes = root_bigint.to_bytes_be();
        Ok(hex::encode(&root_bytes))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_event_serializes_with_camel_case_fields() {
        let event = AuditEvent {
            index: 7,
            leaf_hex: "deadbeef".to_string(),
            operation: "update".to_string(),
            database: "db".to_string(),
            collection: "col".to_string(),
            timestamp: "2026-06-23T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"leafHex\":\"deadbeef\""), "leafHex must be camelCase: {json}");
        assert!(json.contains("\"index\":7"), "index must be present: {json}");
    }
}
