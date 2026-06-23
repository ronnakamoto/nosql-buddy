# ZK-AuditDB Soroban Commitment Contract — Interface Design

**Status:** Draft
**Author:** ZK-AuditDB team
**Last updated:** 2025-06-23

## 1. Overview

This document specifies the on-chain interface for the ZK-AuditDB commitment
contract, a Soroban (Stellar) smart contract that:

1. **Commits** Merkle roots to a tamper-evident, append-only on-chain log.
2. **Verifies** Groth16 inclusion proofs over BN254 using Soroban's native
   BN254 host functions (Protocol 25+).

The contract is the trust anchor for the off-chain ZK-AuditDB proof pipeline.
The off-chain prover (ark-circom + ark-groth16 on BN254) generates proofs that
a given record (leaf) is included in a Merkle tree whose root was committed
on-chain. The contract stores the root and verifies the proof against the
matching verifying key, so that any auditor can independently confirm
inclusion without re-running the prover or trusting the database host.

### 1.1 Proof system assumptions

| Item | Value |
|---|---|
| Curve | BN254 |
| Proof scheme | Groth16 |
| Prover (off-chain) | ark-circom + ark-groth16 (`Bn254`) |
| On-chain verifier | Soroban SDK `env.crypto().bn254()` host functions |
| Public inputs | `[root, leaf_hash, ...circuit-specific public signals]` |
| Pairing equation | `e(-A, B) * e(alpha, beta) * e(vk_x, gamma) * e(C, delta) == 1` |

The off-chain serializer (`zk-audit/src/serialize.rs`) emits points in the
exact big-endian byte layout expected by Soroban's `Bn254G1Affine` /
`Bn254G2Affine` constructors, so the contract receives points as opaque byte
blobs and never has to do endian or coefficient-order conversion on-chain.

### 1.2 Reference implementations

- **BN254 verification pattern:**
  `zk-spike/verifier-contract/src/lib.rs` — the spike `Groth16Verifier`
  computes `vk_x = ic[0] + sum(pub_signals[i] * ic[i+1])` via
  `bn254.g1_mul` / `bn254.g1_add`, then calls
  `bn254.pairing_check(vp1, vp2)` with the four G1/G2 pairs. This contract
  reuses that exact algorithm.
- **Serialization format:**
  `zk-audit/src/serialize.rs` — `g1_to_hex` produces 64-byte
  `X || Y` (big-endian); `g2_to_hex` produces 128-byte
  `X_c1 || X_c0 || Y_c1 || Y_c0` (big-endian). The contract's `Proof` and
  `VerifyingKey` types are the on-chain mirror of `SorobanProof` /
  `SorobanVerifyingKey`.

---

## 2. Types

All types are declared with `#[contracttype]` so they are (de)serializable by
the Soroban environment and usable as function arguments / return values.

### 2.1 `Proof`

The Groth16 proof, mirroring `SorobanProof` from `serialize.rs`.

```rust
#[derive(Clone)]
#[contracttype]
pub struct Proof {
    /// G1 point A: be_bytes(X) || be_bytes(Y) = 64 bytes.
    pub a: Bytes,    // length 64
    /// G2 point B: be_bytes(X_c1) || be_bytes(X_c0) || be_bytes(Y_c1) || be_bytes(Y_c0) = 128 bytes.
    pub b: Bytes,    // length 128
    /// G1 point C: be_bytes(X) || be_bytes(Y) = 64 bytes.
    pub c: Bytes,    // length 64
}
```

**Byte layout** (matches `serialize.rs` exactly):

| Field | Size | Layout |
|---|---|---|
| `a` | 64 B | `X(32 BE) ‖ Y(32 BE)` |
| `b` | 128 B | `X_c1(32 BE) ‖ X_c0(32 BE) ‖ Y_c1(32 BE) ‖ Y_c0(32 BE)` |
| `c` | 64 B | `X(32 BE) ‖ Y(32 BE)` |

> **Design note — `Bytes` vs `Bn254G1Affine`:** The spike verifier uses the
> SDK's `Bn254G1Affine` / `Bn254G2Affine` wrapper types directly. This
> contract instead accepts raw `Bytes` at the boundary and constructs the
> affine types internally via `Bn254G1Affine::from_array(env, &bytes)`. This
> keeps the wire format identical to the off-chain hex output (so the Rust
> client can `hex::decode` and pass `Bytes::from_array` without any
> coordinate reordering) and lets the contract validate lengths before
> touching the host functions. The conversion is a single host call per
> point and has negligible gas cost relative to the pairing check.

