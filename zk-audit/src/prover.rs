//! Groth16 proof generation via ark-circom + ark-groth16.
//!
//! Loads the compiled Circom circuit (R1CS + WASM), computes the witness,
//! generates a Groth16 proof, and verifies it locally.

use ark_bn254::Bn254;
use ark_circom::circom::{CircomCircuit, R1CSFile};
use ark_groth16::{Groth16, prepare_verifying_key};
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};

use crate::error::{ZkAuditError, ZkAuditResult};
use crate::merkle::InclusionProof;
use crate::serialize;

pub type Fr = ark_bn254::Fr;

/// A Groth16 proof with its verifying key, ready for serialization.
pub struct Groth16Proof {
    /// The arkworks proof object.
    pub proof: ark_groth16::Proof<Bn254>,
    /// The verifying key.
    pub vk: ark_groth16::VerifyingKey<Bn254>,
    /// The public inputs (the Merkle root, as a field element).
    pub public_inputs: Vec<Fr>,
}

/// The verifying key for the Merkle inclusion circuit.
pub struct VerifyingKey {
    pub vk: ark_groth16::VerifyingKey<Bn254>,
}

/// The audit prover: loads the circuit and generates Groth16 proofs.
pub struct AuditProver {
    /// The loaded R1CS constraint system (without witness).
    circuit_template: CircomCircuit<Fr>,
    /// Path to the Circom WASM for witness calculation.
    wasm_path: String,
    /// Pre-generated proving key (from a Powers of Tau ceremony).
    /// When present, skips per-proof parameter generation.
    proving_key: Option<ark_groth16::ProvingKey<Bn254>>,
}

impl AuditProver {
    /// Create a new prover from compiled circuit artifacts.
    ///
    /// # Arguments
    /// * `r1cs_path` - Path to the compiled `.r1cs` file
    /// * `wasm_path` - Path to the compiled `_js/<circuit>.wasm` file
    pub fn new(r1cs_path: &str, wasm_path: &str) -> ZkAuditResult<Self> {
        let circuit_template = load_r1cs(r1cs_path)?;

        Ok(Self {
            circuit_template,
            wasm_path: wasm_path.to_string(),
            proving_key: None,
        })
    }

    /// Create a new prover with pre-generated proving key (from a ceremony).
    ///
    /// The proving key file must be in arkworks `CanonicalSerialize` format,
    /// as produced by the `zk-audit-ceremony` tool.
    pub fn with_proving_key(
        r1cs_path: &str,
        wasm_path: &str,
        proving_key_path: &str,
    ) -> ZkAuditResult<Self> {
        let circuit_template = load_r1cs(r1cs_path)?;
        let proving_key = load_proving_key(proving_key_path)?;

        Ok(Self {
            circuit_template,
            wasm_path: wasm_path.to_string(),
            proving_key: Some(proving_key),
        })
    }

    /// Generate a Groth16 proof for the given inclusion proof.
    ///
    /// If a pre-generated proving key was loaded via [`with_proving_key`],
    /// it is used instead of generating fresh parameters on every call.
    /// This is the realistic Powers of Tau path: the ceremony is run once,
    /// and the proving key is reused for every proof.
    pub fn prove(&self, proof: &InclusionProof) -> ZkAuditResult<Groth16Proof> {
        // Step 1: Compute the witness via ark-circom's WitnessCalculator (wasmer).
        let witness = self.compute_witness_wasmer(proof)?;

        // Step 2: Build the circuit with the witness.
        let mut circuit = self.circuit_template.clone();
        circuit.witness = Some(witness);

        let rng = &mut rand::thread_rng();

        // Step 3: Obtain Groth16 parameters.
        let (groth16_proof, vk, public_inputs) = if let Some(pk) = &self.proving_key {
            // Pre-generated proving key from ceremony — fast path.
            let proof = Groth16::<Bn254>::create_random_proof_with_reduction(
                circuit.clone(),
                pk,
                rng,
            )
            .map_err(|e| ZkAuditError::ProofGeneration(format!("proof creation: {}", e)))?;

            let public_inputs = circuit
                .get_public_inputs()
                .ok_or_else(|| ZkAuditError::ProofGeneration("failed to get public inputs".into()))?;

            (proof, pk.vk.clone(), public_inputs)
        } else {
            // Fallback: mock trusted setup (random parameters per proof).
            let params = Groth16::<Bn254>::generate_random_parameters_with_reduction(
                circuit.clone(),
                rng,
            )
            .map_err(|e| ZkAuditError::ProofGeneration(format!("parameter generation: {}", e)))?;

            let proof = Groth16::<Bn254>::create_random_proof_with_reduction(
                circuit.clone(),
                &params,
                rng,
            )
            .map_err(|e| ZkAuditError::ProofGeneration(format!("proof creation: {}", e)))?;

            let public_inputs = circuit
                .get_public_inputs()
                .ok_or_else(|| ZkAuditError::ProofGeneration("failed to get public inputs".into()))?;

            (proof, params.vk, public_inputs)
        };

        // Step 4: Verify locally.
        let pvk = prepare_verifying_key(&vk);
        let verified = Groth16::<Bn254>::verify_proof(&pvk, &groth16_proof, &public_inputs)
            .map_err(|e| ZkAuditError::ProofVerification(format!("verify: {}", e)))?;

        if !verified {
            return Err(ZkAuditError::ProofVerification(
                "local verification failed".into(),
            ));
        }

        Ok(Groth16Proof {
            proof: groth16_proof,
            vk,
            public_inputs,
        })
    }

