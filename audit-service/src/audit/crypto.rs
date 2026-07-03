//! Cryptographic primitives for the ZK audit log.
//!
//! This module provides:
//! - **Age encryption** — envelope-encrypt epoch batches for multi-recipient
//!   confidentiality before IPFS pinning.
//! - **HMAC leaf derivation** — keyed leaves that resist offline dictionary
//!   attacks against public circuit signals.
//! - **Canonical payload encoding** — unambiguous, length-prefixed binary
//!   format that eliminates delimiter-injection vulnerabilities.
//!
//! ## Event versions
//!
//! - **v1** (legacy): payload is `"op|db|col|args"`, leaf is `SHA-256(payload)`.
//! - **v2** (current): payload is base64-encoded canonical bytes,
//!   leaf is `HMAC-SHA-256(k_audit, canonical_bytes)`.

use std::io::{Read, Write};
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use ark_bn254::Fr;
use ark_ff::PrimeField;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::{AuditError, AuditResult};

/// Generate a new age X25519 identity.
///
/// Returns `(secret_key_string, public_key_string)`.
pub fn generate_age_identity() -> (String, String) {
    let identity = age::x25519::Identity::generate();
    let secret = identity.to_string().expose_secret().to_string();
    let recipient = identity.to_public();
    let public = recipient.to_string();
    (secret, public)
}

/// Encrypt a plaintext batch for the given age recipients.
///
/// Each recipient can decrypt the ciphertext with their corresponding
/// age identity. The output is the binary age v1 format.
pub fn encrypt_batch(plaintext: &str, recipients: &[String]) -> AuditResult<Vec<u8>> {
    if recipients.is_empty() {
        return Err(AuditError::Validation(
            "encrypt_batch: no recipients provided".to_string(),
        ));
    }

    let parsed: Vec<Box<dyn age::Recipient>> = recipients
        .iter()
        .map(|s| {
            let r = age::x25519::Recipient::from_str(s)
                .map_err(|e| AuditError::Validation(format!("invalid age recipient: {e}")))?;
            Ok::<_, AuditError>(Box::new(r) as Box<dyn age::Recipient>)
        })
        .collect::<AuditResult<Vec<_>>>()?;

    let encryptor = age::Encryptor::with_recipients(parsed.iter().map(|r| r.as_ref()))
        .map_err(|e| AuditError::Validation(format!("age encryptor init: {e}")))?;

    let mut encrypted = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut encrypted)
        .map_err(|e| AuditError::Validation(format!("age wrap_output: {e}")))?;
    writer
        .write_all(plaintext.as_bytes())
        .map_err(|e| AuditError::Validation(format!("age write: {e}")))?;
    writer
        .finish()
        .map_err(|e| AuditError::Validation(format!("age finish: {e}")))?;

    Ok(encrypted)
}

/// Decrypt an age-encrypted batch using the provided identity.
pub fn decrypt_batch(ciphertext: &[u8], identity_str: &str) -> AuditResult<String> {
    let identity = age::x25519::Identity::from_str(identity_str)
        .map_err(|e| AuditError::Validation(format!("invalid age identity: {e}")))?;

    let decryptor = age::Decryptor::new_buffered(std::io::Cursor::new(ciphertext))
        .map_err(|e| AuditError::Validation(format!("age decryptor init: {e}")))?;

    let mut reader = decryptor
        .decrypt(std::iter::once(&identity as &dyn age::Identity))
        .map_err(|e| AuditError::Validation(format!("age decrypt: {e}")))?;

    let mut decrypted = Vec::new();
    reader
        .read_to_end(&mut decrypted)
        .map_err(|e| AuditError::Validation(format!("age read: {e}")))?;

    String::from_utf8(decrypted)
        .map_err(|e| AuditError::Validation(format!("decrypted bytes are not valid UTF-8: {e}")))
}

/// Generate a random 32-byte HMAC key for leaf derivation.
pub fn generate_leaf_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut key);
    key
}

/// Derive a Merkle leaf field element from canonical payload bytes using
/// HMAC-SHA-256 keyed by `k_audit`.
///
/// The output is truncated to 31.5 bytes (same scheme as the legacy
/// SHA-256-based derivation) and interpreted as a BN254 field element.
pub fn hmac_leaf(key: &[u8; 32], canonical_payload: &[u8]) -> Fr {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(canonical_payload);
    let result = mac.finalize();
    let hash = result.into_bytes();

    let mut bytes = [0u8; 32];
    bytes[..31].copy_from_slice(&hash[..31]);
    bytes[31] = hash[31] & 0x0F;
    Fr::from_be_bytes_mod_order(&bytes)
}

