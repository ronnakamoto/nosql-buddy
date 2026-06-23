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
    shell
        .eval(
            entry,
            request.script,
            request.active_database.unwrap_or(initial_db),
        )
        .await
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