    /// Serialize a proof to the Soroban BN254 hex format for on-chain verification.
    pub fn serialize_for_soroban(proof: &Groth16Proof) -> ZkAuditResult<serialize::SorobanProofArgs> {
        serialize::serialize_proof(&proof.proof, &proof.vk, &proof.public_inputs)
    }

    /// Compute the witness using ark-circom's WitnessCalculator (wasmer-based).
    /// No external snarkjs binary required. Runs inside a Tokio runtime because
    /// wasmer-wasix requires an async reactor.
    ///
    /// When called from within an existing Tokio runtime (e.g. the daemon's
    /// axum handlers), we must not create a nested runtime via `block_on`.
    /// Instead, we spawn a dedicated thread with its own single-threaded
    /// runtime and join it synchronously.
    fn compute_witness_wasmer(&self, proof: &InclusionProof) -> ZkAuditResult<Vec<Fr>> {
        use ark_circom::WitnessCalculator;
        use num_bigint::BigInt;
        use num_traits::Num;
        use wasmer::Store;

        let wasm_path = self.wasm_path.clone();
        let leaf = proof.leaf;
        let path_elements = proof.path_elements.clone();
        let path_indices = proof.path_indices.clone();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("zk-witness".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let rt = match rt {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(Err(ZkAuditError::WitnessGeneration(
                            format!("create tokio runtime: {}", e),
                        )));
                        return;
                    }
                };

                let result = rt.block_on(async {
                    let mut store = Store::default();

                    let mut calculator = WitnessCalculator::new(&mut store, &wasm_path)
                        .map_err(|e| ZkAuditError::WitnessGeneration(format!("load WASM: {}", e)))?;

                    let leaf_bigint = BigInt::from_str_radix(&leaf.to_string(), 10)
                        .map_err(|e| ZkAuditError::WitnessGeneration(format!("parse leaf: {}", e)))?;

                    let path_elements_big: Vec<BigInt> = path_elements
                        .iter()
                        .map(|f| {
                            BigInt::from_str_radix(&f.to_string(), 10)
                                .map_err(|e| ZkAuditError::WitnessGeneration(format!("parse path element: {}", e)))
                        })
                        .collect::<ZkAuditResult<Vec<_>>>()?;

                    let path_indices_big: Vec<BigInt> = path_indices
                        .iter()
                        .map(|i| BigInt::from(*i))
                        .collect();

                    let inputs = vec![
                        ("leaf".to_string(), vec![leaf_bigint]),
                        ("pathElements".to_string(), path_elements_big),
                        ("pathIndices".to_string(), path_indices_big),
                    ];

                    let witness = calculator
                        .calculate_witness_element::<Fr, _>(&mut store, inputs, false)
                        .map_err(|e| ZkAuditError::WitnessGeneration(format!("calculate witness: {}", e)))?;

                    Ok(witness)
                });

                let _ = tx.send(result);
            })
            .map_err(|e| ZkAuditError::WitnessGeneration(format!("spawn witness thread: {}", e)))?;

        rx.recv().map_err(|e| {
            ZkAuditError::WitnessGeneration(format!("witness thread disconnected: {}", e))
        })?
    }
}

