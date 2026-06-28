//! IPC command for the mongo shell.
//!
//! The shell is a real JavaScript REPL backed by `boa_engine`
//! (see `mongo::shell`). Each connection gets its own
//! `Shell` which runs on a dedicated OS thread + a
//! current-thread Tokio runtime. `eval_shell` posts the
//! script to the thread and waits for the result.

use serde::Deserialize;
use std::sync::Arc;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::mongo::client_registry::ClientEntry;
use crate::mongo::operation_recorder::RecordContext;
use crate::mongo::schema::compute_schema_report;
use crate::mongo::shell::ShellResponse;
use crate::mongo::shell_autocomplete::{
    autocomplete_context, filter_by_prefix, partial_token, AutocompleteResponse, CompletionKind,
    COLLECTION_METHODS, GLOBAL_FUNCTIONS, QUERY_OPERATORS, UPDATE_OPERATORS,
};
use crate::state::AppState;
use futures_util::TryStreamExt;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellRequestDto {
    pub connection_id: String,
    pub script: String,
    /// Initial active database. If None, the shell reuses
    /// its last `use <db>` value.
    pub active_database: Option<String>,
    /// Default database to use when the shell is fresh and
    /// no `active_database` is provided.
    pub fallback_database: Option<String>,
}

/// Run a mongo-shell script and return the captured output.
#[tauri::command]
pub async fn eval_shell(
    state: State<'_, AppState>,
    request: ShellRequestDto,
) -> AppResult<ShellResponse> {
    let entry: Arc<ClientEntry> = Arc::new(state.clients.get(&request.connection_id).await?);
    let initial_db = request
        .active_database
        .clone()
        .or(request.fallback_database.clone())
        .unwrap_or_else(|| "admin".to_string());

    let shell = state
        .shell_registry
        .get_or_create(request.connection_id.clone(), initial_db.clone())
        .await?;
    let mut response = shell
        .eval(
            entry.clone(),
            request.script,
            request.active_database.unwrap_or(initial_db),
            Some(state.audit_log.clone()),
        )
        .await?;

    // Record any database operations the shell performed into the Data Timeline.
    if !response.operations.is_empty() {
        let profile_id = entry.profile_id.clone();
        let connection_id = request.connection_id.clone();
        for op in &response.operations {
            let ctx = RecordContext::new(
                profile_id.clone(),
                connection_id.clone(),
                op.database.clone(),
                op.collection.clone(),
            );
            match op.kind {
                crate::mongo::timeline_store::OperationKind::Find
                | crate::mongo::timeline_store::OperationKind::Aggregate => {
                    // Not currently emitted by the shell; skip.
                }
                crate::mongo::timeline_store::OperationKind::InsertOne => {
                    crate::mongo::operation_recorder::record_insert(
                        &state.timeline,
                        &ctx,
                        op.kind,
                        op.update_json.as_deref().unwrap_or(""),
                        op.inserted_count.unwrap_or(1),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::InsertMany => {
                    crate::mongo::operation_recorder::record_insert_many(
                        &state.timeline,
                        &ctx,
                        op.update_json.as_deref().unwrap_or(""),
                        op.inserted_count.unwrap_or(0),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::UpdateOne
                | crate::mongo::timeline_store::OperationKind::UpdateMany
                | crate::mongo::timeline_store::OperationKind::ReplaceOne => {
                    crate::mongo::operation_recorder::record_update(
                        &state.timeline,
                        &ctx,
                        op.kind,
                        op.query_json.as_deref().unwrap_or(""),
                        op.update_json.as_deref().unwrap_or(""),
                        op.matched_count.unwrap_or(0),
                        op.modified_count.unwrap_or(0),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::DeleteOne
                | crate::mongo::timeline_store::OperationKind::DeleteMany => {
                    crate::mongo::operation_recorder::record_delete(
                        &state.timeline,
                        &ctx,
                        op.kind,
                        op.query_json.as_deref().unwrap_or(""),
                        op.deleted_count.unwrap_or(0),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::IndexCreate => {
                    crate::mongo::operation_recorder::record_index_create(
                        &state.timeline,
                        &ctx,
                        op.update_json.as_deref().unwrap_or(""),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::IndexDrop => {
                    crate::mongo::operation_recorder::record_index_drop(
                        &state.timeline,
                        &ctx,
                        op.query_json.as_deref().unwrap_or(""),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::CollectionCreate => {
                    crate::mongo::operation_recorder::record_collection_create(
                        &state.timeline,
                        &ctx,
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::CollectionDrop => {
                    crate::mongo::operation_recorder::record_collection_drop(
                        &state.timeline,
                        &ctx,
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                crate::mongo::timeline_store::OperationKind::CollectionRename => {
                    crate::mongo::operation_recorder::record_collection_rename(
                        &state.timeline,
                        &ctx,
                        op.query_json.as_deref().unwrap_or(""),
                        op.execution_ms.unwrap_or(0),
                        op.errored,
                        op.error_message.clone(),
                    )
                    .await;
                }
                _ => {}
            }
        }
        // Drop the operations from the response before sending it to the frontend
        // to keep the payload focused on UI-visible data.
        response.operations.clear();
    }

    Ok(response)
}

/// Request payload for `shell_autocomplete`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocompleteRequestDto {
    pub connection_id: String,
    /// The full script text up to (but not including) the cursor.
    pub text_before_cursor: String,
    pub active_database: Option<String>,
    pub fallback_database: Option<String>,
}

/// Return context-aware autocomplete suggestions for the shell.
///
/// The pure context-detection logic lives in
/// `mongo::shell_autocomplete`; this command fetches the actual
/// names (collections, fields) from the live Mongo connection.
#[tauri::command]
pub async fn shell_autocomplete(
    state: State<'_, AppState>,
    request: AutocompleteRequestDto,
) -> AppResult<AutocompleteResponse> {
    let entry = state.clients.get(&request.connection_id).await?;
    let active_db = request
        .active_database
        .clone()
        .or(request.fallback_database.clone())
        .unwrap_or_else(|| "admin".to_string());

    let kind = autocomplete_context(&request.text_before_cursor);
    let prefix = partial_token(&request.text_before_cursor);

    let items = match &kind {
        CompletionKind::Databases => {
            let names = entry
                .client
                .list_database_names()
                .await
                .map_err(|e| AppError::Mongo(e.to_string()))?;
            filter_by_prefix(names.iter().map(|s| s.as_str()), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "database".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::Collections => {
            let names = entry
                .client
                .database(&active_db)
                .list_collection_names()
                .await
                .map_err(|e| AppError::Mongo(e.to_string()))?;
            filter_by_prefix(names.iter().map(|s| s.as_str()), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "collection".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::Methods { .. } => {
            // Method names are static — no Mongo call needed.
            filter_by_prefix(COLLECTION_METHODS.iter().copied(), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "method".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::Fields { collection } => {
            // Sample the schema to get field names. We use a
            // small sample size since we only need field names,
            // not distributions.
            let coll = entry
                .client
                .database(&active_db)
                .collection::<bson::Document>(collection);
            let cursor = coll
                .aggregate(vec![bson::doc! { "$sample": { "size": 100_i64 } }])
                .await
                .map_err(|e| AppError::Mongo(e.to_string()))?;
            let docs: Vec<bson::Document> = cursor
                .try_collect()
                .await
                .map_err(|e| AppError::Mongo(e.to_string()))?;
            let report = compute_schema_report(&docs);
            let field_names: Vec<String> = report.fields.iter().map(|f| f.name.clone()).collect();
            filter_by_prefix(field_names.iter().map(|s| s.as_str()), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "field".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::Operators { method } => {
            // Operator names are static — no Mongo call needed.
            // Update-operator methods get update operators; all
            // other methods (find, findOne, delete*, count*,
            // distinct, etc.) default to query operators since
            // filter documents are the more common context.
            let ops: &[&str] = match method.as_str() {
                "updateOne" | "updateMany" | "findOneAndUpdate" => UPDATE_OPERATORS,
                _ => QUERY_OPERATORS,
            };
            filter_by_prefix(ops.iter().copied(), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "operator".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::Globals => {
            // Global function names are static — no Mongo call
            // needed.
            filter_by_prefix(GLOBAL_FUNCTIONS.iter().copied(), &prefix)
                .into_iter()
                .map(|mut item| {
                    item.detail = "global".to_string();
                    item
                })
                .collect()
        }
        CompletionKind::None => Vec::new(),
    };

    Ok(AutocompleteResponse { kind, items })
}
