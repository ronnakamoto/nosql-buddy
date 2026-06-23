//! Audit event interceptor.
//!
//! This module provides hooks that intercept Mongo operations (insert, update,
//! delete) and automatically record audit events in the audit log.
//!
//! In Stage 11, this will be wired into the Mongo command handlers so that
//! every write operation automatically generates an audit entry.

use std::sync::Arc;

use crate::audit::AuditLog;
use crate::error::AppResult;

/// Record an insert operation in the audit log.
pub fn record_insert(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    document_json: &str,
) -> AppResult<u64> {
    use ark_bn254::Fr;
    use ark_ff::PrimeField;

    let payload = format!("insert|{}|{}|{}", database, collection, document_json);
    let hash = sha256_hash(&payload);
    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&hash[..31]);
    bytes[31] &= 0x0F;
    let leaf = Fr::from_be_bytes_mod_order(&bytes);

    audit.record("insert", database, collection, leaf)
}

/// Record an update operation in the audit log.
pub fn record_update(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    filter_json: &str,
    update_json: &str,
) -> AppResult<u64> {
    use ark_bn254::Fr;
    use ark_ff::PrimeField;

    let payload = format!("update|{}|{}|{}|{}", database, collection, filter_json, update_json);
    let hash = sha256_hash(&payload);
    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&hash[..31]);
    bytes[31] &= 0x0F;
    let leaf = Fr::from_be_bytes_mod_order(&bytes);

    audit.record("update", database, collection, leaf)
}

/// Record a delete operation in the audit log.
pub fn record_delete(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    filter_json: &str,
) -> AppResult<u64> {
    use ark_bn254::Fr;
    use ark_ff::PrimeField;

    let payload = format!("delete|{}|{}|{}", database, collection, filter_json);
    let hash = sha256_hash(&payload);
    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&hash[..31]);
    bytes[31] &= 0x0F;
    let leaf = Fr::from_be_bytes_mod_order(&bytes);

    audit.record("delete", database, collection, leaf)
}

fn sha256_hash(input: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_insert_creates_event() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let idx = record_insert(&audit, "test_db", "test_col", r#"{"a":1}"#).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(audit.event_count(), 1);
        assert_eq!(audit.leaf_count(), 1);

        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "insert");
        assert_eq!(events[0].database, "test_db");
        assert_eq!(events[0].collection, "test_col");
    }

    #[test]
    fn test_multiple_events_change_root() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let root0 = audit.root_hex().unwrap();
        record_insert(&audit, "db", "col", r#"{"a":1}"#).unwrap();
        let root1 = audit.root_hex().unwrap();
        record_insert(&audit, "db", "col", r#"{"a":2}"#).unwrap();
        let root2 = audit.root_hex().unwrap();

        assert_ne!(root0, root1);
        assert_ne!(root1, root2);
    }
}
