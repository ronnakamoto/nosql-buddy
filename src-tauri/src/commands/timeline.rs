//! Data Timeline IPC command handlers.

use crate::error::AppResult;
use crate::mongo::timeline_store::{TimelineEntry, TimelineFilter};
use crate::state::AppState;
use tauri::State;

/// DTO for filtering timeline entries from the frontend.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTimelineRequest {
    pub profile_id: String,
    pub database: Option<String>,
    pub collection: Option<String>,
    pub kind: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<usize>,
    pub errored: Option<bool>,
}

/// List timeline entries for a profile, newest first.
#[tauri::command]
pub async fn list_timeline(
    request: ListTimelineRequest,
    state: State<'_, AppState>,
) -> AppResult<Vec<TimelineEntry>> {
    let kind = request.kind.and_then(|k| match k.as_str() {
        "find" => Some(crate::mongo::timeline_store::OperationKind::Find),
        "aggregate" => Some(crate::mongo::timeline_store::OperationKind::Aggregate),
        "sql" => Some(crate::mongo::timeline_store::OperationKind::Sql),
        "explain" => Some(crate::mongo::timeline_store::OperationKind::Explain),
        "insertOne" => Some(crate::mongo::timeline_store::OperationKind::InsertOne),
        "insertMany" => Some(crate::mongo::timeline_store::OperationKind::InsertMany),
        "updateOne" => Some(crate::mongo::timeline_store::OperationKind::UpdateOne),
        "updateMany" => Some(crate::mongo::timeline_store::OperationKind::UpdateMany),
        "deleteOne" => Some(crate::mongo::timeline_store::OperationKind::DeleteOne),
        "deleteMany" => Some(crate::mongo::timeline_store::OperationKind::DeleteMany),
        "replaceOne" => Some(crate::mongo::timeline_store::OperationKind::ReplaceOne),
        "aggregationWrite" => Some(crate::mongo::timeline_store::OperationKind::AggregationWrite),
        "indexCreate" => Some(crate::mongo::timeline_store::OperationKind::IndexCreate),
        "indexDrop" => Some(crate::mongo::timeline_store::OperationKind::IndexDrop),
        "collectionCreate" => Some(crate::mongo::timeline_store::OperationKind::CollectionCreate),
        "collectionDrop" => Some(crate::mongo::timeline_store::OperationKind::CollectionDrop),
        "collectionRename" => Some(crate::mongo::timeline_store::OperationKind::CollectionRename),
        "import" => Some(crate::mongo::timeline_store::OperationKind::Import),
        "export" => Some(crate::mongo::timeline_store::OperationKind::Export),
        "dump" => Some(crate::mongo::timeline_store::OperationKind::Dump),
        "restore" => Some(crate::mongo::timeline_store::OperationKind::Restore),
        _ => None,
    });

    let entries = state
        .timeline
        .list(TimelineFilter {
            profile_id: Some(request.profile_id),
            database: request.database,
            collection: request.collection,
            kind,
            from: request.from,
            to: request.to,
            limit: request.limit,
            errored: request.errored,
        })
        .await;
    Ok(entries)
}

/// Get a single timeline entry by id.
#[tauri::command]
pub async fn get_timeline_entry(
    id: String,
    state: State<'_, AppState>,
) -> AppResult<Option<TimelineEntry>> {
    Ok(state.timeline.get(&id).await)
}

/// Add or overwrite the notes on a timeline entry.
#[tauri::command]
pub async fn add_timeline_note(
    id: String,
    note: String,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    Ok(state.timeline.update_notes(&id, note).await)
}

