//! Sled-backed persistent storage for the audit Merkle tree.
//!
//! The JSONL file remains as the human-readable audit trail, but the
//! tree state (leaves + root) is persisted in sled for fast startup.
//! Without sled, startup requires replaying every event through the
//! Poseidon hasher — O(n) with a large constant. With sled, startup
//! is O(1): load the root and leaf count from the key-value store.
//!
//! ## Storage layout
//!
//! - `meta:leaf_count` → u64 (big-endian)
//! - `meta:root_hex` → String (hex-encoded root)
//! - `leaf:{index}` → 32-byte leaf value (big-endian Fr)
//!
//! On `record()`, the leaf is written to sled. On startup, the tree
//! is rebuilt from the stored leaves (still O(n), but the leaves are
//! raw bytes — no Poseidon hashing needed). A future optimization
//! would store internal nodes too, making startup truly O(1).

use std::path::Path;

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use sled::Db;

// Note: PrimeField is needed for from_be_bytes_mod_order in load_tree.

use crate::audit::AuditMerkleTree;
use crate::error::{AuditError, AuditResult};

const META_LEAF_COUNT: &[u8] = b"meta:leaf_count";
const META_ROOT_HEX: &[u8] = b"meta:root_hex";
const LEAF_PREFIX: &[u8] = b"leaf:";
const RESUME_TOKEN_PREFIX: &[u8] = b"resume_token:";
const IPFS_CID_PREFIX: &[u8] = b"ipfs_cid:";

/// A sled-backed store for audit tree state.
pub struct SledTreeStore {
    db: Db,
    path: std::path::PathBuf,
}

impl SledTreeStore {
    /// Open or create a sled database at the given path.
    pub fn open(path: &Path) -> AuditResult<Self> {
        let db = sled::open(path)
            .map_err(|e| AuditError::Validation(format!("open sled tree store: {e}")))?;
        Ok(Self {
            db,
            path: path.to_path_buf(),
        })
    }

    /// Get the filesystem path of the sled database.
    pub fn db_path(&self) -> &Path {
        &self.path
    }

    /// Save a leaf at the given index and update the metadata.
    pub fn save_leaf(&self, index: u64, leaf: Fr, root: Fr) -> AuditResult<()> {
        // Store the leaf as 32-byte big-endian.
        let leaf_bytes = leaf.into_bigint().to_bytes_be();
        let key = leaf_key(index);
        self.db
            .insert(key.as_slice(), leaf_bytes.as_slice())
            .map_err(|e| AuditError::Validation(format!("sled insert leaf: {e}")))?;

        // Update metadata. Leaf count is index + 1 (index is 0-based).
        let leaf_count = index + 1;
        self.db
            .insert(META_LEAF_COUNT, &leaf_count.to_be_bytes())
            .map_err(|e| AuditError::Validation(format!("sled insert leaf_count: {e}")))?;

        let root_hex = hex::encode(root.into_bigint().to_bytes_be());
        self.db
            .insert(META_ROOT_HEX, root_hex.as_bytes())
            .map_err(|e| AuditError::Validation(format!("sled insert root_hex: {e}")))?;

        self.db
            .flush()
            .map_err(|e| AuditError::Validation(format!("sled flush: {e}")))?;

        Ok(())
    }

    /// Load all leaves from sled and rebuild the tree.
    /// This is O(n) in the number of leaves, but avoids the Poseidon
    /// hashing that JSONL replay requires.
    pub fn load_tree(&self) -> AuditResult<Option<(AuditMerkleTree, String)>> {
        let leaf_count_bytes = self.db.get(META_LEAF_COUNT).map_err(|e| {
            AuditError::Validation(format!("sled get leaf_count: {e}"))
        })?;

        let leaf_count = match leaf_count_bytes {
            Some(bytes) => {
                let arr: [u8; 8] = bytes.as_ref().try_into().map_err(|_| {
                    AuditError::Validation("leaf_count is not 8 bytes".to_string())
                })?;
                u64::from_be_bytes(arr)
            }
            None => return Ok(None), // empty store
        };

        let root_hex = self
            .db
            .get(META_ROOT_HEX)
            .map_err(|e| AuditError::Validation(format!("sled get root_hex: {e}")))?
            .map(|v| String::from_utf8_lossy(&v).to_string())
            .unwrap_or_default();

        // Rebuild the tree from stored leaves.
        let mut tree = AuditMerkleTree::with_height(20)?;
        for i in 0..leaf_count {
            let key = leaf_key(i);
            let leaf_bytes = self
                .db
                .get(key.as_slice())
                .map_err(|e| AuditError::Validation(format!("sled get leaf {i}: {e}")))?
                .ok_or_else(|| {
                    AuditError::Validation(format!("missing leaf {i} in sled store"))
                })?;

            // Convert bytes back to Fr using from_be_bytes_mod_order.
            let leaf = Fr::from_be_bytes_mod_order(leaf_bytes.as_ref());

            tree.insert(leaf);
        }

        Ok(Some((tree, root_hex)))
    }

