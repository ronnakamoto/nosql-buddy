//! ZK-AuditDB Soroban Commitment Contract.
//!
//! Stores Merkle root commitments in an append-only on-chain log and verifies
//! Groth16 inclusion proofs over BN254 using Soroban's native host functions.
//!
//! ## Architecture
//!
//! - **Instance storage**: admin address, sequence counter, current root pointer,
//!   authorized attesters.
//! - **Persistent storage**: append-only root log, root dedup index, oplog
//!   commitments, oplog attestations.
//! - **Events**: `commit_root` emitted on each commit, `attest_oplog` on each
//!   attestation.
//!
//! ## Oplog completeness
//!
//! Each root commitment can optionally carry an **oplog commitment** — a
//! SHA-256 Merkle root over MongoDB's oplog entries for the epoch's time
//! range. This binds the audit log to the oplog, proving that no writes
//! were omitted.
//!
//! Independent attesters (e.g., the auditor/regulator's replica member)
//! can submit ed25519 attestations over the oplog root, providing a
//! durable, on-chain record that they observed the same oplog hash.
//!
//! ## Verification
//!
//! The Groth16 pairing equation: `e(-A, B) * e(alpha, beta) * e(vk_x, gamma) * e(C, delta) == 1`
//! where `vk_x = ic[0] + sum(pub_signals[i] * ic[i+1])`.
//!
//! Points are received as raw `Bytes` (matching the off-chain hex format from
//! `zk-audit/src/serialize.rs`) and converted to `Bn254G1Affine`/`Bn254G2Affine`
//! internally.
//!
//! The verifying key is **not** a caller-supplied argument. It is pinned once
//! at `initialize` and read from storage by `verify_inclusion`; there is no
//! entrypoint to change it afterwards. Accepting a caller-supplied VK would
//! let anyone verify a proof against a VK of their own choosing (e.g. from a
//! trivial circuit with no real constraints), which makes the pairing check
//! meaningless — the only thing actually verified would be `commit_root`'s
//! dedup index, which is already public via `get_root_history`.
//!
//! The public signals are `[root, leaf]`, matching the witness order Circom
//! produces for `merkle_inclusion.circom` (outputs before public inputs).
//! Binding `leaf` publicly is required for soundness of the statement: if
//! only `root` were public, a prover could satisfy the circuit with an
//! unconstrained `leaf` (e.g. `0`, whose path to any non-full tree's root is
//! derivable without ever having seen a real audit entry), so a "valid"
//! proof would show nothing about which entry, if any, was included.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype,
    crypto::bn254::{Bn254G1Affine, Bn254G2Affine, Fr},
    vec, Address, Bytes, BytesN, Env, Map, String, TryFromVal, Vec,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Groth16 proof: G1/G2 points as raw big-endian bytes.
#[derive(Clone)]
#[contracttype]
pub struct Proof {
    /// G1 point A: 64 bytes (X || Y, big-endian).
    pub a: Bytes,
    /// G2 point B: 128 bytes (X_c1 || X_c0 || Y_c1 || Y_c0, big-endian).
    pub b: Bytes,
    /// G1 point C: 64 bytes.
    pub c: Bytes,
}

/// Groth16 verifying key: G1/G2 points as raw big-endian bytes.
#[derive(Clone)]
#[contracttype]
pub struct VerifyingKey {
    pub alpha: Bytes,   // G1, 64 bytes
    pub beta: Bytes,    // G2, 128 bytes
    pub gamma: Bytes,   // G2, 128 bytes
    pub delta: Bytes,   // G2, 128 bytes
    pub ic: Vec<Bytes>, // G1[], each 64 bytes
}

/// An entry in the committed-root history.
#[derive(Clone)]
#[contracttype]
pub struct RootEntry {
    pub sequence: u64,
    pub root: Bytes,     // 32-byte field element, big-endian
    pub timestamp: u64,
    pub metadata: String,
}

