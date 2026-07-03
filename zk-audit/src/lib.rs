//! ZK audit log: tamper-evident Merkle tree + Groth16 proofs for NoSQLBuddy.
//!
//! This crate provides the Rust-native ZK proof generation pipeline for
//! audit log entries. It builds a Poseidon Merkle tree from audit events,
//! generates Groth16 proofs of inclusion, and serializes them for Soroban
//! on-chain verification.
//!
//! ## Architecture
//!
//! - [`merkle`] — Poseidon Merkle tree construction and inclusion proof generation
//! - [`prover`] — Groth16 proof generation via ark-circom + ark-groth16
//! - [`serialize`] — Soroban BN254 hex serialization for on-chain verification
//! - [`error`] — Typed errors for the ZK audit pipeline
//!
//! ## Circuit
//!
//! The circuit (`merkle_inclusion.circom`) proves knowledge of a leaf and
//! its authentication path in a binary Merkle tree whose root is public.
//! Hash: Poseidon(2) (t=3), matching `light-poseidon::new_circom(2)`.

pub mod commitment;
pub mod disclosure;
pub mod error;
pub mod merkle;
pub mod prover;
pub mod serialize;

pub use commitment::{poseidon_leaf_v3, LeafOpening};
pub use disclosure::{DisclosureProver, DisclosureStatement};
pub use error::ZkAuditError;
pub use merkle::{AuditMerkleTree, InclusionProof};
pub use prover::{AuditProver, Groth16Proof, VerifyingKey, generate_and_save_parameters};
pub use serialize::{load_verifying_key_hex, SorobanProofArgs, SorobanVerifyingKey};
