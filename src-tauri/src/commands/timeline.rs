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