### 2.2 `VerifyingKey`

The Groth16 verifying key, mirroring `SorobanVerifyingKey` from
`serialize.rs`.

```rust
#[derive(Clone)]
#[contracttype]
pub struct VerifyingKey {
    /// G1: 64 bytes.
    pub alpha: Bytes,   // length 64
    /// G2: 128 bytes.
    pub beta: Bytes,    // length 128
    /// G2: 128 bytes.
    pub gamma: Bytes,   // length 128
    /// G2: 128 bytes.
    pub delta: Bytes,   // length 128
    /// G1 points IC = gamma_abc_g1: each 64 bytes. ic.len() == pub_signals.len() + 1.
    pub ic: Vec<Bytes>, // each element length 64
}
```

### 2.3 `RootEntry`

An entry in the committed-root history.

```rust
#[derive(Clone)]
#[contracttype]
pub struct RootEntry {
    /// Monotonically increasing sequence number (starts at 1).
    pub sequence: u64,
    /// The committed Merkle root (32-byte field element, big-endian).
    pub root: u256,
    /// Ledger close time when the root was committed (env.ledger().timestamp()).
    pub timestamp: u64,
    /// Free-form metadata supplied by the committer (see §5.3).
    pub metadata: String,
}
```

### 2.4 Errors

```rust
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CommitmentError {
    Unauthorized = 1,
    RootAlreadyCommitted = 2,
    NoRootCommitted = 3,
    InvalidProofEncoding = 4,   // wrong byte length for a G1/G2 point
    MalformedVerifyingKey = 5,  // ic.len() != pub_signals.len() + 1
    InvalidPageSize = 6,
    RootMismatch = 7,           // proof's public root != committed root
    VerificationFailed = 8,     // pairing check returned false
}
```

---

## 3. Contract interface

```rust
#[contract]
pub struct ZkAuditCommitment;

#[contractimpl]
impl ZkAuditCommitment {
    pub fn commit_root(env: Env, root: u256, metadata: String) -> Result<u64, CommitmentError>;
    pub fn verify_inclusion(env: Env, root: u256, proof: Proof, vk: VerifyingKey) -> Result<bool, CommitmentError>;
    pub fn get_current_root(env: Env) -> Option<RootEntry>;
    pub fn get_root_history(env: Env, limit: u32) -> Result<Vec<RootEntry>, CommitmentError>;

    // --- admin ---
    pub fn initialize(env: Env, admin: Address);
    pub fn set_admin(env: Env, new_admin: Address);
    pub fn get_admin(env: Env) -> Address;
}
```

### 3.1 `commit_root`

```text
commit_root(env, root: u256, metadata: String) -> Result<u64, CommitmentError>
```

Commits a new Merkle root to the append-only log.

**Behavior:**
1. Require `env.invoker() == admin` (see §5). Else `Unauthorized`.
2. Read `SEQUENCE` from instance storage, increment it. First commit yields
   sequence `1`.
3. Reject duplicate roots: if `root` already exists in `ROOT_INDEX`
   (Persistent), return `RootAlreadyCommitted`. This prevents replays and
   makes the log a strict set of distinct roots.
4. Build a `RootEntry { sequence, root, timestamp: env.ledger().timestamp(), metadata }`.
5. Write the entry to Persistent storage under key `RootEntry::Root(sequence)`.
6. Insert `root -> sequence` into the `ROOT_INDEX` map (Persistent).
7. Update `CURRENT_ROOT` (Instance) to `sequence`.
8. Emit event `topic = "commit_root"`, data = `(sequence, root, timestamp, metadata)`
   (see §6).
9. Return `sequence`.

**Returns:** the new sequence number (`u64`).

**Errors:** `Unauthorized`, `RootAlreadyCommitted`.

### 3.2 `verify_inclusion`

```text
verify_inclusion(env, root: u256, proof: Proof, vk: VerifyingKey) -> Result<bool, CommitmentError>
```

Verifies a Groth16 proof that some leaf is included in the Merkle tree
identified by `root`.

