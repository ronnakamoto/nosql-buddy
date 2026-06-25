//! Error types that cross the IPC boundary.
//!
//! Every command returns `Result<T, AppError>`. `AppError` derives `serde::Serialize`
//! so it reaches the frontend as a meaningful value, never as a panic. The variants
//! cover MongoDB driver failures, secret-store failures, BSON parsing issues, and
//! the generic validation/not-found categories. Sensitive values (URIs, passwords,
//! raw driver error messages that may include credentials) are passed through a
//! redactor in the `From` impls so they never cross the IPC boundary in plaintext.

use serde::Serialize;

use crate::mongo::redaction::Redactor;

/// The single error type returned by all command handlers.
///
/// One error type per domain is the SRP rule; for NoSQLBuddy a single enum with a
/// variant per failure kind is the right granularity. Split into per-domain error
/// types when a domain grows past a handful of variants.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
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
    #[error("connection not found: {0}")]
    ConnectionNotFound(String),
    #[error("invalid BSON: {0}")]
    InvalidBson(String),
    #[error("credential error: {0}")]
    Credential(String),
    #[error("profile not found: {0}")]
    ProfileNotFound(String),
    #[error("profile already exists: {0}")]
    ProfileExists(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("sql parse error: {0}")]
    SqlParse(String),
    #[error("zk audit error: {0}")]
    ZkAudit(String),
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Internal(format!("serialization error: {err}"))
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::Internal(format!("tauri error: {err}"))
    }
}

impl From<bson::oid::Error> for AppError {
    fn from(err: bson::oid::Error) -> Self {
        AppError::InvalidBson(err.to_string())
    }
}

impl From<bson::ser::Error> for AppError {
    fn from(err: bson::ser::Error) -> Self {
        AppError::InvalidBson(err.to_string())
    }
}

impl From<bson::de::Error> for AppError {
    fn from(err: bson::de::Error) -> Self {
        AppError::InvalidBson(err.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for AppError {
    fn from(err: tokio::time::error::Elapsed) -> Self {
        AppError::Timeout(err.to_string())
    }
}

impl From<keyring::Error> for AppError {
    fn from(err: keyring::Error) -> Self {
        AppError::Credential(err.to_string())
    }
}

impl From<mongodb::error::Error> for AppError {
    fn from(err: mongodb::error::Error) -> Self {
        AppError::Mongo(Redactor::new().redact(&err.to_string()))
    }
}

impl From<zk_audit::ZkAuditError> for AppError {
    fn from(err: zk_audit::ZkAuditError) -> Self {
        AppError::ZkAudit(err.to_string())
    }
}

impl From<audit_service::AuditError> for AppError {
    fn from(err: audit_service::AuditError) -> Self {
        use audit_service::AuditError;
        match err {
            AuditError::Io(msg) => AppError::Io(msg),
            AuditError::NotFound(msg) => AppError::NotFound(msg),
            AuditError::Validation(msg) => AppError::Validation(msg),
            AuditError::Internal(msg) => AppError::Internal(msg),
            AuditError::Mongo(msg) => AppError::Mongo(msg),
            AuditError::Credential(msg) => AppError::Credential(msg),
            AuditError::Timeout(msg) => AppError::Timeout(msg),
            AuditError::ZkAudit(msg) => AppError::ZkAudit(msg),
        }
    }
}

/// Convenience alias used across command handlers.
pub type AppResult<T> = Result<T, AppError>;
