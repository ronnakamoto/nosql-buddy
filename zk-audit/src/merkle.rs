//! Poseidon Merkle tree for audit log entries.
//!
//! Uses `light-poseidon::new_circom(2)` (t=3) to match the Circom circuit's
//! `Poseidon(2)` hash. The tree is binary, with leaves at level 0 and the
//! root at level `height`.

use ark_bn254::Fr;
use ark_ff::AdditiveGroup;
use light_poseidon::{Poseidon, PoseidonHasher};

use crate::error::{ZkAuditError, ZkAuditResult};

/// Default tree height: 20 levels (supports up to 2^20 = ~1M entries).
pub const DEFAULT_HEIGHT: usize = 20;

/// A Poseidon Merkle tree for audit log entries.
pub struct AuditMerkleTree {
    height: usize,
    poseidon: Poseidon<Fr>,
    /// Non-zero leaves: (index, value) pairs.
    leaves: Vec<(usize, Fr)>,
    /// Precomputed hash of an all-zero subtree at each level.
    zero_hashes: Vec<Fr>,
}

/// An inclusion proof for a specific leaf.
#[derive(Debug, Clone)]
pub struct InclusionProof {
    /// The leaf value being proven included.
    pub leaf: Fr,
    /// The leaf's index in the tree (0-based).
    pub leaf_index: usize,
    /// Sibling hashes at each level (bottom to top).
    pub path_elements: Vec<Fr>,
    /// Direction bits: 0 = leaf is left child, 1 = right.
    pub path_indices: Vec<u64>,
    /// The computed Merkle root.
    pub root: Fr,
}

impl AuditMerkleTree {
    /// Create a new Merkle tree with the default height (20).
    pub fn new() -> ZkAuditResult<Self> {
        Self::with_height(DEFAULT_HEIGHT)
    }

    /// Create a new Merkle tree with a custom height.
    pub fn with_height(height: usize) -> ZkAuditResult<Self> {
        let mut poseidon = Poseidon::<Fr>::new_circom(2).map_err(|e| {
            ZkAuditError::MerkleTree(format!("failed to create Poseidon hasher: {}", e))
        })?;
        let zero_hashes = compute_zero_hashes(&mut poseidon, height);
        Ok(Self {
            height,
            poseidon,
            leaves: Vec::new(),
            zero_hashes,
        })
    }

    /// Insert a leaf at the next available index.
    pub fn insert(&mut self, value: Fr) -> usize {
        let index = self.leaves.len();
        self.leaves.push((index, value));
        index
    }

    /// Insert a leaf at a specific index.
    pub fn insert_at(&mut self, index: usize, value: Fr) {
        self.leaves.push((index, value));
    }

    /// Compute the Merkle root from all inserted leaves.
    pub fn root(&mut self) -> ZkAuditResult<Fr> {
        if self.leaves.is_empty() {
            return Ok(self.zero_hashes[self.height]);
        }

        // Build a lookup map for leaves.
        let mut leaf_map: std::collections::HashMap<usize, Fr> =
            std::collections::HashMap::new();
        for &(idx, val) in &self.leaves {
            leaf_map.insert(idx, val);
        }

        compute_subtree_hash(
            &mut self.poseidon,
            &leaf_map,
            0,
            self.height,
            &self.zero_hashes,
        )
    }

    /// Generate an inclusion proof for the leaf at the given index.
    pub fn prove_inclusion(&mut self, leaf_index: usize) -> ZkAuditResult<InclusionProof> {
        let leaf_map: std::collections::HashMap<usize, Fr> = self
            .leaves
            .iter()
            .cloned()
            .collect();

        let leaf = *leaf_map
            .get(&leaf_index)
            .ok_or_else(|| ZkAuditError::MerkleTree(format!("no leaf at index {}", leaf_index)))?;

        let mut path_elements = Vec::with_capacity(self.height);
        let mut path_indices = Vec::with_capacity(self.height);

        let mut current_hash = leaf;
        let mut idx = leaf_index;

        for level in 0..self.height {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            let is_left = idx % 2 == 0;

            let sibling_hash = if level == 0 {
                *leaf_map.get(&sibling_idx).unwrap_or(&Fr::ZERO)
            } else {
                // sibling_idx is the node index at this level; convert to
                // leaf index by shifting left by `level` bits.
                let sibling_leaf_idx = sibling_idx << level;
                compute_subtree_hash(
                    &mut self.poseidon,
                    &leaf_map,
                    sibling_leaf_idx,
                    level,
                    &self.zero_hashes,
                )?
            };

            path_elements.push(sibling_hash);
            path_indices.push(if is_left { 0 } else { 1 });

            let (left, right) = if is_left {
                (current_hash, sibling_hash)
            } else {
                (sibling_hash, current_hash)
            };
            current_hash = self
                .poseidon
                .hash(&[left, right])
                .map_err(|e| ZkAuditError::MerkleTree(format!("Poseidon hash failed: {}", e)))?;

            idx /= 2;
        }

        Ok(InclusionProof {
            leaf,
            leaf_index,
            path_elements,
            path_indices,
            root: current_hash,
        })
    }

