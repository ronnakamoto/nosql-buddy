use std::fs::{self, File};

use ark_bn254::Bn254;
use ark_circom::circom::{CircomCircuit, R1CSFile};
use ark_ff::{BigInteger, PrimeField};
use ark_groth16::{Groth16, prepare_verifying_key};
use ark_serialize::CanonicalSerialize;

type Fr = ark_bn254::Fr;

/// Load the R1CS file from circom compilation.
fn load_r1cs(path: &str) -> CircomCircuit<Fr> {
    let mut file = File::open(path).expect("failed to open R1CS file");
    let r1cs_file = R1CSFile::<Fr>::new(&mut file).expect("failed to parse R1CS");
    let mut r1cs: ark_circom::circom::R1CS<Fr> = r1cs_file.into();

    // The wire_mapping from Circom R1CS uses 1-indexed labels, but the snarkjs
    // witness array is 0-indexed. Setting wire_mapping to None makes ark-circom
    // use the witness array directly (w[i]), which matches the snarkjs order.
    r1cs.wire_mapping = None;

    CircomCircuit {
        r1cs,
        witness: None,
    }
}

/// Read the witness from a snarkjs-generated JSON file.
/// The JSON is an array of decimal strings: ["1", "out", "in0", "in1", ...]
fn load_witness(path: &str) -> Vec<Fr> {
    let data = fs::read_to_string(path).expect("failed to read witness file");
    let vals: serde_json::Value = serde_json::from_str(&data).expect("failed to parse witness JSON");
    let arr = vals.as_array().expect("witness must be an array");
    arr.iter()
        .map(|v| {
            let s = v.as_str().expect("witness elements must be strings");
            let big = num_bigint::BigInt::parse_bytes(s.as_bytes(), 10)
                .expect("failed to parse witness element as decimal");
            let big = big.to_biguint().expect("witness element must be non-negative");
            Fr::from_be_bytes_mod_order(&big.to_bytes_be())
        })
        .collect()
}

/// Serialize a field element to 32 big-endian bytes (owned array).
/// Uses BigInteger::to_bytes_be() for Ethereum/Soroban BN254 compatibility.
/// (arkworks' CanonicalSerialize uses little-endian with flags — wrong for Soroban.)
fn fq_to_bytes<F: PrimeField>(f: &F) -> [u8; 32] {
    let big = f.into_bigint();
    let bytes = big.to_bytes_be();
    let mut arr = [0u8; 32];
    let len = bytes.len();
    arr[32 - len..].copy_from_slice(&bytes);
    arr
}

/// Serialize a G1 affine point to Soroban BN254 format:
/// be_bytes(X) || be_bytes(Y) = 64 bytes → hex string
fn g1_to_hex(p: &ark_bn254::G1Affine) -> String {
    let x = fq_to_bytes(&p.x);
    let y = fq_to_bytes(&p.y);
    let mut result = [0u8; 64];
    result[..32].copy_from_slice(&x);
    result[32..].copy_from_slice(&y);
    hex::encode(result)
}

/// Serialize a G2 affine point to Soroban BN254 format:
/// be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0) = 128 bytes → hex string
fn g2_to_hex(p: &ark_bn254::G2Affine) -> String {
    // arkworks Fq2 is stored as (c0, c1) where c0 is real, c1 is imaginary.
    // Soroban BN254 G2 format: X_c1 || X_c0 || Y_c1 || Y_c0
    let x_c1 = fq_to_bytes(&p.x.c1);
    let x_c0 = fq_to_bytes(&p.x.c0);
    let y_c1 = fq_to_bytes(&p.y.c1);
    let y_c0 = fq_to_bytes(&p.y.c0);

    let mut result = [0u8; 128];
    result[..32].copy_from_slice(&x_c1);
    result[32..64].copy_from_slice(&x_c0);
    result[64..96].copy_from_slice(&y_c1);
    result[96..].copy_from_slice(&y_c0);
    hex::encode(result)
}

