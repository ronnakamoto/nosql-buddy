//! Tests for the ZK-AuditDB commitment contract.

#![cfg(test)]

use super::*;
use soroban_sdk::testutils::Address as _;

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
