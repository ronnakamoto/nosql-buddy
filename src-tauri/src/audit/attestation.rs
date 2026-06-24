//! Multi-publisher threshold attestation for audit log epochs.
//!
//! This module implements K-of-N threshold attestation: multiple
//! independent publishers attest each epoch's Merkle root, and an
//! epoch is considered "verified" when at least K out of N registered
//! publishers have submitted valid attestations.
//!
//! ## Architecture
//!
//! - **Publishers**: identified by their Stellar public key (ed25519).
//!   Each publisher is registered with a name and public key.
//! - **Attestations**: each attestation is an ed25519 signature of the
//!   epoch's root hash, signed by the publisher's private key. The
//!   signature is verified against the publisher's registered public key.
//! - **Threshold**: configurable K-of-N threshold. An epoch is
//!   "threshold-met" when at least K valid attestations have been
//!   submitted.
//!
//! ## Storage
//!
//! Publishers and attestations are stored in sled, keyed by:
//! - `publisher:{address}` → publisher JSON
//! - `attestation:{epoch_number}:{address}` → attestation JSON
//!
//! ## Verification
//!
//! When an attestation is submitted:
//! 1. The publisher must be registered.
//! 2. The signature is verified against the publisher's public key
//!    over the epoch's root hash.
//! 3. If valid, the attestation is stored.
//! 4. The threshold status is recomputed.

use std::sync::Mutex;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

use crate::audit::sled_store::SledTreeStore;
use crate::error::{AppError, AppResult};

/// A registered publisher that can attest epoch roots.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Publisher {
    /// The publisher's Stellar public key (ed25519, 32 bytes, hex-encoded).
    pub public_key: String,
    /// Human-readable name for the publisher.
    pub name: String,
    /// When the publisher was registered (ISO 8601).
    pub registered_at: String,
}

/// An attestation of an epoch's root by a publisher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attestation {
    /// The epoch being attested.
    pub epoch_number: u64,
    /// The root hash being attested (hex).
    pub root_hex: String,
    /// The publisher's public key (hex).
    pub publisher_public_key: String,
    /// The ed25519 signature of the root hash (hex, 64 bytes).
    pub signature: String,
    /// When the attestation was submitted (ISO 8601).
    pub submitted_at: String,
}

/// The threshold status for an epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttestationStatus {
    /// The epoch number.
    pub epoch_number: u64,
    /// The root hash being attested.
    pub root_hex: String,
    /// The threshold K (minimum attestations required).
    pub threshold: usize,
    /// The total number of registered publishers N.
    pub total_publishers: usize,
    /// The number of valid attestations received.
    pub valid_attestations: usize,
    /// Whether the threshold is met (valid_attestations >= threshold).
    pub threshold_met: bool,
    /// The addresses of publishers who have attested.
    pub attested_by: Vec<String>,
    /// The addresses of publishers who have not yet attested.
    pub pending: Vec<String>,
}

/// Manages publishers and attestations for threshold attestation.
pub struct AttestationManager {
    threshold: Mutex<usize>,
    store: Mutex<Option<SledTreeStore>>,
}