**Behavior:**
1. Look up `root` in `ROOT_INDEX`. If absent, return `RootMismatch`. (We do
   not verify proofs against roots that were never committed.)
2. Validate byte lengths of every point in `proof` and `vk`:
   - G1 fields (`a`, `c`, `alpha`, each `ic[i]`) must be 64 bytes.
   - G2 fields (`b`, `beta`, `gamma`, `delta`) must be 128 bytes.
   Else `InvalidProofEncoding`.
3. Construct `Bn254G1Affine` / `Bn254G2Affine` from the byte arrays via
   `from_array`. The host function performs on-curve / subgroup checks.
4. Recover the public signals for the pairing check. The circuit's public
   inputs are `[root, leaf_hash, ...]`; the **first** public input must equal
   the committed `root`. The caller passes `root` separately, and the
   contract reconstructs the `pub_signals` vector as
   `vec![Fr::from_u256(root), ...caller_supplied_remaining]`.
   > **Open question (see §8.1):** whether the remaining public signals are
   > passed in by the caller or whether the VK's `ic` length alone is
   > sufficient. The spike takes `pub_signals: Vec<Fr>` as an explicit
   > argument; this contract folds `root` in automatically and requires the
   > caller to supply the rest. The design below assumes an extended
   > signature `verify_inclusion(env, root, proof, vk, extra_pub_signals:
   > Vec<Fr>)` — see §8.1 for the rationale and the alternative.
5. Check `vk.ic.len() == pub_signals.len() + 1`; else `MalformedVerifyingKey`.
6. Compute `vk_x = ic[0] + Σ pub_signals[i] * ic[i+1]` using
   `bn254.g1_mul` and `bn254.g1_add` (identical to the spike).
7. Build the pairing vectors:
   ```rust
   let neg_a = -proof.a;
   let vp1 = vec![&env, neg_a, vk.alpha, vk_x, proof.c];
   let vp2 = vec![&env, proof.b, vk.beta, vk.gamma, vk.delta];
   ```
8. `bn254.pairing_check(vp1, vp2)`. If `false`, return
   `VerificationFailed` (or `Ok(false)` — see §8.2). If `true`, return
   `Ok(true)`.

**Returns:** `bool` — `true` iff the proof is valid for `root`.

**Errors:** `RootMismatch`, `InvalidProofEncoding`, `MalformedVerifyingKey`,
`VerificationFailed` (only if we choose to error on failure rather than
return `Ok(false)`; see §8.2).

### 3.3 `get_current_root`

```text
get_current_root(env) -> Option<RootEntry>
```

Returns the most recently committed root entry, or `None` if no root has
been committed yet. Reads `CURRENT_ROOT` (Instance) to get the sequence,
then loads `RootEntry::Root(sequence)` from Persistent storage.

### 3.4 `get_root_history`

```text
get_root_history(env, limit: u32) -> Result<Vec<RootEntry>, CommitmentError>
```

Returns up to `limit` most-recent `RootEntry` values, newest-first.

**Pagination:** Because Soroban `Vec` does not support random access by key
range, pagination is implemented by reading from `SEQUENCE` downwards:

```text
total = SEQUENCE
start = total.saturating_sub(limit) + 1   // 1-based sequence
for seq in (start..=total).rev() {
    push RootEntry::Root(seq)
}
```

`limit` is clamped to a configurable `MAX_PAGE_SIZE` (default 100). `0` is
invalid (`InvalidPageSize`). Callers wanting older entries should pass a
`start_after` cursor — see §8.3 for the proposed
`get_root_history_paginated(env, start_after: u64, limit: u32)` extension.

**Errors:** `InvalidPageSize`.

---

## 4. Storage model

Soroban offers two storage types: **Instance** (tied to the contract
instance, small, hot, reset on upgrade unless migrated) and **Persistent**
(permanent, lives across upgrades, rent-paying). The contract uses them
deliberately:

| Key | Storage | Type | Notes |
|---|---|---|---|
| `ADMIN` | Instance | `Address` | Set once in `initialize`. |
| `SEQUENCE` | Instance | `u64` | Monotonic counter. Cheap to read/write on every commit. |
| `CURRENT_ROOT` | Instance | `u64` | Sequence of latest root. Read by `get_current_root` on every call. |
| `RootEntry::Root(seq)` | Persistent | `RootEntry` | The append-only log. Must survive upgrades. |
| `ROOT_INDEX` | Persistent | `Map<u256, u64>` | `root -> sequence`, for dedup and `verify_inclusion` lookup. Must survive upgrades. |