/// Build the canonical, unambiguous binary representation of an audit
/// event payload.
///
/// Format (big-endian length prefixes):
/// ```text
/// [op_len: u32 BE][op_bytes]
/// [db_len: u32 BE][db_bytes]
/// [col_len: u32 BE][col_bytes]
/// [data_len: u32 BE][data_bytes]
/// ```
///
/// This encoding is delimiter-free: no byte sequence inside any field
/// can be mistaken for a boundary, because every field is preceded by
/// an explicit 4-byte length.
pub fn canonical_payload_bytes(
    operation: &str,
    database: &str,
    collection: &str,
    data: &str,
) -> Vec<u8> {
    let mut buf = Vec::new();
    push_length_prefixed(&mut buf, operation.as_bytes());
    push_length_prefixed(&mut buf, database.as_bytes());
    push_length_prefixed(&mut buf, collection.as_bytes());
    push_length_prefixed(&mut buf, data.as_bytes());
    buf
}

/// Decode a canonical payload back into its components.
pub fn decode_canonical_payload(bytes: &[u8]) -> AuditResult<(String, String, String, String)> {
    let mut cursor = std::io::Cursor::new(bytes);

    let op = read_length_prefixed(&mut cursor)?;
    let db = read_length_prefixed(&mut cursor)?;
    let col = read_length_prefixed(&mut cursor)?;
    let data = read_length_prefixed(&mut cursor)?;

    // Ensure we've consumed all bytes (no trailing garbage).
    let pos = cursor.position() as usize;
    if pos != bytes.len() {
        return Err(AuditError::Validation(format!(
            "canonical payload has {} trailing bytes after decode",
            bytes.len() - pos
        )));
    }

    Ok((op, db, col, data))
}

/// Convenience: build the canonical payload bytes for an **insert** operation.
pub fn build_insert_payload(database: &str, collection: &str, document_json: &str) -> Vec<u8> {
    canonical_payload_bytes("insert", database, collection, document_json)
}

/// Convenience: build the canonical payload bytes for an **update** operation.
/// `filter_json` and `update_json` are packed as a JSON array `[filter, update]`
/// so the data field is a single deterministic string.
pub fn build_update_payload(
    database: &str,
    collection: &str,
    filter_json: &str,
    update_json: &str,
) -> Vec<u8> {
    let data = serde_json::to_string(&(filter_json, update_json))
        .unwrap_or_else(|_| format!("[{filter_json},{update_json}]"));
    canonical_payload_bytes("update", database, collection, &data)
}

/// Convenience: build the canonical payload bytes for a **delete** operation.
pub fn build_delete_payload(database: &str, collection: &str, filter_json: &str) -> Vec<u8> {
    canonical_payload_bytes("delete", database, collection, filter_json)
}

/// Convenience: build the canonical payload bytes for a **drop_collection** operation.
pub fn build_drop_collection_payload(database: &str, collection: &str) -> Vec<u8> {
    canonical_payload_bytes("drop_collection", database, collection, "")
}

/// Convenience: build the canonical payload bytes for a **drop_database** operation.
pub fn build_drop_database_payload(database: &str) -> Vec<u8> {
    canonical_payload_bytes("drop_database", database, "", "")
}

/// Convenience: build the canonical payload bytes for a **rename** operation.
pub fn build_rename_payload(database: &str, collection: &str, new_name: &str) -> Vec<u8> {
    canonical_payload_bytes("rename", database, collection, new_name)
}

/// Convenience: build the canonical payload bytes for a **create_index** operation.
pub fn build_create_index_payload(
    database: &str,
    collection: &str,
    keys_json: &str,
    options_json: &str,
) -> Vec<u8> {
    let data = serde_json::to_string(&(keys_json, options_json))
        .unwrap_or_else(|_| format!("[{keys_json},{options_json}]"));
    canonical_payload_bytes("create_index", database, collection, &data)
}