impl AttestationManager {
    /// Create a new attestation manager with the given threshold.
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold: Mutex::new(threshold),
            store: Mutex::new(None),
        }
    }

    /// Set the sled store for persistence. Called during app setup.
    pub fn set_store(&self, store: SledTreeStore) {
        let mut guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(store);
    }

    /// Update the threshold K.
    pub fn set_threshold(&self, threshold: usize) {
        let mut guard = self.threshold.lock().unwrap_or_else(|e| e.into_inner());
        *guard = threshold;
    }

    /// Get the current threshold.
    pub fn threshold(&self) -> usize {
        *self.threshold.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Register a new publisher.
    pub fn add_publisher(&self, public_key: String, name: String) -> AppResult<Publisher> {
        // Validate the public key is a valid ed25519 key.
        let pk_bytes = hex::decode(&public_key)
            .map_err(|e| AppError::Validation(format!("invalid public key hex: {e}")))?;
        if pk_bytes.len() != 32 {
            return Err(AppError::Validation(format!(
                "public key must be 32 bytes, got {}",
                pk_bytes.len()
            )));
        }
        // Verify it's a valid ed25519 verifying key.
        VerifyingKey::from_bytes(&pk_bytes.try_into().unwrap())
            .map_err(|e| AppError::Validation(format!("invalid ed25519 key: {e}")))?;

        // Check for duplicate.
        if self.get_publisher(&public_key)?.is_some() {
            return Err(AppError::Validation(format!(
                "publisher {} already registered",
                public_key
            )));
        }

        let publisher = Publisher {
            public_key: public_key.clone(),
            name,
            registered_at: chrono::Utc::now().to_rfc3339(),
        };

        self.save_publisher(&publisher)?;
        Ok(publisher)
    }

    /// Remove a publisher.
    pub fn remove_publisher(&self, public_key: &str) -> AppResult<()> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let key = publisher_key(public_key);
            store.remove_raw(&key)?;
        }
        Ok(())
    }

    /// Get a publisher by public key.
    pub fn get_publisher(&self, public_key: &str) -> AppResult<Option<Publisher>> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let key = publisher_key(public_key);
            if let Some(data) = store.get_raw(&key)? {
                let publisher: Publisher = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Internal(format!("deserialize publisher: {e}")))?;
                return Ok(Some(publisher));
            }
        }
        Ok(None)
    }

    /// List all registered publishers.
    pub fn list_publishers(&self) -> AppResult<Vec<Publisher>> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            return store.scan_prefix(PUBLISHER_PREFIX);
        }
        Ok(Vec::new())
    }

    /// Submit an attestation for an epoch.
    ///
    /// The attestation is verified:
    /// 1. The publisher must be registered.
    /// 2. The signature must be a valid ed25519 signature of the root hash
    ///    by the publisher's private key.
    pub fn submit_attestation(
        &self,
        epoch_number: u64,
        root_hex: &str,
        publisher_public_key: &str,
        signature_hex: &str,
    ) -> AppResult<Attestation> {
        // 1. Check the publisher is registered.
        let publisher = self.get_publisher(publisher_public_key)?
            .ok_or_else(|| AppError::Validation(format!(
                "publisher {} not registered",
                publisher_public_key
            )))?;

        // 2. Verify the signature.
        verify_signature(&publisher.public_key, root_hex, signature_hex)?;

        // 3. Check for duplicate attestation.
        if self.get_attestation(epoch_number, publisher_public_key)?.is_some() {
            return Err(AppError::Validation(format!(
                "publisher {} has already attested epoch {}",
                publisher_public_key, epoch_number
            )));
        }

        let attestation = Attestation {
            epoch_number,
            root_hex: root_hex.to_string(),
            publisher_public_key: publisher_public_key.to_string(),
            signature: signature_hex.to_string(),
            submitted_at: chrono::Utc::now().to_rfc3339(),
        };

        self.save_attestation(&attestation)?;
        Ok(attestation)
    }

    /// Get an attestation by epoch and publisher.
    pub fn get_attestation(
        &self,
        epoch_number: u64,
        publisher_public_key: &str,
    ) -> AppResult<Option<Attestation>> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let key = attestation_key(epoch_number, publisher_public_key);
            if let Some(data) = store.get_raw(&key)? {
                let attestation: Attestation = serde_json::from_slice(&data)
                    .map_err(|e| AppError::Internal(format!("deserialize attestation: {e}")))?;
                return Ok(Some(attestation));
            }
        }
        Ok(None)
    }

    /// List all attestations for an epoch.
    pub fn list_attestations(&self, epoch_number: u64) -> AppResult<Vec<Attestation>> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let prefix = attestation_prefix(epoch_number);
            return store.scan_prefix(&prefix);
        }
        Ok(Vec::new())
    }

    /// Get the attestation status for an epoch.
    pub fn get_status(&self, epoch_number: u64, root_hex: &str) -> AppResult<AttestationStatus> {
        let threshold = self.threshold();
        let publishers = self.list_publishers()?;
        let total_publishers = publishers.len();

        let attestations = self.list_attestations(epoch_number)?;
        let valid_attestations = attestations.len();

        let attested_by: Vec<String> = attestations
            .iter()
            .map(|a| a.publisher_public_key.clone())
            .collect();

        let pending: Vec<String> = publishers
            .iter()
            .map(|p| p.public_key.clone())
            .filter(|pk| !attested_by.contains(pk))
            .collect();

        Ok(AttestationStatus {
            epoch_number,
            root_hex: root_hex.to_string(),
            threshold,
            total_publishers,
            valid_attestations,
            threshold_met: valid_attestations >= threshold && threshold > 0,
            attested_by,
            pending,
        })
    }

    // ─── Internal storage helpers ─────────────────────────────────────

    fn save_publisher(&self, publisher: &Publisher) -> AppResult<()> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let key = publisher_key(&publisher.public_key);
            let data = serde_json::to_vec(publisher)?;
            store.insert_raw(&key, &data)?;
        }
        Ok(())
    }

    fn save_attestation(&self, attestation: &Attestation) -> AppResult<()> {
        let guard = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(store) = guard.as_ref() {
            let key = attestation_key(attestation.epoch_number, &attestation.publisher_public_key);
            let data = serde_json::to_vec(attestation)?;
            store.insert_raw(&key, &data)?;
        }
        Ok(())
    }
}

