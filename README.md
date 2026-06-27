# NoSQLBuddy

A cross-platform MongoDB management studio for developers, SREs, and database engineers who work across local, staging, and production environments.

NoSQLBuddy connects to MongoDB, lets you browse data, run queries, build aggregations, translate SQL to MongoDB, inspect schema and indexes, and review performance — all from a native desktop app built with Tauri, Rust, React, and TypeScript.

## Contents

- [Quickstart](#quickstart) — run the app and connect in minutes
- [Features](#features)
- [Using NoSQLBuddy](#using-nosqlbuddy) — how to use the core features
- [ZK Audit Log](#zk-audit-log) — [Dev Mode](#dev-mode-full-stack-locally) · [Production Mode](#production-mode-in-app-your-keys)
- [Standalone audit service](#standalone-audit-service-nosqlbuddy-audit) (advanced / reference)
- [Security](#security)
- [Testing](#testing)

## Quickstart

Get the app running and connected to MongoDB in a few minutes.

1. **Install dependencies:**
   ```bash
   git clone https://github.com/ronnakamoto/nosql-buddy.git
   cd nosql-buddy
   npm install
   ```

2. **Get a MongoDB to connect to.** Already have one (local, Atlas, remote)? Use it. Otherwise start a local single-node replica set (requires Docker):
   ```bash
   docker compose up -d   # MongoDB at localhost:27017, seeded with demo data
   ```

3. **Launch NoSQLBuddy:**
   ```bash
   npm run tauri dev
   ```

4. **Connect.** In the app, click **New Connection**, paste a connection string, and click **Connect**:
   - Local dev DB: `mongodb://localhost:27017/?replicaSet=rs0`
   - Atlas / remote: your own `mongodb://…` or `mongodb+srv://…` URI

5. **Explore** your data — see [Using NoSQLBuddy](#using-nosqlbuddy).

> Want the tamper-evident, on-chain audit log? See [ZK Audit Log](#zk-audit-log).

Prerequisites: [Node.js](https://nodejs.org/) 18+, [Rust](https://www.rust-lang.org/tools/install) 1.77+, and (optional) [Docker Desktop](https://www.docker.com/products/docker-desktop/) for the local database and audit stack. Full details under [Getting started](#getting-started).

## Features

- **Connection management** — Save connection profiles with secrets stored in the OS keychain. URIs and credentials are redacted from logs and UI responses.
- **Data browsing** — Query collections, paginate results, edit documents in place, and view JSON or table output.
- **Visual query builder** — Compose filters, projections, and sorts without writing raw JSON.
- **Aggregation editor** — Build and preview aggregation pipelines with syntax-aware JSON editing.
- **SQL to MongoDB** — Translate `SELECT`, `JOIN`, `GROUP BY`, `WHERE`, `ORDER BY`, and `LIMIT` statements into aggregation pipelines.
- **Schema and index analysis** — Infer schema shape, cardinality, and index usage from sampled documents.
- **Explain plan visualization** — Parse `explain` output into a navigable tree to diagnose slow queries.
- **Driver code generation** — Export queries and pipelines to Node.js, Python, Java, C#, Ruby, Rust, and the MongoDB shell.
- **ZK audit log** — Tamper-evident Poseidon Merkle tree, Groth16 inclusion proofs, epoch batching with IPFS publishing and Stellar on-chain commitments (testnet or mainnet), multi-publisher K-of-N threshold attestation, and reader-mode verification against on-chain roots. Two modes: **Dev Mode** (full stack locally via Docker) and **Production Mode** (in-app pipeline with your own keys).
- **Oplog completeness** — Deterministic SHA-256 Merkle tree over MongoDB's oplog (`local.oplog.rs`), binding the audit log to the same ground truth that MongoDB's replication protocol uses. An independent replica member (run by the auditor/regulator) provides a trust anchor that detects any omitted writes. The on-chain commitment stores both the audit log root and the oplog root, and independent attesters submit ed25519 attestations over the oplog root for durable, post-rollover verification.
- **Standalone audit service** — `nosqlbuddy-audit` runs independently of the desktop app, capturing MongoDB change stream events, batching into epochs, publishing to IPFS, and committing Merkle roots on-chain via an HTTP API. Signs transactions natively (ed25519 + Soroban RPC) — no `stellar` CLI required. Includes an interactive `setup` wizard for one-command key generation, contract deployment, and attester authorization.
- **Native desktop experience** — Built on Tauri v2 for a small footprint, native menus, and consistent shortcuts on macOS, Windows, and Linux.

## Tech stack

- **Frontend:** React 18, TypeScript, Vite, visx
- **Backend:** Rust, Tauri v2, Tokio
- **Database:** MongoDB driver for Rust (`mongodb` + `bson`)
- **ZK proofs:** ark-circom, ark-groth16, ark-bn254, light-poseidon, circom circuits
- **On-chain:** Soroban (Stellar testnet + mainnet), native Rust RPC client, native ed25519 transaction signing
- **Decentralized storage:** IPFS (Kubo HTTP API)
- **Audit daemon:** axum HTTP server (standalone binary)
- **Testing:** Playwright (frontend), Cargo (Rust unit + integration tests)

## Getting started

### Prerequisites

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://www.rust-lang.org/tools/install) 1.77+
- A MongoDB instance (local or remote) for live features. No instance handy? `docker compose up -d` starts a **single-node replica set** reachable at `mongodb://localhost:27017/?replicaSet=rs0` (always primary, no extra host configuration).

#### Audit daemon (optional)

- A MongoDB **replica set** or **sharded cluster** (change streams require oplog; standalone mongod won't work)
- A Stellar **secret key** (S... strkey) for signing on-chain transactions. Generate one with `stellar keys generate --global <name> --network testnet` then export it with `stellar keys show <name> --secret-key`. The daemon signs transactions natively — no `stellar` CLI needed at runtime.
- [IPFS](https://docs.ipfs.tech/install/) for batch publishing — either a local Kubo daemon (`ipfs daemon`) or a [Pinata](https://pinata.cloud) account (configured in-app during onboarding)
- Circuit artifacts for Groth16 proof generation: `merkle_inclusion.r1cs` + `merkle_inclusion.wasm` (bundled in `src-tauri/resources/circuits/`)

#### Dev Mode Docker stack (optional)

- [Docker Desktop](https://www.docker.com/products/docker-desktop/) with the Compose plugin
- The 3-node MongoDB replica set running (`docker compose -f docker-compose.audit-db.yml up -d` from the project root). The default `docker compose up -d` starts the single-node dev database instead; the audit features need the 3-node set for the independent attester member.

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

## Using NoSQLBuddy

After you connect (see [Quickstart](#quickstart)), the main workspace puts the core tools one click away:

- **Browse and edit data** — Pick a database and collection in the sidebar to page through documents. Switch between JSON and table views, and edit a document inline then save it back.
- **Run queries** — Type a MongoDB filter in the query bar, or use the **visual query builder** to compose filter, projection, and sort without writing raw JSON.
- **Build aggregations** — Open the aggregation editor to assemble a pipeline stage by stage and preview results as you go.
- **Translate SQL** — Write a `SELECT … FROM … WHERE … JOIN … GROUP BY … ORDER BY … LIMIT` statement and NoSQLBuddy converts it into an aggregation pipeline you can run or copy.
- **Inspect schema and indexes** — Sample a collection to infer its field shape, cardinality, and which indexes are used.
- **Diagnose slow queries** — Run a query with explain to get a navigable execution-plan tree.
- **Generate driver code** — Export any query or pipeline as ready-to-paste code for Node.js, Python, Java, C#, Ruby, Rust, or the mongo shell.

Connection profiles are saved with their secrets in your OS keychain; URIs and credentials are redacted from logs and responses.

## ZK Audit Log

NoSQLBuddy can produce a tamper-evident, independently verifiable audit log of every database write: Merkle-tree proofs committed on-chain (Stellar), batches stored on IPFS, and an independent attester that detects omitted writes.

There are two ways to run it (the Audit tab shows both as cards). Pick based on what you're trying to do:

| | Dev Mode | Production Mode |
|---|---|---|
| **Runs where** | Full stack in Docker on your machine | In-app pipeline — no Docker, no daemons |
| **Stellar keys** | Two testnet keys you generate (publisher + attester) | Your own keypair |
| **Contract** | Bundled testnet contract | Auto-funded on testnet, or your own on mainnet |
| **Network** | Testnet | Testnet or mainnet |
| **MongoDB** | 3-node replica set (`docker-compose.audit-db.yml`) | Your own replica set / cluster |
| **Best for** | Learning and demoing the full system (publisher + independent attester + reader, K-of-N, oplog completeness) | Auditing your real data with keys and a contract you control |

**New to this?** Start with **Dev Mode** to watch the whole system work end to end, then move to **Production Mode** with your own keys.

### Dev Mode (full stack locally)

Runs the **complete audit system** on your machine via Docker — publisher, independent attester, and reader daemons — with K-of-N attestation, oplog completeness verification, and on-chain commitments to Stellar testnet.

**Use this when:** you want to see or demo the entire audit system working end to end, without deploying anything of your own.

**Prerequisites:** Docker Desktop, the 3-node MongoDB replica set running, and a [Pinata](https://pinata.cloud) account (or local IPFS) for batch publishing.

**Steps:**

1. Start the 3-node MongoDB replica set (from the project root):
   ```bash
   docker compose -f docker-compose.audit-db.yml up -d
   ```

2. Run the setup wizard (interactive, in Docker). This one command does all the key work for you — it generates the two independent Stellar keypairs (publisher + attester), funds them on testnet via Friendbot, uses the bundled testnet contract, generates the attester's ed25519 oplog key, authorizes the attester on the contract, and writes `./attester.key` and `.env.audit` into the project root:
   ```bash
   docker compose -f docker-compose.audit.yml run --build --rm setup
   ```
   Press Enter to accept the defaults (testnet, generate both keys, bundled contract, `./attester.key`, `.env.audit`) and enter your Pinata API key/secret when prompted. The wizard runs in a container with the project root mounted, so the files land exactly where the audit stack expects them: `docker-compose.audit.yml` mounts `./attester.key` and reads `.env.audit`.

   > **No `stellar` CLI needed for Dev Mode.** The wizard funds accounts and authorizes the attester with native signing against the bundled testnet contract. The CLI is only required if you choose to *deploy* a brand-new contract, which Dev Mode doesn't.
   >
   > **Why two keys?** The trust model requires the attester to be independent from the operator. If both used the same key, the operator could submit fake attestations themselves, defeating independent verification — so the wizard generates two separate keypairs.

3. Open the app, go to the Audit tab, select **Dev Mode**, and click **Start Stack** (this launches the publisher, attester, and reader containers using the `.env.audit` and `./attester.key` the wizard produced).

4. The live view shows: event feed, epoch progress, on-chain root, K-of-N attestation status, oplog completeness verification, and epoch history — all querying the Docker daemons in real time.

5. Click **Commit Now** to close the epoch, pin to IPFS, and commit the root on-chain.

**Manual Docker commands** (alternative to the in-app Start Stack button):
```bash
docker compose -f docker-compose.audit.yml up -d    # start the audit stack (add --build on first run)
docker compose -f docker-compose.audit.yml ps       # check status
docker compose -f docker-compose.audit.yml logs -f  # tail logs
docker compose -f docker-compose.audit.yml down     # stop the stack
```

### Production Mode (in-app, your keys)

Runs the in-app audit pipeline with **your own Stellar keypair** and contract. Choose testnet or mainnet — this is the "double check" that an audit system you deployed elsewhere works end to end. No daemon, no Docker.

**Use this when:** you want to audit your real data with keys and a contract you control — no Docker, no background daemons.

**What you need:**
- Your Stellar secret key (`S…`). On **testnet** the app auto-funds a fresh contract for you, so the key alone is enough to try it. On **mainnet** you also need your deployed contract ID (`C…`) and an RPC URL.
- A MongoDB **replica set** connection (change streams and oplog require it; a standalone `mongod` won't work).

**Steps:**

1. Open the app, go to the Audit tab, select **Production Mode**.

2. Choose a network: **Testnet** (auto-funded contract) or **Mainnet** (your contract ID + RPC URL).

3. Import your Stellar secret key (S... strkey). It's stored in the OS keychain and never leaves your machine.

4. If mainnet: enter your contract ID (C...) and RPC URL.

5. The live view shows: event feed, epoch progress, on-chain root, verify integrity, per-event proofs, and advanced details — committing via your keypair on your chosen network.

6. Click **Commit Now** to close the epoch, pin to IPFS, and commit the root on-chain via native signing.

**Switching modes:** Click **Settings** in the audit panel, then toggle between Dev and Production. The panel re-routes immediately.

## Standalone audit service (`nosqlbuddy-audit`)

> **Advanced / reference.** Most users don't need this — [Dev Mode](#dev-mode-full-stack-locally) and [Production Mode](#production-mode-in-app-your-keys) above cover the common workflows. This section documents running the audit daemon directly (CLI flags, HTTP API, and the end-to-end protocol).

The audit service runs as a separate process from the desktop app. It captures MongoDB writes via change streams, builds a tamper-evident Poseidon Merkle tree, batches events into epochs, publishes batches to IPFS, and commits Merkle roots to a Soroban contract on Stellar.

### Build

```bash
cd src-tauri
cargo build --bin nosqlbuddy-audit
```

### Run via Docker Compose (alternative to building from source)

If you don't want to install the Rust toolchain, you can run the full audit stack (publisher + attester + reader) via Docker. This uses the same binary, containerized with the Dockerfile at `audit-service/Dockerfile.audit`.

**Prerequisites:** Docker Desktop + the 3-node MongoDB replica set running.

1. Start the 3-node MongoDB replica set (from the project root):
   ```bash
   docker compose -f docker-compose.audit-db.yml up -d
   ```

2. Run the setup wizard (interactive, in Docker). It generates the publisher + attester keypairs, funds them on testnet, uses the bundled testnet contract, generates and authorizes the attester's ed25519 oplog key, and writes `./attester.key` + `.env.audit` into the project root:
   ```bash
   docker compose -f docker-compose.audit.yml run --build --rm setup
   ```
   Accept the defaults and enter your Pinata API key/secret when prompted. See [Dev Mode](#dev-mode-full-stack-locally) for more detail.

3. Start the full audit stack:
   ```bash
   docker compose -f docker-compose.audit.yml up -d --build
   ```

   This brings up three containers:

   | Service | Port | Mode | Connects to | Role |
   |---|---|---|---|---|
   | `publisher` | 9173 | publish | mongo1 (port 27017) | Captures change stream events, manages epochs, publishes to IPFS, commits roots on-chain |
   | `attester` | 9174 | attest | mongo3 (port 27019) | Independently computes oplog hash, submits ed25519 attestations to the contract |
   | `reader` | 9175 | read | mongo3 (port 27019) | Verifies on-chain roots against local audit log and oplog |

4. Manage the stack:
   ```bash
   docker compose -f docker-compose.audit.yml ps       # check status
   docker compose -f docker-compose.audit.yml logs -f  # tail all logs
   docker compose -f docker-compose.audit.yml logs -f publisher  # tail one service
   docker compose -f docker-compose.audit.yml down     # stop the stack (run this before restarting)
   ```

> **Restarting:** Always run `docker compose -f docker-compose.audit.yml down` before `up` if you made changes to `.env.audit` or the Dockerfile. Use `--build` on first run or after code changes.
>
> **Tip:** The audit stack uses a separate Compose project name (`nosqlbuddy-audit`) so it won't trigger orphan-container warnings from the MongoDB replica set. The audit containers reach the replica set over the shared `mongo-net` Docker network by container name (`mongo1`, `mongo2`, `mongo3`), so start the DB with `docker compose -f docker-compose.audit-db.yml up -d` first. The `host.docker.internal` mapping is only used to reach host services such as a local IPFS daemon.

### Setup wizard (one-time)

The `setup` subcommand is an interactive wizard that generates Stellar keypairs, optionally deploys the contract, initializes it, authorizes the attester, and writes `.env.audit`. Run it in Docker (recommended — no Rust toolchain required) from the project root, so the project directory is mounted and the generated `./attester.key` + `.env.audit` land where the audit stack reads them:

```bash
docker compose -f docker-compose.audit.yml run --build --rm setup
```

<details>
<summary>From source (requires the Rust toolchain)</summary>

```bash
# Run from the project root so ./attester.key and .env.audit land there.
cargo run --bin nosqlbuddy-audit -- setup
```
</details>

The wizard walks you through:
1. Choosing a network (testnet or mainnet)
2. Generating (or importing) publisher + attester Stellar keypairs
3. Deploying a new contract or using an existing one
4. Initializing the contract (sets the admin = publisher)
5. Generating the attester's ed25519 oplog signing key
6. Authorizing the attester on the contract
7. Entering Pinata IPFS credentials (optional)
8. Writing `.env.audit` with all values

> **Contract deployment** (deploying a brand-new contract) requires the `stellar` CLI and is only available via the from-source wizard — the Docker image doesn't bundle the CLI. Dev Mode and the default flow use the bundled testnet contract, so the Docker wizard is all you need. The `initialize` and `authorize_attester` calls use native signing — no CLI required.
>
> **Setting up Dev Mode?** This is the recommended one-command path — see [Dev Mode](#dev-mode-full-stack-locally).

### Start the service

**Publisher mode** — captures writes, manages epochs, publishes to IPFS, commits roots on-chain:

```bash
# All commands run from src-tauri/
cd src-tauri

# Basic: connect to MongoDB and listen for changes (no on-chain commits without a key)
cargo run --bin nosqlbuddy-audit -- start \
  --mode publish \
  --mongo-uri "mongodb://localhost:27017"

# Full: with IPFS publishing, native Stellar signing, and proof generation
cargo run --bin nosqlbuddy-audit -- start \
  --mode publish \
  --mongo-uri "mongodb://localhost:27017" \
  --secret-key SXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX \
  --network testnet \
  --circuit-dir ./resources/circuits \
  --ipfs-api http://127.0.0.1:5001 \
  --rpc-url https://soroban-testnet.stellar.org:443
```

> **Tip:** You can also pass the secret key via the `STELLAR_SECRET_KEY` environment variable instead of `--secret-key`.

**Reader mode** — verifies local audit log against on-chain commitments (no MongoDB connection needed):

```bash
cd src-tauri
cargo run --bin nosqlbuddy-audit -- start \
  --mode read \
  --data-dir ~/.local/share/nosqlbuddy-audit
```

**Attester mode** — independent attester that connects to the independent replica member, watches for new epoch commitments on-chain, independently computes the oplog hash, and submits attestations to the contract:

```bash
cd src-tauri
cargo run --bin nosqlbuddy-audit -- start \
  --mode attest \
  --mongo-uri "mongodb://localhost:27019" \
  --attester-secret-key SXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX \
  --network testnet \
  --rpc-url https://soroban-testnet.stellar.org:443
```

> **Tip:** The attester's Stellar account key (`--attester-secret-key`) is separate from the ed25519 oplog signing key (auto-generated at `--attester-key-file`). The Stellar key signs the on-chain `attest_oplog` transaction; the ed25519 key signs the oplog hash itself. You can also pass the Stellar key via the `ATTESTER_SECRET_KEY` environment variable.

### Stop and status

```bash
# Stop a running service
cargo run --bin nosqlbuddy-audit -- stop

# Check if the service is running + health check
cargo run --bin nosqlbuddy-audit -- status

# Stop/status with a custom data dir or port
cargo run --bin nosqlbuddy-audit -- stop --data-dir /path/to/data --port 9174
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
| `--network <testnet\|mainnet>` | `testnet` | Stellar network (sets passphrase + Horizon URL) |
| `--contract-id <C...>` | Testnet contract | Soroban contract ID (required for mainnet) |
| `--horizon-url <url>` | Testnet Horizon | Horizon API URL for account sequence lookups |
| `--secret-key <S...>` | — | Publisher's Stellar secret key (native signing). Also reads `STELLAR_SECRET_KEY` env var |
| `--attester-secret-key <S...>` | — | Attester's Stellar account secret key (native signing). Also reads `ATTESTER_SECRET_KEY` env var |
| `--epoch-threshold <n>` | `100` | Auto-close epoch after N events (0=disabled) |
| `--epoch-time-secs <s>` | `0` | Auto-close epoch after S seconds (0=disabled) |
| `--oplog-hash-required` | — | Fail epoch close if oplog hash computation fails |
| `--attester-key-file <path>` | `<data-dir>/audit/attester.key` | Attester ed25519 oplog signing key (attest mode; generated if missing) |
| `--attester-identity <name>` | — | **Deprecated.** Stellar CLI identity for attester transactions. Use `--attester-secret-key` instead |
| `--attester-address <addr>` | — | Stellar address of the attester (attest mode; derived from keypair if not set) |
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
cd src-tauri && cargo run --bin nosqlbuddy-audit -- start \
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

# Audit service tests only
cargo test -p nosqlbuddy-audit-service

# Full audit module tests
cargo test -p nosqlbuddy-audit-service --all-targets
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
docker compose -f docker-compose.audit-db.yml up -d

# Run the completeness demo
./scripts/oplog-completeness-demo.sh

# Or run individual tests:
cd src-tauri
cargo test --lib audit::oplog_integration -- --ignored --nocapture  # H2 determinism
cargo test --lib audit::oplog_omission -- --ignored --nocapture     # Omission detection
```

#### Known limitations and production notes

- **Replica set required.** The oplog completeness protocol reads `lastCommittedOpTime` from `hello` / `lastWrite.majorityOpTime`. Standalone `mongod` does not expose this and is not supported for on-chain oplog commitments.
- **Attester key setup.** The attester daemon generates an ed25519 signing key on first run (or reads one from `--attester-key-file`). The admin must authorize the attester's Stellar address together with that public key on the contract (`authorize_attester <address> <pubkey>`). The daemon signs the `attest_oplog` transaction natively using the Stellar keypair from `--attester-secret-key` (or the `ATTESTER_SECRET_KEY` env var) — no `stellar` CLI needed.
- **Native signing.** The daemon signs Stellar transactions natively (ed25519 + Soroban RPC simulation + submission). Pass the secret key via `--secret-key` (publisher) or `--attester-secret-key` (attester), or via the `STELLAR_SECRET_KEY` / `ATTESTER_SECRET_KEY` environment variables. The legacy `stellar` CLI fallback (`--attester-identity`) still works but is deprecated.
- **Network and replication lag.** The publisher hashes only entries up to the current majority-committed point. If replication is lagging or the publisher loses its MongoDB connection, epoch close may fail to attach an oplog hash. Use `--oplog-hash-required` to make this fail-fast, or leave it as a warning if the operator wants to close epochs manually.
- **Trust anchor.** The protocol detects an operator that omits writes from the audit log. It does not protect against an attacker who controls the MongoDB primary *and* all independent replica members simultaneously. The auditor's replica must be operated independently.
- **Testnet and mainnet.** The bundled contract ID targets Stellar testnet. For mainnet, pass `--network mainnet --contract-id <your-contract-id> --rpc-url <your-mainnet-rpc>` and ensure your account is funded. The desktop app's Production Mode supports both networks.
- **Deprecation of bare `commit_root`.** The contract still exposes `commit_root` (audit log root only) for backward compatibility. New commitments should use `commit_root_with_oplog` so every audit root is bound to an oplog completeness proof.

## Contributing

Contributions are welcome. Please open an issue or pull request with a clear description of the change, reproduction steps for bugs, and tests where possible.

## License

See the [LICENSE](./LICENSE) file for details.

## Acknowledgments

Built with [Tauri](https://tauri.app/), [React](https://react.dev/), [Vite](https://vitejs.dev/), and the MongoDB Rust driver.
