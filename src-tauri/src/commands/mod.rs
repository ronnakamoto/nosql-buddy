//! Command handlers, grouped by domain (SRP: one file per domain).
//!
//! Each handler is `#[tauri::command]`, returns `Result<T, AppError>`, and never
//! panics (no `unwrap`/`expect`). New domains get a new file and a re-export here.

pub mod connections;
pub mod data_model;
pub mod driver_code;
pub mod dump;
pub mod export;
pub mod import;
pub mod jobs;
pub mod mongo;
pub mod restore;
pub mod settings;
pub mod shell;
pub mod sql;
pub mod system;
pub mod timeline;

use crate::error::AppResult;

/// A simple ping command used to verify the IPC round-trip end-to-end.
#[tauri::command]
pub async fn ping(message: String) -> AppResult<String> {
    Ok(format!("pong: {message}"))
}
