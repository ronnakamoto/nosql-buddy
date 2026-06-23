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

use crate::error::AppResult;
use crate::mongo::client_registry::ClientEntry;
use crate::mongo::shell::ShellResponse;
use crate::state::AppState;

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
