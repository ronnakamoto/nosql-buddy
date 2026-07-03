//! Generate a Merkle root and proof for on-chain testing.
//!
//! Usage:
//!   cargo run --example gen_proof -- <r1cs_path> <wasm_path>
//!
//! Outputs JSON with root (hex), proof args, and VK args ready for
//! `stellar contract invoke`.

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use zk_audit::merkle::AuditMerkleTree;
use zk_audit::prover::{AuditProver, Groth16Proof};

fn main() {
    let mut tree = AuditMerkleTree::with_height(20).unwrap();
    tree.insert(Fr::from(1u64));
    tree.insert(Fr::from(2u64));
    tree.insert(Fr::from(3u64));
    tree.insert(Fr::from(4u64));

    let root = tree.root().unwrap();
    let root_bigint = root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let mut root_arr = [0u8; 32];
    let len = root_bytes.len();
    root_arr[32 - len..].copy_from_slice(&root_bytes);
    let root_hex = hex::encode(root_arr);

    println!("root_hex: {}", root_hex);

    // Generate inclusion proof for leaf at index 2.
    let inclusion = tree.prove_inclusion(2).unwrap();
    assert_eq!(inclusion.root, root);

    let leaf_bytes = inclusion.leaf.into_bigint().to_bytes_be();
    let mut leaf_arr = [0u8; 32];
    leaf_arr[32 - leaf_bytes.len()..].copy_from_slice(&leaf_bytes);
    let leaf_hex = hex::encode(leaf_arr);

    let r1cs = std::env::args().nth(1).unwrap_or_else(|| {
        "../zk-spike/circuits/build/merkle_inclusion.r1cs".to_string()
    });
    let wasm = std::env::args().nth(2).unwrap_or_else(|| {
        "../zk-spike/circuits/build/merkle_inclusion_js/merkle_inclusion.wasm".to_string()
    });

    let prover = AuditProver::new(&r1cs, &wasm).unwrap();
    let groth16_proof: Groth16Proof = prover.prove(&inclusion).unwrap();
    let soroban_args = AuditProver::serialize_for_soroban(&groth16_proof).unwrap();

    // Output as JSON for easy parsing.
    // NOTE: `verify_inclusion(root, leaf, proof)` reads the verifying key
    // from on-chain storage (pinned at `initialize`); it is included here
    // only for reference / for calling `initialize` on a fresh deployment.
    let output = serde_json::json!({
        "root_hex": root_hex,
        "leaf_hex": leaf_hex,
        "proof": {
            "a": soroban_args.proof.a,
            "b": soroban_args.proof.b,
            "c": soroban_args.proof.c,
        },
        "vk": {
            "alpha": soroban_args.vk.alpha,
            "beta": soroban_args.vk.beta,
            "gamma": soroban_args.vk.gamma,
            "delta": soroban_args.vk.delta,
            "ic": soroban_args.vk.ic,
        },
        "pub_signals": soroban_args.pub_signals,
    });

    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
