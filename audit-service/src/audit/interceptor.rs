//! Audit event interceptor.
//!
//! This module provides hooks that intercept Mongo operations (insert, update,
//! delete) and automatically record audit events in the audit log.
//!
//! In Stage 11, this will be wired into the Mongo command handlers so that
//! every write operation automatically generates an audit entry.

use std::sync::Arc;

use crate::audit::{leaf_from_payload, AuditLog};
use crate::error::AuditResult;

/// Record an insert operation in the audit log.
pub fn record_insert(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    document_json: &str,
) -> AuditResult<u64> {
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
) -> AuditResult<u64> {
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
) -> AuditResult<u64> {
    let payload = format!("delete|{}|{}|{}", database, collection, filter_json);
    let leaf = leaf_from_payload("delete", database, collection, &payload);
    audit.record("delete", database, collection, &payload, leaf)
}

/// Record a collection drop in the audit log.
pub fn record_drop_collection(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
) -> AuditResult<u64> {
    let payload = format!("drop_collection|{}|{}", database, collection);
    let leaf = leaf_from_payload("drop_collection", database, collection, &payload);
    audit.record("drop_collection", database, collection, &payload, leaf)
}

/// Record a database drop in the audit log.
pub fn record_drop_database(
    audit: &Arc<AuditLog>,
    database: &str,
) -> AuditResult<u64> {
    let payload = format!("drop_database|{}", database);
    let leaf = leaf_from_payload("drop_database", database, "", &payload);
    audit.record("drop_database", database, "", &payload, leaf)
}

/// Record a collection rename in the audit log.
pub fn record_rename_collection(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    new_name: &str,
) -> AuditResult<u64> {
    let payload = format!("rename|{}|{}|{}", database, collection, new_name);
    let leaf = leaf_from_payload("rename", database, collection, &payload);
    audit.record("rename", database, collection, &payload, leaf)
}

/// Record an index creation in the audit log.
pub fn record_create_index(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    keys_json: &str,
    options_json: &str,
) -> AuditResult<u64> {
    let payload = format!("create_index|{}|{}|{}|{}", database, collection, keys_json, options_json);
    let leaf = leaf_from_payload("create_index", database, collection, &payload);
    audit.record("create_index", database, collection, &payload, leaf)
}

/// Record an index drop in the audit log.
pub fn record_drop_index(
    audit: &Arc<AuditLog>,
    database: &str,
    collection: &str,
    index_name: &str,
) -> AuditResult<u64> {
    let payload = format!("drop_index|{}|{}|{}", database, collection, index_name);
    let leaf = leaf_from_payload("drop_index", database, collection, &payload);
    audit.record("drop_index", database, collection, &payload, leaf)
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

    #[test]
    fn test_record_drop_collection() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let idx = record_drop_collection(&audit, "mydb", "mycoll").unwrap();
        assert_eq!(idx, 0);
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "drop_collection");
        assert_eq!(events[0].database, "mydb");
        assert_eq!(events[0].collection, "mycoll");
    }

    #[test]
    fn test_record_drop_database() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_drop_database(&audit, "mydb").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "drop_database");
        assert_eq!(events[0].database, "mydb");
    }

    #[test]
    fn test_record_rename_collection() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_rename_collection(&audit, "db", "old_name", "new_name").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "rename");
        assert_eq!(events[0].database, "db");
        assert_eq!(events[0].collection, "old_name");
    }

    #[test]
    fn test_record_create_index() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_create_index(&audit, "db", "coll", r#"{"a":1}"#, r#"{"name":"idx_a"}"#).unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "create_index");
    }

    #[test]
    fn test_record_drop_index() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_drop_index(&audit, "db", "coll", "idx_a").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "drop_index");
    }

    #[test]
    fn test_all_new_interceptors_change_root() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let root0 = audit.root_hex().unwrap();
        record_drop_collection(&audit, "db", "c1").unwrap();
        let root1 = audit.root_hex().unwrap();
        record_drop_database(&audit, "db").unwrap();
        let root2 = audit.root_hex().unwrap();
        record_rename_collection(&audit, "db", "c2", "c3").unwrap();
        let root3 = audit.root_hex().unwrap();
        record_create_index(&audit, "db", "c", r#"{"x":1}"#, "{}").unwrap();
        let root4 = audit.root_hex().unwrap();
        record_drop_index(&audit, "db", "c", "idx").unwrap();
        let root5 = audit.root_hex().unwrap();

        assert_ne!(root0, root1);
        assert_ne!(root1, root2);
        assert_ne!(root2, root3);
        assert_ne!(root3, root4);
        assert_ne!(root4, root5);
    }
}
