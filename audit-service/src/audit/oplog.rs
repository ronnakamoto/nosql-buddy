//! Oplog reader and Merkle tree for completeness verification.
//!
//! This module reads MongoDB's oplog (`local.oplog.rs`) — the canonical,
//! mongod-maintained, replication-grade log of every write — and builds
//! a SHA-256 Merkle tree over canonicalized oplog entries.
//!
//! ## Why the oplog (not the change stream)
//!
//! The change stream is our application's *interpretation* of oplog
//! entries. The oplog is the *source of truth* — it's what mongod itself
//! maintains and what secondaries replicate from. By hashing the oplog
//! directly, we bind the audit log to the same ground truth that
//! MongoDB's own replication protocol uses.
//!
//! ## Majority-committed point (H4)
//!
//! The oplog contains entries that have not yet reached majority commit
//! and can be rolled back. We only hash entries up to the
//! `lastCommittedOpTime` to ensure we're committing to entries that are
//! durable and will not vanish.
//!
//! ## Merkle tree (M4)
//!
//! We use a SHA-256 binary Merkle tree (not a hash chain) so that
//! inclusion proofs work: we can prove a specific oplog entry is in the
//! committed tree without revealing the others. This is separate from
//! the Poseidon Merkle tree used for the audit log (which is ZK-
//! compatible). The oplog Merkle tree is the completeness layer; the
//! audit log Merkle tree is the integrity + ZK layer.
//!
//! ## Epoch ranges
//!
//! An epoch's oplog range is defined as a half-open interval
//! `[start_ts, end_ts)` on the `ts` field (BSON Timestamp), where both
//! boundaries are at or before the majority-committed point. The entry
//! count `M` and the boundary timestamps are part of the on-chain
//! commitment.

use bson::Document;
use futures_util::StreamExt;
use mongodb::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit::oplog_canon::hash_oplog_entry;
use crate::error::{AuditError, AuditResult};

/// A BSON Timestamp as stored in the oplog `ts` field.
/// Consists of `time` (seconds since epoch) and `increment` (counter
/// within the same second).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogTimestamp {
    pub time: u32,
    pub increment: u32,
}

impl OplogTimestamp {
    /// The zero timestamp (used as the start of time).
    pub fn zero() -> Self {
        Self {
            time: 0,
            increment: 0,
        }
    }

    /// Convert to a BSON Timestamp for querying.
    fn to_bson(&self) -> bson::Timestamp {
        bson::Timestamp {
            time: self.time,
            increment: self.increment,
        }
    }

    /// Convert to hex string for display.
    pub fn to_hex(&self) -> String {
        format!("{:08x}{:08x}", self.time, self.increment)
    }

    /// Parse from a BSON Timestamp.
    fn from_bson(ts: bson::Timestamp) -> Self {
        Self {
            time: ts.time,
            increment: ts.increment,
        }
    }

    /// Pack the timestamp into a single u64 for on-chain storage.
    ///
    /// The on-chain contract uses the same layout: `(time << 32) | increment`.
    pub fn pack_u64(&self) -> u64 {
        ((self.time as u64) << 32) | (self.increment as u64)
    }

    /// Unpack a u64 into an OplogTimestamp.
    pub fn unpack_u64(packed: u64) -> Self {
        Self {
            time: (packed >> 32) as u32,
            increment: (packed & 0xFFFFFFFF) as u32,
        }
    }
}

impl std::fmt::Display for OplogTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.time, self.increment)
    }
}

/// The result of reading and hashing an epoch's oplog range.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogRange {
    /// The epoch number this range belongs to.
    pub epoch_number: u64,
    /// The first oplog timestamp in the range (inclusive).
    pub start_ts: OplogTimestamp,
    /// The last oplog timestamp in the range (exclusive — entries with
    /// `ts < end_ts` are included).
    pub end_ts: OplogTimestamp,
    /// The number of oplog entries in the range.
    pub entry_count: u64,
    /// The SHA-256 Merkle root over the canonicalized oplog entries.
    pub oplog_merkle_root_hex: String,
    /// The majority-committed oplog timestamp at the time of reading
    /// (the end_ts is guaranteed to be <= this).
    pub majority_commit_ts: OplogTimestamp,
}

