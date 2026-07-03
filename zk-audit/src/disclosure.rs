//! Audited-Action Disclosure proofs (leaf v3).
//!
//! Generates Groth16 proofs for the `audited_action.circom` circuit: a
//! statement that an audit event with certain properties exists in the
//! committed Merkle tree, without revealing the event.
//!
//! Public signals, in circuit witness order (Circom puts `main`'s outputs
//! before its public inputs):
//!
//! ```text
//! [root, leaf, opPred, collPred, tsMin, tsMax, checkOp, checkColl, checkTs]
//! ```
//!
//! The private witness is the v3 leaf commitment's opening
//! ([`LeafOpening`]) plus the Merkle authentication path
//! ([`InclusionProof`]). The verifier learns only the predicate parameters
//! and that the proof verifies — not the document, database, exact
//! timestamp, salt, or commitment key.

use num_bigint::BigInt;
use num_traits::Num;

use crate::commitment::LeafOpening;
use crate::error::{ZkAuditError, ZkAuditResult};
use crate::merkle::InclusionProof;
use crate::prover::{
    compute_witness, load_proving_key, load_r1cs, prove_with_witness, Fr, Groth16Proof,
};

/// The public statement of a disclosure proof: which predicate checks are
/// enabled and their parameters. Every field here becomes a **public
/// signal** — bound into the proof on-chain — so a proof cannot be replayed
/// against a different claim.
#[derive(Debug, Clone)]
pub struct DisclosureStatement {
    /// `str_to_field(operation)` — compared against the private `opH` when
    /// `check_op` is set.
    pub op_pred: Fr,
    /// `str_to_field(collection)` — compared against the private `collH`
    /// when `check_coll` is set.
    pub coll_pred: Fr,
    /// Inclusive lower timestamp bound (Unix seconds).
    pub ts_min: u64,
    /// Inclusive upper timestamp bound (Unix seconds).
    pub ts_max: u64,
    pub check_op: bool,
    pub check_coll: bool,
    pub check_ts: bool,
}

/// The disclosure prover: loads the `audited_action` circuit and generates
/// Groth16 disclosure proofs.
pub struct DisclosureProver {
    circuit_template: ark_circom::circom::CircomCircuit<Fr>,
    wasm_path: String,
    proving_key: Option<ark_groth16::ProvingKey<ark_bn254::Bn254>>,
}

impl DisclosureProver {
    /// Create a prover from compiled circuit artifacts (mock per-proof setup).
    pub fn new(r1cs_path: &str, wasm_path: &str) -> ZkAuditResult<Self> {
        Ok(Self {
            circuit_template: load_r1cs(r1cs_path)?,
            wasm_path: wasm_path.to_string(),
            proving_key: None,
        })
    }

    /// Create a prover with a pre-generated proving key (from
    /// `zk-audit-ceremony`).
    pub fn with_proving_key(
        r1cs_path: &str,
        wasm_path: &str,
        proving_key_path: &str,
    ) -> ZkAuditResult<Self> {
        Ok(Self {
            circuit_template: load_r1cs(r1cs_path)?,
            wasm_path: wasm_path.to_string(),
            proving_key: Some(load_proving_key(proving_key_path)?),
        })
    }

