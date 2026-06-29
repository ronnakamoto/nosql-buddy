//! Operation Recorder — intercepts database writes and records them into the
//! Data Timeline before/after execution.
//!
//! This module provides thin helper functions that wrap the existing write
//! commands in `commands::mongo`.  Each helper:
//! 1. Creates a `TimelineEntry` draft
//! 2. Executes the original operation
//! 3. Fills in result counts / error state
//! 4. Appends the entry to `TimelineStore`
//!
//! In Phase 2 (Safe Change Mode) these helpers will be extended to capture
//! pre-images and compute risk scores.

use crate::mongo::timeline_store::{OperationKind, TimelineEntry, TimelineStore};

/// Context about the operation being recorded.
pub struct RecordContext {
    pub profile_id: String,
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub environment_tag: String,
    pub actor: String,
}

impl RecordContext {
    pub fn new(
        profile_id: String,
        connection_id: String,
        database: String,
        collection: String,
    ) -> Self {
        Self {
            profile_id,
            connection_id,
            database,
            collection,
            environment_tag: String::new(),
            actor: "local-user".to_string(),
        }
    }

    pub fn with_environment_tag(mut self, tag: String) -> Self {
        self.environment_tag = tag;
        self
    }
}

/// Optional Safe Change preview data to attach to a timeline entry.
/// All fields are optional so callers that don't have preview data
/// (e.g. non-Safe-Change paths) can pass `SafeChangeMetadata::default()`.
#[derive(Debug, Default)]
pub struct SafeChangeMetadata {
    pub risk_score: Option<u8>,
    pub risk_reasons: Option<Vec<String>>,
    pub rollback_script: Option<String>,
    pub rollback_level: crate::mongo::timeline_store::RollbackLevel,
}

