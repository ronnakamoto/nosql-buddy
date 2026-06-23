//! Soroban BN254 hex serialization for on-chain verification.
//!
//! Serializes arkworks Groth16 proofs and verifying keys to the hex format
//! expected by Soroban's BN254 host functions:
//! - G1: be_bytes(X) || be_bytes(Y) = 64 bytes
//! - G2: be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0) = 128 bytes
//!
//! Uses `BigInteger::to_bytes_be()` (not `CanonicalSerialize`, which is LE with flags).

use ark_bn254::{Bn254, G1Affine, G2Affine};
use ark_ff::{BigInteger, PrimeField};
use serde::{Deserialize, Serialize};

use crate::error::ZkAuditResult;
use crate::prover::Fr;

/// Serialized proof + VK + public signals, ready for Soroban invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanProofArgs {
    pub vk: SorobanVerifyingKey,
    pub proof: SorobanProof,
    pub pub_signals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanVerifyingKey {
    pub alpha: String,   // G1, 64 bytes hex
    pub beta: String,    // G2, 128 bytes hex
    pub gamma: String,   // G2, 128 bytes hex
    pub delta: String,   // G2, 128 bytes hex
    pub ic: Vec<String>, // G1[], each 64 bytes hex
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SorobanProof {
    pub a: String, // G1, 64 bytes hex
    pub b: String, // G2, 128 bytes hex
    pub c: String, // G1, 64 bytes hex
}

/// Serialize a field element to 32 big-endian bytes.
fn fq_to_bytes(f: &impl PrimeField) -> [u8; 32] {
    let big = f.into_bigint();
    let bytes = big.to_bytes_be();
    let mut arr = [0u8; 32];
    let len = bytes.len();
    arr[32 - len..].copy_from_slice(&bytes);
    arr
}

/// Serialize a G1 affine point to 64-byte hex (X || Y, big-endian).
fn g1_to_hex(p: &G1Affine) -> String {
    let x = fq_to_bytes(&p.x);
    let y = fq_to_bytes(&p.y);
    let mut result = [0u8; 64];
    result[..32].copy_from_slice(&x);
    result[32..].copy_from_slice(&y);
    hex::encode(result)
}

/// Serialize a G2 affine point to 128-byte hex.
/// Format: X_c1 || X_c0 || Y_c1 || Y_c0 (big-endian).
fn g2_to_hex(p: &G2Affine) -> String {
    let x_c1 = fq_to_bytes(&p.x.c1);
    let x_c0 = fq_to_bytes(&p.x.c0);
    let y_c1 = fq_to_bytes(&p.y.c1);
    let y_c0 = fq_to_bytes(&p.y.c0);

    let mut result = [0u8; 128];
    result[..32].copy_from_slice(&x_c1);
    result[32..64].copy_from_slice(&x_c0);
    result[64..96].copy_from_slice(&y_c1);
    result[96..].copy_from_slice(&y_c0);
    hex::encode(result)
}

/// Serialize a full proof + VK + public signals for Soroban.
pub fn serialize_proof(
    proof: &ark_groth16::Proof<Bn254>,
    vk: &ark_groth16::VerifyingKey<Bn254>,
    public_inputs: &[Fr],
) -> ZkAuditResult<SorobanProofArgs> {
    Ok(SorobanProofArgs {
        vk: SorobanVerifyingKey {
            alpha: g1_to_hex(&vk.alpha_g1),
            beta: g2_to_hex(&vk.beta_g2),
            gamma: g2_to_hex(&vk.gamma_g2),
            delta: g2_to_hex(&vk.delta_g2),
            ic: vk.gamma_abc_g1.iter().map(g1_to_hex).collect(),
        },
        proof: SorobanProof {
            a: g1_to_hex(&proof.a),
            b: g2_to_hex(&proof.b),
            c: g1_to_hex(&proof.c),
        },
        pub_signals: public_inputs.iter().map(|f| f.to_string()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_bn254::Fq;

    #[test]
    fn test_fq_to_bytes_is_big_endian() {
        let one = Fq::from(1u64);
        let bytes = fq_to_bytes(&one);
        // Big-endian: last byte should be 1.
        assert_eq!(bytes[31], 1);
        assert_eq!(bytes[0], 0);
    }

    #[test]
    fn test_g1_to_hex_length() {
        // The identity point's serialization should still be 128 hex chars (64 bytes).
        let identity = G1Affine::identity();
        let hex_str = g1_to_hex(&identity);
        assert_eq!(hex_str.len(), 128); // 64 bytes * 2 hex chars
    }

    #[test]
    fn test_g2_to_hex_length() {
        let identity = G2Affine::identity();
        let hex_str = g2_to_hex(&identity);
        assert_eq!(hex_str.len(), 256); // 128 bytes * 2 hex chars
    }
}
