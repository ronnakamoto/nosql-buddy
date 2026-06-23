//! ZK-AuditDB Soroban Commitment Contract.
//!
//! Stores Merkle root commitments in an append-only on-chain log and verifies
//! Groth16 inclusion proofs over BN254 using Soroban's native host functions.
//!
//! ## Architecture
//!
//! - **Instance storage**: admin address, sequence counter, current root pointer.
//! - **Persistent storage**: append-only root log, root dedup index.
//! - **Events**: `commit_root` emitted on each commit.
//!
//! ## Verification
//!
//! The Groth16 pairing equation: `e(-A, B) * e(alpha, beta) * e(vk_x, gamma) * e(C, delta) == 1`
//! where `vk_x = ic[0] + sum(pub_signals[i] * ic[i+1])`.
//!
//! Points are received as raw `Bytes` (matching the off-chain hex format from
//! `zk-audit/src/serialize.rs`) and converted to `Bn254G1Affine`/`Bn254G2Affine`
//! internally.

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
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
pub enum InstanceKey {
    Admin,
    Sequence,
    CurrentRoot,
}

#[contracttype]
pub enum PersistentKey {
    /// (sequence: u64) -> RootEntry
    RootEntry(u64),
    /// root bytes -> sequence
    RootIndex,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct ZkAuditCommitment;

#[contractimpl]
impl ZkAuditCommitment {
    /// Initialize the contract with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&InstanceKey::Admin) {
            panic!("contract already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&InstanceKey::Admin, &admin);
        env.storage().instance().set(&InstanceKey::Sequence, &0u64);
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

    /// Verify a Groth16 inclusion proof against a committed root.
    pub fn verify_inclusion(
        env: Env,
        root: Bytes,
        proof: Proof,
        vk: VerifyingKey,
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

        // 2. Validate byte lengths.
        if proof.a.len() != 64 || proof.c.len() != 64 {
            return Err(CommitmentError::InvalidProofEncoding);
        }
        if proof.b.len() != 128 {
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

        // 3. Convert bytes to BN254 affine points.
        let bn254 = env.crypto().bn254();

        let a = g1_from_bytes(&env, &proof.a);
        let b = g2_from_bytes(&env, &proof.b);
        let c = g1_from_bytes(&env, &proof.c);

        let alpha = g1_from_bytes(&env, &vk.alpha);
        let beta = g2_from_bytes(&env, &vk.beta);
        let gamma = g2_from_bytes(&env, &vk.gamma);
        let delta = g2_from_bytes(&env, &vk.delta);

        // 4. Construct public inputs: [root] (the only public signal for
        //    the merkle_inclusion circuit).
        let root_n32: BytesN<32> = BytesN::<32>::try_from_val(&env, &root.to_val())
            .map_err(|_| CommitmentError::InvalidProofEncoding)?;
        let root_fr = Fr::from_bytes(root_n32);

        let pub_signals = vec![&env, root_fr];

        // ic.len() must be pub_signals.len() + 1.
        if vk.ic.len() != pub_signals.len() as u32 + 1 {
            return Err(CommitmentError::MalformedVerifyingKey);
        }

        // 5. Compute vk_x = ic[0] + sum(pub_signals[i] * ic[i+1])
        let ic0_bytes = vk.ic.get(0).unwrap();
        let mut vk_x = g1_from_bytes(&env, &ic0_bytes);

        for (i, signal) in pub_signals.iter().enumerate() {
            let ic_bytes = vk.ic.get((i + 1) as u32).unwrap();
            let ic_point = g1_from_bytes(&env, &ic_bytes);
            let prod = bn254.g1_mul(&ic_point, &signal);
            vk_x = bn254.g1_add(&vk_x, &prod);
        }

        // 6. Pairing check: e(-A, B) * e(alpha, beta) * e(vk_x, gamma) * e(C, delta) == 1
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