/// A SHA-256 binary Merkle tree over oplog entry hashes.
///
/// This is a simple, non-ZK Merkle tree. It uses SHA-256 (not Poseidon)
/// because the oplog hash is a commitment layer, not a ZK circuit input.
/// Inclusion proofs from this tree are plain SHA-256 Merkle proofs.
pub struct OplogMerkleTree {
    leaves: Vec<[u8; 32]>,
    /// All nodes, level by level. nodes[0] = leaves, nodes[1] = parents, etc.
    /// The root is nodes.last().last().
    nodes: Vec<Vec<[u8; 32]>>,
    root: [u8; 32],
}

impl OplogMerkleTree {
    /// Build a Merkle tree from a list of leaf hashes.
    pub fn from_leaves(leaves: Vec<[u8; 32]>) -> Self {
        if leaves.is_empty() {
            // Empty tree: root is the hash of nothing (SHA-256 of empty).
            let mut hasher = Sha256::new();
            hasher.update(b"empty_oplog_tree");
            let root: [u8; 32] = hasher.finalize().into();
            return Self {
                leaves,
                nodes: vec![],
                root,
            };
        }

        let mut nodes = vec![leaves.clone()];
        let mut current = leaves.clone();

        while current.len() > 1 {
            let mut next = Vec::with_capacity((current.len() + 1) / 2);
            let mut i = 0;
            while i < current.len() {
                let left = current[i];
                let right = if i + 1 < current.len() {
                    current[i + 1]
                } else {
                    // Odd node: duplicate it.
                    current[i]
                };
                let mut hasher = Sha256::new();
                hasher.update(left);
                hasher.update(right);
                let parent: [u8; 32] = hasher.finalize().into();
                next.push(parent);
                i += 2;
            }
            nodes.push(next.clone());
            current = next;
        }

        let root = current[0];
        Self {
            leaves,
            nodes,
            root,
        }
    }

    /// Get the Merkle root.
    pub fn root(&self) -> [u8; 32] {
        self.root
    }

    /// Get the Merkle root as hex.
    pub fn root_hex(&self) -> String {
        hex::encode(self.root)
    }

    /// Get the number of leaves.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }

    /// Generate a Merkle inclusion proof for the leaf at the given index.
    /// Returns the sibling hashes from the leaf level up to the root.
    pub fn prove_inclusion(&self, leaf_index: usize) -> Option<Vec<[u8; 32]>> {
        if leaf_index >= self.leaves.len() {
            return None;
        }
        if self.nodes.is_empty() {
            return Some(vec![]);
        }

        let mut proof = Vec::new();
        let mut idx = leaf_index;

        for level in 0..self.nodes.len() - 1 {
            let level_nodes = &self.nodes[level];
            // Sibling is at idx ^ 1 (XOR with 1 flips the last bit).
            let sibling_idx = idx ^ 1;
            if sibling_idx < level_nodes.len() {
                proof.push(level_nodes[sibling_idx]);
            } else {
                // Odd node at this level — sibling is itself (duplicated).
                proof.push(level_nodes[idx]);
            }
            idx /= 2;
        }

        Some(proof)
    }

    /// Verify a Merkle inclusion proof.
    pub fn verify_inclusion(
        leaf: [u8; 32],
        leaf_index: usize,
        proof: &[[u8; 32]],
        root: [u8; 32],
    ) -> bool {
        let mut current = leaf;
        let mut idx = leaf_index;

        for sibling in proof {
            let mut hasher = Sha256::new();
            if idx % 2 == 0 {
                // Current is left child.
                hasher.update(current);
                hasher.update(sibling);
            } else {
                // Current is right child.
                hasher.update(sibling);
                hasher.update(current);
            }
            current = hasher.finalize().into();
            idx /= 2;
        }

        current == root
    }
}

/// Query the majority-committed oplog timestamp from the replica set.
///
/// This is the point up to which all writes are durably replicated to
/// a majority of voting members. Entries beyond this point may be
/// rolled back. We only hash entries up to this timestamp (H4).
pub async fn get_majority_commit_ts(client: &Client) -> AuditResult<OplogTimestamp> {
    let result = client
        .database("admin")
        .run_command(bson::doc! { "hello": 1 })
        .await?;

    // The `lastCommittedOpTime` field (if present) contains {ts: Timestamp, t: Int64}.
    if let Some(bson::Bson::Document(op_time)) = result.get("lastCommittedOpTime") {
        if let Some(bson::Bson::Timestamp(ts)) = op_time.get("ts") {
            return Ok(OplogTimestamp::from_bson(*ts));
        }
    }

    // MongoDB 4.4+ returns the majority point under `lastWrite.majorityOpTime.ts`.
    if let Some(bson::Bson::Document(last_write)) = result.get("lastWrite") {
        if let Some(bson::Bson::Document(majority_op_time)) = last_write.get("majorityOpTime") {
            if let Some(bson::Bson::Timestamp(ts)) = majority_op_time.get("ts") {
                return Ok(OplogTimestamp::from_bson(*ts));
            }
        }
    }

    // lastCommittedOpTime/lastWrite are only present on replica set members. If we are
    // running against a standalone mongod, the caller must opt into standalone
    // mode explicitly; silently using wall-clock time as an oplog position is
    // wrong and would allow hashing entries that can be rolled back.
    Err(AuditError::Validation(
        "lastCommittedOpTime not found — connect to a replica set member or enable standalone mode".to_string(),
    ))
}

