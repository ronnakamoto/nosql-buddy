# NoSQLBuddy

A cross-platform MongoDB management studio for developers, SREs, and database engineers who work across local, staging, and production environments.

NoSQLBuddy connects to MongoDB, lets you browse data, run queries, build aggregations, translate SQL to MongoDB, inspect schema and indexes, and review performance — all from a native desktop app built with Tauri, Rust, React, and TypeScript.

## Features

- **Connection management** — Save connection profiles with secrets stored in the OS keychain. URIs and credentials are redacted from logs and UI responses.
- **Data browsing** — Query collections, paginate results, edit documents in place, and view JSON or table output.
- **Visual query builder** — Compose filters, projections, and sorts without writing raw JSON.
- **Aggregation editor** — Build and preview aggregation pipelines with syntax-aware JSON editing.
- **SQL to MongoDB** — Translate `SELECT`, `JOIN`, `GROUP BY`, `WHERE`, `ORDER BY`, and `LIMIT` statements into aggregation pipelines.
- **Schema and index analysis** — Infer schema shape, cardinality, and index usage from sampled documents.
- **Explain plan visualization** — Parse `explain` output into a navigable tree to diagnose slow queries.
- **Driver code generation** — Export queries and pipelines to Node.js, Python, Java, C#, Ruby, Rust, and the MongoDB shell.
- **ZK audit log** — Tamper-evident Poseidon Merkle tree, Groth16 inclusion proofs, epoch batching with IPFS publishing and Stellar testnet commitments, multi-publisher K-of-N threshold attestation, and reader-mode verification against on-chain roots.
- **Oplog completeness** — Deterministic SHA-256 Merkle tree over MongoDB's oplog (`local.oplog.rs`), binding the audit log to the same ground truth that MongoDB's replication protocol uses. An independent replica member (run by the auditor/regulator) provides a trust anchor that detects any omitted writes. The on-chain commitment stores both the audit log root and the oplog root, and independent attesters submit ed25519 attestations over the oplog root for durable, post-rollover verification.
- **Standalone audit daemon** — `nosqlbuddy-auditd` runs independently of the desktop app, capturing MongoDB change stream events, batching into epochs, publishing to IPFS, and committing Merkle roots on-chain via an HTTP API.
- **Native desktop experience** — Built on Tauri v2 for a small footprint, native menus, and consistent shortcuts on macOS, Windows, and Linux.

## Tech stack

- **Frontend:** React 18, TypeScript, Vite, visx
- **Backend:** Rust, Tauri v2, Tokio
- **Database:** MongoDB driver for Rust (`mongodb` + `bson`)
- **ZK proofs:** ark-circom, ark-groth16, ark-bn254, light-poseidon, circom circuits
- **On-chain:** Soroban (Stellar testnet), native Rust RPC client, `stellar` CLI for writes
- **Decentralized storage:** IPFS (Kubo HTTP API)
- **Audit daemon:** axum HTTP server (standalone binary)
- **Testing:** Playwright (frontend), Cargo (Rust unit + integration tests)

## Getting started

### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install) 1.77+
- A MongoDB instance (local or remote) for live features

#### Audit daemon (optional)

