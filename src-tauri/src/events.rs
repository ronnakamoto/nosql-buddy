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

fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_string())
}
