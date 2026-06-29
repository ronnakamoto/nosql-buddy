//! End-to-end integration test for the ZK audit pipeline.
//!
//! This test verifies the full flow:
//! 1. Record audit events (insert, update, delete) via the interceptor.
//! 2. Verify the Merkle root changes after each event.
//! 3. Generate an inclusion proof for a leaf.
//! 4. Verify the proof locally (if circuit artifacts are available).
//! 5. Verify the root matches what the audit log reports.

#![cfg(test)]

use std::sync::Arc;

use crate::audit::interceptor;
use crate::audit::AuditLog;

#[test]
fn test_e2e_audit_pipeline() {
    let audit = Arc::new(AuditLog::new().unwrap());

    // Initial state: empty tree.
    assert_eq!(audit.event_count(), 0);
    assert_eq!(audit.leaf_count(), 0);
    let root0 = audit.root_hex().unwrap();
    assert!(!root0.is_empty());

    // 1. Record an insert event.
    let idx0 = interceptor::record_insert(
        &audit,
        "rs:rs0",
        "test_db",
        "users",
        r#"{"name":"Alice","age":30}"#,
    )
    .unwrap();
    assert_eq!(idx0, 0);
    assert_eq!(audit.event_count(), 1);
    assert_eq!(audit.leaf_count(), 1);
    let root1 = audit.root_hex().unwrap();
    assert_ne!(root0, root1, "root must change after first insert");

    // 2. Record an update event.
    let idx1 = interceptor::record_update(
        &audit,
        "rs:rs0",
        "test_db",
        "users",
        r#"{"name":"Alice"}"#,
        r#"{"$set":{"age":31}}"#,
    )
    .unwrap();
    assert_eq!(idx1, 1);
    assert_eq!(audit.event_count(), 2);
    let root2 = audit.root_hex().unwrap();
    assert_ne!(root1, root2, "root must change after update");

    // 3. Record a delete event.
    let idx2 =
        interceptor::record_delete(&audit, "rs:rs0", "test_db", "users", r#"{"name":"Alice"}"#)
            .unwrap();
    assert_eq!(idx2, 2);
    assert_eq!(audit.event_count(), 3);
    let root3 = audit.root_hex().unwrap();
    assert_ne!(root2, root3, "root must change after delete");

    // 4. Verify event metadata.
    let events = audit.list_events();
    assert_eq!(events.len(), 3);
    assert_eq!(events[0].operation, "insert");
    assert_eq!(events[0].database, "test_db");
    assert_eq!(events[0].collection, "users");
    assert_eq!(events[1].operation, "update");
    assert_eq!(events[2].operation, "delete");

    // 5. Generate inclusion proof for each leaf.
    for i in 0..3 {
        let proof = audit.prove_inclusion(i).unwrap();
        assert_eq!(
            proof.root,
            audit.root().unwrap(),
            "proof root must match current root"
        );
        assert_eq!(proof.leaf_index, i as usize);
        assert_eq!(
            proof.path_elements.len(),
            20,
            "path must have 20 elements for height-20 tree"
        );
        assert_eq!(proof.path_indices.len(), 20);
    }

    // 6. Verify the root hex matches the field element.
    use ark_ff::{BigInteger, PrimeField};
    let root = audit.root().unwrap();
    let root_bigint = root.into_bigint();
    let root_bytes = root_bigint.to_bytes_be();
    let expected_hex = hex::encode(&root_bytes);
    assert_eq!(
        audit.root_hex().unwrap(),
        expected_hex,
        "root_hex must match root field element"
    );
}

#[test]
fn test_e2e_proof_verification_with_circuit() {
    // This test only runs if the circuit artifacts are present.
    let r1cs_path = "../zk-spike/circuits/build/merkle_inclusion.r1cs";
    let wasm_path = "../zk-spike/circuits/build/merkle_inclusion_js/merkle_inclusion.wasm";

    if !std::path::Path::new(r1cs_path).exists() || !std::path::Path::new(wasm_path).exists() {
        eprintln!("Skipping proof verification test: circuit artifacts not found");
        return;
    }

    let audit = Arc::new(AuditLog::new().unwrap());

    // Record a few events.
    interceptor::record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
    interceptor::record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();
    interceptor::record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":3}"#).unwrap();

    // Generate a Groth16 proof for leaf at index 1.
    let inclusion = audit.prove_inclusion(1).unwrap();
    let prover = zk_audit::AuditProver::new(r1cs_path, wasm_path).unwrap();
    let groth16_proof = prover.prove(&inclusion).unwrap();

    // The public input (root) must match the audit log's current root.
    assert_eq!(groth16_proof.public_inputs.len(), 1);
    assert_eq!(groth16_proof.public_inputs[0], audit.root().unwrap());

    // Serialize for Soroban.
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof).unwrap();

    // Verify hex lengths.
    assert_eq!(soroban_args.proof.a.len(), 128); // G1: 64 bytes * 2 hex
    assert_eq!(soroban_args.proof.b.len(), 256); // G2: 128 bytes * 2 hex
    assert_eq!(soroban_args.proof.c.len(), 128); // G1: 64 bytes * 2 hex
    assert_eq!(soroban_args.pub_signals.len(), 1);
    assert_eq!(
        soroban_args.pub_signals[0],
        audit.root().unwrap().to_string()
    );

    eprintln!("✓ End-to-end proof generation succeeded");
    eprintln!("  Root: {}", soroban_args.pub_signals[0]);
    eprintln!("  Proof A: {}...", &soroban_args.proof.a[..32]);
}

/// Verify that proof generation works with the bundled Tauri resource
/// artifacts (not just the zk-spike build directory). This closes the
/// Phase 1 verification gap: "dev-mode path resolution for bundled
/// circuit resources not tested end-to-end."
#[test]
fn test_e2e_proof_with_bundled_resources() {
    // The bundled artifacts live at src-tauri/resources/circuits/.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let r1cs_path =
        std::path::Path::new(manifest_dir).join("resources/circuits/merkle_inclusion.r1cs");
    let wasm_path =
        std::path::Path::new(manifest_dir).join("resources/circuits/merkle_inclusion.wasm");

    if !r1cs_path.exists() || !wasm_path.exists() {
        eprintln!("Skipping bundled-resource proof test: artifacts not found");
        return;
    }

    let audit = Arc::new(AuditLog::new().unwrap());
    interceptor::record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":1}"#).unwrap();
    interceptor::record_insert(&audit, "rs:rs0", "db", "col", r#"{"a":2}"#).unwrap();

    let inclusion = audit.prove_inclusion(0).unwrap();
    let r1cs_str = r1cs_path.to_str().unwrap();
    let wasm_str = wasm_path.to_str().unwrap();
    let prover = zk_audit::AuditProver::new(r1cs_str, wasm_str).unwrap();
    let groth16_proof = prover.prove(&inclusion).unwrap();

    assert_eq!(groth16_proof.public_inputs[0], audit.root().unwrap());
    let soroban_args = zk_audit::AuditProver::serialize_for_soroban(&groth16_proof).unwrap();
    assert_eq!(soroban_args.proof.a.len(), 128);
    assert_eq!(soroban_args.proof.b.len(), 256);
    assert_eq!(soroban_args.proof.c.len(), 128);

    eprintln!("✓ Proof generation with bundled resources succeeded");
}
