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
        // A "not primary" rejection (server code 10107 / NotWritablePrimary)
        // means the connection is pinned to a replica-set secondary, so it
        // can't accept writes. This happens when a single-host URI lands on a
        // member that is currently a secondary (e.g. after an election). The
        // raw driver text is opaque, so surface an actionable hint instead.
        if is_not_writable_primary(&err) {
            return AppError::Mongo(
                "write rejected: connected to a replica-set secondary, which cannot accept \
                 writes (server error 10107, NotWritablePrimary). Reconnect using a \
                 replica-set connection string that lists every member and the set name \
                 (e.g. mongodb://host1,host2,host3/?replicaSet=<name>) so the driver routes \
                 writes to the current primary. Avoid pinning to one node with \
                 directConnection=true for write workloads."
                    .to_string(),
            );
        }
        AppError::Mongo(Redactor::new().redact(&err.to_string()))
    }
}

/// Whether a MongoDB error is a "not primary" write rejection (server error
/// code 10107, `NotWritablePrimary`), in any of the shapes the driver reports
/// it (command error, write error, or bulk write error).
fn is_not_writable_primary(err: &mongodb::error::Error) -> bool {
    use mongodb::error::ErrorKind;

    const NOT_WRITABLE_PRIMARY: i32 = 10107;

    match err.kind.as_ref() {
        ErrorKind::Command(cmd) => cmd.code == NOT_WRITABLE_PRIMARY,
        ErrorKind::Write(failure) => match failure {
            mongodb::error::WriteFailure::WriteError(we) => we.code == NOT_WRITABLE_PRIMARY,
            _ => false,
        },
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .values()
            .any(|we| we.code == NOT_WRITABLE_PRIMARY),
        _ => false,
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

impl AppError {
    /// Construct a redacted `Mongo` error from any displayable error.
    ///
    /// Many call sites map driver/cursor errors with
    /// `.map_err(|e| AppError::Mongo(e.to_string()))`, which bypasses the
    /// redaction in `From<mongodb::error::Error>` and can leak a connection
    /// URI (`user:pass@host`) embedded in the driver message. Use this helper
    /// so the message is always passed through the `Redactor` first.
    pub fn mongo<E: std::fmt::Display>(err: E) -> Self {
        AppError::Mongo(Redactor::new().redact(&err.to_string()))
    }
}
