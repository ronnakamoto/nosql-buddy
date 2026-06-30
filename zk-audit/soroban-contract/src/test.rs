//! Tests for the ZK-AuditDB commitment contract.

#![cfg(test)]

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use soroban_sdk::testutils::Address as _;
use stellar_strkey::Strkey;

/// Generate a test attester: a random ed25519 keypair, its Stellar account
/// address, and the 32-byte public key to register with the contract.
fn generate_attester(env: &Env) -> (Address, SigningKey, BytesN<32>) {
    let mut csprng = OsRng;
    let signing_key = SigningKey::generate(&mut csprng);
    let public_key_bytes = signing_key.verifying_key().to_bytes();
    let strkey = Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(public_key_bytes));
    let address = Address::from_string(&String::from_str(env, &strkey.to_string()));
    let public_key = BytesN::from_array(env, &public_key_bytes);
    (address, signing_key, public_key)
}

/// Sign the oplog attestation message: sha256(oplog_root || oplog_end_ts.to_be_bytes()).
fn sign_oplog_attestation(signing_key: &SigningKey, oplog_root: &[u8; 32], oplog_end_ts: u64) -> [u8; 64] {
    let mut message = [0u8; 40];
    message[0..32].copy_from_slice(oplog_root);
    message[32..40].copy_from_slice(&oplog_end_ts.to_be_bytes());
    let message_hash = Sha256::digest(&message);
    signing_key.sign(&message_hash).to_bytes()
}

#[test]
fn test_initialize_and_admin() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let got_admin = client.get_admin();
    assert_eq!(got_admin, admin);
    assert_eq!(client.get_threshold(), 1);
}

#[test]
fn test_set_threshold() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    client.set_threshold(&2u32);
    assert_eq!(client.get_threshold(), 2);

    let result = client.try_set_threshold(&0u32);
    assert!(result.is_err());
}

#[test]
fn test_commit_root() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0u8; 32]);
    let metadata = String::from_str(&env, "first commit");
    let seq = client.commit_root(&root, &metadata);
    assert_eq!(seq, 1);

    let current = client.get_current_root();
    assert!(current.is_some());
    let entry = current.unwrap();
    assert_eq!(entry.sequence, 1);
    assert_eq!(entry.root, root);
    assert_eq!(entry.metadata, metadata);
}

#[test]
fn test_commit_multiple_roots() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root1 = Bytes::from_array(&env, &[1u8; 32]);
    let root2 = Bytes::from_array(&env, &[2u8; 32]);
    let root3 = Bytes::from_array(&env, &[3u8; 32]);

    let seq1 = client.commit_root(&root1, &String::from_str(&env, "r1"));
    let seq2 = client.commit_root(&root2, &String::from_str(&env, "r2"));
    let seq3 = client.commit_root(&root3, &String::from_str(&env, "r3"));

    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);
    assert_eq!(seq3, 3);

    let current = client.get_current_root().unwrap();
    assert_eq!(current.sequence, 3);
    assert_eq!(current.root, root3);
}

#[test]
fn test_duplicate_root_rejected() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0xAA; 32]);
    client.commit_root(&root, &String::from_str(&env, "first"));

    let result = client.try_commit_root(&root, &String::from_str(&env, "second"));
    assert!(result.is_err());
}

#[test]
fn test_root_history() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    for i in 1..=5u8 {
        let mut arr = [0u8; 32];
        arr[0] = i;
        let root = Bytes::from_array(&env, &arr);
        client.commit_root(&root, &String::from_str(&env, "test"));
    }

    let history = client.get_root_history(&3);
    assert_eq!(history.len(), 3);
    assert_eq!(history.get(0).unwrap().sequence, 5);
    assert_eq!(history.get(1).unwrap().sequence, 4);
    assert_eq!(history.get(2).unwrap().sequence, 3);
}

#[test]
fn test_invalid_page_size() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let result = client.try_get_root_history(&0);
    assert!(result.is_err());

    let result = client.try_get_root_history(&101);
    assert!(result.is_err());
}

#[test]
fn test_verify_inclusion_root_not_committed() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0u8; 32]);
    let proof = Proof {
        a: Bytes::from_array(&env, &[0u8; 64]),
        b: Bytes::from_array(&env, &[0u8; 128]),
        c: Bytes::from_array(&env, &[0u8; 64]),
    };
    let vk = VerifyingKey {
        alpha: Bytes::from_array(&env, &[0u8; 64]),
        beta: Bytes::from_array(&env, &[0u8; 128]),
        gamma: Bytes::from_array(&env, &[0u8; 128]),
        delta: Bytes::from_array(&env, &[0u8; 128]),
        ic: Vec::new(&env),
    };

    let result = client.try_verify_inclusion(&root, &proof, &vk);
    assert!(result.is_err());
}

