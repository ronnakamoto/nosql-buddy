pragma circom 2.2.2;

include "node_modules/circomlib/circuits/poseidon.circom";

// Poseidon hash with T=3 (2 inputs + 1 capacity), matching rs-merkle-tree's
// PoseidonHasher (BN254 Circom T3) for binary Merkle tree nodes.
// circomlib's Poseidon(nInputs) uses t = nInputs + 1, so Poseidon(2) => t=3.
// Computes Poseidon(inputs[0], inputs[1]) and exposes the hash.
template PoseidonHashT3() {
    signal input inputs[2];
    signal output out;

    component hasher = Poseidon(2);
    hasher.inputs[0] <== inputs[0];
    hasher.inputs[1] <== inputs[1];

    out <== hasher.out;
}

component main = PoseidonHashT3();