/// Read oplog entries in the range [start_ts, end_ts), ordered by ts.
///
/// Only entries with `op` in {i, u, d, c} are included — these are the
/// write operations. Command entries (e.g., no-op heartbeats) are
/// excluded because they don't represent user writes.
///
/// The entries are read from `local.oplog.rs` on the connected member.
/// For completeness verification, the caller should connect to the
/// **independent** replica member, not the operator's server (C1).
pub async fn read_oplog_range(
    client: &Client,
    start_ts: OplogTimestamp,
    end_ts: OplogTimestamp,
) -> AuditResult<Vec<Document>> {
    let oplog = client
        .database("local")
        .collection::<Document>("oplog.rs");

    let filter = bson::doc! {
        "ts": {
            "$gte": start_ts.to_bson(),
            "$lt": end_ts.to_bson(),
        },
        // Only write operations: insert, update, delete, command (e.g., create/drop).
        // Exclude 'n' (no-op) entries which are heartbeats.
        "op": { "$in": ["i", "u", "d", "c"] },
    };

    let options = mongodb::options::FindOptions::builder()
        .sort(bson::doc! { "ts": 1 }) // ascending by timestamp
        .batch_size(1000)
        .build();

    let mut cursor = oplog.find(filter).with_options(options).await?;

    let mut entries = Vec::new();
    while let Some(doc) = cursor.next().await {
        entries.push(doc?);
    }

    Ok(entries)
}

/// Compute the oplog Merkle root for a range of oplog entries.
///
/// This is the core completeness primitive:
/// 1. Read oplog entries in [start_ts, end_ts) from the connected member.
/// 2. Canonicalize each entry using the `oplog-hash-v1` spec.
/// 3. SHA-256 hash each canonicalized entry.
/// 4. Build a SHA-256 Merkle tree over the leaf hashes.
/// 5. Return the root + metadata.
///
/// Two independent parties reading the same oplog range from the same
/// replica set (under w:"majority") will produce the same root.
pub async fn compute_oplog_range_hash(
    client: &Client,
    epoch_number: u64,
    start_ts: OplogTimestamp,
    end_ts: OplogTimestamp,
) -> AuditResult<OplogRange> {
    // Ensure end_ts is at or before the majority-committed point (H4).
    let majority_ts = get_majority_commit_ts(client).await?;
    let safe_end_ts = if end_ts > majority_ts {
        tracing::warn!(
            "end_ts {} is beyond majority commit point {}; clamping",
            end_ts,
            majority_ts
        );
        majority_ts
    } else {
        end_ts
    };

    let entries = read_oplog_range(client, start_ts, safe_end_ts).await?;

    let leaf_hashes: Vec<[u8; 32]> = entries.iter().map(hash_oplog_entry).collect();
    let tree = OplogMerkleTree::from_leaves(leaf_hashes);

    Ok(OplogRange {
        epoch_number,
        start_ts,
        end_ts: safe_end_ts,
        entry_count: entries.len() as u64,
        oplog_merkle_root_hex: tree.root_hex(),
        majority_commit_ts: majority_ts,
    })
}

/// Read ALL oplog entries from the beginning up to the majority-committed
/// point. Used for the initial epoch (epoch 0) where there's no prior
/// start_ts.
pub async fn read_oplog_from_beginning(
    client: &Client,
    end_ts: OplogTimestamp,
) -> AuditResult<Vec<Document>> {
    let oplog = client
        .database("local")
        .collection::<Document>("oplog.rs");

    let filter = bson::doc! {
        "ts": { "$lt": end_ts.to_bson() },
        "op": { "$in": ["i", "u", "d", "c"] },
    };

    let options = mongodb::options::FindOptions::builder()
        .sort(bson::doc! { "ts": 1 })
        .batch_size(1000)
        .build();

    let mut cursor = oplog.find(filter).with_options(options).await?;

    let mut entries = Vec::new();
    while let Some(doc) = cursor.next().await {
        entries.push(doc?);
    }

    Ok(entries)
}

