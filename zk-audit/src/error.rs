//! Typed errors for the ZK audit pipeline.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ZkAuditError {
    #[error("failed to load R1CS circuit: {0}")]
    CircuitLoad(String),

    #[error("witness generation failed: {0}")]
    WitnessGeneration(String),

    #[error("proof generation failed: {0}")]
    ProofGeneration(String),

    #[error("proof verification failed: {0}")]
    ProofVerification(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("merkle tree error: {0}")]
    MerkleTree(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type ZkAuditResult<T> = Result<T, ZkAuditError>;
