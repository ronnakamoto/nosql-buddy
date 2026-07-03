//! Audit event interceptor.
//!
//! This module provides hooks that intercept Mongo operations (insert, update,
//! delete) and automatically record audit events in the audit log.
//!
//! In Stage 11, this will be wired into the Mongo command handlers so that
//! every write operation automatically generates an audit entry.

use std::sync::Arc;

use crate::audit::{crypto, leaf_from_payload, AuditLog};
use crate::error::AuditResult;

/// Record an insert operation in the audit log.
pub fn record_insert(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    document_json: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_insert_payload(database, collection, document_json);
        return audit.record_v3(deployment_id, "insert", database, collection, &canonical);
    }
    let payload = format!("insert|{}|{}|{}", database, collection, document_json);
    let leaf = leaf_from_payload("insert", database, collection, &payload);
    audit.record(
        deployment_id,
        "insert",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record an update operation in the audit log.
pub fn record_update(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    filter_json: &str,
    update_json: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical =
            crypto::build_update_payload(database, collection, filter_json, update_json);
        return audit.record_v3(deployment_id, "update", database, collection, &canonical);
    }
    let payload = format!(
        "update|{}|{}|{}|{}",
        database, collection, filter_json, update_json
    );
    let leaf = leaf_from_payload("update", database, collection, &payload);
    audit.record(
        deployment_id,
        "update",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record a delete operation in the audit log.
pub fn record_delete(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    filter_json: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_delete_payload(database, collection, filter_json);
        return audit.record_v3(deployment_id, "delete", database, collection, &canonical);
    }
    let payload = format!("delete|{}|{}|{}", database, collection, filter_json);
    let leaf = leaf_from_payload("delete", database, collection, &payload);
    audit.record(
        deployment_id,
        "delete",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record a collection drop in the audit log.
pub fn record_drop_collection(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_drop_collection_payload(database, collection);
        return audit.record_v3(
            deployment_id,
            "drop_collection",
            database,
            collection,
            &canonical,
        );
    }
    let payload = format!("drop_collection|{}|{}", database, collection);
    let leaf = leaf_from_payload("drop_collection", database, collection, &payload);
    audit.record(
        deployment_id,
        "drop_collection",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record a database drop in the audit log.
pub fn record_drop_database(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_drop_database_payload(database);
        return audit.record_v3(deployment_id, "drop_database", database, "", &canonical);
    }
    let payload = format!("drop_database|{}", database);
    let leaf = leaf_from_payload("drop_database", database, "", &payload);
    audit.record(deployment_id, "drop_database", database, "", &payload, leaf)
}

/// Record a collection rename in the audit log.
pub fn record_rename_collection(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    new_name: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_rename_payload(database, collection, new_name);
        return audit.record_v3(deployment_id, "rename", database, collection, &canonical);
    }
    let payload = format!("rename|{}|{}|{}", database, collection, new_name);
    let leaf = leaf_from_payload("rename", database, collection, &payload);
    audit.record(
        deployment_id,
        "rename",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record an index creation in the audit log.
pub fn record_create_index(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    keys_json: &str,
    options_json: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical =
            crypto::build_create_index_payload(database, collection, keys_json, options_json);
        return audit.record_v3(
            deployment_id,
            "create_index",
            database,
            collection,
            &canonical,
        );
    }
    let payload = format!(
        "create_index|{}|{}|{}|{}",
        database, collection, keys_json, options_json
    );
    let leaf = leaf_from_payload("create_index", database, collection, &payload);
    audit.record(
        deployment_id,
        "create_index",
        database,
        collection,
        &payload,
        leaf,
    )
}

/// Record an index drop in the audit log.
pub fn record_drop_index(
    audit: &Arc<AuditLog>,
    deployment_id: &str,
    database: &str,
    collection: &str,
    index_name: &str,
) -> AuditResult<u64> {
    if audit.has_leaf_key() {
        let canonical = crypto::build_drop_index_payload(database, collection, index_name);
        return audit.record_v3(deployment_id, "drop_index", database, collection, &canonical);
    }
    let payload = format!("drop_index|{}|{}|{}", database, collection, index_name);
    let leaf = leaf_from_payload("drop_index", database, collection, &payload);
    audit.record(
        deployment_id,
        "drop_index",
        database,
        collection,
        &payload,
        leaf,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_insert_creates_event() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let idx = record_insert(&audit, "rs:rs0", "test_db", "test_col", r#"{"a":1}"#).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(audit.event_count(), 1);
        assert_eq!(audit.leaf_count(), 1);

        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "insert");
        assert_eq!(events[0].database, "test_db");
        assert_eq!(events[0].collection, "test_col");
        assert_eq!(events[0].deployment_id, "rs:rs0");
        assert_eq!(events[0].sequence, 0);
    }

    #[test]
    fn test_multiple_events_change_root() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let root0 = audit.root_hex().unwrap();
        record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
        let root1 = audit.root_hex().unwrap();
        record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();
        let root2 = audit.root_hex().unwrap();

        assert_ne!(root0, root1);
        assert_ne!(root1, root2);
    }

    #[test]
    fn test_sequence_is_monotonic_per_domain() {
        let audit = Arc::new(AuditLog::new().unwrap());
        // Same (deployment, database) → monotonically increasing sequence.
        record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":2}"#).unwrap();
        record_update(&audit, "rs:rs0", "sales", "orders", "{}", "{}").unwrap();
        let events = audit.list_events();
        let sales: Vec<u64> = events
            .iter()
            .filter(|e| e.deployment_id == "rs:rs0" && e.database == "sales")
            .map(|e| e.sequence)
            .collect();
        assert_eq!(sales, vec![0, 1, 2]);
    }

    #[test]
    fn test_sequence_is_independent_across_domains() {
        let audit = Arc::new(AuditLog::new().unwrap());
        // Different database, same deployment → independent counter.
        record_insert(&audit, "rs:rs0", "sales", "orders", r#"{"a":1}"#).unwrap();
        record_insert(&audit, "rs:rs0", "billing", "invoices", r#"{"a":1}"#).unwrap();
        // Different deployment, same database name → independent counter.
        record_insert(&audit, "rs:rs1", "sales", "orders", r#"{"a":1}"#).unwrap();
        let events = audit.list_events();
        let seq = |dep: &str, db: &str| {
            events
                .iter()
                .find(|e| e.deployment_id == dep && e.database == db)
                .map(|e| e.sequence)
                .unwrap()
        };
        assert_eq!(seq("rs:rs0", "sales"), 0);
        assert_eq!(seq("rs:rs0", "billing"), 0);
        assert_eq!(seq("rs:rs1", "sales"), 0);
    }

    #[test]
    fn test_record_drop_collection() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let idx = record_drop_collection(&audit, "rs:rs0", "mydb", "mycoll").unwrap();
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
        record_drop_database(&audit, "rs:rs0", "mydb").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "drop_database");
        assert_eq!(events[0].database, "mydb");
    }

    #[test]
    fn test_record_rename_collection() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_rename_collection(&audit, "rs:rs0", "db", "old_name", "new_name").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "rename");
        assert_eq!(events[0].database, "db");
        assert_eq!(events[0].collection, "old_name");
    }

    #[test]
    fn test_record_create_index() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_create_index(
            &audit,
            "rs:rs0",
            "db",
            "coll",
            r#"{"a":1}"#,
            r#"{"name":"idx_a"}"#,
        )
        .unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "create_index");
    }

    #[test]
    fn test_record_drop_index() {
        let audit = Arc::new(AuditLog::new().unwrap());
        record_drop_index(&audit, "rs:rs0", "db", "coll", "idx_a").unwrap();
        let events = audit.list_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].operation, "drop_index");
    }

    #[test]
    fn test_all_new_interceptors_change_root() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let root0 = audit.root_hex().unwrap();
        record_drop_collection(&audit, "rs:rs0", "db", "c1").unwrap();
        let root1 = audit.root_hex().unwrap();
        record_drop_database(&audit, "rs:rs0", "db").unwrap();
        let root2 = audit.root_hex().unwrap();
        record_rename_collection(&audit, "rs:rs0", "db", "c2", "c3").unwrap();
        let root3 = audit.root_hex().unwrap();
        record_create_index(&audit, "rs:rs0", "db", "c", r#"{"x":1}"#, "{}").unwrap();
        let root4 = audit.root_hex().unwrap();
        record_drop_index(&audit, "rs:rs0", "db", "c", "idx").unwrap();
        let root5 = audit.root_hex().unwrap();

        assert_ne!(root0, root1);
        assert_ne!(root1, root2);
        assert_ne!(root2, root3);
        assert_ne!(root3, root4);
        assert_ne!(root4, root5);
    }
}
