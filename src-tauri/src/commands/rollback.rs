//! IPC commands for rollback execution.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::state::AppState;
use tauri::State;

/// Request to execute a rollback script from a timeline entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRollbackRequest {
    pub timeline_entry_id: String,
    pub connection_id: String,
}

/// Result of a rollback execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecuteRollbackResult {
    /// The rollback script that was executed.
    pub script: String,
    /// Shell output (stdout) from executing the script.
    pub output: String,
    /// Whether any errors were reported.
    pub errored: bool,
    /// Error message if errored.
    pub error_message: Option<String>,
}

/// Execute the rollback script stored in a timeline entry.
///
/// The rollback script is run through the existing shell evaluator against
/// the specified connection. The timeline entry must have a non-null
/// `rollback_script`; otherwise an error is returned.
#[tauri::command]
pub async fn execute_rollback(
    request: ExecuteRollbackRequest,
    state: State<'_, AppState>,
) -> AppResult<ExecuteRollbackResult> {
    // Retrieve the timeline entry.
    let entry = state
        .timeline
        .get(&request.timeline_entry_id)
        .await
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "timeline entry '{}' not found",
                request.timeline_entry_id
            ))
        })?;

    // Ensure there is a rollback script to execute.
    let script = entry.rollback_script.ok_or_else(|| {
        AppError::Validation(
            "this timeline entry has no rollback script; the operation was \
             either a read, had rollback_level=none, or pre-image capture \
             was not available"
                .into(),
        )
    })?;

    // Get the client entry for this connection.
    let client_entry = Arc::new(state.clients.get(&request.connection_id).await?);
    let initial_db = entry.database.clone();

    // Get or create a shell for this connection.
    let shell = state
        .shell_registry
        .get_or_create(request.connection_id.clone(), initial_db.clone())
        .await?;

    // Execute via the shell evaluator.
    let shell_response = shell
        .eval(client_entry, script.clone(), initial_db, None)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Derive a flat output string from the shell output lines.
    let output_lines: Vec<String> = shell_response
        .outputs
        .iter()
        .map(|o| match o {
            crate::mongo::shell::ShellOutput::Text { value } => value.clone(),
            crate::mongo::shell::ShellOutput::Json { value } => value.to_string(),
            crate::mongo::shell::ShellOutput::Error { value } => value.clone(),
            crate::mongo::shell::ShellOutput::Table { value } => {
                value.columns.join("\t")
            }
        })
        .collect();
    let output = output_lines.join("\n");

    // Check whether any error outputs were emitted.
    let errored = shell_response.outputs.iter().any(|o| {
        matches!(o, crate::mongo::shell::ShellOutput::Error { .. })
    });

    let error_message = if errored {
        output.lines().last().map(|s| s.to_string())
    } else {
        None
    };

    Ok(ExecuteRollbackResult {
        script,
        output,
        errored,
        error_message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execute_rollback_request_is_camel_case() {
        let req = ExecuteRollbackRequest {
            timeline_entry_id: "entry-123".into(),
            connection_id: "conn-abc".into(),
        };
        let j = serde_json::to_value(&req).unwrap();
        assert_eq!(j["timelineEntryId"], serde_json::json!("entry-123"));
        assert_eq!(j["connectionId"], serde_json::json!("conn-abc"));
        assert!(j.get("timeline_entry_id").is_none(), "snake_case must be absent");
    }

    #[test]
    fn execute_rollback_result_is_camel_case() {
        let res = ExecuteRollbackResult {
            script: "db.users.insertMany([])".into(),
            output: "inserted 0 documents".into(),
            errored: false,
            error_message: None,
        };
        let j = serde_json::to_value(&res).unwrap();
        assert_eq!(j["script"], serde_json::json!("db.users.insertMany([])"));
        assert_eq!(j["errored"], serde_json::json!(false));
        assert!(j["errorMessage"].is_null());
        assert!(j.get("error_message").is_none(), "snake_case must be absent");
    }

    #[test]
    fn execute_rollback_result_with_error_round_trips() {
        let res = ExecuteRollbackResult {
            script: "db.c.updateMany({}, {$set:{x:1}})".into(),
            output: "MongoError: not authorized".into(),
            errored: true,
            error_message: Some("MongoError: not authorized".into()),
        };
        let json = serde_json::to_string(&res).unwrap();
        let back: ExecuteRollbackResult = serde_json::from_str(&json).unwrap();
        assert!(back.errored);
        assert_eq!(back.error_message, Some("MongoError: not authorized".into()));
    }
}
