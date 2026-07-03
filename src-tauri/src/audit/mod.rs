//! Tauri-specific audit module: re-exports the core audit types from
//! `audit_service` and adds Tauri command handlers, dev-stack orchestration,
//! and audit-mode configuration that depend on Tauri APIs.
//!
//! The core audit logic (Merkle tree, Stellar commitments, change streams,
//! oplog, attestation, etc.) lives in the `audit_service` crate. This module
//! re-exports everything so the rest of `app_lib` can use `crate::audit::*`
//! without caring about the split.

// Re-export all core audit submodules from the audit_service crate.
pub use audit_service::audit::{
    attestation, change_stream, crypto, dev_proxy, dev_setup, epoch, interceptor, ipfs, leaf_from_payload,
    oplog, oplog_canon, pinata, reader, sled_store, stellar, stellar_native, stellar_rpc,
    verification_store, AuditEvent, AuditLog, DomainRetentionRoot,
};

// Tauri-specific submodules that depend on tauri::AppHandle / tauri::State.
pub mod audit_mode;
pub mod commands;
pub mod dev_stack;

// Re-export dev_proxy functions as Tauri commands (the #[tauri::command]
// wrappers were split from the proxy logic when the core moved to audit_service).
use crate::error::AppResult;
use serde_json::Value;

/// GET from a local audit service (port + path). Returns the raw JSON.
#[tauri::command]
pub async fn audit_dev_proxy_get(port: u16, path: String) -> AppResult<Value> {
    Ok(dev_proxy::proxy_get(port, &path).await?)
}

/// POST to a local audit service (port + path + JSON body).
#[tauri::command]
pub async fn audit_dev_proxy_post(
    port: u16,
    path: String,
    body: Option<Value>,
) -> AppResult<Value> {
    Ok(dev_proxy::proxy_post(port, &path, body.unwrap_or(Value::Null)).await?)
}