impl Default for AttestationManager {
    fn default() -> Self {
        Self::new(2) // Default: 2-of-N threshold
    }
}

// ─── Signature verification ───────────────────────────────────────────

/// Verify an ed25519 signature of a root hash.
fn verify_signature(
    public_key_hex: &str,
    root_hex: &str,
    signature_hex: &str,
) -> AppResult<()> {
    let pk_bytes = hex::decode(public_key_hex)
        .map_err(|e| AppError::Validation(format!("decode public key: {e}")))?;
    let sig_bytes = hex::decode(signature_hex)
        .map_err(|e| AppError::Validation(format!("decode signature: {e}")))?;
    let root_bytes = hex::decode(root_hex)
        .map_err(|e| AppError::Validation(format!("decode root: {e}")))?;

    if pk_bytes.len() != 32 {
        return Err(AppError::Validation(format!(
            "public key must be 32 bytes, got {}",
            pk_bytes.len()
        )));
    }
    if sig_bytes.len() != 64 {
        return Err(AppError::Validation(format!(
            "signature must be 64 bytes, got {}",
            sig_bytes.len()
        )));
    }

    let pk_array: [u8; 32] = pk_bytes.as_slice().try_into().unwrap();
    let sig_array: [u8; 64] = sig_bytes.as_slice().try_into().unwrap();

    let verifying_key = VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| AppError::Validation(format!("invalid public key: {e}")))?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify(&root_bytes, &signature)
        .map_err(|e| AppError::Validation(format!("signature verification failed: {e}")))
}

// ─── Sled key helpers ─────────────────────────────────────────────────

const PUBLISHER_PREFIX: &[u8] = b"publisher:";
const ATTESTATION_PREFIX: &[u8] = b"attestation:";

fn publisher_key(public_key: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(PUBLISHER_PREFIX.len() + public_key.len());
    key.extend_from_slice(PUBLISHER_PREFIX);
    key.extend_from_slice(public_key.as_bytes());
    key
}

fn attestation_key(epoch_number: u64, public_key: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(ATTESTATION_PREFIX.len() + 8 + 1 + public_key.len());
    key.extend_from_slice(ATTESTATION_PREFIX);
    key.extend_from_slice(&epoch_number.to_be_bytes());
    key.push(b':');
    key.extend_from_slice(public_key.as_bytes());
    key
}

