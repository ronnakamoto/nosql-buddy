pragma circom 2.2.2;

include "node_modules/circomlib/circuits/poseidon.circom";
include "node_modules/circomlib/circuits/comparators.circom";
include "node_modules/circomlib/circuits/bitify.circom";

// ZK-AuditDB: Audited-Action Disclosure circuit (leaf v3).
//
// Proves that an audit event with certain properties exists in a Merkle
// tree whose root is committed on-chain — WITHOUT revealing the event.
//
// Statement:
//   "I know the opening (key, opH, dbH, collH, ts, docH, salt) of the
//    public commitment `leaf`, and a Merkle path from `leaf` to the public
//    `root`, such that the enabled predicate checks hold:
//      - checkOp   = 1 → opH   == opPred   (operation equality)
//      - checkColl = 1 → collH == collPred (collection equality)
//      - checkTs   = 1 → tsMin <= ts <= tsMax (timestamp range)"
//
// The leaf is the v3 keyed Poseidon vector commitment produced off-circuit
// by `zk_audit::commitment::poseidon_leaf_v3`:
//
//   leaf = Poseidon(7)(key, opH, dbH, collH, ts, docH, salt)
//
// The verifier learns ONLY the predicate parameters and the yes/no result.
// The document content (docH preimage), database, exact timestamp, salt,
// and commitment key stay private. This is a statement a plain Merkle
// proof cannot make: it requires opening a hiding commitment in-circuit.
//
// Public signals (Circom orders main's outputs before its public inputs):
//   [root, leaf, opPred, collPred, tsMin, tsMax, checkOp, checkColl, checkTs]
//
// `leaf` MUST be public (binds the proof to a specific committed entry —
// see merkle_inclusion.circom for why an unbound leaf is vacuous), and
// every predicate parameter MUST be public (folded into vk_x on-chain) or
// a valid proof could be replayed against a different claim.
//
// Soundness notes:
//   - Enable flags are constrained to {0, 1}.
//   - `ts` is range-constrained to 64 bits before comparison (circomlib
//     comparators are only sound on pre-range-checked inputs).
//   - tsMin/tsMax are public and chosen by the verifier; the verifier is
//     responsible for passing values < 2^64.
template AuditedAction(height) {
    // ── Public inputs ───────────────────────────────────────────────
    signal input leaf;      // v3 commitment (public: binds proof to an entry)
    signal input opPred;    // str_to_field(operation) predicate parameter
    signal input collPred;  // str_to_field(collection) predicate parameter
    signal input tsMin;     // inclusive lower bound, Unix seconds
    signal input tsMax;     // inclusive upper bound, Unix seconds
    signal input checkOp;   // 1 = enforce operation equality
    signal input checkColl; // 1 = enforce collection equality
    signal input checkTs;   // 1 = enforce timestamp range

    // ── Private witness: commitment opening ─────────────────────────
    signal input key;    // domain leaf key as field element
    signal input opH;    // str_to_field(operation)
    signal input dbH;    // str_to_field(database)
    signal input collH;  // str_to_field(collection)
    signal input ts;     // event Unix timestamp (seconds)
    signal input docH;   // bytes_to_field(canonical_payload)
    signal input salt;   // keyed per-leaf salt

    // ── Private witness: Merkle authentication path ─────────────────
    signal input pathElements[height];
    signal input pathIndices[height];

    // ── Public output ────────────────────────────────────────────────
    signal output root;

    // 1. Open the commitment: the public leaf must equal the Poseidon
    //    vector commitment over the private field values.
    component commit = Poseidon(7);
    commit.inputs[0] <== key;
    commit.inputs[1] <== opH;
    commit.inputs[2] <== dbH;
    commit.inputs[3] <== collH;
    commit.inputs[4] <== ts;
    commit.inputs[5] <== docH;
    commit.inputs[6] <== salt;
    commit.out === leaf;

    // 2. Constrain enable flags to {0, 1}.
    checkOp * (checkOp - 1) === 0;
    checkColl * (checkColl - 1) === 0;
    checkTs * (checkTs - 1) === 0;

    // 3. Equality predicates (enforced only when enabled).
    checkOp * (opH - opPred) === 0;
    checkColl * (collH - collPred) === 0;

    // 4. Timestamp range predicate: tsMin <= ts <= tsMax when enabled.
    //    Range-constrain ts to 64 bits first — comparators are only sound
    //    on inputs already known to fit their bit width.
    component tsBits = Num2Bits(64);
    tsBits.in <== ts;

    component geMin = GreaterEqThan(64);
    geMin.in[0] <== ts;
    geMin.in[1] <== tsMin;

    component leMax = LessEqThan(64);
    leMax.in[0] <== ts;
    leMax.in[1] <== tsMax;

    signal inRange;
    inRange <== geMin.out * leMax.out;
    checkTs * (1 - inRange) === 0;

    // 5. Merkle authentication path from leaf to root (identical to
    //    merkle_inclusion.circom).
    component hashers[height];

    signal levelHash[height + 1];
    signal lhTimesIdx[height];   // levelHash[i] * pathIndices[i]
    signal peTimesIdx[height];   // pathElements[i] * pathIndices[i]
    signal left[height];
    signal right[height];

    levelHash[0] <== leaf;

    for (var i = 0; i < height; i++) {
        // Constrain direction bit to {0, 1}.
        pathIndices[i] * (pathIndices[i] - 1) === 0;

        lhTimesIdx[i] <== levelHash[i] * pathIndices[i];
        peTimesIdx[i] <== pathElements[i] * pathIndices[i];

        // left  = levelHash * (1 - idx) + pathElements * idx
        left[i] <== levelHash[i] - lhTimesIdx[i] + peTimesIdx[i];

        // right = levelHash * idx + pathElements * (1 - idx)
        right[i] <== lhTimesIdx[i] + pathElements[i] - peTimesIdx[i];

        hashers[i] = Poseidon(2);
        hashers[i].inputs[0] <== left[i];
        hashers[i].inputs[1] <== right[i];

        levelHash[i + 1] <== hashers[i].out;
    }

    root <== levelHash[height];
}

// Main: 20-level tree (matches the audit log's AuditMerkleTree).
// Public inputs: the leaf plus every predicate parameter — all of them must
// be bound into the proof or the statement is malleable.
component main {public [leaf, opPred, collPred, tsMin, tsMax, checkOp, checkColl, checkTs]} = AuditedAction(20);