/// Convenience: build the canonical payload bytes for a **drop_index** operation.
pub fn build_drop_index_payload(database: &str, collection: &str, index_name: &str) -> Vec<u8> {
    canonical_payload_bytes("drop_index", database, collection, index_name)
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn push_length_prefixed(buf: &mut Vec<u8>, data: &[u8]) {
    buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
    buf.extend_from_slice(data);
}

fn read_length_prefixed(cursor: &mut std::io::Cursor<&[u8]>) -> AuditResult<String> {
    let mut len_buf = [0u8; 4];
    std::io::Read::read_exact(cursor, &mut len_buf)
        .map_err(|e| AuditError::Validation(format!("canonical payload read length: {e}")))?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut data = vec![0u8; len];
    std::io::Read::read_exact(cursor, &mut data)
        .map_err(|e| AuditError::Validation(format!("canonical payload read data: {e}")))?;

    String::from_utf8(data)
        .map_err(|e| AuditError::Validation(format!("canonical payload utf-8: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_payload_round_trip() {
        let bytes = canonical_payload_bytes("insert", "shopkeeper", "inventory", r#"{"sku":"A"}"#);
        let (op, db, col, data) = decode_canonical_payload(&bytes).unwrap();
        assert_eq!(op, "insert");
        assert_eq!(db, "shopkeeper");
        assert_eq!(col, "inventory");
        assert_eq!(data, r#"{"sku":"A"}"#);
    }

    #[test]
    fn canonical_payload_resists_injection() {
        // A collection name containing a pipe or JSON delimiter must not
        // create ambiguity.
        let bytes = canonical_payload_bytes(
            "insert",
            "db",
            "col|special",
            r#"{"a":"b|c"}"#,
        );
        let (op, db, col, data) = decode_canonical_payload(&bytes).unwrap();
        assert_eq!(op, "insert");
        assert_eq!(db, "db");
        assert_eq!(col, "col|special");
        assert_eq!(data, r#"{"a":"b|c"}"#);
    }

    #[test]
    fn hmac_leaf_is_deterministic() {
        let key = [0xABu8; 32];
        let payload = b"test payload";
        let leaf1 = hmac_leaf(&key, payload);
        let leaf2 = hmac_leaf(&key, payload);
        assert_eq!(leaf1, leaf2);
    }

    #[test]
    fn hmac_leaf_differs_with_different_keys() {
        let key1 = [0xABu8; 32];
        let key2 = [0xCDu8; 32];
        let payload = b"test payload";
        let leaf1 = hmac_leaf(&key1, payload);
        let leaf2 = hmac_leaf(&key2, payload);
        assert_ne!(leaf1, leaf2);
    }

    #[test]
    fn age_encrypt_decrypt_round_trip() {
        let (secret, public) = generate_age_identity();
        let plaintext = "hello epoch batch\nline 2\n";
        let encrypted = encrypt_batch(plaintext, &[public.clone()]).unwrap();
        let decrypted = decrypt_batch(&encrypted, &secret).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn age_multi_recipient_decrypt() {
        let (secret_a, public_a) = generate_age_identity();
        let (_secret_b, public_b) = generate_age_identity();
        let plaintext = "multi-recipient test";
        let encrypted = encrypt_batch(plaintext, &[public_a.clone(), public_b]).unwrap();
        let decrypted = decrypt_batch(&encrypted, &secret_a).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    /// E2E: encrypt a realistic epoch batch, decrypt with auditor identity,
    /// and verify the plaintext JSONL lines are intact.
    #[test]
    fn e2e_encrypt_decrypt_epoch_batch() {
        let (operator_secret, operator_public) = generate_age_identity();
        let (auditor_secret, auditor_public) = generate_age_identity();

        let batch = r#"{"index":0,"operation":"insert","database":"shopkeeper","collection":"inventory_log","payload":"insert|shopkeeper|inventory_log|{\"sku\":\"WH-1000XM6\"}","leaf_hex":"1a2b...","root_after":"3c4d...","timestamp":"2026-07-03T04:37:32Z"}
{"index":1,"operation":"update","database":"shopkeeper","collection":"inventory_log","payload":"update|shopkeeper|inventory_log|{\"_id\":{\"$oid\":\"6a473c6f\"}}|{\"$set\":{\"sku\":\"WH-1000XM6\"}}","leaf_hex":"5e6f...","root_after":"7a8b...","timestamp":"2026-07-03T04:37:33Z"}"#;

        let encrypted = encrypt_batch(batch, &[operator_public, auditor_public]).unwrap();
        let decrypted = decrypt_batch(&encrypted, &auditor_secret).unwrap();
        assert_eq!(decrypted, batch);

        // Verify operator can also decrypt.
        let decrypted_op = decrypt_batch(&encrypted, &operator_secret).unwrap();
        assert_eq!(decrypted_op, batch);
    }

    /// Negative: an encrypted batch must not contain any plaintext payload
    /// substrings (the v1 pipe-delimited format).
    #[test]
    fn encrypted_batch_contains_no_plaintext_payloads() {
        let (secret, public) = generate_age_identity();
        let batch = "insert|shopkeeper|inventory_log|{\"sku\":\"WH-1000XM6\"}\n";
        let encrypted = encrypt_batch(batch, &[public]).unwrap();

        // The ciphertext must not contain the original plaintext.
        assert!(
            !encrypted.windows(batch.len()).any(|w| w == batch.as_bytes()),
            "encrypted batch must not contain the original plaintext bytes"
        );

        // But decryption must recover it exactly.
        let decrypted = decrypt_batch(&encrypted, &secret).unwrap();
        assert_eq!(decrypted, batch);
    }

    /// E2E: v2 canonical payload → HMAC leaf round-trip.
    #[test]
    fn e2e_v2_payload_leaf_round_trip() {
        let key = generate_leaf_key();
        let canonical = build_insert_payload("shopkeeper", "inventory_log", r#"{"sku":"WH-1000XM6"}"#);
        let leaf = hmac_leaf(&key, &canonical);

        // Same canonical bytes → same leaf.
        let leaf2 = hmac_leaf(&key, &canonical);
        assert_eq!(leaf, leaf2);

        // Different payload → different leaf.
        let canonical2 = build_insert_payload("shopkeeper", "inventory_log", r#"{"sku":"WH-1000XM7"}"#);
        let leaf3 = hmac_leaf(&key, &canonical2);
        assert_ne!(leaf, leaf3);
    }
}