/// Get the latest oplog timestamp from the connected member.
/// This is the timestamp of the most recent oplog entry (not necessarily
/// majority-committed). Used to determine the current end of the oplog.
pub async fn get_latest_oplog_ts(client: &Client) -> AuditResult<OplogTimestamp> {
    let oplog = client
        .database("local")
        .collection::<Document>("oplog.rs");

    let options = mongodb::options::FindOptions::builder()
        .sort(bson::doc! { "ts": -1 }) // descending — get the latest
        .limit(1)
        .build();

    let mut cursor = oplog
        .find(bson::doc! { "op": { "$in": ["i", "u", "d", "c"] } })
        .with_options(options)
        .await?;

    if let Some(doc) = cursor.next().await {
        let doc = doc?;
        if let Some(bson::Bson::Timestamp(ts)) = doc.get("ts") {
            return Ok(OplogTimestamp::from_bson(*ts));
        }
    }

    Ok(OplogTimestamp::zero())
}

/// Build an oplog Merkle tree from a list of raw oplog entries.
/// This is used when the entries have already been read (e.g., from
/// a cached range) and we just need to compute the tree.
pub fn build_oplog_tree(entries: &[Document]) -> OplogMerkleTree {
    let leaf_hashes: Vec<[u8; 32]> = entries.iter().map(hash_oplog_entry).collect();
    OplogMerkleTree::from_leaves(leaf_hashes)
}

