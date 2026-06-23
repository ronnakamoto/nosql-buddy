//! Audit event interceptor.
//!
//! This module provides hooks that intercept Mongo operations (insert, update,
//! delete) and automatically record audit events in the audit log.
//!
//! In Stage 11, this will be wired into the Mongo command handlers so that
//! every write operation automatically generates an audit entry.

use std::sync::Arc;

use crate::audit::{leaf_from_payload, AuditLog};
use crate::error::AppResult;

/// Record an insert operation in the audit log.
pub fn record_insert(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    document_json: &str,
) -> AppResult<u64> {
    let payload = format!("insert|{}|{}|{}", database, collection, document_json);
    let leaf = leaf_from_payload("insert", database, collection, &payload);
    audit.record("insert", database, collection, &payload, leaf)
}

/// Record an update operation in the audit log.
pub fn record_update(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    filter_json: &str,
    update_json: &str,
) -> AppResult<u64> {
    let payload = format!("update|{}|{}|{}|{}", database, collection, filter_json, update_json);
    let leaf = leaf_from_payload("update", database, collection, &payload);
    audit.record("update", database, collection, &payload, leaf)
}

/// Record a delete operation in the audit log.
pub fn record_delete(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    filter_json: &str,
) -> AppResult<u64> {
    let payload = format!("delete|{}|{}|{}", database, collection, filter_json);
    let leaf = leaf_from_payload("delete", database, collection, &payload);
    audit.record("delete", database, collection, &payload, leaf)
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