### 4.1 Why split Instance vs Persistent?

- **Instance** for `SEQUENCE` / `CURRENT_ROOT` / `ADMIN`: these are tiny,
  read on nearly every call, and cheap. Instance storage has lower gas for
  hot reads. `ADMIN` is administrative and can be re-set on upgrade via
  `initialize`/`set_admin`.
- **Persistent** for the root log and dedup index: these are the
  tamper-evident record. They **must** survive contract upgrades, and
  Persistent storage is the only type guaranteed to persist across a
  `contract.update` Wasm replacement. Instance storage is *not* automatically
  migrated on upgrade; keeping the audit log in Persistent avoids any data
  loss during upgrades.

### 4.2 Key layout (concrete)

```rust
#[contracttype]
pub enum StorageKey {
    Admin,
    Sequence,
    CurrentRoot,
    RootIndex,          // Map<u256, u64>
    Root(u64),          // RootEntry by sequence
}
```

`RootIndex` is stored as a single `soroban_sdk::Map<u256, u64>` under the
`RootIndex` key. For very large histories this could be split, but a Merkle
root set is expected to stay in the low thousands; a single map is fine and
keeps dedup O(1).

---

## 5. Access control

### 5.1 Admin role

A single `admin: Address` is stored in Instance storage and set via
`initialize(env, admin)` (called once at deployment). Only `admin` may call
`commit_root`. `set_admin(env, new_admin)` allows rotation; it requires the
invoker to be the current admin.

```rust
fn require_admin(env: &Env) -> Result<(), CommitmentError> {
    let admin: Address = env.storage().instance().get(&StorageKey::Admin).unwrap();
    if env.invoker() != admin {
        return Err(CommitmentError::Unauthorized);
    }
    Ok(())
}
```

### 5.2 Who can call `verify_inclusion` / `get_*`?

**Anyone.** These are read-only / verification functions and are
permissionless. The whole point of the audit log is that any third-party
auditor can verify inclusion without trusting the committer.

### 5.3 `metadata` field

`metadata` is a free-form `String` supplied by the committer. Recommended
off-chain convention (enforced by the off-chain client, not the contract):
a short JSON blob such as `{"tree_height":20,"leaf_count":1024,"db_epoch":42}`.
The contract does **not** parse it; it only stores and echoes it. A max
length (e.g. 256 bytes) should be enforced to bound gas — see §7.

---

## 6. Event schema

Soroban events are `(topics: Vec<Val>, data: Val)`. The contract emits one
event per `commit_root` call.

### 6.1 `commit_root` event

```text
topics: ["commit_root", contract_id]
data:   (sequence: u64, root: u256, timestamp: u64, metadata: String)
```

- `topics[0]` = `Symbol("commit_root")` — fixed topic for filtering.
- `topics[1]` = the contract's own `Address` (so events from multiple
  deployed instances can be disambiguated).
- `data` = a `(u64, u256, u64, String)` tuple, matching the `RootEntry`
  fields exactly. This lets off-chain indexers reconstruct the full entry
  from the event alone without a `get_root_history` round-trip.

```rust
env.events().publish(
    (Symbol::new(&env, "commit_root"), env.current_contract_address()),
    (sequence, root, timestamp, metadata),
);
```

### 6.2 Verification events (optional)

Optionally emit a `verify_inclusion` event so auditors can see on-chain
verification activity:

```text
topics: ["verify_inclusion", contract_id]
data:   (root: u256, result: bool, caller: Address)
```

This is **optional** and increases gas per verification; it should be
gated behind a flag (see §8.4) or omitted in v1.

---

## 7. Gas considerations

### 7.1 Cost profile

| Operation | Dominant cost | Notes |
|---|---|---|
| `commit_root` | 1 Instance write (`SEQUENCE`, `CURRENT_ROOT`) + 1 Persistent write (`RootEntry`) + 1 Persistent map update (`ROOT_INDEX`) + event | O(1). Cheap. |
| `verify_inclusion` | 1 Persistent read (`ROOT_INDEX`) + **multi-scalar mul** (`n` `g1_mul` + `n` `g1_add`) + **1 pairing check** (4 pairs) | This is by far the most expensive call. See §7.2. |
| `get_current_root` | 1 Instance read + 1 Persistent read | O(1). Cheap. |
| `get_root_history` | `limit` Persistent reads | O(limit). Bounded by `MAX_PAGE_SIZE`. |