- A MongoDB **replica set** or **sharded cluster** (change streams require oplog; standalone mongod won't work)
- [IPFS Kubo daemon](https://docs.ipfs.tech/install/) running locally for batch publishing (`ipfs daemon`)
- [Stellar CLI](https://docs.stellar.org/docs/build/guides/cli) installed and configured for testnet (`stellar keys generate --global --network testnet`)
- Circuit artifacts for Groth16 proof generation: `merkle_inclusion.r1cs` + `merkle_inclusion.wasm` (bundled in `src-tauri/resources/circuits/`)

### Installation

```bash
git clone https://github.com/ronnakamoto/nosql-buddy.git
cd nosql-buddy
npm install
```

### Development

Run the full Tauri dev environment (Rust backend + Vite frontend):

```bash
npm run tauri dev
```

Run the frontend alone against a mocked backend:

```bash
npm run dev
```

### Build

```bash
npm run build      # Production frontend build
npm run tauri build  # Native app bundle for the current platform
```

## Standalone audit daemon (`nosqlbuddy-auditd`)

The audit daemon runs as a separate process from the desktop app. It captures MongoDB writes via change streams, builds a tamper-evident Poseidon Merkle tree, batches events into epochs, publishes batches to IPFS, and commits Merkle roots to a Soroban contract on Stellar testnet.

### Build

```bash
cd src-tauri
cargo build --bin nosqlbuddy-auditd
```

### Run

**Publisher mode** — captures writes, manages epochs, publishes to IPFS, commits roots on-chain:

```bash
# All daemon commands run from src-tauri/
cd src-tauri

# Basic: connect to MongoDB and listen for changes
cargo run --bin nosqlbuddy-auditd -- \
  --mode publish \
  --mongo-uri "mongodb://localhost:27017"

# Full: with IPFS publishing, Stellar commitment, and proof generation
cargo run --bin nosqlbuddy-auditd -- \
  --mode publish \
  --mongo-uri "mongodb://localhost:27017" \
  --circuit-dir ./resources/circuits \
  --ipfs-api http://127.0.0.1:5001 \
  --rpc-url https://soroban-testnet.stellar.org:443
```

**Reader mode** — verifies local audit log against on-chain commitments (no MongoDB connection needed):

```bash
cd src-tauri
cargo run --bin nosqlbuddy-auditd -- \
  --mode read \
  --data-dir ~/.local/share/nosqlbuddy-auditd
```

**Attester mode** — independent attester that connects to the independent replica member, watches for new epoch commitments on-chain, independently computes the oplog hash, and submits attestations to the contract:

```bash
cd src-tauri
cargo run --bin nosqlbuddy-auditd -- \
  --mode attest \
  --mongo-uri "mongodb://localhost:27019" \
  --rpc-url https://soroban-testnet.stellar.org:443 \
  --attester-identity attester \
  --attester-address GXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
```

### CLI options

| Flag | Default | Description |
|---|---|---|
| `--mode <publish\|read\|attest>` | `publish` | Daemon mode |
| `--mongo-uri <uri>` | — | MongoDB connection URI (required for `publish` and `attest`) |
| `--data-dir <dir>` | OS data dir | Data directory for audit log + sled |
| `--port <port>` | `9173` | HTTP API port |
| `--circuit-dir <dir>` | — | Circuit artifacts dir (for `/proof/:index`) |
| `--ipfs-api <url>` | `http://127.0.0.1:5001` | IPFS Kubo HTTP API URL |
| `--rpc-url <url>` | Stellar testnet | Soroban RPC URL |
| `--epoch-threshold <n>` | `100` | Auto-close epoch after N events (0=disabled) |
| `--epoch-time-secs <s>` | `0` | Auto-close epoch after S seconds (0=disabled) |
| `--oplog-hash-required` | — | Fail epoch close if oplog hash computation fails |
| `--attester-key-file <path>` | `<data-dir>/audit/attester.key` | Attester ed25519 signing key (attest mode) |
| `--attester-identity <name>` | — | Stellar CLI identity for attester transactions (attest mode) |
| `--attester-address <addr>` | — | Stellar address of the attester (attest mode) |
| `--help` | — | Show help |

### HTTP API

All endpoints are on `http://localhost:9173`. Both modes share common endpoints; publisher mode adds epoch, IPFS, Stellar, and attestation endpoints.

**Common (both modes):**

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Audit log status (root, leaf count, event count) |
| `GET` | `/events` | List all recorded audit events |
| `GET` | `/root` | Current Merkle root (hex) |
| `POST` | `/proof/:index` | Generate Groth16 inclusion proof (requires `--circuit-dir`) |

**Publisher mode:**

| Method | Path | Description |
|---|---|---|
| `GET` | `/epochs` | List all epochs (open and closed, with oplog hash fields) |
| `GET` | `/epoch/current` | Get the current open epoch |
| `POST` | `/epoch/close` | Close current epoch, freeze root + compute oplog hash |
| `POST` | `/epoch/:n/commit` | Commit epoch root + oplog hash to Stellar testnet |
| `POST` | `/epoch/:n/publish-ipfs` | Publish epoch events to IPFS |
| `GET` | `/epoch/:n/ipfs-cid` | Get IPFS CID for a published epoch |
| `GET` | `/onchain-root` | Latest committed root (via native RPC) |
| `GET` | `/ipfs/check` | Check if IPFS daemon is reachable |
| `GET` | `/publishers` | List registered publishers |
| `POST` | `/publishers` | Register a publisher (`{publicKey, name}`) |
| `DELETE` | `/publishers/:key` | Remove a publisher |
| `POST` | `/attestations` | Submit an attestation |
| `GET` | `/attestations/:epoch` | List attestations for an epoch |
| `GET` | `/attestations/:epoch/status` | Attestation threshold status |
| `GET` | `/threshold` | Get K-of-N threshold |
| `POST` | `/threshold` | Set K-of-N threshold (`{threshold}`) |

**Reader mode:**

| Method | Path | Description |
|---|---|---|
| `GET` | `/reader/verify` | Verify local log against on-chain root |
| `GET` | `/reader/verify-oplog` | Verify oplog integrity (three-way compare: on-chain vs. auditor) |
| `GET` | `/reader/onchain-root` | Get on-chain root (via native RPC) |
| `POST` | `/reader/rebuild` | Rebuild/verify from chain + IPFS |

**Attester mode:**

| Method | Path | Description |
|---|---|---|
| `GET` | `/attest/status` | Attester daemon status |
| `POST` | `/attest/scan` | Scan for unattested epochs and submit attestations |
| `GET` | `/attest/attestations/:n` | List attestations for an epoch |

### Example: end-to-end flow

```bash
# 1. Start IPFS daemon
ipfs daemon &

# 2. Start the audit daemon in publisher mode (from src-tauri/)
cd src-tauri && cargo run --bin nosqlbuddy-auditd -- \
  --mode publish \
  --mongo-uri "mongodb://localhost:27017"

# 3. Write some data to MongoDB (triggers change stream events)
mongosh --eval 'db.test.insertOne({a: 1})'

# 4. Close the epoch to freeze the root
curl -X POST http://localhost:9173/epoch/close

# 5. Publish the epoch batch to IPFS
curl -X POST http://localhost:9173/epoch/0/publish-ipfs

# 6. Commit the root to Stellar testnet
curl -X POST http://localhost:9173/epoch/0/commit

# 7. Verify: check on-chain root matches local root
curl http://localhost:9173/onchain-root
curl http://localhost:9173/root

# 8. (Optional) Generate a Groth16 inclusion proof
curl -X POST http://localhost:9173/proof/0
```

## Testing

```bash
# Frontend type check and production build
npm run build

# Frontend smoke test (Playwright)
npx playwright test

# Rust linting
cd src-tauri
cargo clippy --all-targets --all-features -- -D warnings

# Rust tests
cargo test --all-targets

# Audit daemon tests only
cargo test --lib auditd

# Full audit module tests (92 tests)
cargo test --lib audit::
```

## Security

- Connection secrets are stored in the OS keychain, not in plaintext files or settings.
- Passwords and URIs are redacted from error messages, logs, and IPC responses.
- Tauri capabilities are scoped to the minimum permissions required by the main window.

### Oplog completeness protocol

The ZK audit log guarantees **integrity** (no tampering with recorded events) and **inclusion** (proofs that a specific event is in the log). The **oplog completeness** protocol adds a third guarantee: **no writes were omitted from the audit log**.

#### How it works

1. **MongoDB's oplog is the source of truth.** Under `w:"majority"`, every write is replicated to all replica members' oplogs. The operator cannot prevent an oplog entry from being created without breaking replication.

2. **Canonical serialization (`oplog-hash-v1`).** Each oplog entry is serialized to deterministic bytes using a canonical projection of stable fields (`ts`, `t`, `op`, `ns`, `ui`, `o`, `o2`, `v`) with sorted keys and explicit type tags. Two independent parties hashing the same entry always produce the same bytes.

3. **SHA-256 Merkle tree.** The canonicalized entries are hashed into a binary Merkle tree. The root captures completeness, ordering, and integrity in a single 32-byte value. Inclusion proofs work for individual entries.

4. **Majority-committed point.** Only entries up to `lastCommittedOpTime` are hashed, ensuring we commit to durable entries that won't be rolled back.

5. **On-chain commitment.** The oplog Merkle root is stored alongside the audit log root in the Soroban contract (`commit_root_with_oplog`). This binds the audit log to the oplog on-chain.

6. **Independent attester.** An independent replica member (run by the auditor/regulator) independently computes the oplog hash and submits an ed25519 attestation to the contract. This provides a durable, on-chain record that survives oplog rollover.

7. **Three-way compare.** The auditor's verification tool (`/reader/verify-oplog`) compares the on-chain oplog root with the hash computed from the independent replica. If they match, the audit log is complete. If they differ, an omission is detected.

#### Information-theoretic limit

To verify that a private log contains *all* entries from a private data source, at least one independent party must have some form of access to that source. No purely cryptographic protocol between the operator and a zero-access verifier can prove completeness — the operator can always hide entries from a verifier that never sees the source.

This is the same class of limit Satoshi faced with double-spending. His solution was not to eliminate access, but to make it universal and unbypassable via the P2P broadcast protocol. Our analog: MongoDB's replication protocol forces every write to an independent replica member under `w:"majority"`, and NoSQLBuddy turns that member's verification into a one-click product.

#### Preconditions

Two preconditions (both required, both explicit):

1. **Independent member** — at least one voting replica member the operator does not control, computing the oplog hash from its own replicated copy.
2. **Fresh attestation** — the independent member signs each epoch's oplog hash while the entries are still in the oplog. The on-chain signature is the durable guarantee that survives oplog rollover.

#### Privacy model

The public sees only hashes — the oplog Merkle root, the audit log root, and ZK proofs. No database content is leaked on-chain. The auditor/regulator sees the oplog on the independent member (they are legally entitled to this access). NoSQLBuddy's reader mode shows only the hash comparison result, not the raw oplog data.

CSFLE (Client-Side Field-Level Encryption) is an optional privacy enhancement (T2 tier) that makes the independent member see ciphertext instead of plaintext. T3 adds TEE-based observers for plaintext-capable independent compute under hardware attestation. The hackathon deliverable is T1 (base tier).

#### Secure-config tiers

| Tier | Config | What it adds |
|---|---|---|
| **T1 — Base (this project)** | Oplog binding + independent member + NoSQLBuddy reader | Deterministic completeness; auditor sees oplog on the independent member; public sees only hashes |
| **T2 — + CSFLE** | T1 + field-level encryption | Independent member sees ciphertext, not plaintext — full privacy-preserving verification |
| **T3 — + TEE** | T2 + TEE observers | Plaintext-capable independent compute under hardware attestation |

#### Honest residual assumption

There is **one** assumption: at least one independent replica member computes and signs the oplog hash honestly, while the entries are fresh. This is the Bitcoin 51% assumption equivalent. If the independent member colludes with the operator (or doesn't run), the operator can fake the oplog hash. But:

- The independent member is operated by the auditor/regulator (legally distinct from the operator).
- The member has its own replicated copy (the operator can't prevent replication under `w:"majority"`).
- The attester signs while fresh (the on-chain signature is durable).
- The verification is cryptographic (the hash comparison is binary).

#### Threat model

| Threat | Mitigation |
|---|---|
| Operator omits a write from the audit log | The oplog hash includes all writes; omitting even one changes the root, detected by the three-way compare |
| Operator serves a doctored oplog to the auditor | The auditor connects to their own independent replica member (Layer 0), not the operator's server |
| Oplog entries roll over (capped collection) | The independent attester signs each epoch's oplog hash while present, providing a durable on-chain record |
| Operator commits a different hash than their oplog contains | The on-chain oplog root is compared with the auditor's independent computation |
| Replication lag causes inconsistent views | Only entries up to the majority-committed point are hashed |

#### Running the demo

```bash
# Start the 3-member replica set
docker compose up -d

# Run the completeness demo
./scripts/oplog-completeness-demo.sh

# Or run individual tests:
cd src-tauri
cargo test --lib audit::oplog_integration -- --ignored --nocapture  # H2 determinism
cargo test --lib audit::oplog_omission -- --ignored --nocapture     # Omission detection
```

#### Known limitations and production notes

- **Replica set required.** The oplog completeness protocol reads `lastCommittedOpTime` from `hello` / `lastWrite.majorityOpTime`. Standalone `mongod` does not expose this and is not supported for on-chain oplog commitments.
- **Attester key setup.** The attester daemon generates an ed25519 signing key on first run (or reads one from `--attester-key-file`). The admin must authorize the attester's Stellar address together with that public key on the contract (`authorize_attester <address> <pubkey>`). The daemon still needs a separate Stellar CLI identity (`--attester-identity`) to sign the invoke transaction.
- **Network and replication lag.** The publisher hashes only entries up to the current majority-committed point. If replication is lagging or the publisher loses its MongoDB connection, epoch close may fail to attach an oplog hash. Use `--oplog-hash-required` to make this fail-fast, or leave it as a warning if the operator wants to close epochs manually.
- **Trust anchor.** The protocol detects an operator that omits writes from the audit log. It does not protect against an attacker who controls the MongoDB primary *and* all independent replica members simultaneously. The auditor's replica must be operated independently.
- **Testnet.** The contract and example scripts target Stellar testnet. Mainnet deployment requires funded accounts, a different `--rpc-url`, and updated contract deployment.
- **Deprecation of bare `commit_root`.** The contract still exposes `commit_root` (audit log root only) for backward compatibility. New commitments should use `commit_root_with_oplog` so every audit root is bound to an oplog completeness proof.

## Contributing

Contributions are welcome. Please open an issue or pull request with a clear description of the change, reproduction steps for bugs, and tests where possible.

## License

See the [LICENSE](./LICENSE) file for details.

## Acknowledgments

Built with [Tauri](https://tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/), and the MongoDB Rust driver.