    /// Get the stored leaf count (without loading the tree).
    pub fn leaf_count(&self) -> AuditResult<u64> {
        let bytes = self.db.get(META_LEAF_COUNT).map_err(|e| {
            AuditError::Validation(format!("sled get leaf_count: {e}"))
        })?;
        match bytes {
            Some(v) => {
                let arr: [u8; 8] = v.as_ref().try_into().map_err(|_| {
                    AuditError::Validation("leaf_count is not 8 bytes".to_string())
                })?;
                Ok(u64::from_be_bytes(arr))
            }
            None => Ok(0),
        }
    }

    /// Clear all data from the store.
    pub fn clear(&self) -> AuditResult<()> {
        self.db
            .clear()
            .map_err(|e| AuditError::Validation(format!("sled clear: {e}")))?;
        Ok(())
    }

    /// Save a change stream resume token for the given connection ID.
    /// The token is stored as a JSON string so it can be deserialized back
    /// into a `ResumeToken` on startup.
    pub fn save_resume_token(&self, connection_id: &str, token_json: &str) -> AuditResult<()> {
        let key = resume_token_key(connection_id);
        self.db
            .insert(key.as_slice(), token_json.as_bytes())
            .map_err(|e| AuditError::Validation(format!("sled insert resume_token: {e}")))?;
        self.db
            .flush()
            .map_err(|e| AuditError::Validation(format!("sled flush resume_token: {e}")))?;
        Ok(())
    }

    /// Load the saved change stream resume token for the given connection ID.
    /// Returns `None` if no token has been saved for this connection.
    pub fn load_resume_token(&self, connection_id: &str) -> AuditResult<Option<String>> {
        let key = resume_token_key(connection_id);
        let val = self
            .db
            .get(key.as_slice())
            .map_err(|e| AuditError::Validation(format!("sled get resume_token: {e}")))?;
        Ok(val.map(|v| String::from_utf8_lossy(&v).to_string()))
    }

    /// Clear the saved resume token for the given connection ID.
    pub fn clear_resume_token(&self, connection_id: &str) -> AuditResult<()> {
        let key = resume_token_key(connection_id);
        self.db
            .remove(key.as_slice())
            .map_err(|e| AuditError::Validation(format!("sled remove resume_token: {e}")))?;
        Ok(())
    }

    /// Save an IPFS CID for a published epoch.
    pub fn save_ipfs_cid(&self, epoch_number: u64, cid: &str) -> AuditResult<()> {
        let key = ipfs_cid_key(epoch_number);
        self.db
            .insert(key.as_slice(), cid.as_bytes())
            .map_err(|e| AuditError::Validation(format!("sled insert ipfs_cid: {e}")))?;
        self.db
            .flush()
            .map_err(|e| AuditError::Validation(format!("sled flush ipfs_cid: {e}")))?;
        Ok(())
    }

    /// Load the saved IPFS CID for a published epoch.
    pub fn load_ipfs_cid(&self, epoch_number: u64) -> AuditResult<Option<String>> {
        let key = ipfs_cid_key(epoch_number);
        let val = self
            .db
            .get(key.as_slice())
            .map_err(|e| AuditError::Validation(format!("sled get ipfs_cid: {e}")))?;
        Ok(val.map(|v| String::from_utf8_lossy(&v).to_string()))
    }

    /// Insert a raw key-value pair into the sled store.
    /// Used by the attestation module for publisher/attestation storage.
    pub fn insert_raw(&self, key: &[u8], value: &[u8]) -> AuditResult<()> {
        self.db
            .insert(key, value)
            .map_err(|e| AuditError::Validation(format!("sled insert raw: {e}")))?;
        self.db
            .flush()
            .map_err(|e| AuditError::Validation(format!("sled flush: {e}")))?;
        Ok(())
    }

