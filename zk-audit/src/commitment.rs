//! Leaf v3: keyed Poseidon vector commitment over structured event fields.
//!
//! The v1/v2 leaf derivations reduce the whole audit event to a single
//! opaque hash (SHA-256, then keyed HMAC-SHA-256) *before* the ZK circuit
//! ever sees it. That makes any statement about the event's contents
//! unprovable in zero knowledge — the circuit can only show "this opaque
//! value is in the tree", which a plain Merkle path proof shows equally
//! well.
//!
//! v3 instead commits to the event's fields **individually**, using a
//! Poseidon hash (circuit-native, matching circomlib) keyed by the secret
//! domain leaf key:
//!
//! ```text
//! leaf = Poseidon7(k, op_h, db_h, coll_h, ts, doc_h, salt)
//! ```
//!
//! | Input    | Derivation                                              |
//! |----------|---------------------------------------------------------|
//! | `k`      | domain leaf key (32 bytes) mapped into the field        |
//! | `op_h`   | `str_to_field(operation)`                               |
//! | `db_h`   | `str_to_field(database)`                                |
//! | `coll_h` | `str_to_field(collection)`                              |
//! | `ts`     | event Unix timestamp (seconds) as a field element       |
//! | `doc_h`  | `bytes_to_field(canonical_payload)` — binds full content|
//! | `salt`   | `HMAC-SHA-256(k, 0x03 ‖ canonical_payload)` → field     |
//!
//! Properties:
//! - **Binding**: Poseidon is collision-resistant over Fr; the leaf commits
//!   to each field and (via `doc_h`) to the exact canonical payload bytes.
//! - **Hiding / dictionary resistance**: the secret `k` inside the hash
//!   plus the keyed per-leaf `salt` mean an attacker who sees the public
//!   leaf cannot confirm guesses of event contents — the same property the
//!   v2 HMAC leaf provides.
//! - **Provability**: a circuit can open the commitment (one Poseidon
//!   evaluation, ~a few hundred constraints) and prove predicates over the
//!   field values — operation/collection equality, timestamp ranges — while
//!   revealing nothing but the predicate's public parameters.
//!
//! Everything is derived deterministically from `(k, event fields)`, so
//! replay/reader verification recomputes identical leaves from the JSONL
//! log with no extra persisted material.
//!
//! Compatibility with circomlib's `Poseidon(7)` is asserted by test vectors
//! generated with `circomlibjs` (see `circom_poseidon7_vector` below); the
//! arity-2 tree hash compatibility was established by `zk-spike/poseidon-check`.

use ark_bn254::Fr;
use ark_ff::PrimeField;
use hmac::{Hmac, Mac};
use light_poseidon::{Poseidon, PoseidonHasher};
use sha2::{Digest, Sha256};

use crate::error::{ZkAuditError, ZkAuditResult};

/// Number of field elements committed by the v3 leaf (Poseidon arity).
pub const COMMITMENT_ARITY: usize = 7;

/// Domain-separation prefix for salt derivation (v3 = 0x03).
const SALT_DOMAIN_PREFIX: u8 = 0x03;

/// The private opening of a v3 leaf commitment: every field element that
/// went into the Poseidon hash, in input order. This is exactly the witness
/// a disclosure circuit needs to open the commitment.
#[derive(Debug, Clone)]
pub struct LeafOpening {
    /// Domain leaf key as a field element.
    pub key: Fr,
    /// `str_to_field(operation)`.
    pub op_h: Fr,
    /// `str_to_field(database)`.
    pub db_h: Fr,
    /// `str_to_field(collection)`.
    pub coll_h: Fr,
    /// Event Unix timestamp (seconds).
    pub ts: u64,
    /// `bytes_to_field(canonical_payload)`.
    pub doc_h: Fr,
    /// Keyed per-leaf salt.
    pub salt: Fr,
}

impl LeafOpening {
    /// The Poseidon inputs in commitment order.
    pub fn to_inputs(&self) -> [Fr; COMMITMENT_ARITY] {
        [
            self.key,
            self.op_h,
            self.db_h,
            self.coll_h,
            Fr::from(self.ts),
            self.doc_h,
            self.salt,
        ]
    }
}

/// Map arbitrary bytes into the BN254 scalar field via SHA-256, using the
/// same 31.5-byte truncation scheme as the v1/v2 leaf derivations (take the
/// first 31 bytes, mask the top nibble of byte 31) so the value is always
/// canonically in-field.
pub fn bytes_to_field(bytes: &[u8]) -> Fr {
    let hash = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out[..31].copy_from_slice(&hash[..31]);
    out[31] = hash[31] & 0x0F;
    Fr::from_be_bytes_mod_order(&out)
}

/// Map a string into the field. Used for `op` / `db` / `collection` so a
/// verifier can recompute the public predicate parameter (e.g.
/// `str_to_field("delete")`) independently.
pub fn str_to_field(s: &str) -> Fr {
    bytes_to_field(s.as_bytes())
}

/// Map the 32-byte domain leaf key into the field.
pub fn key_to_field(key: &[u8; 32]) -> Fr {
    Fr::from_be_bytes_mod_order(key)
}