#[test]
fn test_verify_inclusion_invalid_encoding() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0u8; 32]);
    client.commit_root(&root, &String::from_str(&env, "test"));

    let proof = Proof {
        a: Bytes::from_array(&env, &[0u8; 32]), // wrong: should be 64
        b: Bytes::from_array(&env, &[0u8; 128]),
        c: Bytes::from_array(&env, &[0u8; 64]),
    };
    let vk = VerifyingKey {
        alpha: Bytes::from_array(&env, &[0u8; 64]),
        beta: Bytes::from_array(&env, &[0u8; 128]),
        gamma: Bytes::from_array(&env, &[0u8; 128]),
        delta: Bytes::from_array(&env, &[0u8; 128]),
        ic: Vec::new(&env),
    };

    let result = client.try_verify_inclusion(&root, &proof, &vk);
    assert!(result.is_err());
}

#[test]
fn test_no_current_root() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let current = client.get_current_root();
    assert!(current.is_none());
}

// ─── Oplog commitment tests ───────────────────────────────────────────

#[test]
fn test_commit_root_with_oplog() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let metadata = String::from_str(&env, "epoch=0 oplog=true");

    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,          // oplog_start_ts
        &1000u64,       // oplog_end_ts
        &42u64,         // oplog_entry_count
        &metadata,
    );
    assert_eq!(seq, 1);

    // The root entry should be stored.
    let current = client.get_current_root();
    assert!(current.is_some());
    let entry = current.unwrap();
    assert_eq!(entry.sequence, 1);
    assert_eq!(entry.root, root);

    // The oplog commitment should be stored.
    let oplog = client.get_oplog_commitment(&1u64);
    assert_eq!(oplog.oplog_root, oplog_root);
    assert_eq!(oplog.oplog_start_ts, 0);
    assert_eq!(oplog.oplog_end_ts, 1000);
    assert_eq!(oplog.oplog_entry_count, 42);
}

#[test]
fn test_commit_root_with_oplog_invalid_root_length() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let bad_oplog_root = Bytes::from_array(&env, &[0xbb; 16]); // wrong length
    let metadata = String::from_str(&env, "test");

    let result = client.try_commit_root_with_oplog(
        &root,
        &bad_oplog_root,
        &0u64,
        &1000u64,
        &42u64,
        &metadata,
    );
    assert!(result.is_err());
}

#[test]
fn test_get_oplog_commitment_not_found() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    let result = client.try_get_oplog_commitment(&999u64);
    assert!(result.is_err());
}

#[test]
fn test_authorize_and_attest_oplog() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester, attester_key, public_key) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    // Authorize the attester with its registered public key.
    client.authorize_attester(&attester, &public_key);

    // Commit a root with oplog hash.
    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );
    assert_eq!(seq, 1);

    // Submit a valid attestation signed over the oplog commitment.
    let signature_bytes = sign_oplog_attestation(&attester_key, &[0xbb; 32], oplog_end_ts);
    let signature = Bytes::from_array(&env, &signature_bytes);
    client.attest_oplog(&attester, &seq, &signature);

    // Verify attestation was recorded.
    let attestations = client.get_oplog_attestations(&seq);
    assert_eq!(attestations.len(), 1);
    assert_eq!(attestations.get(0).unwrap().attester, attester);
    assert_eq!(attestations.get(0).unwrap().signature, signature);
}

#[test]
fn test_attest_oplog_unauthorized_attester() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (unauthorized_attester, unauthorized_key, _) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    // Commit a root with oplog hash.
    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );

    // Try to attest without being authorized (but with a structurally valid signature).
    let signature_bytes = sign_oplog_attestation(&unauthorized_key, &[0xbb; 32], oplog_end_ts);
    let signature = Bytes::from_array(&env, &signature_bytes);
    let result = client.try_attest_oplog(&unauthorized_attester, &seq, &signature);
    assert!(result.is_err());
}

#[test]
fn test_attest_oplog_duplicate_rejected() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester, attester_key, public_key) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);
    client.authorize_attester(&attester, &public_key);

    // Commit a root with oplog hash.
    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );

    // First attestation succeeds.
    let signature_bytes = sign_oplog_attestation(&attester_key, &[0xbb; 32], oplog_end_ts);
    let signature = Bytes::from_array(&env, &signature_bytes);
    client.attest_oplog(&attester, &seq, &signature);

    // Second attestation from same attester fails.
    let result = client.try_attest_oplog(&attester, &seq, &signature);
    assert!(result.is_err());
}

#[test]
fn test_attest_oplog_invalid_signature_length() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester, _, public_key) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);
    client.authorize_attester(&attester, &public_key);

    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &1000u64,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );

    // Signature of wrong length (32 instead of 64).
    let bad_sig = Bytes::from_array(&env, &[0xcc; 32]);
    let result = client.try_attest_oplog(&attester, &seq, &bad_sig);
    assert!(result.is_err());
}