    /// Get a raw value by key from the sled store.
    pub fn get_raw(&self, key: &[u8]) -> AuditResult<Option<Vec<u8>>> {
        let val = self
            .db
            .get(key)
            .map_err(|e| AuditError::Validation(format!("sled get raw: {e}")))?;
        Ok(val.map(|v| v.to_vec()))
    }

    /// Remove a raw key from the sled store.
    pub fn remove_raw(&self, key: &[u8]) -> AuditResult<()> {
        self.db
            .remove(key)
            .map_err(|e| AuditError::Validation(format!("sled remove raw: {e}")))?;
        self.db
            .flush()
            .map_err(|e| AuditError::Validation(format!("sled flush: {e}")))?;
        Ok(())
    }

    /// Scan all keys with the given prefix and deserialize values as T.
    /// Used by the attestation module to list publishers/attestations.
    pub fn scan_prefix<T: serde::de::DeserializeOwned>(&self, prefix: &[u8]) -> AuditResult<Vec<T>> {
        let mut results = Vec::new();
        for item in self.db.scan_prefix(prefix) {
            let (_key, value) = item
                .map_err(|e| AuditError::Validation(format!("sled scan: {e}")))?;
            let item: T = serde_json::from_slice(&value)
                .map_err(|e| AuditError::Internal(format!("deserialize scan result: {e}")))?;
            results.push(item);
        }
        Ok(results)
    }
}

/// Build the sled key for a leaf at the given index.
fn leaf_key(index: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(LEAF_PREFIX.len() + 8);
    key.extend_from_slice(LEAF_PREFIX);
    key.extend_from_slice(&index.to_be_bytes());
    key
}

/// Build the sled key for a resume token for the given connection ID.
fn resume_token_key(connection_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(RESUME_TOKEN_PREFIX.len() + connection_id.len());
    key.extend_from_slice(RESUME_TOKEN_PREFIX);
    key.extend_from_slice(connection_id.as_bytes());
    key
}