### 7.2 BN254 verification gas

The spike's `test_verify_multiplier2_proof` prints a cost estimate; the
dominant components are:

- `g1_mul` × `(pub_signals.len())` — one per non-constant public input.
- `g1_add` × `(pub_signals.len())` — accumulation into `vk_x`.
- `pairing_check` with 4 G1/G2 pairs — the single biggest host-function cost.

For the ZK-AuditDB inclusion circuit the public inputs are expected to be
`[root, leaf_hash]` (2 signals), so `vk_x` needs 2 `g1_mul` + 2 `g1_add`.
The pairing check is independent of public-input count.

**Mitigations:**
- Keep the circuit's public-input count minimal (every extra public input
  adds a `g1_mul` + `g1_add`).
- Do **not** store the full `VerifyingKey` on-chain and reload it per call;
  have the caller pass it. Storing a ~1 KB VK in Persistent storage and
  reading it every verification would add a large read cost and would
  couple the contract to a single circuit. Passing the VK as an argument
  keeps the contract circuit-agnostic and lets the off-chain prover pick
  the VK.
- Consider a `verify_inclusion_batched` function (§8.5) that amortizes the
  `ROOT_INDEX` read across multiple proofs against the same root.

### 7.3 `metadata` length cap

Enforce `metadata.len() <= MAX_METADATA_LEN` (e.g. 256 bytes) in
`commit_root` to bound Persistent write cost and event size.

---

## 8. Design decisions & open questions

### 8.1 Public signals: caller-supplied vs folded

The spike's `verify_proof(vk, proof, pub_signals)` takes all public signals
explicitly. This contract's `verify_inclusion(env, root, proof, vk)` takes
`root` separately because the contract must tie the proof to a **committed**
root. Two options:

- **(A) Fold `root` in (proposed):** signature becomes
  `verify_inclusion(env, root, proof, vk, extra_pub_signals: Vec<Fr>)`.
  The contract builds `pub_signals = vec![Fr::from_u256(root), ...extra]`
  and checks `vk.ic.len() == pub_signals.len() + 1`. This guarantees the
  proof is bound to the on-chain root and removes one class of caller error
  (passing a `root` that doesn't match the proof's first public input).
- **(B) Caller supplies all:** `verify_inclusion(env, root, proof, vk,
  pub_signals: Vec<Fr>)`, and the contract asserts
  `pub_signals[0] == Fr::from_u256(root)`. Simpler internally but shifts
  the correctness burden to the caller.

**Recommendation:** Option (A). It is safer and matches the mental model
"prove inclusion *in this committed root*." The `root` argument is the
anchor; the contract enforces that the proof's first public input is exactly
that root.

### 8.2 `verify_inclusion` return on failure

Two conventions:

- **Error on failure:** return `Err(VerificationFailed)` / `Err(RootMismatch)`.
  Makes failures explicit and unambiguous in transaction results.
- **`Ok(false)` on failure:** return `bool` uniformly. Easier for clients
  that just want a yes/no.

**Recommendation:** Return `Ok(false)` for a *cryptographic* failure
(pairing check false) so the function is a pure predicate, but return
`Err(...)` for *structural* failures (`RootMismatch`,
`InvalidProofEncoding`, `MalformedVerifyingKey`) because those indicate
caller bugs, not "the leaf is not included." This matches the spike's
`Result<bool, Groth16Error>` pattern.

### 8.3 Pagination cursor

`get_root_history(env, limit)` returns the newest `limit` entries. For
deep history, add:

```rust
pub fn get_root_history_paginated(env: Env, start_after: u64, limit: u32)
    -> Result<Vec<RootEntry>, CommitmentError>;
```

Returns entries with `sequence <= start_after`, newest-first. `start_after =
u64::MAX` means "start from the current tip." This avoids O(n) scanning for
large logs. **Recommend including in v1** since it is cheap to add.

### 8.4 Optional verification event

Gate the `verify_inclusion` event behind an Instance-stored boolean flag
`EMIT_VERIFY_EVENTS` (default `false`), toggled by admin. Keeps default gas
low while allowing auditors to enable tracing when needed.

