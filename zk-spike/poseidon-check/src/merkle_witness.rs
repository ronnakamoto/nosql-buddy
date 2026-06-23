use ark_bn254::Fr;
use ark_ff::{AdditiveGroup, PrimeField};
use light_poseidon::{Poseidon, PoseidonHasher};

/// Build a Merkle tree path for a leaf, optimized to avoid computing the
/// full 2^20 tree. We only compute nodes along the authentication path,
/// using precomputed zero-subtree hashes for padding.
///
/// Tree: binary, depth 20, Poseidon(2) hash (t=3, matching circomlib).

fn poseidon_hash(p: &mut Poseidon<Fr>, left: Fr, right: Fr) -> Fr {
    p.hash(&[left, right]).expect("poseidon hash failed")
}

/// Precompute the hash of an all-zero subtree of each height.
/// zero_hashes[h] = hash of a subtree with 2^h zero leaves.
/// zero_hashes[0] = Fr::ZERO (a zero leaf)
/// zero_hashes[h] = Poseidon(zero_hashes[h-1], zero_hashes[h-1])
fn compute_zero_hashes(p: &mut Poseidon<Fr>, height: usize) -> Vec<Fr> {
    let mut zero_hashes = vec![Fr::ZERO; height + 1];
    for h in 1..=height {
        zero_hashes[h] = poseidon_hash(p, zero_hashes[h - 1], zero_hashes[h - 1]);
    }
    zero_hashes
}

fn main() {
    let mut poseidon = Poseidon::<Fr>::new_circom(2).expect("failed to create Poseidon");

    // Test: 4 leaves at indices 0-3, values 1-4.
    let leaves: Vec<(usize, Fr)> = (1..=4u64)
        .enumerate()
        .map(|(i, v)| (i, Fr::from(v)))
        .collect();
    let height: usize = 20;

    // Precompute zero-subtree hashes.
    let zero_hashes = compute_zero_hashes(&mut poseidon, height);

    // Build a map of non-zero leaves.
    let mut leaf_map: std::collections::HashMap<usize, Fr> = std::collections::HashMap::new();
    for (idx, val) in &leaves {
        leaf_map.insert(*idx, *val);
    }

    // Compute inclusion proof for leaf at index 2 (value 3).
    let leaf_index: usize = 2;
    let leaf_value = leaf_map[&leaf_index];

    // For each level, compute the sibling hash.
    // At level 0, siblings are either other leaves or zero.
    // At higher levels, if both children are zero, the parent is zero_hashes[level+1].
    let mut path_elements: Vec<Fr> = Vec::with_capacity(height);
    let mut path_indices: Vec<u64> = Vec::with_capacity(height);

    // We need to compute the hash at each level for the current node and its sibling.
    // current_hash starts as the leaf value.
    let mut current_hash = leaf_value;
    let mut idx = leaf_index;

    for level in 0..height {
        let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
        let is_left = idx % 2 == 0;

        // Compute sibling hash.
        let sibling_hash = if level == 0 {
            // At leaf level, check if sibling is a real leaf or zero.
            *leaf_map.get(&sibling_idx).unwrap_or(&Fr::ZERO)
        } else {
            // At higher levels, sibling_idx is the node index at this level;
            // convert to leaf index by shifting left by `level` bits.
            let sibling_leaf_idx = sibling_idx << level;
            compute_subtree_hash(&mut poseidon, &leaf_map, sibling_leaf_idx, level, &zero_hashes)
        };

        path_elements.push(sibling_hash);
        path_indices.push(if is_left { 0 } else { 1 });

        // Compute current hash for next level.
        let (left, right) = if is_left {
            (current_hash, sibling_hash)
        } else {
            (sibling_hash, current_hash)
        };
        current_hash = poseidon_hash(&mut poseidon, left, right);

        idx /= 2;
    }

    let root = current_hash;
    println!("Merkle root: {}", root);
    println!("Leaf index: {}", leaf_index);
    println!("Leaf value: {}", leaf_value);
    println!("Path indices (first 5): {:?}", &path_indices[..5]);
    println!("Path elements (first 5):");
    for (i, pe) in path_elements.iter().take(5).enumerate() {
        println!("  [{}] = {}", i, pe);
    }

    // Build the Circom input JSON.
    let path_elements_json: Vec<String> = path_elements.iter().map(|f| f.to_string()).collect();
    let path_indices_json: Vec<String> = path_indices.iter().map(|i| i.to_string()).collect();

    let input = serde_json::json!({
        "leaf": leaf_value.to_string(),
        "pathElements": path_elements_json,
        "pathIndices": path_indices_json,
    });

    let json_str = serde_json::to_string_pretty(&input).unwrap();
    println!("\n=== Circom input JSON ===");
    println!("{}", json_str);

    std::fs::write("merkle_input.json", &json_str).expect("failed to write input JSON");
    println!("\nWritten to merkle_input.json");
    println!("Expected root (public output): {}", root);
    std::fs::write("merkle_root.txt", root.to_string()).expect("failed to write root");
}

/// Compute the hash of a subtree rooted at the given index at the given level.
/// `leaf_idx` is the index at the leaf level (level 0).
/// `level` is the level of the subtree root (0 = leaf, 1 = parent of 2 leaves, etc.)
/// The subtree covers leaves [leaf_idx, leaf_idx + 2^level).
fn compute_subtree_hash(
    p: &mut Poseidon<Fr>,
    leaf_map: &std::collections::HashMap<usize, Fr>,
    leaf_idx: usize,
    level: usize,
    zero_hashes: &[Fr],
) -> Fr {
    if level == 0 {
        return *leaf_map.get(&leaf_idx).unwrap_or(&Fr::ZERO);
    }

    // Check if any non-zero leaves exist in this subtree's range.
    let range_start = leaf_idx;
    let range_end = leaf_idx + (1 << level);
    let has_nonzero = leaf_map.keys().any(|&k| k >= range_start && k < range_end);

    if !has_nonzero {
        return zero_hashes[level];
    }

    // Recursively compute left and right children.
    let left = compute_subtree_hash(p, leaf_map, leaf_idx, level - 1, zero_hashes);
    let right = compute_subtree_hash(
        p,
        leaf_map,
        leaf_idx + (1 << (level - 1)),
        level - 1,
        zero_hashes,
    );
    poseidon_hash(p, left, right)
}
