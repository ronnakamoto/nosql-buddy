pragma circom 2.2.2;

include "node_modules/circomlib/circuits/poseidon.circom";

// ZK-AuditDB: Merkle inclusion proof circuit.
//
// Proves knowledge of a leaf and its authentication path in a binary Merkle
// tree whose root is public (committed on-chain via Soroban).
//
// Tree properties:
//   - Hash: Poseidon(2) → t=3 (2 inputs + 1 capacity), matching
//     rs-merkle-tree's PoseidonHasher (BN254 Circom T3).
//   - Node = Poseidon(left_child, right_child).
//   - Leaf = field element (audit entry hash, computed off-circuit).
//
// Public inputs:
//   - root: the Merkle tree root (committed on-chain).
//
// Private inputs:
//   - leaf: the leaf value being proven included.
//   - pathElements[height]: sibling hashes at each level.
//   - pathIndices[height]: direction bit (0 = leaf is left child, 1 = right).
//
// The direction bits are constrained to {0, 1} for soundness.
template MerkleInclusion(height) {
    signal input leaf;
    signal input pathElements[height];
    signal input pathIndices[height];
    signal output root;

    component hashers[height];

    // Intermediate signals for quadratic-safe constraint decomposition.
    signal levelHash[height + 1];
    signal lhTimesIdx[height];   // levelHash[i] * pathIndices[i]
    signal peTimesIdx[height];   // pathElements[i] * pathIndices[i]
    signal left[height];
    signal right[height];

    // Start from the leaf.
    levelHash[0] <== leaf;

    for (var i = 0; i < height; i++) {
        // Constrain direction bit to {0, 1}.
        pathIndices[i] * (pathIndices[i] - 1) === 0;

        // Break products into separate quadratic constraints.
        lhTimesIdx[i] <== levelHash[i] * pathIndices[i];
        peTimesIdx[i] <== pathElements[i] * pathIndices[i];

        // left  = levelHash * (1 - idx) + pathElements * idx
        //       = levelHash - lhTimesIdx + peTimesIdx
        left[i] <== levelHash[i] - lhTimesIdx[i] + peTimesIdx[i];

        // right = levelHash * idx + pathElements * (1 - idx)
        //       = lhTimesIdx + pathElements - peTimesIdx
        right[i] <== lhTimesIdx[i] + pathElements[i] - peTimesIdx[i];

        // Hash the pair.
        hashers[i] = Poseidon(2);
        hashers[i].inputs[0] <== left[i];
        hashers[i].inputs[1] <== right[i];

        levelHash[i + 1] <== hashers[i].out;
    }

    // The computed root is the public output.
    root <== levelHash[height];
}

// Main circuit: 20-level Merkle tree (supports up to 2^20 = ~1M entries).
// The depth is a compile-time constant; change and recompile for different sizes.
component main = MerkleInclusion(20);
