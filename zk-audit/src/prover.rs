//! Groth16 proof generation via ark-circom + ark-groth16.
//!
//! Loads the compiled Circom circuit (R1CS + WASM), computes the witness,
//! generates a Groth16 proof, and verifies it locally.

use ark_bn254::Bn254;
use ark_circom::circom::{CircomCircuit, R1CSFile};
use ark_groth16::{Groth16, prepare_verifying_key};

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
        })
    }

    /// Generate a Groth16 proof for the given inclusion proof.
    ///
    /// This performs a mock trusted setup (random parameters) for each proof.
    /// In production, this should use a real Powers of Tau ceremony via
    /// `read_zkey`.
    pub fn prove(&self, proof: &InclusionProof) -> ZkAuditResult<Groth16Proof> {
        // Step 1: Compute the witness via ark-circom's WitnessCalculator (wasmer).
        let witness = self.compute_witness_wasmer(proof)?;

        // Step 2: Build the circuit with the witness.
        let mut circuit = self.circuit_template.clone();
        circuit.witness = Some(witness);

        // Step 3: Generate parameters (mock trusted setup).
        let rng = &mut rand::thread_rng();
        let params = Groth16::<Bn254>::generate_random_parameters_with_reduction(
            circuit.clone(),
            rng,
        )
        .map_err(|e| ZkAuditError::ProofGeneration(format!("parameter generation: {}", e)))?;

        // Step 4: Generate the proof.
        let groth16_proof = Groth16::<Bn254>::create_random_proof_with_reduction(
            circuit.clone(),
            &params,
            rng,
        )
        .map_err(|e| ZkAuditError::ProofGeneration(format!("proof creation: {}", e)))?;

        // Step 5: Verify locally.
        let public_inputs = circuit
            .get_public_inputs()
            .ok_or_else(|| ZkAuditError::ProofGeneration("failed to get public inputs".into()))?;

        let pvk = prepare_verifying_key(&params.vk);
        let verified = Groth16::<Bn254>::verify_proof(&pvk, &groth16_proof, &public_inputs)
            .map_err(|e| ZkAuditError::ProofVerification(format!("verify: {}", e)))?;

        if !verified {
            return Err(ZkAuditError::ProofVerification(
                "local verification failed".into(),
            ));
        }

        Ok(Groth16Proof {
            proof: groth16_proof,
            vk: params.vk,
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
    fn compute_witness_wasmer(&self, proof: &InclusionProof) -> ZkAuditResult<Vec<Fr>> {
        use ark_circom::WitnessCalculator;
        use num_bigint::BigInt;
        use num_traits::Num;
        use wasmer::Store;

        // wasmer-wasix requires a Tokio 1.x reactor. Create a runtime for the
        // witness calculation. This is reentrant-safe because we block_on the
        // entire computation.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ZkAuditError::WitnessGeneration(format!("create tokio runtime: {}", e)))?;

        rt.block_on(async {
            let mut store = Store::default();

            // Load the WASM module.
            let mut calculator = WitnessCalculator::new(&mut store, &self.wasm_path)
                .map_err(|e| ZkAuditError::WitnessGeneration(format!("load WASM: {}", e)))?;

            // Convert inclusion proof to Circom inputs.
            let leaf_bigint = BigInt::from_str_radix(&proof.leaf.to_string(), 10)
                .map_err(|e| ZkAuditError::WitnessGeneration(format!("parse leaf: {}", e)))?;

            let path_elements: Vec<BigInt> = proof
                .path_elements
                .iter()
                .map(|f| {
                    BigInt::from_str_radix(&f.to_string(), 10)
                        .map_err(|e| ZkAuditError::WitnessGeneration(format!("parse path element: {}", e)))
                })
                .collect::<ZkAuditResult<Vec<_>>>()?;

            let path_indices: Vec<BigInt> = proof
                .path_indices
                .iter()
                .map(|i| BigInt::from(*i))
                .collect();

            let inputs = vec![
                ("leaf".to_string(), vec![leaf_bigint]),
                ("pathElements".to_string(), path_elements),
                ("pathIndices".to_string(), path_indices),
            ];

            let witness = calculator
                .calculate_witness_element::<Fr, _>(&mut store, inputs, false)
                .map_err(|e| ZkAuditError::WitnessGeneration(format!("calculate witness: {}", e)))?;

            Ok(witness)
        })
    }
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
}
