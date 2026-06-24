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

### CLI options

| Flag | Default | Description |
|---|---|---|
| `--mode <publish\|read>` | `publish` | Daemon mode |
| `--mongo-uri <uri>` | — | MongoDB connection URI (required for `publish`) |
| `--data-dir <dir>` | OS data dir | Data directory for audit log + sled |
| `--port <port>` | `9173` | HTTP API port |
| `--circuit-dir <dir>` | — | Circuit artifacts dir (for `/proof/:index`) |
| `--ipfs-api <url>` | `http://127.0.0.1:5001` | IPFS Kubo HTTP API URL |
| `--rpc-url <url>` | Stellar testnet | Soroban RPC URL |
| `--epoch-threshold <n>` | `100` | Auto-close epoch after N events (0=disabled) |
| `--epoch-time-secs <s>` | `0` | Auto-close epoch after S seconds (0=disabled) |
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
| `GET` | `/epochs` | List all epochs (open and closed) |
| `GET` | `/epoch/current` | Get the current open epoch |
| `POST` | `/epoch/close` | Close current epoch and freeze its root |
| `POST` | `/epoch/:n/commit` | Commit epoch root to Stellar testnet |
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
| `GET` | `/reader/onchain-root` | Get on-chain root (via native RPC) |
| `POST` | `/reader/rebuild` | Rebuild/verify from chain + IPFS |

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

## Contributing

Contributions are welcome. Please open an issue or pull request with a clear description of the change, reproduction steps for bugs, and tests where possible.

## License

See the [LICENSE](./LICENSE) file for details.

## Acknowledgments

Built with [Tauri](https://tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/), and the MongoDB Rust driver.
