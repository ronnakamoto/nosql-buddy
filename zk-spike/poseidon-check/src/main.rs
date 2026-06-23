use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use light_poseidon::{Poseidon, PoseidonBytesHasher, PoseidonHasher};

/// Stage 4 verification: confirm that the Rust Poseidon (light-poseidon
/// new_circom(2), used by rs-merkle-tree's PoseidonHasher) produces the same
/// hash as circomlib's Poseidon(2) for the same inputs.
///
/// Circom side (circomlib Poseidon(2), t=3):
///   Poseidon(1, 2) = 7853200120776062878684798364095072458815029376092732009249414926327459813530
///
/// If these match, the off-chain Merkle tree (rs-merkle-tree) and the in-circuit
/// Poseidon are using identical parameters, and a ZK proof over the Merkle root
/// is sound.

// The expected hash from circomlib Poseidon(2) with inputs [1, 2].
const CIRCOM_POSEIDON_1_2: &str =
    "7853200120776062878684798364095072458815029376092732009249414926327459813530";

fn main() {
    println!("=== Poseidon Compatibility Check (Stage 4) ===\n");

    // Build the circom-compatible Poseidon hasher with 2 inputs (t=3).
    let mut poseidon = Poseidon::<Fr>::new_circom(2).expect("failed to create Poseidon");

    // Method 1: hash field elements directly.
    let inputs = vec![Fr::from(1u64), Fr::from(2u64)];
    let hash_fr = poseidon.hash(&inputs).expect("hash failed");
    let hash_fr_str = hash_fr.to_string();

    println!("Circom  Poseidon(1, 2) = {}", CIRCOM_POSEIDON_1_2);
    println!("Rust    Poseidon(1, 2) = {}  (field-element API)", hash_fr_str);
    println!("Match (field API):     {}", hash_fr_str == CIRCOM_POSEIDON_1_2);

    // Method 2: hash via bytes (big-endian), as rs-merkle-tree's
    // PoseidonHasher does internally (hash_bytes_be).
    let one_bytes = Fr::from(1u64).into_bigint().to_bytes_be();
    let two_bytes = Fr::from(2u64).into_bigint().to_bytes_be();
    let hash_bytes = poseidon
        .hash_bytes_be(&[&one_bytes, &two_bytes])
        .expect("hash_bytes failed");
    let hash_bytes_fr = Fr::from_be_bytes_mod_order(&hash_bytes);
    let hash_bytes_str = hash_bytes_fr.to_string();

    println!(
        "Rust    Poseidon(1, 2) = {}  (bytes-be API, as rs-merkle-tree uses)",
        hash_bytes_str
    );
    println!("Match (bytes API):     {}", hash_bytes_str == CIRCOM_POSEIDON_1_2);

    // Overall verdict.
    let field_ok = hash_fr_str == CIRCOM_POSEIDON_1_2;
    let bytes_ok = hash_bytes_str == CIRCOM_POSEIDON_1_2;

    println!("\n=== Verdict ===");
    if field_ok && bytes_ok {
        println!("PASS: light-poseidon new_circom(2) == circomlib Poseidon(2).");
        println!("rs-merkle-tree's PoseidonHasher is compatible with the Circom circuit.");
        std::process::exit(0);
    } else {
        println!("FAIL: Poseidon hash mismatch!");
        println!("  field API match: {}", field_ok);
        println!("  bytes API match: {}", bytes_ok);
        std::process::exit(1);
    }
}
