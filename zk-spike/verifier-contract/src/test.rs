#![cfg(test)]
extern crate std;

use core::str::FromStr;
use soroban_sdk::{
    crypto::bn254::{
        Bn254G1Affine, Bn254G2Affine, Fr, BN254_G1_SERIALIZED_SIZE, BN254_G2_SERIALIZED_SIZE,
    },
    Bytes, Env, U256, Vec,
};

use crate::{Groth16Verifier, Groth16VerifierClient, Proof, VerificationKey};

use ark_bn254::Fq;
use ark_ff::{BigInteger, PrimeField};
use std::fs;

/// Convert a decimal string (Fq element) to 32 big-endian bytes.
fn fq_to_bytes(s: &str) -> [u8; 32] {
    let fq = Fq::from_str(s).unwrap();
    let bigint = fq.into_bigint();
    let bytes = bigint.to_bytes_be(); // Vec<u8>, big-endian
    let mut arr = [0u8; 32];
    let len = bytes.len();
    arr[32 - len..].copy_from_slice(&bytes);
    arr
}

/// Construct a Bn254G1Affine from two decimal-string coordinates (X, Y).
/// Soroban G1 format: be_bytes(X) || be_bytes(Y) = 64 bytes.
fn g1_from_coords(env: &Env, x: &str, y: &str) -> Bn254G1Affine {
    let mut buf = [0u8; BN254_G1_SERIALIZED_SIZE];
    buf[..32].copy_from_slice(&fq_to_bytes(x));
    buf[32..].copy_from_slice(&fq_to_bytes(y));
    Bn254G1Affine::from_array(env, &buf)
}

/// Construct a Bn254G2Affine from four decimal-string coordinates.
/// snarkjs Fq2 format: [c0, c1] where c0 is real, c1 is imaginary.
/// Soroban G2 format: be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0) = 128 bytes.
fn g2_from_snarkjs(env: &Env, x_c0: &str, x_c1: &str, y_c0: &str, y_c1: &str) -> Bn254G2Affine {
    let mut buf = [0u8; BN254_G2_SERIALIZED_SIZE];
    // X: c1 || c0
    buf[..32].copy_from_slice(&fq_to_bytes(x_c1));
    buf[32..64].copy_from_slice(&fq_to_bytes(x_c0));
    // Y: c1 || c0
    buf[64..96].copy_from_slice(&fq_to_bytes(y_c1));
    buf[96..].copy_from_slice(&fq_to_bytes(y_c0));
    Bn254G2Affine::from_array(env, &buf)
}

/// Parse a snarkjs verification_key.json into a Soroban VerificationKey.
fn parse_vk(env: &Env, path: &str) -> VerificationKey {
    let data = fs::read_to_string(path).unwrap();
    let vk: serde_json::Value = serde_json::from_str(&data).unwrap();

    let alpha = g1_from_coords(
        env,
        vk["vk_alpha_1"][0].as_str().unwrap(),
        vk["vk_alpha_1"][1].as_str().unwrap(),
    );

    let beta = g2_from_snarkjs(
        env,
        vk["vk_beta_2"][0][0].as_str().unwrap(),
        vk["vk_beta_2"][0][1].as_str().unwrap(),
        vk["vk_beta_2"][1][0].as_str().unwrap(),
        vk["vk_beta_2"][1][1].as_str().unwrap(),
    );

    let gamma = g2_from_snarkjs(
        env,
        vk["vk_gamma_2"][0][0].as_str().unwrap(),
        vk["vk_gamma_2"][0][1].as_str().unwrap(),
        vk["vk_gamma_2"][1][0].as_str().unwrap(),
        vk["vk_gamma_2"][1][1].as_str().unwrap(),
    );

    let delta = g2_from_snarkjs(
        env,
        vk["vk_delta_2"][0][0].as_str().unwrap(),
        vk["vk_delta_2"][0][1].as_str().unwrap(),
        vk["vk_delta_2"][1][0].as_str().unwrap(),
        vk["vk_delta_2"][1][1].as_str().unwrap(),
    );

    let ic_arr = vk["IC"].as_array().unwrap();
    let mut ic = Vec::new(env);
    for p in ic_arr {
        ic.push_back(g1_from_coords(
            env,
            p[0].as_str().unwrap(),
            p[1].as_str().unwrap(),
        ));
    }

    VerificationKey {
        alpha,
        beta,
        gamma,
        delta,
        ic,
    }
}

/// Parse a snarkjs proof.json into a Soroban Proof.
fn parse_proof(env: &Env, path: &str) -> Proof {
    let data = fs::read_to_string(path).unwrap();
    let proof: serde_json::Value = serde_json::from_str(&data).unwrap();

    let a = g1_from_coords(
        env,
        proof["pi_a"][0].as_str().unwrap(),
        proof["pi_a"][1].as_str().unwrap(),
    );

    let b = g2_from_snarkjs(
        env,
        proof["pi_b"][0][0].as_str().unwrap(),
        proof["pi_b"][0][1].as_str().unwrap(),
        proof["pi_b"][1][0].as_str().unwrap(),
        proof["pi_b"][1][1].as_str().unwrap(),
    );

    let c = g1_from_coords(
        env,
        proof["pi_c"][0].as_str().unwrap(),
        proof["pi_c"][1].as_str().unwrap(),
    );

    Proof { a, b, c }
}

/// Parse a snarkjs public.json into a Vec<Fr>.
fn parse_public(env: &Env, path: &str) -> Vec<Fr> {
    let data = fs::read_to_string(path).unwrap();
    let public: serde_json::Value = serde_json::from_str(&data).unwrap();
    let arr = public.as_array().unwrap();
    let mut vals = Vec::new(env);
    for v in arr {
        let s = v.as_str().unwrap();
        let bytes = fq_to_bytes(s);
        let soroban_bytes = Bytes::from_array(env, &bytes);
        let u256 = U256::from_be_bytes(env, &soroban_bytes);
        vals.push_back(Fr::from_u256(u256));
    }
    vals
}

fn create_client(e: &Env) -> Groth16VerifierClient<'_> {
    Groth16VerifierClient::new(e, &e.register(Groth16Verifier {}, ()))
}

#[test]
fn test_verify_multiplier2_proof() {
    let env = Env::default();

    let base = "../circuits/build";
    let vk_path = std::format!("{}/multiplier2_vkey.json", base);
    let proof_path = std::format!("{}/multiplier2_proof.json", base);
    let public_path = std::format!("{}/multiplier2_public.json", base);

    let vk = parse_vk(&env, &vk_path);
    let proof = parse_proof(&env, &proof_path);
    let pub_signals = parse_public(&env, &public_path);

    let client = create_client(&env);

    // Test 1: Verify with correct public output (c = 33).
    let res = client.verify_proof(&vk, &proof, &pub_signals);
    assert_eq!(res, true, "proof should verify with correct public output");

    // Print the cost breakdown.
    env.cost_estimate().budget().print();

    // Test 2: Verify with incorrect public output (c = 22) — should fail.
    let wrong_pub = Vec::from_array(
        &env,
        [Fr::from_u256(U256::from_u32(&env, 22))],
    );
    let res_wrong = client.verify_proof(&vk, &proof, &wrong_pub);
    assert_eq!(res_wrong, false, "proof should NOT verify with wrong public output");
}