    /// Convert an inclusion proof to the Circom input JSON format.
    pub fn to_circom_input(proof: &InclusionProof) -> serde_json::Value {
        serde_json::json!({
            "leaf": proof.leaf.to_string(),
            "pathElements": proof.path_elements.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
            "pathIndices": proof.path_indices.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
        })
    }

    /// Number of leaves inserted.
    pub fn leaf_count(&self) -> usize {
        self.leaves.len()
    }
}

impl Default for AuditMerkleTree {
    fn default() -> Self {
        Self::new().expect("failed to create default Merkle tree: Poseidon init failed")
    }
}

/// Compute the hash of an all-zero subtree at each height.
/// zero_hashes[0] = Fr::ZERO (a zero leaf)
/// zero_hashes[h] = Poseidon(zero_hashes[h-1], zero_hashes[h-1])
fn compute_zero_hashes(p: &mut Poseidon<Fr>, height: usize) -> Vec<Fr> {
    let mut zero_hashes = vec![Fr::ZERO; height + 1];
    for h in 1..=height {
        zero_hashes[h] = p
            .hash(&[zero_hashes[h - 1], zero_hashes[h - 1]])
            .expect("zero hash computation failed");
    }
    zero_hashes
}

/// Recursively compute the hash of a subtree rooted at the given leaf index
/// at the given level. Uses zero_hashes for padding when no leaves exist
/// in a subtree range.
fn compute_subtree_hash(
    p: &mut Poseidon<Fr>,
    leaf_map: &std::collections::HashMap<usize, Fr>,
    leaf_idx: usize,
    level: usize,
    zero_hashes: &[Fr],
) -> ZkAuditResult<Fr> {
    if level == 0 {
        return Ok(*leaf_map.get(&leaf_idx).unwrap_or(&Fr::ZERO));
    }

    let range_start = leaf_idx;
    let range_end = leaf_idx + (1 << level);
    let has_nonzero = leaf_map.keys().any(|&k| k >= range_start && k < range_end);

    if !has_nonzero {
        return Ok(zero_hashes[level]);
    }

    let left = compute_subtree_hash(p, leaf_map, leaf_idx, level - 1, zero_hashes)?;
    let right = compute_subtree_hash(
        p,
        leaf_map,
        leaf_idx + (1 << (level - 1)),
        level - 1,
        zero_hashes,
    )?;
    p.hash(&[left, right])
        .map_err(|e| ZkAuditError::MerkleTree(format!("subtree hash failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tree_root_is_zero_hash() {
        let mut tree = AuditMerkleTree::with_height(4).unwrap();
        let root = tree.root().unwrap();
        // Root of an empty tree is the zero hash at the top level.
        assert_eq!(root, tree.zero_hashes[4]);
    }

    #[test]
    fn test_single_leaf_root() {
        let mut tree = AuditMerkleTree::with_height(4).unwrap();
        tree.insert(Fr::from(42u64));
        let root = tree.root().unwrap();
        // Root should not be zero.
        assert_ne!(root, Fr::ZERO);
    }

    #[test]
    fn test_inclusion_proof_verifies_root() {
        let mut tree = AuditMerkleTree::with_height(4).unwrap();
        tree.insert(Fr::from(1u64));
        tree.insert(Fr::from(2u64));
        tree.insert(Fr::from(3u64));
        tree.insert(Fr::from(4u64));

        let root = tree.root().unwrap();
        let proof = tree.prove_inclusion(2).unwrap();

        // The proof's root must match the tree's root.
        assert_eq!(proof.root, root);
        assert_eq!(proof.leaf, Fr::from(3u64));
        assert_eq!(proof.leaf_index, 2);
        assert_eq!(proof.path_elements.len(), 4);
        assert_eq!(proof.path_indices.len(), 4);
    }

    #[test]
    fn test_known_root_matches_spike() {
        // 4 leaves (1,2,3,4), depth 20, proving inclusion of leaf at index 2 (value 3).
        // Root computed with the corrected sibling-index logic (leaf-index shifted by level).
        let mut tree = AuditMerkleTree::with_height(20).unwrap();
        tree.insert(Fr::from(1u64));
        tree.insert(Fr::from(2u64));
        tree.insert(Fr::from(3u64));
        tree.insert(Fr::from(4u64));

        let root = tree.root().unwrap();
        let expected = "4049438903814075631061804710736864908079133440291667789166416441530877358393";
        assert_eq!(root.to_string(), expected);
    }
}
