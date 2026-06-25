//! NoSQLBuddy audit service library.
//!
//! Contains the tamper-evident Merkle audit log, Stellar on-chain commitment
//! logic, change-stream interception, epoch management, attestation, and the
//! standalone audit daemon (HTTP API + publisher/attester/reader modes).
//!
//! This crate is Tauri-free and can be built/run independently of the desktop
//! application.

pub mod audit;
pub mod auditd;
pub mod error;

pub use error::{AuditError, AuditResult};
