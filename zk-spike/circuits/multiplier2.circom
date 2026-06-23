pragma circom 2.2.2;

// Minimal circuit: prove knowledge of a, b such that a * b = c (c public).
// This is the simplest non-trivial Groth16 circuit — used to de-risk the
// end-to-end Circom -> snarkjs -> Soroban BN254 verification path.
template Multiplier2() {
    signal input a;
    signal input b;
    signal output c;

    c <== a * b;
}

component main = Multiplier2();