    /// Generate a disclosure proof.
    ///
    /// `opening` is the v3 commitment's private opening; `inclusion` is the
    /// Merkle path for the same leaf (its `leaf` field must equal the
    /// Poseidon commitment over `opening`, or witness generation fails the
    /// circuit's `commit.out === leaf` constraint).
    pub fn prove(
        &self,
        opening: &LeafOpening,
        inclusion: &InclusionProof,
        statement: &DisclosureStatement,
    ) -> ZkAuditResult<Groth16Proof> {
        let fr_big = |f: &Fr, label: &str| -> ZkAuditResult<BigInt> {
            BigInt::from_str_radix(&f.to_string(), 10)
                .map_err(|e| ZkAuditError::WitnessGeneration(format!("parse {label}: {e}")))
        };

        let path_elements: Vec<BigInt> = inclusion
            .path_elements
            .iter()
            .map(|f| fr_big(f, "path element"))
            .collect::<ZkAuditResult<Vec<_>>>()?;
        let path_indices: Vec<BigInt> = inclusion
            .path_indices
            .iter()
            .map(|i| BigInt::from(*i))
            .collect();

        let flag = |b: bool| vec![BigInt::from(u8::from(b))];

        let inputs = vec![
            // Public inputs.
            ("leaf".to_string(), vec![fr_big(&inclusion.leaf, "leaf")?]),
            ("opPred".to_string(), vec![fr_big(&statement.op_pred, "opPred")?]),
            (
                "collPred".to_string(),
                vec![fr_big(&statement.coll_pred, "collPred")?],
            ),
            ("tsMin".to_string(), vec![BigInt::from(statement.ts_min)]),
            ("tsMax".to_string(), vec![BigInt::from(statement.ts_max)]),
            ("checkOp".to_string(), flag(statement.check_op)),
            ("checkColl".to_string(), flag(statement.check_coll)),
            ("checkTs".to_string(), flag(statement.check_ts)),
            // Private witness: commitment opening.
            ("key".to_string(), vec![fr_big(&opening.key, "key")?]),
            ("opH".to_string(), vec![fr_big(&opening.op_h, "opH")?]),
            ("dbH".to_string(), vec![fr_big(&opening.db_h, "dbH")?]),
            ("collH".to_string(), vec![fr_big(&opening.coll_h, "collH")?]),
            ("ts".to_string(), vec![BigInt::from(opening.ts)]),
            ("docH".to_string(), vec![fr_big(&opening.doc_h, "docH")?]),
            ("salt".to_string(), vec![fr_big(&opening.salt, "salt")?]),
            // Private witness: Merkle path.
            ("pathElements".to_string(), path_elements),
            ("pathIndices".to_string(), path_indices),
        ];

        let witness = compute_witness(&self.wasm_path, inputs)?;
        prove_with_witness(&self.circuit_template, witness, self.proving_key.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commitment::{poseidon_leaf_v3, str_to_field};
    use crate::merkle::AuditMerkleTree;
    use std::path::Path;

    const R1CS_PATH: &str = "../zk-spike/circuits/build/audited_action.r1cs";
    const WASM_PATH: &str =
        "../zk-spike/circuits/build/audited_action_js/audited_action.wasm";

    fn circuit_exists() -> bool {
        Path::new(R1CS_PATH).exists() && Path::new(WASM_PATH).exists()
    }

    fn setup_tree() -> (AuditMerkleTree, LeafOpening, InclusionProof) {
        let key = [0x42u8; 32];
        let (leaf, opening) = poseidon_leaf_v3(
            &key,
            "delete",
            "shopkeeper",
            "orders",
            1_700_000_500,
            b"canonical-payload",
        )
        .unwrap();

        let mut tree = AuditMerkleTree::with_height(20).unwrap();
        tree.insert(Fr::from(11u64)); // unrelated neighbors
        let idx = tree.insert(leaf);
        tree.insert(Fr::from(22u64));

        let inclusion = tree.prove_inclusion(idx).unwrap();
        assert_eq!(inclusion.leaf, leaf);
        (tree, opening, inclusion)
    }

    #[test]
    fn disclosure_proof_round_trip() {
        if !circuit_exists() {
            eprintln!("Skipping disclosure_proof_round_trip: circuit not found");
            return;
        }
        let (_tree, opening, inclusion) = setup_tree();

        let statement = DisclosureStatement {
            op_pred: str_to_field("delete"),
            coll_pred: str_to_field("orders"),
            ts_min: 1_700_000_000,
            ts_max: 1_700_001_000,
            check_op: true,
            check_coll: true,
            check_ts: true,
        };

        let prover = DisclosureProver::new(R1CS_PATH, WASM_PATH).unwrap();
        let proof = prover.prove(&opening, &inclusion, &statement).unwrap();

        // Public signals: [root, leaf, opPred, collPred, tsMin, tsMax,
        // checkOp, checkColl, checkTs].
        assert_eq!(proof.public_inputs.len(), 9);
        assert_eq!(proof.public_inputs[0], inclusion.root);
        assert_eq!(proof.public_inputs[1], inclusion.leaf);
        assert_eq!(proof.public_inputs[2], statement.op_pred);
        assert_eq!(proof.public_inputs[3], statement.coll_pred);
        assert_eq!(proof.public_inputs[4], Fr::from(statement.ts_min));
        assert_eq!(proof.public_inputs[5], Fr::from(statement.ts_max));
        assert_eq!(proof.public_inputs[6], Fr::from(1u64));
        assert_eq!(proof.public_inputs[7], Fr::from(1u64));
        assert_eq!(proof.public_inputs[8], Fr::from(1u64));
    }

    #[test]
    fn disclosure_proof_rejects_false_predicate() {
        if !circuit_exists() {
            eprintln!("Skipping disclosure_proof_rejects_false_predicate: circuit not found");
            return;
        }
        let (_tree, opening, inclusion) = setup_tree();

        // Claim it was an "insert" — the event is a "delete". The witness
        // must fail the circuit's constraints.
        let statement = DisclosureStatement {
            op_pred: str_to_field("insert"),
            coll_pred: str_to_field("orders"),
            ts_min: 0,
            ts_max: u64::MAX >> 1,
            check_op: true,
            check_coll: false,
            check_ts: false,
        };

        let prover = DisclosureProver::new(R1CS_PATH, WASM_PATH).unwrap();
        assert!(prover.prove(&opening, &inclusion, &statement).is_err());
    }

    #[test]
    fn disclosure_proof_rejects_out_of_range_timestamp() {
        if !circuit_exists() {
            eprintln!(
                "Skipping disclosure_proof_rejects_out_of_range_timestamp: circuit not found"
            );
            return;
        }
        let (_tree, opening, inclusion) = setup_tree();

        // Event ts is 1_700_000_500 — claim a range that excludes it.
        let statement = DisclosureStatement {
            op_pred: str_to_field("delete"),
            coll_pred: str_to_field("orders"),
            ts_min: 1_700_001_000,
            ts_max: 1_700_002_000,
            check_op: false,
            check_coll: false,
            check_ts: true,
        };

        let prover = DisclosureProver::new(R1CS_PATH, WASM_PATH).unwrap();
        assert!(prover.prove(&opening, &inclusion, &statement).is_err());
    }
}