#[test]
fn test_revoke_attester() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester, attester_key, public_key) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);
    client.authorize_attester(&attester, &public_key);

    // Commit and attest successfully.
    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );
    let signature_bytes = sign_oplog_attestation(&attester_key, &[0xbb; 32], oplog_end_ts);
    let signature = Bytes::from_array(&env, &signature_bytes);
    client.attest_oplog(&attester, &seq, &signature);

    // Revoke the attester.
    client.revoke_attester(&attester);

    // Commit another epoch and try to attest — should fail.
    let root2 = Bytes::from_array(&env, &[0xdd; 32]);
    let oplog_root2 = Bytes::from_array(&env, &[0xee; 32]);
    let oplog_end_ts2 = 2000u64;
    let seq2 = client.commit_root_with_oplog(
        &root2,
        &oplog_root2,
        &1000u64,
        &oplog_end_ts2,
        &10u64,
        &String::from_str(&env, "epoch=1"),
    );

    let signature2_bytes = sign_oplog_attestation(&attester_key, &[0xee; 32], oplog_end_ts2);
    let signature2 = Bytes::from_array(&env, &signature2_bytes);
    let result = client.try_attest_oplog(&attester, &seq2, &signature2);
    assert!(result.is_err());
}

#[test]
fn test_verify_attestation() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester, attester_key, public_key) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);

    // Authorize the attester with its registered public key.
    client.authorize_attester(&attester, &public_key);

    // Commit a root with oplog hash.
    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );
    assert_eq!(seq, 1);

    // Submit a valid attestation signed over the oplog commitment.
    let signature_bytes = sign_oplog_attestation(&attester_key, &[0xbb; 32], oplog_end_ts);
    let signature = Bytes::from_array(&env, &signature_bytes);
    client.attest_oplog(&attester, &seq, &signature);

    // Verify attestation: attester is still authorized, so all_match is true.
    let verification = client.verify_attestation(&seq);
    assert_eq!(verification.sequence, seq);
    assert_eq!(verification.oplog_root, oplog_root);
    assert_eq!(verification.attestation_count, 1);
    assert_eq!(verification.authorized_count, 1);
    assert_eq!(verification.threshold, 1);
    assert_eq!(verification.all_match, true);
    assert_eq!(verification.verdict, String::from_str(&env, "verified"));

    // Test the "no_attestations" case on a sequence with no attestations.
    // Commit a second epoch with an oplog commitment but no attestations.
    let root2 = Bytes::from_array(&env, &[0xdd; 32]);
    let oplog_root2 = Bytes::from_array(&env, &[0xee; 32]);
    let seq2 = client.commit_root_with_oplog(
        &root2,
        &oplog_root2,
        &1000u64,
        &2000u64,
        &10u64,
        &String::from_str(&env, "epoch=1"),
    );

    let verification2 = client.verify_attestation(&seq2);
    assert_eq!(verification2.sequence, seq2);
    assert_eq!(verification2.oplog_root, oplog_root2);
    assert_eq!(verification2.attestation_count, 0);
    assert_eq!(verification2.authorized_count, 0);
    assert_eq!(verification2.all_match, false);
    assert_eq!(verification2.verdict, String::from_str(&env, "no_attestations"));
}

#[test]
fn test_verify_attestation_threshold_k_of_n() {
    let env = Env::default();
    let contract_id = env.register_contract(None, ZkAuditCommitment);
    let client = ZkAuditCommitmentClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let (attester1, key1, pk1) = generate_attester(&env);
    let (attester2, key2, pk2) = generate_attester(&env);
    env.mock_all_auths();
    client.initialize(&admin);
    client.authorize_attester(&attester1, &pk1);
    client.authorize_attester(&attester2, &pk2);

    // Require 2-of-N.
    client.set_threshold(&2u32);

    let root = Bytes::from_array(&env, &[0xaa; 32]);
    let oplog_root = Bytes::from_array(&env, &[0xbb; 32]);
    let oplog_end_ts = 1000u64;
    let seq = client.commit_root_with_oplog(
        &root,
        &oplog_root,
        &0u64,
        &oplog_end_ts,
        &42u64,
        &String::from_str(&env, "epoch=0"),
    );

    // First attester signs — authorized but below the threshold of 2.
    let sig1 = Bytes::from_array(&env, &sign_oplog_attestation(&key1, &[0xbb; 32], oplog_end_ts));
    client.attest_oplog(&attester1, &seq, &sig1);

    let v1 = client.verify_attestation(&seq);
    assert_eq!(v1.threshold, 2);
    assert_eq!(v1.authorized_count, 1);
    assert_eq!(v1.all_match, false);
    assert_eq!(v1.verdict, String::from_str(&env, "threshold_not_met"));

    // Second authorized attester signs — threshold met.
    let sig2 = Bytes::from_array(&env, &sign_oplog_attestation(&key2, &[0xbb; 32], oplog_end_ts));
    client.attest_oplog(&attester2, &seq, &sig2);

    let v2 = client.verify_attestation(&seq);
    assert_eq!(v2.authorized_count, 2);
    assert_eq!(v2.all_match, true);
    assert_eq!(v2.verdict, String::from_str(&env, "verified"));

    // Revoking one attester invalidates its on-record attestation, so the
    // count of authorized attestations no longer matches the total — the
    // verdict reflects an unauthorized attestation on record.
    client.revoke_attester(&attester2);
    let v3 = client.verify_attestation(&seq);
    assert_eq!(v3.authorized_count, 1);
    assert_eq!(v3.all_match, false);
    assert_eq!(v3.verdict, String::from_str(&env, "unauthorized_attester"));
}