/// Load a serialized proving key from disk (arkworks CanonicalSerialize format).
fn load_proving_key(path: &str) -> ZkAuditResult<ark_groth16::ProvingKey<Bn254>> {
    let file = std::fs::File::open(path)
        .map_err(|e| ZkAuditError::CircuitLoad(format!("open proving key {}: {}", path, e)))?;
    let mut reader = std::io::BufReader::new(file);
    let pk = ark_groth16::ProvingKey::<Bn254>::deserialize_uncompressed_unchecked(&mut reader)
        .map_err(|e| ZkAuditError::CircuitLoad(format!("deserialize proving key: {}", e)))?;
    Ok(pk)
}

/// Generate Groth16 parameters (mock trusted setup) from an R1CS file and
/// serialize the proving key + verifying key to disk.
///
/// This is the "ceremony" step. Run it once, then use `with_proving_key`
/// for all subsequent proof generation.
///
/// In production, this should be replaced by a multi-party ceremony where
/// each contributor adds randomness to the toxic waste. For dev/test, a
/// single random setup is sufficient.
pub fn generate_and_save_parameters(
    r1cs_path: &str,
    proving_key_path: &str,
    verifying_key_path: &str,
) -> ZkAuditResult<()> {
    let circuit = load_r1cs(r1cs_path)?;

    let rng = &mut rand::thread_rng();
    let params = Groth16::<Bn254>::generate_random_parameters_with_reduction(circuit.clone(), rng)
        .map_err(|e| ZkAuditError::ProofGeneration(format!("parameter generation: {}", e)))?;

    // Serialize proving key.
    {
        let file = std::fs::File::create(proving_key_path)
            .map_err(|e| ZkAuditError::Io(e))?;
        let mut writer = std::io::BufWriter::new(file);
        params
            .serialize_uncompressed(&mut writer)
            .map_err(|e| ZkAuditError::Serialization(format!("serialize proving key: {}", e)))?;
    }

    // Serialize verifying key.
    {
        let file = std::fs::File::create(verifying_key_path)
            .map_err(|e| ZkAuditError::Io(e))?;
        let mut writer = std::io::BufWriter::new(file);
        params
            .vk
            .serialize_uncompressed(&mut writer)
            .map_err(|e| ZkAuditError::Serialization(format!("serialize verifying key: {}", e)))?;
    }

    Ok(())
}