/// Serialize an Fr element to 32-byte big-endian hex.
fn fr_to_hex(f: &Fr) -> String {
    let big = f.into_bigint();
    let bytes = big.to_bytes_be();
    let mut result = [0u8; 32];
    let len = bytes.len();
    result[32 - len..].copy_from_slice(&bytes);
    hex::encode(result)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let circuit_name = args.get(1).map(|s| s.as_str()).unwrap_or("multiplier2");
    let input_path = args.get(2).map(|s| s.as_str()).unwrap_or("../circuits/multiplier_input.json");

    println!("=== Rust-Native Groth16 Proof Generation (ark-circom + ark-groth16) ===");
    println!("   Circuit: {}\n", circuit_name);

    let build_dir = "../circuits/build";
    let r1cs_path = format!("../circuits/build/{}.r1cs", circuit_name);
    let wasm_path = format!("../circuits/build/{}_js/{}.wasm", circuit_name, circuit_name);
    let witness_wtns = &format!("{}/{}_witness.wtns", build_dir, circuit_name);
    let witness_json = &format!("{}/{}_witness.json", build_dir, circuit_name);

    // Step 1: Load R1CS
    println!("1. Loading R1CS from {}...", r1cs_path);
    let mut circuit = load_r1cs(&r1cs_path);
    println!("   R1CS loaded: {} constraints, {} inputs, {} aux",
             circuit.r1cs.constraints.len(),
             circuit.r1cs.num_inputs,
             circuit.r1cs.num_aux);

    // Step 2: Generate witness via snarkjs (if not already present)
    println!("\n2. Loading witness...");
    if !std::path::Path::new(witness_json).exists() {
        println!("   Witness JSON not found, generating with snarkjs...");
        let status = std::process::Command::new("snarkjs")
            .args(["wtns", "calculate", &wasm_path, input_path, witness_wtns])
            .status()
            .expect("failed to run snarkjs wtns calculate");
        assert!(status.success(), "snarkjs witness generation failed");

        let status = std::process::Command::new("snarkjs")
            .args(["wtns", "export", "json", witness_wtns, witness_json])
            .status()
            .expect("failed to run snarkjs wtns export json");
        assert!(status.success(), "snarkjs witness export failed");
    }

    let witness = load_witness(witness_json);
    println!("   Witness loaded: {} elements", witness.len());
    println!("   witness[0] (const): {}", witness[0]);
    println!("   witness[1] (out):   {}", witness[1]);

    circuit.witness = Some(witness);

    // Step 3: Generate proving/verifying keys (mock trusted setup)
    println!("\n3. Generating Groth16 parameters (mock trusted setup)...");
    let rng = &mut rand::thread_rng();
    let params = Groth16::<Bn254>::generate_random_parameters_with_reduction(circuit.clone(), rng)
        .expect("failed to generate parameters");
    println!("   Parameters generated successfully");

    // Step 4: Generate proof
    println!("\n4. Generating Groth16 proof...");
    let proof = Groth16::<Bn254>::create_random_proof_with_reduction(circuit.clone(), &params, rng)
        .expect("failed to generate proof");
    println!("   Proof generated successfully");

    // Step 5: Verify locally with ark-groth16
    println!("\n5. Verifying proof locally with ark-groth16...");
    let public_inputs = circuit.get_public_inputs().expect("failed to get public inputs");
    println!("   Public inputs ({}):", public_inputs.len());
    for (i, pi) in public_inputs.iter().enumerate() {
        println!("     [{}] = {}", i, pi);
    }

    let pvk = prepare_verifying_key(&params.vk);
    let verified = Groth16::<Bn254>::verify_proof(&pvk, &proof, &public_inputs)
        .expect("failed to verify proof");
    println!("   Local verify: {}", if verified { "PASS" } else { "FAIL" });
    assert!(verified, "local verification failed");

    // Step 6: Serialize to Soroban BN254 format
    println!("\n6. Serializing proof + VK to Soroban BN254 format...");

    let proof_json = serde_json::json!({
        "a": g1_to_hex(&proof.a),
        "b": g2_to_hex(&proof.b),
        "c": g1_to_hex(&proof.c),
    });

    let vk_json = serde_json::json!({
        "alpha": g1_to_hex(&params.vk.alpha_g1),
        "beta": g2_to_hex(&params.vk.beta_g2),
        "gamma": g2_to_hex(&params.vk.gamma_g2),
        "delta": g2_to_hex(&params.vk.delta_g2),
        "ic": params.vk.gamma_abc_g1.iter().map(g1_to_hex).collect::<Vec<_>>(),
    });

    let pub_signals: Vec<String> = public_inputs.iter().map(|f| f.to_string()).collect();

    let output = serde_json::json!({
        "vk": vk_json,
        "proof": proof_json,
        "pub_signals": pub_signals,
    });

    let output_path = format!("rust_proof_args_{}.json", circuit_name);
    fs::write(&output_path, serde_json::to_string_pretty(&output).unwrap())
        .expect("failed to write output");
    println!("   Written to {}", output_path);
    println!("   proof.a (G1, 64 bytes): {}...{}", &proof_json["a"].as_str().unwrap()[..16], &proof_json["a"].as_str().unwrap()[120..]);
    println!("   proof.b (G2, 128 bytes): {}...{}", &proof_json["b"].as_str().unwrap()[..16], &proof_json["b"].as_str().unwrap()[240..]);
    println!("   pub_signals: {:?}", pub_signals);

    println!("\n=== Done. Submit to testnet with: ===");
    println!("stellar contract invoke --id CBNDLBF2B42P5ZKFKJCUODTLTHCOMY5IB7C3MN7RJ3RPXJ5BWCBII73C --source spike --network testnet -- verify_proof --vk '<vk_json>' --proof '<proof_json>' --pub_signals '<pub_signals>'");
}