fn attestation_prefix(epoch_number: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(ATTESTATION_PREFIX.len() + 8 + 1);
    key.extend_from_slice(ATTESTATION_PREFIX);
    key.extend_from_slice(&epoch_number.to_be_bytes());
    key.push(b':');
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{SigningKey, Signer};
    use rand::rngs::OsRng;

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nosqlbuddy-attestation-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_manager_with_store(threshold: usize) -> (AttestationManager, std::path::PathBuf) {
        let dir = tempdir();
        let db_path = dir.join("db");
        let store = SledTreeStore::open(&db_path).unwrap();
        let mgr = AttestationManager::new(threshold);
        mgr.set_store(store);
        (mgr, dir)
    }

    fn generate_keypair() -> (SigningKey, String) {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let public_key_hex = hex::encode(signing_key.verifying_key().to_bytes());
        (signing_key, public_key_hex)
    }

    fn sign_root(signing_key: &SigningKey, root_hex: &str) -> String {
        let root_bytes = hex::decode(root_hex).unwrap();
        let sig = signing_key.sign(&root_bytes);
        hex::encode(sig.to_bytes())
    }

    #[test]
    fn attestation_manager_default_threshold_is_2() {
        let mgr = AttestationManager::default();
        assert_eq!(mgr.threshold(), 2);
    }

    #[test]
    fn add_and_list_publisher() {
        let (mgr, dir) = make_manager_with_store(2);
        let (_, pk1) = generate_keypair();
        let (_, pk2) = generate_keypair();

        mgr.add_publisher(pk1.clone(), "Alice".to_string()).unwrap();
        mgr.add_publisher(pk2.clone(), "Bob".to_string()).unwrap();

        let publishers = mgr.list_publishers().unwrap();
        assert_eq!(publishers.len(), 2);

        let alice = mgr.get_publisher(&pk1).unwrap().unwrap();
        assert_eq!(alice.name, "Alice");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_duplicate_publisher_fails() {
        let (mgr, dir) = make_manager_with_store(2);
        let (_, pk) = generate_keypair();

        mgr.add_publisher(pk.clone(), "Alice".to_string()).unwrap();
        let result = mgr.add_publisher(pk, "Alice again".to_string());
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_publisher_with_invalid_key_fails() {
        let (mgr, dir) = make_manager_with_store(2);
        let result = mgr.add_publisher("not-a-valid-key".to_string(), "Bad".to_string());
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_publisher() {
        let (mgr, dir) = make_manager_with_store(2);
        let (_, pk) = generate_keypair();

        mgr.add_publisher(pk.clone(), "Alice".to_string()).unwrap();
        assert!(mgr.get_publisher(&pk).unwrap().is_some());

        mgr.remove_publisher(&pk).unwrap();
        assert!(mgr.get_publisher(&pk).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn submit_and_verify_attestation() {
        let (mgr, dir) = make_manager_with_store(2);
        let (signing_key, pk) = generate_keypair();

        mgr.add_publisher(pk.clone(), "Alice".to_string()).unwrap();

        let root_hex = hex::encode(&[0xAB; 32]);
        let sig = sign_root(&signing_key, &root_hex);

        let attestation = mgr
            .submit_attestation(0, &root_hex, &pk, &sig)
            .unwrap();

        assert_eq!(attestation.epoch_number, 0);
        assert_eq!(attestation.root_hex, root_hex);
        assert_eq!(attestation.publisher_public_key, pk);

        let attestations = mgr.list_attestations(0).unwrap();
        assert_eq!(attestations.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn submit_attestation_with_invalid_signature_fails() {
        let (mgr, dir) = make_manager_with_store(2);
        let (_, pk) = generate_keypair();

        mgr.add_publisher(pk.clone(), "Alice".to_string()).unwrap();

        let root_hex = hex::encode(&[0xAB; 32]);
        let bad_sig = hex::encode(&[0xFF; 64]);

        let result = mgr.submit_attestation(0, &root_hex, &pk, &bad_sig);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn submit_attestation_from_unregistered_publisher_fails() {
        let (mgr, dir) = make_manager_with_store(2);
        let (signing_key, pk) = generate_keypair();

        let root_hex = hex::encode(&[0xAB; 32]);
        let sig = sign_root(&signing_key, &root_hex);

        // Don't register the publisher.
        let result = mgr.submit_attestation(0, &root_hex, &pk, &sig);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_attestation_fails() {
        let (mgr, dir) = make_manager_with_store(2);
        let (signing_key, pk) = generate_keypair();

        mgr.add_publisher(pk.clone(), "Alice".to_string()).unwrap();

        let root_hex = hex::encode(&[0xAB; 32]);
        let sig = sign_root(&signing_key, &root_hex);

        mgr.submit_attestation(0, &root_hex, &pk, &sig).unwrap();
        let result = mgr.submit_attestation(0, &root_hex, &pk, &sig);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn threshold_met_when_enough_attestations() {
        let (mgr, dir) = make_manager_with_store(2);
        let (sk1, pk1) = generate_keypair();
        let (sk2, pk2) = generate_keypair();
        let (_sk3, pk3) = generate_keypair();

        mgr.add_publisher(pk1.clone(), "Alice".to_string()).unwrap();
        mgr.add_publisher(pk2.clone(), "Bob".to_string()).unwrap();
        mgr.add_publisher(pk3.clone(), "Carol".to_string()).unwrap();

        let root_hex = hex::encode(&[0xAB; 32]);

        // 0 attestations — not met.
        let status = mgr.get_status(0, &root_hex).unwrap();
        assert!(!status.threshold_met);
        assert_eq!(status.valid_attestations, 0);
        assert_eq!(status.total_publishers, 3);
        assert_eq!(status.pending.len(), 3);

        // 1 attestation — not met (threshold = 2).
        let sig1 = sign_root(&sk1, &root_hex);
        mgr.submit_attestation(0, &root_hex, &pk1, &sig1).unwrap();
        let status = mgr.get_status(0, &root_hex).unwrap();
        assert!(!status.threshold_met);
        assert_eq!(status.valid_attestations, 1);
        assert_eq!(status.attested_by.len(), 1);
        assert_eq!(status.pending.len(), 2);

        // 2 attestations — threshold met!
        let sig2 = sign_root(&sk2, &root_hex);
        mgr.submit_attestation(0, &root_hex, &pk2, &sig2).unwrap();
        let status = mgr.get_status(0, &root_hex).unwrap();
        assert!(status.threshold_met);
        assert_eq!(status.valid_attestations, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attestation_status_serializes_camel_case() {
        let status = AttestationStatus {
            epoch_number: 5,
            root_hex: "abc".to_string(),
            threshold: 3,
            total_publishers: 5,
            valid_attestations: 2,
            threshold_met: false,
            attested_by: vec!["pk1".to_string()],
            pending: vec!["pk2".to_string(), "pk3".to_string()],
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"epochNumber\":5"));
        assert!(json.contains("\"rootHex\":\"abc\""));
        assert!(json.contains("\"totalPublishers\":5"));
        assert!(json.contains("\"validAttestations\":2"));
        assert!(json.contains("\"thresholdMet\":false"));
        assert!(json.contains("\"attestedBy\""));
        assert!(json.contains("\"pending\""));
    }

    #[test]
    fn publisher_serializes_camel_case() {
        let p = Publisher {
            public_key: "abc".to_string(),
            name: "Alice".to_string(),
            registered_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"publicKey\":\"abc\""));
        assert!(json.contains("\"registeredAt\""));
    }

    #[test]
    fn attestation_serializes_camel_case() {
        let a = Attestation {
            epoch_number: 1,
            root_hex: "root".to_string(),
            publisher_public_key: "pk".to_string(),
            signature: "sig".to_string(),
            submitted_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"epochNumber\":1"));
        assert!(json.contains("\"rootHex\":\"root\""));
        assert!(json.contains("\"publisherPublicKey\":\"pk\""));
        assert!(json.contains("\"submittedAt\""));
    }
}