/// Oplog completeness commitment for an epoch.
///
/// Binds an audit log root to MongoDB's oplog, proving that no writes
/// were omitted from the audit log. The `oplog_root` is a SHA-256
/// Merkle root over canonicalized oplog entries in the range
/// `[oplog_start_ts, oplog_end_ts)`.
///
/// Timestamps are packed as `(time << 32) | increment` to fit in a u64.
#[derive(Clone)]
#[contracttype]
pub struct OplogCommitment {
    /// SHA-256 Merkle root over canonicalized oplog entries (32 bytes).
    pub oplog_root: Bytes,
    /// Packed oplog start timestamp (inclusive).
    pub oplog_start_ts: u64,
    /// Packed oplog end timestamp (exclusive).
    pub oplog_end_ts: u64,
    /// Number of oplog entries in the range.
    pub oplog_entry_count: u64,
}

/// An independent attester's signature over an oplog commitment.
///
/// The attester (e.g., the auditor's replica member) signs the
/// `oplog_root` and `oplog_end_ts` with their ed25519 key, providing
/// a durable on-chain record that they observed the same oplog hash.
/// This survives oplog rollover (C2 fix).
#[derive(Clone)]
#[contracttype]
pub struct OplogAttestation {
    /// The attester's address (must be authorized).
    pub attester: Address,
    /// Ed25519 signature over `sha256(oplog_root || oplog_end_ts)`.
    pub signature: Bytes,
    /// Ledger timestamp when the attestation was submitted.
    pub timestamp: u64,
}

