//! Event payload types and emission helpers.
//!
//! The Rust backend emits events with `app.emit()`; the frontend listens with
//! `listen()`. Payloads are serializable. Keep payload types here so the
//! frontend's `src/ipc/events.ts` can mirror them.

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Payload for the `connection-opened` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionOpenedPayload {
    pub connection_id: String,
    pub profile_id: String,
    pub name: String,
}

/// Payload for the `connection-closed` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionClosedPayload {
    pub connection_id: String,
    pub profile_id: String,
    pub at: String,
}

/// Payload for the `audit-setup-progress` event: one (secret-redacted) line of
/// output from the audit setup wizard, streamed live to the UI so users can see
/// progress (key generation, funding, contract deploy, attester authorization).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditSetupProgressPayload {
    pub line: String,
}

/// Emit a single line of audit-setup progress. Best-effort: emission failures
/// are ignored so they never abort the setup run.
pub fn emit_audit_setup_progress(app: &AppHandle, line: &str) {
    let _ = app.emit(
        "audit-setup-progress",
        AuditSetupProgressPayload {
            line: line.to_string(),
        },
    );
}

/// Payload for the `import-export-progress` event: a periodic snapshot of a
/// running import or export job. Emitted at most a few times per second
/// (throttled in the pipeline) so a million-row job never floods the IPC
/// channel. `total` is `None` when the source size is not cheaply known.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportExportProgressPayload {
    pub job_id: String,
    /// One of: "reading", "writing", "copying", "done", "error".
    pub phase: String,
    pub processed: u64,
    pub total: Option<u64>,
    pub message: String,
}

/// Emit a single import/export progress snapshot. Best-effort: emission
/// failures are ignored so they never abort the job.
pub fn emit_import_export_progress(app: &AppHandle, payload: ImportExportProgressPayload) {
    let _ = app.emit("import-export-progress", payload);
}

pub fn emit_connection_opened(
    app: &AppHandle,
    connection_id: &str,
    profile_id: &str,
    name: &str,
) -> tauri::Result<()> {
    app.emit(
        "connection-opened",
        ConnectionOpenedPayload {
            connection_id: connection_id.to_string(),
            profile_id: profile_id.to_string(),
            name: name.to_string(),
        },
    )
}

pub fn emit_connection_closed(
    app: &AppHandle,
    connection_id: &str,
    profile_id: &str,
) -> tauri::Result<()> {
    app.emit(
        "connection-closed",
        ConnectionClosedPayload {
            connection_id: connection_id.to_string(),
            profile_id: profile_id.to_string(),
            at: chrono_now(),
        },
    )
}

/// Payload for the `job-status-changed` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatusChangedPayload {
    pub job_id: String,
    pub status: String,
    pub message: String,
    pub finished_at: Option<String>,
}

/// Emit when a job transitions status (Queued→Running→Done/Failed/Cancelled).
pub fn emit_job_status_changed(app: &AppHandle, job_id: &str, status: &str, message: &str, finished_at: Option<String>) {
    let _ = app.emit(
        "job-status-changed",
        JobStatusChangedPayload {
            job_id: job_id.to_string(),
            status: status.to_string(),
            message: message.to_string(),
            finished_at,
        },
    );
}

/// Payload for the `job-log-entry` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLogEntryPayload {
    pub job_id: String,
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

/// Emit a single job log line. Best-effort: never blocks the job.
pub fn emit_job_log_entry(app: &AppHandle, job_id: &str, timestamp: &str, level: &str, message: &str) {
    let _ = app.emit(
        "job-log-entry",
        JobLogEntryPayload {
            job_id: job_id.to_string(),
            timestamp: timestamp.to_string(),
            level: level.to_string(),
            message: message.to_string(),
        },
    );
}

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