/// Compute the oplog Merkle root from a list of raw entries (without
/// reading from MongoDB). Useful for testing and for the auditor's
/// independent verification.
pub fn compute_oplog_root_hex(entries: &[Document]) -> String {
    build_oplog_tree(entries).root_hex()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn test_empty_tree_has_defined_root() {
        let tree = OplogMerkleTree::from_leaves(vec![]);
        // Empty tree should have a defined, non-zero root.
        assert_ne!(tree.root_hex(), hex::encode([0u8; 32]));
    }

    #[test]
    fn test_single_leaf_tree() {
        let leaf = [1u8; 32];
        let tree = OplogMerkleTree::from_leaves(vec![leaf]);
        // Single leaf: root = leaf (no hashing needed).
        assert_eq!(tree.root(), leaf);
        assert_eq!(tree.leaf_count(), 1);
    }

    #[test]
    fn test_two_leaf_tree() {
        let leaf0 = [1u8; 32];
        let leaf1 = [2u8; 32];
        let tree = OplogMerkleTree::from_leaves(vec![leaf0, leaf1]);

        let mut hasher = Sha256::new();
        hasher.update(leaf0);
        hasher.update(leaf1);
        let expected: [u8; 32] = hasher.finalize().into();

        assert_eq!(tree.root(), expected);
    }

    #[test]
    fn test_three_leaf_tree_odd_node_duplicated() {
        let leaf0 = [1u8; 32];
        let leaf1 = [2u8; 32];
        let leaf2 = [3u8; 32];
        let tree = OplogMerkleTree::from_leaves(vec![leaf0, leaf1, leaf2]);

        // Level 0: [leaf0, leaf1, leaf2]
        // Level 1: [H(leaf0, leaf1), H(leaf2, leaf2)]  (leaf2 duplicated)
        // Level 2: [H(H(leaf0, leaf1), H(leaf2, leaf2))]
        let mut h01 = Sha256::new();
        h01.update(leaf0);
        h01.update(leaf1);
        let n01: [u8; 32] = h01.finalize().into();

        let mut h22 = Sha256::new();
        h22.update(leaf2);
        h22.update(leaf2);
        let n22: [u8; 32] = h22.finalize().into();

        let mut h_root = Sha256::new();
        h_root.update(n01);
        h_root.update(n22);
        let expected: [u8; 32] = h_root.finalize().into();

        assert_eq!(tree.root(), expected);
    }

    #[test]
    fn test_inclusion_proof_single_leaf() {
        let leaf = [1u8; 32];
        let tree = OplogMerkleTree::from_leaves(vec![leaf]);
        let proof = tree.prove_inclusion(0).unwrap();
        assert!(proof.is_empty(), "single leaf has no siblings");
        assert!(OplogMerkleTree::verify_inclusion(leaf, 0, &proof, tree.root()));
    }

    #[test]
    fn test_inclusion_proof_two_leaves() {
        let leaf0 = [1u8; 32];
        let leaf1 = [2u8; 32];
        let tree = OplogMerkleTree::from_leaves(vec![leaf0, leaf1]);
        let root = tree.root();

        let proof0 = tree.prove_inclusion(0).unwrap();
        assert_eq!(proof0.len(), 1);
        assert_eq!(proof0[0], leaf1);
        assert!(OplogMerkleTree::verify_inclusion(leaf0, 0, &proof0, root));

        let proof1 = tree.prove_inclusion(1).unwrap();
        assert_eq!(proof1.len(), 1);
        assert_eq!(proof1[0], leaf0);
        assert!(OplogMerkleTree::verify_inclusion(leaf1, 1, &proof1, root));
    }

    #[test]
    fn test_inclusion_proof_three_leaves() {
        let leaves = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let tree = OplogMerkleTree::from_leaves(leaves.clone());
        let root = tree.root();

        for (i, &leaf) in leaves.iter().enumerate() {
            let proof = tree.prove_inclusion(i).unwrap();
            assert!(
                OplogMerkleTree::verify_inclusion(leaf, i, &proof, root),
                "inclusion proof for leaf {} should verify",
                i
            );
        }
    }

    #[test]
    fn test_inclusion_proof_rejects_wrong_leaf() {
        let leaves = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let tree = OplogMerkleTree::from_leaves(leaves);
        let root = tree.root();

        let proof = tree.prove_inclusion(0).unwrap();
        let wrong_leaf = [99u8; 32];
        assert!(
            !OplogMerkleTree::verify_inclusion(wrong_leaf, 0, &proof, root),
            "wrong leaf should not verify"
        );
    }

    #[test]
    fn test_inclusion_proof_rejects_wrong_index() {
        let leaves = vec![[1u8; 32], [2u8; 32], [3u8; 32]];
        let tree = OplogMerkleTree::from_leaves(leaves.clone());
        let root = tree.root();

        let proof = tree.prove_inclusion(0).unwrap();
        // Claim leaf0 is at index 1 (wrong).
        assert!(
            !OplogMerkleTree::verify_inclusion(leaves[0], 1, &proof, root),
            "wrong index should not verify"
        );
    }

    #[test]
    fn test_compute_oplog_root_hex_deterministic() {
        let entries = vec![
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 1 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 1i32 },
                "v": 2i32,
            },
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 2 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 2i32 },
                "v": 2i32,
            },
        ];

        let root1 = compute_oplog_root_hex(&entries);
        let root2 = compute_oplog_root_hex(&entries);
        assert_eq!(root1, root2, "same entries must produce same root");
    }

    #[test]
    fn test_compute_oplog_root_changes_on_omission() {
        let entries_full = vec![
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 1 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 1i32 },
                "v": 2i32,
            },
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 2 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 2i32 },
                "v": 2i32,
            },
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 3 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 3i32 },
                "v": 2i32,
            },
        ];

        // Omit the second entry (simulating omission).
        let entries_omitted = vec![
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 1 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 1i32 },
                "v": 2i32,
            },
            doc! {
                "ts": bson::Timestamp { time: 1000, increment: 3 },
                "op": "i",
                "ns": "db.coll",
                "o": doc! { "x": 3i32 },
                "v": 2i32,
            },
        ];

        let root_full = compute_oplog_root_hex(&entries_full);
        let root_omitted = compute_oplog_root_hex(&entries_omitted);
        assert_ne!(
            root_full, root_omitted,
            "omitting an entry must change the root — this is the completeness guarantee"
        );
    }

    #[test]
    fn test_oplog_timestamp_ordering() {
        let ts1 = OplogTimestamp { time: 1000, increment: 1 };
        let ts2 = OplogTimestamp { time: 1000, increment: 2 };
        let ts3 = OplogTimestamp { time: 1001, increment: 0 };

        assert!(ts1 < ts2, "same time, higher increment is greater");
        assert!(ts2 < ts3, "higher time is greater");
        assert!(ts1 < ts3);
    }

    #[test]
    fn test_oplog_timestamp_hex_roundtrip() {
        let ts = OplogTimestamp { time: 0x12345678, increment: 0x9abcdef0 };
        let hex = ts.to_hex();
        assert_eq!(hex, "123456789abcdef0");
    }
}