### 8.5 Batched verification (future)

```rust
pub fn verify_inclusion_batched(
    env: Env,
    root: u256,
    proofs: Vec<Proof>,
    vk: VerifyingKey,
    extra_pub_signals: Vec<Vec<Fr>>,
) -> Result<Vec<bool>, CommitmentError>;
```

Amortizes the single `ROOT_INDEX` read and (if the VK is eventually stored
on-chain) the VK read across many proofs. Not in v1; listed for
completeness.

---

## 9. Upgrade path

### 9.1 Wasm replacement

Soroban contracts are upgraded by replacing the Wasm via
`contract.update`. **Persistent storage survives**; Instance storage does
**not** automatically survive (it is tied to the old instance and must be
migrated explicitly).

Because of this:

- **The audit log (`RootEntry`, `ROOT_INDEX`) is in Persistent storage and
  survives upgrades by construction.** No migration needed.
- **`ADMIN`, `SEQUENCE`, `CURRENT_ROOT` are in Instance storage.** An
  upgrade must re-seed these from Persistent storage. The upgrade flow is:

  1. Deploy new Wasm via `contract.update`.
  2. Call a new `migrate_from_persistent(env)` function (added in the new
     Wasm) that:
     - Recomputes `SEQUENCE` as the max `sequence` across all `RootEntry`
       keys (or, cheaper, stores a `MAX_SEQUENCE` mirror in Persistent).
     - Recomputes `CURRENT_ROOT` = `SEQUENCE`.
     - Re-seeds `ADMIN` from a Persistent `ADMIN_PERSISTENT` mirror (see
       below).
  3. Alternatively, store `SEQUENCE`, `CURRENT_ROOT`, and `ADMIN` in
     Persistent storage too and only keep a cached copy in Instance. This
     trades a slightly higher commit cost for zero-migration upgrades.
     **Recommendation: store `ADMIN` in Persistent (mirror) to avoid
     lockout on upgrade; keep `SEQUENCE`/`CURRENT_ROOT` in Instance with a
     Persistent `MAX_SEQUENCE` mirror for cheap re-seed.**

### 9.2 VK / circuit upgrades

The contract is **circuit-agnostic**: the VK is passed per call, not
stored. When the off-chain circuit changes (new leaf format, new public
inputs), no contract upgrade is needed — the off-chain prover simply
passes the new VK. This is a major reason to keep the VK out of storage.

If a canonical VK ever needs to be pinned on-chain (e.g. for a "trusted
VK registry"), add an admin-gated `register_vk(env, circuit_id: Symbol, vk:
VerifyingKey)` and have `verify_inclusion` optionally accept a `circuit_id`
to load it. This is a v2 feature.

### 9.3 Backward compatibility

- Never reuse `StorageKey` enum discriminants; only append new variants.
- New functions are additive; do not change existing function signatures.
  If a signature must change, deploy a new contract and provide a
  read-only migration view.

---

## 10. File layout (proposed, for implementation phase)

```
zk-audit/soroban-contract/
├── INTERFACE.md            # this document
├── Cargo.toml              # soroban-sdk =25.1.0
├── src/
│   ├── lib.rs              # contract + contractimpl
│   ├── types.rs            # Proof, VerifyingKey, RootEntry, StorageKey, CommitmentError
│   ├── storage.rs          # storage helpers (get/set admin, sequence, root entry, index)
│   ├── verify.rs           # BN254 Groth16 verification (mirrors spike lib.rs)
│   └── test.rs             # integration tests using arkworks-generated fixtures
└── test_snapshots/         # soroban-sdk test snapshots
```

---

## 11. Open items before implementation

1. **Confirm public-input layout** of the inclusion circuit: is the first
   public input the root, or is it `(root, leaf_hash)` in a different
   order? This determines the folding logic in §8.1.
2. **Decide on `MAX_PAGE_SIZE` and `MAX_METADATA_LEN`** concrete values.
3. **Decide on `verify_inclusion` event** (default off vs on).
4. **Decide Instance-vs-Persistent split for `SEQUENCE`/`CURRENT_ROOT`**
   given the upgrade story in §9.1.
5. **Pin `soroban-sdk` version** (spike uses `=25.1.0`; confirm
   BN254 host functions are stable in that version).