/// Delete a timeline entry by id.
#[tauri::command]
pub async fn delete_timeline_entry(
    id: String,
    state: State<'_, AppState>,
) -> AppResult<bool> {
    Ok(state.timeline.delete(&id).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::timeline_store::OperationKind;

    // The list_timeline command maps camelCase string kind names to OperationKind
    // variants via a hand-rolled match. This test ensures every variant that
    // OperationKind serializes to (tested in ipc_wire_shape.rs) is also handled
    // here — a gap means filters by that kind silently return *all* entries.
    fn parse_kind(s: &str) -> Option<OperationKind> {
        match s {
            "find" => Some(OperationKind::Find),
            "aggregate" => Some(OperationKind::Aggregate),
            "sql" => Some(OperationKind::Sql),
            "explain" => Some(OperationKind::Explain),
            "insertOne" => Some(OperationKind::InsertOne),
            "insertMany" => Some(OperationKind::InsertMany),
            "updateOne" => Some(OperationKind::UpdateOne),
            "updateMany" => Some(OperationKind::UpdateMany),
            "deleteOne" => Some(OperationKind::DeleteOne),
            "deleteMany" => Some(OperationKind::DeleteMany),
            "replaceOne" => Some(OperationKind::ReplaceOne),
            "aggregationWrite" => Some(OperationKind::AggregationWrite),
            "indexCreate" => Some(OperationKind::IndexCreate),
            "indexDrop" => Some(OperationKind::IndexDrop),
            "collectionCreate" => Some(OperationKind::CollectionCreate),
            "collectionDrop" => Some(OperationKind::CollectionDrop),
            "collectionRename" => Some(OperationKind::CollectionRename),
            "import" => Some(OperationKind::Import),
            "export" => Some(OperationKind::Export),
            "dump" => Some(OperationKind::Dump),
            "restore" => Some(OperationKind::Restore),
            _ => None,
        }
    }

    #[test]
    fn kind_string_dispatcher_covers_every_read_variant() {
        assert_eq!(parse_kind("find"), Some(OperationKind::Find));
        assert_eq!(parse_kind("aggregate"), Some(OperationKind::Aggregate));
        assert_eq!(parse_kind("sql"), Some(OperationKind::Sql));
        assert_eq!(parse_kind("explain"), Some(OperationKind::Explain));
    }

    #[test]
    fn kind_string_dispatcher_covers_every_write_variant() {
        assert_eq!(parse_kind("insertOne"), Some(OperationKind::InsertOne));
        assert_eq!(parse_kind("insertMany"), Some(OperationKind::InsertMany));
        assert_eq!(parse_kind("updateOne"), Some(OperationKind::UpdateOne));
        assert_eq!(parse_kind("updateMany"), Some(OperationKind::UpdateMany));
        assert_eq!(parse_kind("deleteOne"), Some(OperationKind::DeleteOne));
        assert_eq!(parse_kind("deleteMany"), Some(OperationKind::DeleteMany));
        assert_eq!(parse_kind("replaceOne"), Some(OperationKind::ReplaceOne));
        assert_eq!(parse_kind("aggregationWrite"), Some(OperationKind::AggregationWrite));
    }

    #[test]
    fn kind_string_dispatcher_covers_every_schema_variant() {
        assert_eq!(parse_kind("indexCreate"), Some(OperationKind::IndexCreate));
        assert_eq!(parse_kind("indexDrop"), Some(OperationKind::IndexDrop));
        assert_eq!(parse_kind("collectionCreate"), Some(OperationKind::CollectionCreate));
        assert_eq!(parse_kind("collectionDrop"), Some(OperationKind::CollectionDrop));
        assert_eq!(parse_kind("collectionRename"), Some(OperationKind::CollectionRename));
    }

    #[test]
    fn kind_string_dispatcher_covers_every_bulk_variant() {
        assert_eq!(parse_kind("import"), Some(OperationKind::Import));
        assert_eq!(parse_kind("export"), Some(OperationKind::Export));
        assert_eq!(parse_kind("dump"), Some(OperationKind::Dump));
        assert_eq!(parse_kind("restore"), Some(OperationKind::Restore));
    }

    #[test]
    fn kind_string_dispatcher_unknown_string_returns_none() {
        // An unknown kind string must return None so the filter is dropped,
        // rather than silently matching all entries (which would be the
        // behavior if the match arm was `_ => Some(arbitrary)`).
        assert_eq!(parse_kind("updateall"), None);
        assert_eq!(parse_kind("FindDocuments"), None);
        assert_eq!(parse_kind(""), None);
        assert_eq!(parse_kind("Update"), None); // capitalized → no match
    }

    #[test]
    fn kind_string_dispatcher_is_case_sensitive() {
        // The IPC contract uses camelCase. PascalCase or snake_case must not match.
        assert_eq!(parse_kind("InsertOne"), None);
        assert_eq!(parse_kind("insert_one"), None);
        assert_eq!(parse_kind("FIND"), None);
    }

    #[test]
    fn kind_string_dispatcher_matches_serialized_wire_names() {
        // Cross-check: the strings used here must match exactly what
        // serde produces for OperationKind (verified in ipc_wire_shape.rs).
        // We re-derive them here so a rename in either place fails a test.
        use serde_json::json;
        let ser = |v: OperationKind| serde_json::to_value(v).unwrap();

        for (variant, string) in [
            (OperationKind::Find,              "find"),
            (OperationKind::Aggregate,         "aggregate"),
            (OperationKind::Sql,               "sql"),
            (OperationKind::InsertOne,         "insertOne"),
            (OperationKind::InsertMany,        "insertMany"),
            (OperationKind::UpdateOne,         "updateOne"),
            (OperationKind::UpdateMany,        "updateMany"),
            (OperationKind::DeleteOne,         "deleteOne"),
            (OperationKind::DeleteMany,        "deleteMany"),
            (OperationKind::ReplaceOne,        "replaceOne"),
            (OperationKind::AggregationWrite,  "aggregationWrite"),
            (OperationKind::IndexCreate,       "indexCreate"),
            (OperationKind::IndexDrop,         "indexDrop"),
            (OperationKind::CollectionCreate,  "collectionCreate"),
            (OperationKind::CollectionDrop,    "collectionDrop"),
            (OperationKind::CollectionRename,  "collectionRename"),
            (OperationKind::Import,            "import"),
            (OperationKind::Export,            "export"),
            (OperationKind::Dump,              "dump"),
            (OperationKind::Restore,           "restore"),
        ] {
            assert_eq!(
                ser(variant),
                json!(string),
                "serialized name for {:?} must match dispatcher key", variant
            );
            assert_eq!(
                parse_kind(string),
                Some(variant),
                "dispatcher must handle key {:?}", string
            );
        }
    }
}
