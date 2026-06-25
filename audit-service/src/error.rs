//! Error types for the audit service.
//!
//! `AuditError` is the single error type used across all audit modules.
//! It covers I/O, validation, internal, not-found, timeout, credential,
//! and ZK audit failures. The Tauri app wraps this via `From<AuditError>`
//! into its broader `AppError`.

use serde::Serialize;

/// The single error type returned by all audit service functions.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AuditError {
    #[error("io error: {0}")]
    Io(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("internal error: {0}")]
    Internal(String),
    #[error("mongo error: {0}")]
    Mongo(String),
    #[error("credential error: {0}")]
    Credential(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("zk audit error: {0}")]
    ZkAudit(String),
}

impl From<std::io::Error> for AuditError {
    fn from(err: std::io::Error) -> Self {
        AuditError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for AuditError {
    fn from(err: serde_json::Error) -> Self {
        AuditError::Internal(format!("serialization error: {err}"))
    }
}

impl From<tokio::time::error::Elapsed> for AuditError {
    fn from(err: tokio::time::error::Elapsed) -> Self {
        AuditError::Timeout(err.to_string())
    }
}

impl From<keyring::Error> for AuditError {
    fn from(err: keyring::Error) -> Self {
        AuditError::Credential(err.to_string())
    }
}

impl From<zk_audit::ZkAuditError> for AuditError {
    fn from(err: zk_audit::ZkAuditError) -> Self {
        AuditError::ZkAudit(err.to_string())
    }
}

impl From<mongodb::error::Error> for AuditError {
    fn from(err: mongodb::error::Error) -> Self {
        AuditError::Mongo(err.to_string())
    }
}

/// Convenience alias used across audit modules.
pub type AuditResult<T> = Result<T, AuditError>;