/// Derive the keyed per-leaf salt: `HMAC-SHA-256(k, 0x03 ‖ payload)`
/// truncated into the field. Deterministic, so replay recomputes it; secret,
/// because it depends on `k`.
pub fn derive_salt(key: &[u8; 32], canonical_payload: &[u8]) -> Fr {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(&[SALT_DOMAIN_PREFIX]);
    mac.update(canonical_payload);
    let out = mac.finalize().into_bytes();
    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&out[..31]);
    bytes[31] = out[31] & 0x0F;
    Fr::from_be_bytes_mod_order(&bytes)
}

/// Hash a full input vector with circom-compatible `Poseidon(7)`.
pub fn poseidon7(inputs: &[Fr; COMMITMENT_ARITY]) -> ZkAuditResult<Fr> {
    let mut poseidon = Poseidon::<Fr>::new_circom(COMMITMENT_ARITY).map_err(|e| {
        ZkAuditError::MerkleTree(format!("failed to create Poseidon({COMMITMENT_ARITY}): {e}"))
    })?;
    poseidon
        .hash(inputs)
        .map_err(|e| ZkAuditError::MerkleTree(format!("Poseidon hash failed: {e}")))
}

/// Derive the v3 leaf commitment and its private opening.
///
/// `canonical_payload` must be the same canonical byte encoding used by the
/// v2 HMAC leaf (`canonical_payload_bytes(op, db, coll, data)`), so `doc_h`
/// binds the commitment to the exact event content.
pub fn poseidon_leaf_v3(
    key: &[u8; 32],
    operation: &str,
    database: &str,
    collection: &str,
    ts: u64,
    canonical_payload: &[u8],
) -> ZkAuditResult<(Fr, LeafOpening)> {
    let opening = LeafOpening {
        key: key_to_field(key),
        op_h: str_to_field(operation),
        db_h: str_to_field(database),
        coll_h: str_to_field(collection),
        ts,
        doc_h: bytes_to_field(canonical_payload),
        salt: derive_salt(key, canonical_payload),
    };
    let leaf = poseidon7(&opening.to_inputs())?;
    Ok((leaf, opening))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-check vectors generated with circomlibjs 0.1.7:
    ///
    /// ```js
    /// const p = await buildPoseidon();
    /// p.F.toString(p([1n,2n,3n,4n,5n,6n,7n]))
    /// p.F.toString(p([123456789n, 987654321n, 42n, 0n, 1n, 2n, 3n]))
    /// ```
    ///
    /// If these fail, light-poseidon's Poseidon(7) parameters diverge from
    /// circomlib's and the disclosure circuit would be unsound against the
    /// Rust-derived leaves. This is the go/no-go gate for leaf v3.
    #[test]
    fn circom_poseidon7_vector() {
        let inputs = [
            Fr::from(1u64),
            Fr::from(2u64),
            Fr::from(3u64),
            Fr::from(4u64),
            Fr::from(5u64),
            Fr::from(6u64),
            Fr::from(7u64),
        ];
        let hash = poseidon7(&inputs).unwrap();
        assert_eq!(
            hash.to_string(),
            "12748163991115452309045839028154629052133952896122405799815156419278439301912"
        );

        let inputs_b = [
            Fr::from(123456789u64),
            Fr::from(987654321u64),
            Fr::from(42u64),
            Fr::from(0u64),
            Fr::from(1u64),
            Fr::from(2u64),
            Fr::from(3u64),
        ];
        let hash_b = poseidon7(&inputs_b).unwrap();
        assert_eq!(
            hash_b.to_string(),
            "1299903751346159901124014977795263663458182655823165426719276329669413163132"
        );
    }

    #[test]
    fn leaf_v3_is_deterministic() {
        let key = [0xABu8; 32];
        let (leaf1, _) =
            poseidon_leaf_v3(&key, "insert", "db", "coll", 1_700_000_000, b"payload").unwrap();
        let (leaf2, _) =
            poseidon_leaf_v3(&key, "insert", "db", "coll", 1_700_000_000, b"payload").unwrap();
        assert_eq!(leaf1, leaf2);
    }

    #[test]
    fn leaf_v3_differs_with_different_keys() {
        let (leaf1, _) =
            poseidon_leaf_v3(&[0xABu8; 32], "insert", "db", "coll", 0, b"payload").unwrap();
        let (leaf2, _) =
            poseidon_leaf_v3(&[0xCDu8; 32], "insert", "db", "coll", 0, b"payload").unwrap();
        assert_ne!(leaf1, leaf2);
    }

    #[test]
    fn leaf_v3_binds_every_field() {
        let key = [0x11u8; 32];
        let base = poseidon_leaf_v3(&key, "insert", "db", "coll", 100, b"p").unwrap().0;
        let variants = [
            poseidon_leaf_v3(&key, "update", "db", "coll", 100, b"p").unwrap().0,
            poseidon_leaf_v3(&key, "insert", "db2", "coll", 100, b"p").unwrap().0,
            poseidon_leaf_v3(&key, "insert", "db", "coll2", 100, b"p").unwrap().0,
            poseidon_leaf_v3(&key, "insert", "db", "coll", 101, b"p").unwrap().0,
            poseidon_leaf_v3(&key, "insert", "db", "coll", 100, b"q").unwrap().0,
        ];
        for v in variants {
            assert_ne!(base, v);
        }
    }

    #[test]
    fn opening_recomputes_leaf() {
        let key = [0x22u8; 32];
        let (leaf, opening) =
            poseidon_leaf_v3(&key, "delete", "shop", "orders", 1_650_000_000, b"doc").unwrap();
        assert_eq!(poseidon7(&opening.to_inputs()).unwrap(), leaf);
    }
}