/// Build the sled key for an IPFS CID for the given epoch number.
fn ipfs_cid_key(epoch_number: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(IPFS_CID_PREFIX.len() + 8);
    key.extend_from_slice(IPFS_CID_PREFIX);
    key.extend_from_slice(&epoch_number.to_be_bytes());
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::leaf_from_payload;

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nosqlbuddy-sled-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sled_store_starts_empty() {
        let dir = tempdir();
        let store = SledTreeStore::open(&dir).unwrap();
        assert_eq!(store.leaf_count().unwrap(), 0);
        let loaded = store.load_tree().unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn sled_store_saves_and_loads_leaves() {
        let dir = tempdir();
        let db_path = dir.join("db");
        {
            let store = SledTreeStore::open(&db_path).unwrap();

            let leaf1 = leaf_from_payload("insert", "db", "col", r#"{"a":1}"#);
            let leaf2 = leaf_from_payload("insert", "db", "col", r#"{"a":2}"#);

            // Save two leaves. We need the root after each insert.
            let mut tree = AuditMerkleTree::with_height(20).unwrap();
            tree.insert(leaf1);
            let root1 = tree.root().unwrap();
            store.save_leaf(0, leaf1, root1).unwrap();

            tree.insert(leaf2);
            let root2 = tree.root().unwrap();
            store.save_leaf(1, leaf2, root2).unwrap();

            // Load from sled (same session).
            let (mut loaded_tree, loaded_root) = store.load_tree().unwrap().unwrap();
            assert_eq!(loaded_tree.leaf_count(), 2);
            assert_eq!(loaded_tree.root().unwrap(), root2);
            assert_eq!(loaded_root, hex::encode(&root2.into_bigint().to_bytes_be()));
        }
        // Clean up: remove the temp dir after the test.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_persists_across_reopen() {
        let dir = tempdir();
        let db_path = dir.join("db");

        // First session: save a leaf.
        {
            let store = SledTreeStore::open(&db_path).unwrap();
            let leaf = leaf_from_payload("insert", "db", "col", r#"{"x":1}"#);
            let mut tree = AuditMerkleTree::with_height(20).unwrap();
            tree.insert(leaf);
            let root = tree.root().unwrap();
            store.save_leaf(0, leaf, root).unwrap();
            assert_eq!(store.leaf_count().unwrap(), 1);
            // Explicitly drop to close the sled db and release the file lock.
            drop(store);
        }

        // Give sled a moment to release the file lock.
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Second session: reopen and verify.
        {
            let store = SledTreeStore::open(&db_path).unwrap();
            assert_eq!(store.leaf_count().unwrap(), 1);
            let (tree, _root_hex) = store.load_tree().unwrap().unwrap();
            assert_eq!(tree.leaf_count(), 1);
            drop(store);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_clear_resets_state() {
        let dir = tempdir();
        let db_path = dir.join("db");
        let store = SledTreeStore::open(&db_path).unwrap();

        let leaf = leaf_from_payload("insert", "db", "col", "{}");
        let mut tree = AuditMerkleTree::with_height(20).unwrap();
        tree.insert(leaf);
        let root = tree.root().unwrap();
        store.save_leaf(0, leaf, root).unwrap();
        assert_eq!(store.leaf_count().unwrap(), 1);

        store.clear().unwrap();
        assert_eq!(store.leaf_count().unwrap(), 0);
        assert!(store.load_tree().unwrap().is_none());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_resume_token_round_trip() {
        let dir = tempdir();
        let db_path = dir.join("db");
        let store = SledTreeStore::open(&db_path).unwrap();

        // No token initially.
        assert!(store.load_resume_token("conn1").unwrap().is_none());

        // Save a token.
        let token_json = r#"{"_data":"82661234567890abcdef"}"#;
        store.save_resume_token("conn1", token_json).unwrap();

        // Load it back.
        let loaded = store.load_resume_token("conn1").unwrap();
        assert_eq!(loaded.as_deref(), Some(token_json));

        // Different connection has no token.
        assert!(store.load_resume_token("conn2").unwrap().is_none());

        // Clear it.
        store.clear_resume_token("conn1").unwrap();
        assert!(store.load_resume_token("conn1").unwrap().is_none());

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_resume_token_persists_across_reopen() {
        let dir = tempdir();
        let db_path = dir.join("db");

        {
            let store = SledTreeStore::open(&db_path).unwrap();
            store
                .save_resume_token("conn-a", r#"{"_data":"token-a"}"#)
                .unwrap();
            drop(store);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));

        {
            let store = SledTreeStore::open(&db_path).unwrap();
            let loaded = store.load_resume_token("conn-a").unwrap();
            assert_eq!(loaded.as_deref(), Some(r#"{"_data":"token-a"}"#));
            drop(store);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_resume_token_isolated_per_connection() {
        let dir = tempdir();
        let db_path = dir.join("db");
        let store = SledTreeStore::open(&db_path).unwrap();

        store
            .save_resume_token("conn1", r#"{"_data":"token-1"}"#)
            .unwrap();
        store
            .save_resume_token("conn2", r#"{"_data":"token-2"}"#)
            .unwrap();

        assert_eq!(
            store.load_resume_token("conn1").unwrap().as_deref(),
            Some(r#"{"_data":"token-1"}"#)
        );
        assert_eq!(
            store.load_resume_token("conn2").unwrap().as_deref(),
            Some(r#"{"_data":"token-2"}"#)
        );

        // Clearing one doesn't affect the other.
        store.clear_resume_token("conn1").unwrap();
        assert!(store.load_resume_token("conn1").unwrap().is_none());
        assert_eq!(
            store.load_resume_token("conn2").unwrap().as_deref(),
            Some(r#"{"_data":"token-2"}"#)
        );

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_ipfs_cid_round_trip() {
        let dir = tempdir();
        let db_path = dir.join("db");
        let store = SledTreeStore::open(&db_path).unwrap();

        // No CID initially.
        assert!(store.load_ipfs_cid(0).unwrap().is_none());

        // Save a CID.
        store.save_ipfs_cid(0, "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi").unwrap();

        // Load it back.
        let loaded = store.load_ipfs_cid(0).unwrap();
        assert_eq!(
            loaded.as_deref(),
            Some("bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi")
        );

        // Different epoch has no CID.
        assert!(store.load_ipfs_cid(1).unwrap().is_none());

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sled_store_ipfs_cid_persists_across_reopen() {
        let dir = tempdir();
        let db_path = dir.join("db");

        {
            let store = SledTreeStore::open(&db_path).unwrap();
            store.save_ipfs_cid(5, "bafy123").unwrap();
            drop(store);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));

        {
            let store = SledTreeStore::open(&db_path).unwrap();
            let loaded = store.load_ipfs_cid(5).unwrap();
            assert_eq!(loaded.as_deref(), Some("bafy123"));
            drop(store);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
