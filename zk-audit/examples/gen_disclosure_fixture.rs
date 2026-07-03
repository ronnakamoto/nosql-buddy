//! Generate a real Audited-Action Disclosure proof fixture for the Soroban
//! contract tests.
//!
//! Usage:
//!   cargo run --example gen_disclosure_fixture -- [r1cs_path] [wasm_path] [out_path]
//!
//! Builds a v3 leaf commitment, inserts it into a Merkle tree, proves the
//! disclosure statement ("a delete on `orders` happened within [tsMin,
//! tsMax]"), and writes every argument of the contract's
//! `verify_disclosure` — plus the matching VK — as hex JSON.

use ark_ff::{BigInteger, PrimeField};
use zk_audit::commitment::{poseidon_leaf_v3, str_to_field};
use zk_audit::disclosure::{DisclosureProver, DisclosureStatement};
use zk_audit::merkle::AuditMerkleTree;
use zk_audit::prover::AuditProver;

fn fr_hex(f: &ark_bn254::Fr) -> String {
    let bytes = f.into_bigint().to_bytes_be();
    let mut arr = [0u8; 32];
    arr[32 - bytes.len()..].copy_from_slice(&bytes);
    hex::encode(arr)
}

fn main() {
    let r1cs = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../zk-spike/circuits/build/audited_action.r1cs".to_string());
    let wasm = std::env::args().nth(2).unwrap_or_else(|| {
        "../zk-spike/circuits/build/audited_action_js/audited_action.wasm".to_string()
    });
    let out = std::env::args()
        .nth(3)
        .unwrap_or_else(|| "soroban-contract/fixtures/disclosure_fixture.json".to_string());

    // A v3 event: delete on shopkeeper.orders at a fixed timestamp.
    let key = [0x42u8; 32];
    let ts: u64 = 1_700_000_500;
    let (leaf, opening) =
        poseidon_leaf_v3(&key, "delete", "shopkeeper", "orders", ts, b"canonical-payload")
            .unwrap();

    let mut tree = AuditMerkleTree::with_height(20).unwrap();
    tree.insert(ark_bn254::Fr::from(11u64));
    let idx = tree.insert(leaf);
    tree.insert(ark_bn254::Fr::from(22u64));
    let inclusion = tree.prove_inclusion(idx).unwrap();

    let statement = DisclosureStatement {
        op_pred: str_to_field("delete"),
        coll_pred: str_to_field("orders"),
        ts_min: 1_700_000_000,
        ts_max: 1_700_001_000,
        check_op: true,
        check_coll: true,
        check_ts: true,
    };

    let prover = DisclosureProver::new(&r1cs, &wasm).unwrap();
    let proof = prover.prove(&opening, &inclusion, &statement).unwrap();
    let args = AuditProver::serialize_for_soroban(&proof).unwrap();

    let output = serde_json::json!({
        "root_hex": fr_hex(&inclusion.root),
        "leaf_hex": fr_hex(&inclusion.leaf),
        "op_pred_hex": fr_hex(&statement.op_pred),
        "coll_pred_hex": fr_hex(&statement.coll_pred),
        "ts_min": statement.ts_min,
        "ts_max": statement.ts_max,
        "check_op": statement.check_op,
        "check_coll": statement.check_coll,
        "check_ts": statement.check_ts,
        "proof": { "a": args.proof.a, "b": args.proof.b, "c": args.proof.c },
        "vk": {
            "alpha": args.vk.alpha,
            "beta": args.vk.beta,
            "gamma": args.vk.gamma,
            "delta": args.vk.delta,
            "ic": args.vk.ic,
        },
        "pub_signals": args.pub_signals,
    });

    if let Some(parent) = std::path::Path::new(&out).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&out, serde_json::to_string_pretty(&output).unwrap()).unwrap();
    println!("wrote {out}");
}