/// Record a find operation to the timeline (for query history migration).
pub async fn record_find(
    store: &TimelineStore,
    ctx: &RecordContext,
    filter_json: &str,
    returned_count: u64,
    execution_ms: u64,
    errored: bool,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Find,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(filter_json.to_string()))
    .returned_count(returned_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an aggregation operation to the timeline.
pub async fn record_aggregate(
    store: &TimelineStore,
    ctx: &RecordContext,
    pipeline_json: &str,
    returned_count: u64,
    execution_ms: u64,
    errored: bool,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Aggregate,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(pipeline_json.to_string()))
    .returned_count(returned_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an insert operation to the timeline.
///
/// The caller must supply the exact kind (`InsertOne` or `InsertMany`); the
/// count alone is ambiguous because `insertMany` with one document is still
/// `insertMany`.
pub async fn record_insert(
    store: &TimelineStore,
    ctx: &RecordContext,
    kind: OperationKind,
    document_json: &str,
    inserted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        kind,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .update_json(Some(document_json.to_string()))
    .inserted_count(inserted_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an update operation to the timeline.
///
/// The caller must supply the exact kind (`UpdateOne`, `UpdateMany`, or
/// `ReplaceOne`) because the matched count does not reliably distinguish them.
pub async fn record_update(
    store: &TimelineStore,
    ctx: &RecordContext,
    kind: OperationKind,
    filter_json: &str,
    update_json: &str,
    matched_count: u64,
    modified_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
    safe_change: SafeChangeMetadata,
) {
    let mut builder = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        kind,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(filter_json.to_string()))
    .update_json(Some(update_json.to_string()))
    .matched_count(matched_count)
    .modified_count(modified_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .rollback_level(safe_change.rollback_level)
    .rollback_script(safe_change.rollback_script);
    if let Some(score) = safe_change.risk_score {
        builder = builder.risk_score(score);
    }
    if let Some(reasons) = safe_change.risk_reasons {
        builder = builder.risk_reasons(reasons);
    }
    let entry = builder.executed_at(chrono_now()).build();

    store.append(entry).await;
}

/// Record a delete operation to the timeline.
///
/// The caller must supply the exact kind (`DeleteOne` or `DeleteMany`).
pub async fn record_delete(
    store: &TimelineStore,
    ctx: &RecordContext,
    kind: OperationKind,
    filter_json: &str,
    deleted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
    safe_change: SafeChangeMetadata,
) {
    let mut builder = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        kind,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(filter_json.to_string()))
    .deleted_count(deleted_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .rollback_level(safe_change.rollback_level)
    .rollback_script(safe_change.rollback_script);
    if let Some(score) = safe_change.risk_score {
        builder = builder.risk_score(score);
    }
    if let Some(reasons) = safe_change.risk_reasons {
        builder = builder.risk_reasons(reasons);
    }
    let entry = builder.executed_at(chrono_now()).build();

    store.append(entry).await;
}

/// Record a replace-one operation to the timeline.
pub async fn record_replace_one(
    store: &TimelineStore,
    ctx: &RecordContext,
    filter_json: &str,
    replacement_json: &str,
    matched_count: u64,
    modified_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
    safe_change: SafeChangeMetadata,
) {
    record_update(
        store,
        ctx,
        OperationKind::ReplaceOne,
        filter_json,
        replacement_json,
        matched_count,
        modified_count,
        execution_ms,
        errored,
        error_message,
        safe_change,
    )
    .await;
}

/// Record an insert-many operation to the timeline.
pub async fn record_insert_many(
    store: &TimelineStore,
    ctx: &RecordContext,
    documents_json: &str,
    inserted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    record_insert(
        store,
        ctx,
        OperationKind::InsertMany,
        documents_json,
        inserted_count,
        execution_ms,
        errored,
        error_message,
    )
    .await;
}

/// Record a collection create operation to the timeline.
pub async fn record_collection_create(
    store: &TimelineStore,
    ctx: &RecordContext,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::CollectionCreate,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record a collection drop operation to the timeline.
pub async fn record_collection_drop(
    store: &TimelineStore,
    ctx: &RecordContext,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::CollectionDrop,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record a collection rename operation to the timeline.
pub async fn record_collection_rename(
    store: &TimelineStore,
    ctx: &RecordContext,
    target_collection: &str,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::CollectionRename,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(target_collection.to_string()))
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an import operation to the timeline.
pub async fn record_import(
    store: &TimelineStore,
    ctx: &RecordContext,
    source_summary_json: &str,
    inserted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Import,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(source_summary_json.to_string()))
    .inserted_count(inserted_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an export operation to the timeline.
pub async fn record_export(
    store: &TimelineStore,
    ctx: &RecordContext,
    source_summary_json: &str,
    exported_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Export,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(source_summary_json.to_string()))
    .returned_count(exported_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record a dump operation for a single collection to the timeline.
pub async fn record_dump_collection(
    store: &TimelineStore,
    ctx: &RecordContext,
    dumped_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Dump,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .returned_count(dumped_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record a restore operation for a single collection to the timeline.
pub async fn record_restore_collection(
    store: &TimelineStore,
    ctx: &RecordContext,
    inserted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::Restore,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .inserted_count(inserted_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an index creation to the timeline.
pub async fn record_index_create(
    store: &TimelineStore,
    ctx: &RecordContext,
    index_definition_json: &str,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::IndexCreate,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .update_json(Some(index_definition_json.to_string()))
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record an index drop to the timeline.
pub async fn record_index_drop(
    store: &TimelineStore,
    ctx: &RecordContext,
    index_name: &str,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let entry = TimelineEntry::builder(
        uuid::Uuid::new_v4().to_string(),
        ctx.profile_id.clone(),
        OperationKind::IndexDrop,
    )
    .connection_id(ctx.connection_id.clone())
    .database(ctx.database.clone())
    .collection(ctx.collection.clone())
    .actor(ctx.actor.clone())
    .environment_tag(ctx.environment_tag.clone())
    .query_json(Some(index_name.to_string()))
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::timeline_store::{OperationKind, TimelineFilter, TimelineStore};

    fn test_ctx(profile_id: &str) -> RecordContext {
        RecordContext::new(
            profile_id.into(),
            "conn-1".into(),
            "test-db".into(),
            "test-col".into(),
        )
    }

    #[tokio::test]
    async fn record_find_creates_entry() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");
        record_find(&store, &ctx, r#"{"status":"active"}"#, 42, 120, false).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, OperationKind::Find);
        assert_eq!(entries[0].query_json, Some(r#"{"status":"active"}"#.into()));
        assert_eq!(entries[0].returned_count, Some(42));
        assert_eq!(entries[0].execution_ms, Some(120));
        assert!(!entries[0].errored);
    }

    #[tokio::test]
    async fn record_aggregate_creates_entry() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");
        record_aggregate(&store, &ctx, r#"[{"$match":{"x":1}}]"#, 10, 200, false).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, OperationKind::Aggregate);
        assert_eq!(entries[0].query_json, Some(r#"[{"$match":{"x":1}}]"#.into()));
        assert_eq!(entries[0].returned_count, Some(10));
    }

    #[tokio::test]
    async fn record_insert_one_vs_many() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_insert(
            &store,
            &ctx,
            OperationKind::InsertOne,
            r#"{"name":"A"}"#,
            1,
            50,
            false,
            None,
        )
        .await;
        record_insert(
            &store,
            &ctx,
            OperationKind::InsertMany,
            r#"[{"name":"A"},{"name":"B"}]"#,
            2,
            80,
            false,
            None,
        )
        .await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 2);

        let one = entries.iter().find(|e| e.inserted_count == Some(1)).unwrap();
        assert_eq!(one.kind, OperationKind::InsertOne);

        let many = entries.iter().find(|e| e.inserted_count == Some(2)).unwrap();
        assert_eq!(many.kind, OperationKind::InsertMany);
    }

    #[tokio::test]
    async fn record_insert_many_helper_uses_correct_kind_even_for_one_doc() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_insert_many(&store, &ctx, r#"[{"name":"A"}]"#, 1, 40, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, OperationKind::InsertMany);
        assert_eq!(entries[0].inserted_count, Some(1));
    }

    #[tokio::test]
    async fn record_update_one_vs_many_and_replace() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_update(
            &store,
            &ctx,
            OperationKind::UpdateOne,
            r#"{"_id":1}"#,
            r#"{"$set":{"x":1}}"#,
            1,
            1,
            30,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;
        record_update(
            &store,
            &ctx,
            OperationKind::UpdateMany,
            r#"{}"#,
            r#"{"$set":{"x":1}}"#,
            5,
            5,
            60,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;
        record_replace_one(
            &store,
            &ctx,
            r#"{"_id":1}"#,
            r#"{"name":"B"}"#,
            1,
            1,
            25,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;

        let entries = store.list(TimelineFilter::default()).await;
        let one = entries.iter().find(|e| e.kind == OperationKind::UpdateOne).unwrap();
        assert_eq!(one.matched_count, Some(1));

        let many = entries.iter().find(|e| e.kind == OperationKind::UpdateMany).unwrap();
        assert_eq!(many.matched_count, Some(5));

        let repl = entries.iter().find(|e| e.kind == OperationKind::ReplaceOne).unwrap();
        assert_eq!(repl.update_json, Some(r#"{"name":"B"}"#.into()));
    }

    #[tokio::test]
    async fn record_update_many_with_zero_matches_keeps_kind() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_update(
            &store,
            &ctx,
            OperationKind::UpdateMany,
            r#"{"_id":999}"#,
            r#"{"$set":{"x":1}}"#,
            0,
            0,
            10,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries[0].kind, OperationKind::UpdateMany);
    }

    #[tokio::test]
    async fn record_delete_one_vs_many() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_delete(
            &store,
            &ctx,
            OperationKind::DeleteOne,
            r#"{"_id":1}"#,
            1,
            20,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;
        record_delete(
            &store,
            &ctx,
            OperationKind::DeleteMany,
            r#"{}"#,
            10,
            100,
            false,
            None,
            SafeChangeMetadata::default(),
        )
        .await;

        let entries = store.list(TimelineFilter::default()).await;
        let one = entries.iter().find(|e| e.kind == OperationKind::DeleteOne).unwrap();
        assert_eq!(one.deleted_count, Some(1));

        let many = entries.iter().find(|e| e.kind == OperationKind::DeleteMany).unwrap();
        assert_eq!(many.deleted_count, Some(10));
    }

    #[tokio::test]
    async fn record_index_create_and_drop() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_index_create(&store, &ctx, r#"{"field":1}"#, 500, false, None).await;
        record_index_drop(&store, &ctx, "idx_name", 100, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 2);

        let create = entries.iter().find(|e| e.kind == OperationKind::IndexCreate).unwrap();
        assert_eq!(create.update_json, Some(r#"{"field":1}"#.into()));

        let drop = entries.iter().find(|e| e.kind == OperationKind::IndexDrop).unwrap();
        assert_eq!(drop.query_json, Some("idx_name".into()));
    }

    #[tokio::test]
    async fn record_collection_operations() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_collection_create(&store, &ctx, 100, false, None).await;
        record_collection_drop(&store, &ctx, 50, false, None).await;
        record_collection_rename(&store, &ctx, "new_col", 75, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.kind == OperationKind::CollectionCreate));
        assert!(entries.iter().any(|e| e.kind == OperationKind::CollectionDrop));

        let rename = entries.iter().find(|e| e.kind == OperationKind::CollectionRename).unwrap();
        assert_eq!(rename.query_json, Some("new_col".into()));
    }

    #[tokio::test]
    async fn record_bulk_operations() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_import(
            &store,
            &ctx,
            r#"{"source":"file.json","format":"json"}"#,
            100,
            200,
            false,
            None,
        )
        .await;
        record_export(
            &store,
            &ctx,
            r#"{"destination":"out.csv","format":"csv"}"#,
            250,
            300,
            false,
            None,
        )
        .await;
        record_dump_collection(&store, &ctx, 500, 400, false, None).await;
        record_restore_collection(&store, &ctx, 50, 150, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        let import = entries.iter().find(|e| e.kind == OperationKind::Import).unwrap();
        assert_eq!(import.inserted_count, Some(100));
        assert_eq!(import.query_json, Some(r#"{"source":"file.json","format":"json"}"#.into()));

        let export = entries.iter().find(|e| e.kind == OperationKind::Export).unwrap();
        assert_eq!(export.returned_count, Some(250));

        let dump = entries.iter().find(|e| e.kind == OperationKind::Dump).unwrap();
        assert_eq!(dump.returned_count, Some(500));

        let restore = entries.iter().find(|e| e.kind == OperationKind::Restore).unwrap();
        assert_eq!(restore.inserted_count, Some(50));
    }

    #[tokio::test]
    async fn record_with_error_message() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_insert(
            &store,
            &ctx,
            OperationKind::InsertOne,
            r#"bad json"#,
            0,
            0,
            true,
            Some("BSON parse error".into()),
        )
        .await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].errored);
        assert_eq!(entries[0].error_message, Some("BSON parse error".into()));
    }

    #[tokio::test]
    async fn multiple_profiles_are_isolated() {
        let store = TimelineStore::new();
        let ctx_a = test_ctx("profile-a");
        let ctx_b = test_ctx("profile-b");

        record_find(&store, &ctx_a, r#"{}"#, 10, 50, false).await;
        record_find(&store, &ctx_b, r#"{}"#, 20, 60, false).await;

        let a_entries = store.list(TimelineFilter {
            profile_id: Some("profile-a".into()),
            ..Default::default()
        }).await;
        assert_eq!(a_entries.len(), 1);
        assert_eq!(a_entries[0].returned_count, Some(10));

        let b_entries = store.list(TimelineFilter {
            profile_id: Some("profile-b".into()),
            ..Default::default()
        }).await;
        assert_eq!(b_entries.len(), 1);
        assert_eq!(b_entries[0].returned_count, Some(20));
    }

    #[tokio::test]
    async fn record_update_with_safe_change_metadata_stores_risk_and_rollback() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_update(
            &store,
            &ctx,
            OperationKind::UpdateMany,
            r#"{"status":"trial"}"#,
            r#"{"$set":{"status":"expired"}}"#,
            100,
            100,
            50,
            false,
            None,
            SafeChangeMetadata {
                risk_score: Some(75),
                risk_reasons: Some(vec!["production environment".into(), "updateMany".into()]),
                rollback_script: Some(r#"{"op":"bulkWrite","ops":[{"updateOne":{"filter":{"_id":1},"update":{"$set":{"status":"trial"}}}}]}"#.into()),
                rollback_level: crate::mongo::timeline_store::RollbackLevel::Full,
            },
        ).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.risk_score, Some(75));
        assert_eq!(e.risk_reasons, Some(vec!["production environment".into(), "updateMany".into()]));
        assert!(e.rollback_script.is_some());
        assert!(e.rollback_script.as_ref().unwrap().contains("bulkWrite"));
        assert_eq!(e.rollback_level, crate::mongo::timeline_store::RollbackLevel::Full);
    }

    #[tokio::test]
    async fn record_delete_with_safe_change_metadata_stores_risk_and_rollback() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_delete(
            &store,
            &ctx,
            OperationKind::DeleteMany,
            r#"{"status":"failed"}"#,
            42,
            30,
            false,
            None,
            SafeChangeMetadata {
                risk_score: Some(85),
                risk_reasons: Some(vec!["deleteMany".into(), "broad filter".into()]),
                rollback_script: Some(r#"{"op":"insertMany","docs":[{"_id":1}]}"#.into()),
                rollback_level: crate::mongo::timeline_store::RollbackLevel::Full,
            },
        ).await;

        let entries = store.list(TimelineFilter::default()).await;
        let e = &entries[0];
        assert_eq!(e.risk_score, Some(85));
        assert_eq!(e.rollback_level, crate::mongo::timeline_store::RollbackLevel::Full);
        assert!(e.rollback_script.is_some());
    }

    #[tokio::test]
    async fn record_update_without_safe_change_metadata_stores_none() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_update(
            &store,
            &ctx,
            OperationKind::UpdateOne,
            r#"{"_id":1}"#,
            r#"{"$set":{"x":1}}"#,
            1,
            1,
            10,
            false,
            None,
            SafeChangeMetadata::default(),
        ).await;

        let entries = store.list(TimelineFilter::default()).await;
        let e = &entries[0];
        assert_eq!(e.risk_score, None);
        assert_eq!(e.risk_reasons, None);
        assert_eq!(e.rollback_script, None);
        assert_eq!(e.rollback_level, crate::mongo::timeline_store::RollbackLevel::None);
    }

    #[tokio::test]
    async fn record_replace_one_with_safe_change_metadata() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_replace_one(
            &store,
            &ctx,
            r#"{"_id":1}"#,
            r#"{"name":"NewName"}"#,
            1,
            1,
            15,
            false,
            None,
            SafeChangeMetadata {
                risk_score: Some(30),
                risk_reasons: Some(vec!["replaceOne".into()]),
                rollback_script: Some(r#"{"op":"replaceOne","doc":{"name":"OldName"}}"#.into()),
                rollback_level: crate::mongo::timeline_store::RollbackLevel::Full,
            },
        ).await;

        let entries = store.list(TimelineFilter::default()).await;
        let e = &entries[0];
        assert_eq!(e.risk_score, Some(30));
        assert!(e.rollback_script.is_some());
    }
}