/// Result of verifying oplog attestations for a sequence.
///
/// Reports how many attestations exist, how many are from currently-authorized
/// attesters, and an overall verdict describing the attestation state.
#[derive(Clone)]
#[contracttype]
pub struct AttestationVerification {
    /// The sequence number being verified.
    pub sequence: u64,
    /// The oplog root from the commitment (what was attested to).
    pub oplog_root: Bytes,
    /// Total number of attestations on record for this sequence.
    pub attestation_count: u32,
    /// Number of attestations from currently-authorized attesters.
    pub authorized_count: u32,
    /// The K-of-N threshold in effect (minimum distinct authorized attesters
    /// required for a "verified" verdict).
    pub threshold: u32,
    /// True if all attestations are from authorized attesters AND the count of
    /// authorized attesters meets the threshold.
    pub all_match: bool,
    /// "verified" if authorized attestations >= threshold,
    /// "no_attestations" if count == 0,
    /// "unauthorized_attester" if any attester is no longer authorized,
    /// "threshold_not_met" if all authorized but fewer than the threshold.
    pub verdict: String,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CommitmentError {
    Unauthorized = 1,
    RootAlreadyCommitted = 2,
    NoRootCommitted = 3,
    InvalidProofEncoding = 4,
    MalformedVerifyingKey = 5,
    InvalidPageSize = 6,
    RootNotCommitted = 7,
    VerificationFailed = 8,
    NotInitialized = 9,
    AttesterNotAuthorized = 10,
    OplogCommitmentNotFound = 11,
    InvalidSignature = 12,
    DuplicateAttestation = 13,
    InvalidOplogRoot = 14,
    InvalidTimestampRange = 15,
    InvalidThreshold = 16,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
pub enum InstanceKey {
    Admin,
    Sequence,
    CurrentRoot,
    /// Set of authorized attester addresses.
    AuthorizedAttesters,
    /// K-of-N attestation threshold (u32, minimum 1).
    Threshold,
    /// The Groth16 verifying key for the `merkle_inclusion` circuit, pinned
    /// once at `initialize` and immutable thereafter. `verify_inclusion`
    /// always uses this key; callers cannot supply their own, which would
    /// let anyone verify proofs against a VK of their own choosing.
    VerifyingKey,
}

#[contracttype]
pub enum PersistentKey {
    /// (sequence: u64) -> RootEntry
    RootEntry(u64),
    /// root bytes -> sequence
    RootIndex,
    /// (sequence: u64) -> OplogCommitment
    OplogCommitment(u64),
    /// (sequence: u64) -> Vec<OplogAttestation>
    OplogAttestations(u64),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct ZkAuditCommitment;

#[contractimpl]
impl ZkAuditCommitment {
    /// Initialize the contract with an admin address and the Groth16
    /// verifying key for the `merkle_inclusion` circuit.
    ///
    /// The verifying key is the trust anchor for `verify_inclusion`: it is
    /// stored once here and can never be changed afterwards (there is no
    /// `set_verifying_key`), so a compromised or malicious admin cannot
    /// retroactively swap in a VK for a different circuit/statement.
    pub fn initialize(env: Env, admin: Address, vk: VerifyingKey) {
        if env.storage().instance().has(&InstanceKey::Admin) {
            panic!("contract already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::Sequence, &0u64);
        // Default threshold is 1 (any single authorized attester verifies an
        // epoch). Admins raise this via `set_threshold` for multi-auditor trust.
        env.storage().instance().set(&InstanceKey::Threshold, &1u32);
        env.storage().instance().set(&InstanceKey::VerifyingKey, &vk);
    }

    /// Get the pinned Groth16 verifying key.
    pub fn get_verifying_key(env: Env) -> Result<VerifyingKey, CommitmentError> {
        env.storage()
            .instance()
            .get(&InstanceKey::VerifyingKey)
            .ok_or(CommitmentError::NotInitialized)
    }

    /// Set a new admin. Only the current admin can call this.
    pub fn set_admin(env: Env, new_admin: Address) {
        let admin = Self::get_admin_or_panic(&env);
        admin.require_auth();
        env.storage().instance().set(&InstanceKey::Admin, &new_admin);
    }

    /// Get the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, CommitmentError> {
        Self::get_admin_or_err(&env)
    }

    /// Set the K-of-N attestation threshold: the minimum number of distinct
    /// currently-authorized attesters required for an epoch's attestation to
    /// be considered "verified". Only the admin can call this. Must be >= 1.
    pub fn set_threshold(env: Env, threshold: u32) -> Result<(), CommitmentError> {
        let admin = Self::get_admin_or_err(&env)?;
        admin.require_auth();
        if threshold < 1 {
            return Err(CommitmentError::InvalidThreshold);
        }
        env.storage()
            .instance()
            .set(&InstanceKey::Threshold, &threshold);
        Ok(())
    }

    /// Get the current K-of-N attestation threshold. Defaults to 1 for
    /// contracts initialized before threshold support was added.
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&InstanceKey::Threshold)
            .unwrap_or(1u32)
    }

    /// Commit a new Merkle root to the append-only log.
    pub fn commit_root(
        env: Env,
        root: Bytes,
        metadata: String,
    ) -> Result<u64, CommitmentError> {
        let admin = Self::get_admin_or_err(&env)?;
        admin.require_auth();

        // Check for duplicate root.
        let root_index: Map<Bytes, u64> = env
            .storage()
            .persistent()
            .get(&PersistentKey::RootIndex)
            .unwrap_or_else(|| Map::new(&env));

        if root_index.contains_key(root.clone()) {
            return Err(CommitmentError::RootAlreadyCommitted);
        }

        // Increment sequence.
        let mut sequence: u64 = env
            .storage()
            .instance()
            .get(&InstanceKey::Sequence)
            .unwrap_or(0);
        sequence += 1;
        env.storage().instance().set(&InstanceKey::Sequence, &sequence);

        let timestamp = env.ledger().timestamp();

        let entry = RootEntry {
            sequence,
            root: root.clone(),
            timestamp,
            metadata: metadata.clone(),
        };

        // Store the entry.
        env.storage()
            .persistent()
            .set(&PersistentKey::RootEntry(sequence), &entry);

        // Update root index.
        let mut root_index = root_index;
        root_index.set(root.clone(), sequence);
        env.storage()
            .persistent()
            .set(&PersistentKey::RootIndex, &root_index);

        // Update current root pointer.
        env.storage()
            .instance()
            .set(&InstanceKey::CurrentRoot, &sequence);

        // Emit event.
        env.events().publish(
            (String::from_str(&env, "commit_root"),),
            (sequence, root, timestamp, metadata),
        );

        Ok(sequence)
    }

    /// Commit a Merkle root with an oplog completeness commitment.
    ///
    /// This is the primary commit function for the oplog-completeness
    /// protocol. It stores both the audit log root and the oplog Merkle
    /// root, binding them together on-chain. An auditor can later verify
    /// that the oplog root matches what they independently computed from
    /// their replica member.
    ///
    /// Timestamps are packed as `(time << 32) | increment`.
    pub fn commit_root_with_oplog(
        env: Env,
        root: Bytes,
        oplog_root: Bytes,
        oplog_start_ts: u64,
        oplog_end_ts: u64,
        oplog_entry_count: u64,
        metadata: String,
    ) -> Result<u64, CommitmentError> {
        // Validate oplog root is 32 bytes.
        if oplog_root.len() != 32 {
            return Err(CommitmentError::InvalidOplogRoot);
        }

        // Validate timestamp range.
        if oplog_end_ts < oplog_start_ts {
            return Err(CommitmentError::InvalidTimestampRange);
        }

        // Inlined root-commit logic so that audit root, sequence, and oplog
        // commitment are all updated in a single atomic transaction.
        let root_index: Map<Bytes, u64> = env
            .storage()
            .persistent()
            .get(&PersistentKey::RootIndex)
            .unwrap_or_else(|| Map::new(&env));

        if root_index.contains_key(root.clone()) {
            return Err(CommitmentError::RootAlreadyCommitted);
        }

        let mut sequence: u64 = env
            .storage()
            .instance()
            .get(&InstanceKey::Sequence)
            .unwrap_or(0);
        sequence += 1;
        env.storage().instance().set(&InstanceKey::Sequence, &sequence);

        let timestamp = env.ledger().timestamp();

        let entry = RootEntry {
            sequence,
            root: root.clone(),
            timestamp,
            metadata: metadata.clone(),
        };

        env.storage()
            .persistent()
            .set(&PersistentKey::RootEntry(sequence), &entry);

        let mut root_index = root_index;
        root_index.set(root.clone(), sequence);
        env.storage()
            .persistent()
            .set(&PersistentKey::RootIndex, &root_index);

        env.storage()
            .instance()
            .set(&InstanceKey::CurrentRoot, &sequence);

        // Store the oplog commitment.
        let oplog_commitment = OplogCommitment {
            oplog_root: oplog_root.clone(),
            oplog_start_ts,
            oplog_end_ts,
            oplog_entry_count,
        };

        env.storage().persistent().set(
            &PersistentKey::OplogCommitment(sequence),
            &oplog_commitment,
        );

        // Emit events.
        env.events().publish(
            (String::from_str(&env, "commit_root"),),
            (sequence, root, timestamp, metadata),
        );
        env.events().publish(
            (String::from_str(&env, "commit_oplog"),),
            (sequence, oplog_root, oplog_end_ts, oplog_entry_count),
        );

        Ok(sequence)
    }

    /// Get the oplog commitment for a given sequence number.
    pub fn get_oplog_commitment(
        env: Env,
        sequence: u64,
    ) -> Result<OplogCommitment, CommitmentError> {
        env.storage()
            .persistent()
            .get(&PersistentKey::OplogCommitment(sequence))
            .ok_or(CommitmentError::OplogCommitmentNotFound)
    }

    /// Authorize an attester address and its ed25519 public key. Only the admin can call this.
    ///
    /// The public key is the raw 32-byte ed25519 public key that will be used
    /// to verify oplog attestations on-chain.
    pub fn authorize_attester(env: Env, attester: Address, public_key: BytesN<32>) {
        let admin = Self::get_admin_or_panic(&env);
        admin.require_auth();

        let mut attesters: Map<Address, BytesN<32>> = env
            .storage()
            .instance()
            .get(&InstanceKey::AuthorizedAttesters)
            .unwrap_or_else(|| Map::new(&env));
        attesters.set(attester, public_key);
        env.storage()
            .instance()
            .set(&InstanceKey::AuthorizedAttesters, &attesters);
    }

    /// Revoke an attester's authorization. Only the admin can call this.
    pub fn revoke_attester(env: Env, attester: Address) {
        let admin = Self::get_admin_or_panic(&env);
        admin.require_auth();

        if let Some(mut attesters) = env
            .storage()
            .instance()
            .get::<InstanceKey, Map<Address, BytesN<32>>>(&InstanceKey::AuthorizedAttesters)
        {
            attesters.remove(attester.clone());
            env.storage()
                .instance()
                .set(&InstanceKey::AuthorizedAttesters, &attesters);
        }
    }

    /// Submit an oplog attestation as an independent attester.
    ///
    /// The attester's address must be authorized by the admin. The
    /// transaction must be signed by the attester's key (enforced by
    /// `require_auth`). The `signature` is an ed25519 signature over
    /// `sha256(oplog_root || oplog_end_ts.to_be_bytes())` and is verified
    /// on-chain against the public key registered by `authorize_attester`.
    ///
    /// This provides a durable, on-chain record that the attester observed
    /// the same oplog hash — even after the oplog rolls over (C2 fix).
    pub fn attest_oplog(
        env: Env,
        attester: Address,
        sequence: u64,
        signature: Bytes,
    ) -> Result<(), CommitmentError> {
        // 1. Require auth from the attester.
        attester.require_auth();

        // 2. Get the oplog commitment.
        let oplog_commitment: OplogCommitment = env
            .storage()
            .persistent()
            .get(&PersistentKey::OplogCommitment(sequence))
            .ok_or(CommitmentError::OplogCommitmentNotFound)?;

        // 3. Check attester is authorized and retrieve its registered public key.
        let attesters: Map<Address, BytesN<32>> = env
            .storage()
            .instance()
            .get(&InstanceKey::AuthorizedAttesters)
            .unwrap_or_else(|| Map::new(&env));

        let public_key = attesters
            .get(attester.clone())
            .ok_or(CommitmentError::AttesterNotAuthorized)?;

        // 4. Validate signature length (ed25519 = 64 bytes) and convert.
        if signature.len() != 64 {
            return Err(CommitmentError::InvalidSignature);
        }
        let signature_n64: BytesN<64> = BytesN::<64>::try_from_val(&env, &signature.to_val())
            .map_err(|_| CommitmentError::InvalidSignature)?;

        // 5. Check for duplicate attestation by this attester.
        let existing: Vec<OplogAttestation> = env
            .storage()
            .persistent()
            .get(&PersistentKey::OplogAttestations(sequence))
            .unwrap_or_else(|| Vec::new(&env));

        for existing_att in existing.iter() {
            if existing_att.attester == attester {
                return Err(CommitmentError::DuplicateAttestation);
            }
        }

        // 6. Verify the ed25519 signature over sha256(oplog_root || oplog_end_ts.to_be_bytes()).
        let mut message = Bytes::new(&env);
        message.append(&oplog_commitment.oplog_root);
        message.append(&Bytes::from_array(&env, &oplog_commitment.oplog_end_ts.to_be_bytes()));
        let message_hash = env.crypto().sha256(&message);
        let message_hash_bytes = Bytes::from_array(&env, &message_hash.to_array());
        env.crypto()
            .ed25519_verify(&public_key, &message_hash_bytes, &signature_n64);

        // 7. Record the attestation.
        let timestamp = env.ledger().timestamp();
        let attestation = OplogAttestation {
            attester: attester.clone(),
            signature: signature.clone(),
            timestamp,
        };

        let mut attestations = existing;
        attestations.push_back(attestation);

        env.storage().persistent().set(
            &PersistentKey::OplogAttestations(sequence),
            &attestations,
        );

        // 8. Emit event.
        env.events().publish(
            (String::from_str(&env, "attest_oplog"),),
            (sequence, attester, timestamp),
        );

        Ok(())
    }

    /// Get all oplog attestations for a given sequence.
    pub fn get_oplog_attestations(
        env: Env,
        sequence: u64,
    ) -> Vec<OplogAttestation> {
        env.storage()
            .persistent()
            .get(&PersistentKey::OplogAttestations(sequence))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Verify oplog attestations for a given sequence.
    ///
    /// Retrieves the oplog commitment and all recorded attestations for the
    /// sequence, then checks each attester against the current
    /// `AuthorizedAttesters` map. Returns an [`AttestationVerification`]
    /// describing how many attestations are from currently-authorized
    /// attesters and an overall verdict.
    pub fn verify_attestation(
        env: Env,
        sequence: u64,
    ) -> Result<AttestationVerification, CommitmentError> {
        // 1. Get the oplog commitment (return error if missing).
        let commitment: OplogCommitment = env
            .storage()
            .persistent()
            .get(&PersistentKey::OplogCommitment(sequence))
            .ok_or(CommitmentError::OplogCommitmentNotFound)?;

        // 2. Get all attestations for this sequence (default to empty).
        let attestations: Vec<OplogAttestation> = env
            .storage()
            .persistent()
            .get(&PersistentKey::OplogAttestations(sequence))
            .unwrap_or_else(|| Vec::new(&env));

        // 3. Load the current authorized attesters map.
        let attesters: Map<Address, BytesN<32>> = env
            .storage()
            .instance()
            .get(&InstanceKey::AuthorizedAttesters)
            .unwrap_or_else(|| Map::new(&env));

        // 4. Count attestations from currently-authorized attesters.
        let attestation_count = attestations.len();
        let mut authorized_count: u32 = 0;
        for attestation in attestations.iter() {
            if attesters.contains_key(attestation.attester.clone()) {
                authorized_count += 1;
            }
        }
        let threshold = Self::get_threshold(env.clone());

        // 5. Compute the verdict and all_match flag.
        let (all_match, verdict) = if attestation_count == 0 {
            (false, String::from_str(&env, "no_attestations"))
        } else if authorized_count < attestation_count {
            (false, String::from_str(&env, "unauthorized_attester"))
        } else if authorized_count >= threshold {
            (true, String::from_str(&env, "verified"))
        } else {
            (false, String::from_str(&env, "threshold_not_met"))
        };

        Ok(AttestationVerification {
            sequence,
            oplog_root: commitment.oplog_root,
            attestation_count,
            authorized_count,
            threshold,
            all_match,
            verdict,
        })
    }

    /// Verify a Groth16 inclusion proof that `leaf` is included in the
    /// Merkle tree identified by the committed `root`.
    ///
    /// The verifying key is never taken from the caller — it is read from
    /// the pinned `VerifyingKey` set at `initialize`. Accepting a
    /// caller-supplied VK here would let anyone produce a "valid" proof for
    /// an arbitrary (or trivial) circuit and statement, defeating the point
    /// of on-chain verification entirely.
    ///
    /// `leaf` is a public signal (not just `root`) so the proof is bound to
    /// a specific audit-entry hash rather than merely "some leaf hashes up
    /// to this root" — see `merkle_inclusion.circom` for why an unbound
    /// leaf makes the statement vacuous.
    pub fn verify_inclusion(
        env: Env,
        root: Bytes,
        leaf: Bytes,
        proof: Proof,
    ) -> Result<bool, CommitmentError> {
        // 1. Verify the root was committed.
        let root_index: Map<Bytes, u64> = env
            .storage()
            .persistent()
            .get(&PersistentKey::RootIndex)
            .unwrap_or_else(|| Map::new(&env));

        if !root_index.contains_key(root.clone()) {
            return Err(CommitmentError::RootNotCommitted);
        }

        // 2. Load the pinned verifying key.
        let vk: VerifyingKey = env
            .storage()
            .instance()
            .get(&InstanceKey::VerifyingKey)
            .ok_or(CommitmentError::NotInitialized)?;

        // 3. Validate byte lengths.
        if proof.a.len() != 64 || proof.c.len() != 64 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        if proof.b.len() != 128 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        if leaf.len() != 32 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        if vk.alpha.len() != 64 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        if vk.beta.len() != 128 || vk.gamma.len() != 128 || vk.delta.len() != 128 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        for ic_point in vk.ic.iter() {
            if ic_point.len() != 64 {
                return Err(CommitmentError::InvalidProofEncoding);
            }
        }

        // 4. Convert bytes to BN254 affine points.
        let bn254 = env.crypto().bn254();

        let a = g1_from_bytes(&env, &proof.a);
        let b = g2_from_bytes(&env, &proof.b);
        let c = g1_from_bytes(&env, &proof.c);

        let alpha = g1_from_bytes(&env, &vk.alpha);
        let beta = g2_from_bytes(&env, &vk.beta);
        let gamma = g2_from_bytes(&env, &vk.gamma);
        let delta = g2_from_bytes(&env, &vk.delta);

        // 5. Construct public inputs: [root, leaf], matching the witness
        //    order produced by Circom (`main`'s outputs — here `root` —
        //    come before its `public` inputs — here `leaf`).
        let root_n32: BytesN<32> = BytesN::<32>::try_from_val(&env, &root.to_val())
            .map_err(|_| CommitmentError::InvalidProofEncoding)?;
        let leaf_n32: BytesN<32> = BytesN::<32>::try_from_val(&env, &leaf.to_val())
            .map_err(|_| CommitmentError::InvalidProofEncoding)?;
        let root_fr = Fr::from_bytes(root_n32);
        let leaf_fr = Fr::from_bytes(leaf_n32);

        let pub_signals = vec![&env, root_fr, leaf_fr];

        // ic.len() must be pub_signals.len() + 1.
        if vk.ic.len() != pub_signals.len() as u32 + 1 {
            return Err(CommitmentError::MalformedVerifyingKey);
        }

        // 6. Compute vk_x = ic[0] + sum(pub_signals[i] * ic[i+1])
        let ic0_bytes = vk.ic.get(0).unwrap();
        let mut vk_x = g1_from_bytes(&env, &ic0_bytes);

        for (i, signal) in pub_signals.iter().enumerate() {
            let ic_bytes = vk.ic.get((i + 1) as u32).unwrap();
            let ic_point = g1_from_bytes(&env, &ic_bytes);
            let prod = bn254.g1_mul(&ic_point, &signal);
            vk_x = bn254.g1_add(&vk_x, &prod);
        }

        // 7. Pairing check: e(-A, B) * e(alpha, beta) * e(vk_x, gamma) * e(C, delta) == 1
        let neg_a = -a;
        let vp1 = vec![&env, neg_a, alpha, vk_x, c];
        let vp2 = vec![&env, b, beta, gamma, delta];

        let verified = bn254.pairing_check(vp1, vp2);

        if !verified {
            return Err(CommitmentError::VerificationFailed);
        }

        Ok(true)
    }

    /// Get the latest committed root and its sequence number.
    pub fn get_current_root(env: Env) -> Option<RootEntry> {
        let sequence: u64 = env
            .storage()
            .instance()
            .get(&InstanceKey::CurrentRoot)?;
        env.storage()
            .persistent()
            .get(&PersistentKey::RootEntry(sequence))
    }

    /// Get committed root history (most recent first, paginated).
    pub fn get_root_history(env: Env, limit: u32) -> Result<Vec<RootEntry>, CommitmentError> {
        if limit == 0 || limit > 100 {
            return Err(CommitmentError::InvalidPageSize);
        }

        let current: u64 = env
            .storage()
            .instance()
            .get(&InstanceKey::CurrentRoot)
            .unwrap_or(0);

        if current == 0 {
            return Ok(Vec::new(&env));
        }

        let mut result = Vec::new(&env);
        let start = if current >= limit as u64 {
            current - limit as u64 + 1
        } else {
            1
        };

        for seq in (start..=current).rev() {
            if let Some(entry) = env
                .storage()
                .persistent()
                .get::<PersistentKey, RootEntry>(&PersistentKey::RootEntry(seq))
            {
                result.push_back(entry);
            }
        }

        Ok(result)
    }

    // --- helpers ---

    fn get_admin_or_err(env: &Env) -> Result<Address, CommitmentError> {
        env.storage()
            .instance()
            .get(&InstanceKey::Admin)
            .ok_or(CommitmentError::NotInitialized)
    }

    fn get_admin_or_panic(env: &Env) -> Address {
        Self::get_admin_or_err(env).expect("contract not initialized")
    }
}

/// Convert a 64-byte `Bytes` to a `Bn254G1Affine`.
fn g1_from_bytes(env: &Env, bytes: &Bytes) -> Bn254G1Affine {
    let bytes_n: BytesN<64> = BytesN::<64>::try_from_val(env, &bytes.to_val())
        .unwrap();
    Bn254G1Affine::from_bytes(bytes_n)
}

/// Convert a 128-byte `Bytes` to a `Bn254G2Affine`.
fn g2_from_bytes(env: &Env, bytes: &Bytes) -> Bn254G2Affine {
    let bytes_n: BytesN<128> = BytesN::<128>::try_from_val(env, &bytes.to_val())
        .unwrap();
    Bn254G2Affine::from_bytes(bytes_n)
}

mod test;
