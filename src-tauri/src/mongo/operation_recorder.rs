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
pub async fn record_insert(
    store: &TimelineStore,
    ctx: &RecordContext,
    document_json: &str,
    inserted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let kind = if inserted_count > 1 {
        OperationKind::InsertMany
    } else {
        OperationKind::InsertOne
    };

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
pub async fn record_update(
    store: &TimelineStore,
    ctx: &RecordContext,
    filter_json: &str,
    update_json: &str,
    matched_count: u64,
    modified_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let kind = if matched_count > 1 {
        OperationKind::UpdateMany
    } else {
        OperationKind::UpdateOne
    };

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
    .query_json(Some(filter_json.to_string()))
    .update_json(Some(update_json.to_string()))
    .matched_count(matched_count)
    .modified_count(modified_count)
    .execution_ms(execution_ms)
    .errored(errored)
    .error_message(error_message)
    .executed_at(chrono_now())
    .build();

    store.append(entry).await;
}

/// Record a delete operation to the timeline.
pub async fn record_delete(
    store: &TimelineStore,
    ctx: &RecordContext,
    filter_json: &str,
    deleted_count: u64,
    execution_ms: u64,
    errored: bool,
    error_message: Option<String>,
) {
    let kind = if deleted_count > 1 {
        OperationKind::DeleteMany
    } else {
        OperationKind::DeleteOne
    };

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
    .query_json(Some(filter_json.to_string()))
    .deleted_count(deleted_count)
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

        record_insert(&store, &ctx, r#"{"name":"A"}"#, 1, 50, false, None).await;
        record_insert(&store, &ctx, r#"[{"name":"A"},{"name":"B"}]"#, 2, 80, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        assert_eq!(entries.len(), 2);

        let one = entries.iter().find(|e| e.inserted_count == Some(1)).unwrap();
        assert_eq!(one.kind, OperationKind::InsertOne);

        let many = entries.iter().find(|e| e.inserted_count == Some(2)).unwrap();
        assert_eq!(many.kind, OperationKind::InsertMany);
    }

    #[tokio::test]
    async fn record_update_one_vs_many() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_update(&store, &ctx, r#"{"_id":1}"#, r#"{"$set":{"x":1}}"#, 1, 1, 30, false, None).await;
        record_update(&store, &ctx, r#"{}"#, r#"{"$set":{"x":1}}"#, 5, 5, 60, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        let one = entries.iter().find(|e| e.matched_count == Some(1)).unwrap();
        assert_eq!(one.kind, OperationKind::UpdateOne);

        let many = entries.iter().find(|e| e.matched_count == Some(5)).unwrap();
        assert_eq!(many.kind, OperationKind::UpdateMany);
    }

    #[tokio::test]
    async fn record_delete_one_vs_many() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_delete(&store, &ctx, r#"{"_id":1}"#, 1, 20, false, None).await;
        record_delete(&store, &ctx, r#"{}"#, 10, 100, false, None).await;

        let entries = store.list(TimelineFilter::default()).await;
        let one = entries.iter().find(|e| e.deleted_count == Some(1)).unwrap();
        assert_eq!(one.kind, OperationKind::DeleteOne);

        let many = entries.iter().find(|e| e.deleted_count == Some(10)).unwrap();
        assert_eq!(many.kind, OperationKind::DeleteMany);
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
    async fn record_with_error_message() {
        let store = TimelineStore::new();
        let ctx = test_ctx("p1");

        record_insert(
            &store,
            &ctx,
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
}