/// Load an R1CS file and return a CircomCircuit template (no witness).
fn load_r1cs(path: &str) -> ZkAuditResult<CircomCircuit<Fr>> {
    let file = std::fs::File::open(path)
        .map_err(|e| ZkAuditError::CircuitLoad(format!("open {}: {}", path, e)))?;
    let mut file = std::io::BufReader::new(file);
    let r1cs_file = R1CSFile::<Fr>::new(&mut file)
        .map_err(|e| ZkAuditError::CircuitLoad(format!("parse R1CS: {}", e)))?;

    let mut r1cs: ark_circom::circom::R1CS<Fr> = r1cs_file.into();
    // The wire_mapping from Circom uses 1-indexed labels, but the snarkjs
    // witness array is 0-indexed. Setting to None uses the witness directly.
    r1cs.wire_mapping = None;

    Ok(CircomCircuit {
        r1cs,
        witness: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merkle::AuditMerkleTree;
    use std::path::Path;

    /// R1CS and WASM paths from the spike. These tests are skipped if the
    /// compiled circuit artifacts don't exist (e.g. in CI without circom).
    const R1CS_PATH: &str = "../zk-spike/circuits/build/merkle_inclusion.r1cs";
    const WASM_PATH: &str = "../zk-spike/circuits/build/merkle_inclusion_js/merkle_inclusion.wasm";

    fn circuit_exists() -> bool {
        Path::new(R1CS_PATH).exists() && Path::new(WASM_PATH).exists()
    }

    #[test]
    fn test_load_r1cs() {
        if !circuit_exists() {
            eprintln!("Skipping test_load_r1cs: circuit not found");
            return;
        }
        let circuit = load_r1cs(R1CS_PATH).unwrap();
        assert!(!circuit.r1cs.constraints.is_empty());
        assert!(circuit.r1cs.num_inputs > 0);
    }

    #[test]
    fn test_full_prove_and_serialize() {
        if !circuit_exists() {
            eprintln!("Skipping test_full_prove_and_serialize: circuit not found");
            return;
        }

        // Build a tree with 4 leaves.
        let mut tree = AuditMerkleTree::with_height(20).unwrap();
        tree.insert(Fr::from(1u64));
        tree.insert(Fr::from(2u64));
        tree.insert(Fr::from(3u64));
        tree.insert(Fr::from(4u64));

        let root = tree.root().unwrap();

        // Generate inclusion proof for leaf at index 2.
        let inclusion = tree.prove_inclusion(2).unwrap();
        assert_eq!(inclusion.root, root);

        // Create the prover and generate a Groth16 proof.
        let prover = AuditProver::new(R1CS_PATH, WASM_PATH).unwrap();
        let groth16_proof = prover.prove(&inclusion).unwrap();

        // The public input (root) must match.
        assert_eq!(groth16_proof.public_inputs.len(), 1);
        assert_eq!(groth16_proof.public_inputs[0], root);

        // Serialize to Soroban format.
        let soroban_args = AuditProver::serialize_for_soroban(&groth16_proof).unwrap();

        // Verify hex lengths.
        assert_eq!(soroban_args.proof.a.len(), 128);   // G1: 64 bytes
        assert_eq!(soroban_args.proof.b.len(), 256);   // G2: 128 bytes
        assert_eq!(soroban_args.proof.c.len(), 128);   // G1: 64 bytes
        assert_eq!(soroban_args.vk.alpha.len(), 128);  // G1: 64 bytes
        assert_eq!(soroban_args.vk.beta.len(), 256);   // G2: 128 bytes
        assert_eq!(soroban_args.vk.gamma.len(), 256);  // G2: 128 bytes
        assert_eq!(soroban_args.vk.delta.len(), 256);  // G2: 128 bytes
        assert!(!soroban_args.vk.ic.is_empty());

        // Public signals must contain the root as a decimal string.
        assert_eq!(soroban_args.pub_signals.len(), 1);
        assert_eq!(soroban_args.pub_signals[0], root.to_string());
    }

    #[test]
    fn test_ceremony_proving_key() {
        if !circuit_exists() {
            eprintln!("Skipping test_ceremony_proving_key: circuit not found");
            return;
        }

        // Build a tree with 3 leaves.
        let mut tree = AuditMerkleTree::with_height(20).unwrap();
        tree.insert(Fr::from(10u64));
        tree.insert(Fr::from(20u64));
        tree.insert(Fr::from(30u64));

        let inclusion = tree.prove_inclusion(1).unwrap();

        // Generate ceremony parameters to a temp directory.
        let tmp = tempfile::tempdir().unwrap();
        let pkey = tmp.path().join("test.pkey");
        let vkey = tmp.path().join("test.vkey");

        generate_and_save_parameters(
            R1CS_PATH,
            pkey.to_str().unwrap(),
            vkey.to_str().unwrap(),
        )
        .expect("ceremony generation failed");

        assert!(pkey.exists());
        assert!(vkey.exists());

        // Create prover with the pre-generated key and generate a proof.
        let prover = AuditProver::with_proving_key(
            R1CS_PATH,
            WASM_PATH,
            pkey.to_str().unwrap(),
        )
        .expect("failed to create prover with proving key");

        let groth16_proof = prover.prove(&inclusion).expect("proof generation failed");

        // Verify the proof is valid.
        assert_eq!(groth16_proof.public_inputs.len(), 1);
        assert_eq!(groth16_proof.public_inputs[0], tree.root().unwrap());

        // Serialize and check format.
        let soroban_args = AuditProver::serialize_for_soroban(&groth16_proof).unwrap();
        assert_eq!(soroban_args.proof.a.len(), 128);
        assert_eq!(soroban_args.proof.b.len(), 256);
        assert_eq!(soroban_args.proof.c.len(), 128);
    }
}
