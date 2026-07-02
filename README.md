# NoSQLBuddy

A cross-platform MongoDB management studio for developers, SREs, and database engineers who work across local, staging, and production environments.

NoSQLBuddy connects to MongoDB, lets you browse data, run queries, build aggregations, translate SQL to MongoDB, inspect schema and indexes, and review performance — all from a native desktop app built with Tauri, Rust, React, and TypeScript.

## Contents

- [Quickstart](#quickstart) — run the app and connect in minutes
- [No MongoDB yet?](#no-mongodb-yet-run-the-seeded-demo-database) — run a seeded local database
- [Features](#features)
- [Using NoSQLBuddy](#using-nosqlbuddy) — how to use the core features
- [ZK Audit Log](#zk-audit-log) — how the audit system works, roles, and modes
  - [ZK implementation](#zk-implementation-circuit-prover-and-on-chain-verifier) — circuit, prover, and on-chain verifier
  - [On-chain contract reference](#on-chain-contract-reference)
  - [Dev Mode](#dev-mode-full-stack-locally) · [Production Mode](#production-mode-in-app-your-keys)
  - [Audit domains and selective disclosure](#audit-domains-and-selective-disclosure-desktop-app)
- [Standalone audit service](#standalone-audit-service-nosqlbuddy-audit) — CLI, HTTP API, and protocol reference
- [Deploying the audit service to a server](#deploying-the-audit-service-to-a-server) — separated publisher/attester deployment
- [Security](#security) — oplog completeness protocol, threat model, and privacy tiers
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
   New to MongoDB or don't have one running? See [No MongoDB yet?](#no-mongodb-yet-run-the-seeded-demo-database) for what this starts and how to manage it.

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

## No MongoDB yet? Run the seeded demo database

NoSQLBuddy doesn't bundle a database — point it at any MongoDB you already have (local, Atlas, or remote). If you don't have one, this repo ships a one-command local database preloaded with realistic demo data, so you have something to browse right away.

Requires [Docker Desktop](https://www.docker.com/products/docker-desktop/). From the project root:

```bash
docker compose up -d
```

This starts a **single-node MongoDB replica set** at `localhost:27017` and seeds it with a sample e-commerce dataset (the `shopkeeper` database):

| Collection | Contents |
|---|---|
| `products` | 12 products with price, cost, stock, ratings, tags, and nested specs |
| `categories` | 5 product categories |
| `customers` | 5 customers with nested name and address objects |
| `orders` | 6 orders across delivered / shipped / processing / pending / cancelled states |
| `inventory_log` | Stock movements from orders and supplier restocks |

Unique, text, and compound indexes are created too, so schema/index analysis and explain plans have something to work with.

Connect the app with this URI (the **New Connection** dialog):

```
mongodb://localhost:27017/?replicaSet=rs0
```

A single-member replica set is always its own primary, so writes never fail with `NotWritablePrimary`, and it still supports change streams and transactions — everything NoSQLBuddy's live features (including the audit log) need. No `/etc/hosts` aliases or `directConnection` tricks required.

Manage it from the project root:

```bash
docker compose ps                # status
docker compose logs -f           # tail logs
docker compose down              # stop (data is kept in a named volume)
docker compose down -v           # stop and wipe all data
docker compose run --rm seeder   # re-seed the demo data
```

> This single-node dev database is **separate** from the 3-node replica set used by the audit trust-anchor demo (`docker-compose.audit-db.yml`). For everyday app use, the single-node DB above is all you need.

## Features

- **Connection management** — Save connection profiles with secrets stored in the OS keychain. URIs and credentials are redacted from logs and UI responses.
- **Data browsing** — Query collections, paginate results, edit documents in place, and view JSON or table output.
- **Visual query builder** — Compose filters, projections, and sorts without writing raw JSON.
- **Aggregation editor** — Build and preview aggregation pipelines with syntax-aware JSON editing.
- **SQL to MongoDB** — Translate `SELECT`, `JOIN`, `GROUP BY`, `WHERE`, `ORDER BY`, and `LIMIT` statements into aggregation pipelines.
- **Schema and index analysis** — Infer schema shape, cardinality, and index usage from sampled documents.
- **Explain plan visualization** — Parse `explain` output into a navigable tree to diagnose slow queries.
- **Driver code generation** — Export queries and pipelines to Node.js, Python, Java, C#, Ruby, Rust, and the MongoDB shell.
- **ZK audit log** — Tamper-evident Poseidon Merkle tree, Groth16 inclusion proofs, epoch batching with IPFS publishing and Stellar on-chain commitments (testnet or mainnet), multi-publisher K-of-N threshold attestation, and reader-mode verification against on-chain roots. Two modes: **Dev Mode** (full stack locally via Docker, available now) and **Production Mode** (in-app pipeline with your own keys, coming soon).
- **Audit domains & selective disclosure** — Events are segmented per `(deployment, database)` domain, each with its own secondary Merkle root, so you can prove one tenant's record without revealing any other domain's data. An aggregation **super-root** over all domain roots (anchored in the on-chain commit metadata) lets you prove a domain is part of the committed state, and per-domain **legal hold** and **retention/pruning** manage lifecycle while keeping the anchored history intact and verifiable.
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
- The 3-node MongoDB replica set is started automatically by the in-app **Start Stack** button (which runs `docker-compose.audit-db.yml` + `docker-compose.audit.yml` together). You can also start it manually (`docker compose -f docker-compose.audit-db.yml up -d`) if you want to inspect it before launching the audit services.

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

NoSQLBuddy can produce a tamper-evident, independently verifiable audit log of every database write: every insert, update, and delete is captured into a cryptographic Merkle tree, sealed into batches, and anchored on the Stellar blockchain so no one can alter history undetected.

### How it works (the short version)

1. **Capture.** Every MongoDB write (insert, update, delete) is recorded into a tamper-evident Poseidon Merkle tree.
2. **Seal.** Writes are grouped into an "epoch" (batch). Sealing the epoch freezes its Merkle root, a single fingerprint that commits to every event in the batch.
3. **Anchor.** The Merkle root is committed on-chain (Stellar blockchain) and the full batch is stored on IPFS. This makes the fingerprint public and permanent.
4. **Prove.** For any individual record, you can generate a zero-knowledge inclusion proof that it is part of a sealed batch, without revealing any other record.
5. **Verify.** Anyone can independently verify that a record is in the on-chain log, and that no writes were omitted.

### ZK implementation: circuit, prover, and on-chain verifier

The zero-knowledge proof system is the core of the audit log's integrity guarantee. This section explains the three components: the Circom circuit, the off-chain prover, and the on-chain Soroban verifier.

#### 1. The circuit (Circom + Poseidon)

**File:** [`zk-spike/circuits/merkle_inclusion.circom`](zk-spike/circuits/merkle_inclusion.circom)

The circuit proves **Merkle inclusion**: that a specific leaf (an audit entry) is part of a Merkle tree whose root is publicly committed on-chain. The tree uses **Poseidon(2)** — a ZK-friendly hash with `t=3` (2 inputs + 1 capacity field) — matching `light-poseidon`'s `new_circom(2)` on the Rust side and Circom's `Poseidon(2)` on the circuit side. Poseidon is the hash function Stellar added as a **native host function in Protocol 25**, making it the natural choice for ZK proofs that verify on Stellar.

| Input | Visibility | Description |
|---|---|---|
| `root` | **Public** | The Merkle tree root (committed on-chain via Soroban) |
| `leaf` | Private | The audit entry hash being proven included |
| `pathElements[20]` | Private | Sibling hashes at each level of the tree |
| `pathIndices[20]` | Private | Direction bits (0 = left child, 1 = right), constrained to {0, 1} |

The tree is 20 levels deep, supporting up to 2²⁰ ≈ 1M entries per epoch. The circuit reconstructs the root from the leaf and authentication path, and constrains the output to equal the public `root`. If the proof verifies, a judge knows the leaf is in the committed tree — without learning which leaf, or anything about any other entry.

#### 2. The prover (off-chain: ark-circom + ark-groth16)

**Files:** [`zk-audit/src/prover.rs`](zk-audit/src/prover.rs), [`zk-audit/src/merkle.rs`](zk-audit/src/merkle.rs), [`zk-audit/src/bin/ceremony.rs`](zk-audit/src/bin/ceremony.rs)

Proof generation runs off-chain in Rust:

1. **Witness computation** — `ark-circom` loads the compiled circuit (`merkle_inclusion.r1cs` + `.wasm`) and computes the witness via Wasmer, given the leaf and its authentication path.
2. **Groth16 proof** — `ark-groth16` on the BN254 curve generates a zero-knowledge proof that the witness satisfies the circuit's constraints. The proof is a triple of elliptic-curve points (A ∈ G1, B ∈ G2, C ∈ G1).
3. **Trusted setup** — The `zk-audit-ceremony` binary runs the Powers of Tau ceremony once to produce a stable proving key (`.pkey`) and verifying key (`.vkey`) in arkworks binary format. The proving key is reused for every proof; the verifying key is deployed with the contract.

```
audit entry → Poseidon hash → leaf in Merkle tree → authentication path
  → Circom witness → Groth16 proof (A, B, C on BN254)
```

#### 3. The on-chain verifier (Soroban + BN254 host functions)

**File:** [`zk-audit/soroban-contract/src/lib.rs`](zk-audit/soroban-contract/src/lib.rs) — function `verify_inclusion`

The Soroban contract verifies Groth16 proofs **on-chain** using Stellar's native BN254 host functions, introduced in **Protocol 25 ("X-Ray")** and expanded in **Protocol 26 ("Yardstick")**. These host functions move the heavy elliptic-curve math into the protocol layer, making on-chain proof verification affordable enough to run for every batch.

The verification algorithm:

1. **Check the root was committed** — the contract looks up the root in its append-only on-chain log. You can't verify a proof against a root that was never anchored.
2. **Deserialize proof and verifying key** — points are received as raw big-endian bytes (G1: 64 bytes, G2: 128 bytes), matching the off-chain serializer in [`zk-audit/src/serialize.rs`](zk-audit/src/serialize.rs).
3. **Compute `vk_x`** — `vk_x = ic[0] + Σ pub_signals[i] · ic[i+1]` using `bn254.g1_mul` and `bn254.g1_add` (Protocol 25 host functions).
4. **Pairing check** — verifies the Groth16 pairing equation:
   ```
   e(-A, B) · e(α, β) · e(vk_x, γ) · e(C, δ) == 1
   ```
   via `bn254.pairing_check` (Protocol 25/26 host function). If the pairing holds, the proof is valid.

The contract uses these Soroban BN254 host functions: `g1_mul`, `g1_add`, `pairing_check`, and `Fr` field arithmetic — all from Protocol 25/26. This is what makes on-chain Groth16 verification cost-effective on Stellar.

#### What is zero-knowledge about this

The proof reveals **nothing** about the private inputs. An auditor verifying the proof on-chain learns only:

- A specific leaf exists in the committed Merkle tree (inclusion)
- The root matches the on-chain commitment (integrity)

They do **not** learn:

- Which audit entry the leaf corresponds to
- Any sibling hashes or tree structure
- Any document content, field names, or data from the database

The database content never leaves the operator's infrastructure. Only a 32-byte hash (the root) and a zero-knowledge proof go on-chain. This is the core ZK value: prove the audit log is complete and correct, without revealing the data being audited.

### On-chain contract reference

The verifier contract is deployed on **Stellar testnet**:

```
Contract ID: CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5
```

You can inspect committed roots and transactions on [stellar.expert](https://stellar.expert/explorer/testnet/contract/CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5). The contract exposes:

| Function | What it does |
|---|---|
| `commit_root(root, metadata)` | Anchor a Merkle root on-chain (admin-gated) |
| `commit_root_with_oplog(root, oplog_root, metadata)` | Anchor root + oplog completeness hash |
| `verify_inclusion(root, proof, vk)` | **Verify a Groth16 proof on-chain** via BN254 host functions |
| `get_current_root()` | Read the latest committed root |
| `attest_oplog(epoch, oplog_root, signature)` | Independent attester signs the oplog hash |

To independently verify a proof: obtain a Groth16 proof and verifying key from the prover (see the [example flow](#example-end-to-end-flow) below), call `verify_inclusion` on the contract with the committed root, and the BN254 pairing check runs on-chain. The full contract source is in [`zk-audit/soroban-contract/`](zk-audit/soroban-contract/), with interface documentation in [`INTERFACE.md`](zk-audit/soroban-contract/INTERFACE.md).

> **Dev Mode deploys your own contract.** When you run Dev Mode setup, the wizard deploys a fresh per-user contract on testnet (your publisher key becomes admin). The contract ID above is the shared default; your Dev Mode instance will have its own. Check **Advanced → Your contract** in the Audit tab for the active ID.

### Roles: publisher vs attester

The trust model requires two independent parties. No single party should hold both roles:

| Role | Run by | What it does |
|---|---|---|
| **Publisher** (operator) | The company being audited | Captures writes, builds the Merkle tree, seals batches, anchors roots on-chain |
| **Attester** (auditor/regulator) | An independent third party | Independently verifies the oplog (MongoDB's replication log) and signs an on-chain attestation that no writes were omitted |
| **Reader** | Either party | Verifies on-chain roots against a local copy of the audit log |

> **Why separate keys?** If the publisher could also submit attestations, they could omit writes and self-attest the doctored log. Separating the keys across organizations makes this impossible. Dev Mode generates both keys on one machine for convenience; production deployments separate them across servers (see [Deploying the audit service to a server](#deploying-the-audit-service-to-a-server)).

### Two ways to run it

The Audit tab offers two modes:

| | Dev Mode | Production Mode |
|---|---|---|
| **Status** | Available | Coming soon |
| **Runs where** | Full stack in Docker on your machine | In-app pipeline — no Docker, no daemons |
| **Stellar keys** | Two testnet keys you generate (publisher + attester) | Your own keypair |
| **Contract** | Auto-deployed per-user contract on testnet (owned by your publisher key) | Auto-deployed per-user contract on testnet (owned by your key), or your own on mainnet |
| **Network** | Testnet | Testnet or mainnet |
| **MongoDB** | 3-node replica set (`docker-compose.audit-db.yml`) | Your own replica set / cluster |
| **Best for** | Learning and demoing the full system end to end | Auditing your real data with keys and a contract you control |

**New to this?** Start with **Dev Mode** to watch the whole system work end to end. (Production Mode is not yet available in the UI.)

### Dev Mode (full stack locally)

Runs the **complete audit system** on your machine via Docker — publisher, independent attester, and reader daemons — with K-of-N attestation, oplog completeness verification, and on-chain commitments to Stellar testnet.

> **Dev Mode convenience:** Both the publisher and attester keys are generated on your machine for demonstration purposes. In production, these run on separate servers controlled by separate parties (see [Deploying the audit service to a server](#deploying-the-audit-service-to-a-server)).

**Use this when:** you want to see or demo the entire audit system working end to end, without deploying anything of your own.

**Prerequisites:** Docker Desktop, and (optional) a [Pinata](https://pinata.cloud) account or local IPFS for batch publishing. **Start Stack** brings up the 3-node MongoDB replica set for you.

**Steps (no terminal needed):**

1. Open the app, go to the Audit tab, and select **Dev Mode**.

2. Click **Set up**. This runs the setup wizard for you — no terminal required. It generates the two independent Stellar keypairs (publisher + attester), funds them on testnet via Friendbot, deploys a fresh audit contract so the publisher becomes its admin, generates the attester's ed25519 oplog key, authorizes the attester on the contract, and writes `attester.key` + `.env.audit`. Enter your Pinata API key/secret in the form to enable IPFS publishing (optional). Your secret keys are stored locally and are never displayed.

   > **No host `stellar` CLI or Rust toolchain needed for Dev Mode.** The setup runs inside the audit Docker image, which bundles the `stellar` CLI and a prebuilt contract WASM. The wizard deploys the contract and authorizes the attester from there, so deploying a per-user contract is the default — your publisher key owns it, which is required for committing roots and authorizing the attester.
   >
   > **Why two keys?** The trust model requires the attester to be independent from the operator. If both used the same key, the operator could submit fake attestations themselves, defeating independent verification — so the wizard generates two separate keypairs.

3. Click **Start Stack**. This brings up the 3-node MongoDB replica set and the publisher, attester, and reader containers using the `.env.audit` and `attester.key` that setup produced.

4. Write data to the audited MongoDB endpoint (`mongodb://127.0.0.1:27020/?directConnection=true`) to populate the audit log. The live view shows:
   - **Event feed** — real-time stream of captured inserts, updates, and deletes
   - **Epoch progress** — how many events are in the current batch (fills up to 100 by default)
   - **On-chain root** — the last Merkle root committed to Stellar
   - **Multi-party sign-off** (K-of-N) — how many independent attesters have signed the batch
   - **Oplog completeness** — verifies no writes were omitted by comparing against MongoDB's replication log
   - **Epoch history** — list of all sealed and committed batches

5. Click **Seal Batch** to close the current epoch and freeze its Merkle root.

6. Click **Commit Batch** to pin the batch to IPFS and commit the root on-chain (Stellar testnet).

> **Installed (packaged) app:** everything above works the same in an installed build (dmg/msi/etc.). The **Set up** and **Start Stack** buttons pull the published audit image automatically — no source tree and no local `--build`.

**Manual Docker commands** (alternative to the in-app Start Stack button):
```bash
docker compose -f docker-compose.audit-db.yml up -d  # start the 3-node replica set first
docker compose -f docker-compose.audit.yml up -d      # then the audit stack (add --build on first run)
docker compose -f docker-compose.audit.yml ps         # check status
docker compose -f docker-compose.audit.yml logs -f    # tail logs
docker compose -f docker-compose.audit.yml down       # stop the stack
```

#### Clean restart (reset everything and start fresh)

There are two levels of reset. Use the first if you just want to clear audit data and re-run; use the second for a completely clean slate (new keys, new contract, new database).

**Option A: Reset audit data only (keep keys and contract).** This wipes the audit log, Merkle tree, and attester state, but preserves your Stellar keypairs, contract, and on-chain history. The 3-node MongoDB replica set keeps its data too.

In the app, click **Reset Data** (next to **Stop** in the stack status bar). Or from the terminal:
```bash
docker compose -f docker-compose.audit.yml down -v     # stop audit stack + wipe daemon volumes
docker compose -f docker-compose.audit.yml up -d        # restart with the same .env.audit
```

**Option B: Full clean restart (new keys, new contract, new database).** This tears down everything, including the MongoDB replica set and credentials. After this, re-run setup as if starting from scratch.

```bash
# 1. Stop and remove everything (audit stack + 3-node replica set + all volumes)
docker compose -f docker-compose.audit.yml down -v
docker compose -f docker-compose.audit-db.yml down -v

# 2. Delete credentials so the setup wizard generates fresh ones
rm -f .env.audit attester.key

# 3. Start the replica set fresh
docker compose -f docker-compose.audit-db.yml up -d

# 4. Re-run setup (deploys a new contract with new keys)
docker compose -f docker-compose.audit.yml run --rm setup
# Non-interactive: docker compose -f docker-compose.audit.yml run --rm -e DEPLOY_CHOICE=deploy setup setup --non-interactive

# 5. Start the audit stack
docker compose -f docker-compose.audit.yml up -d
```

> **What survives a full reset?** On-chain Stellar history is permanent and cannot be undone. Previous testnet commits remain visible on Stellar Explorer, but the new setup creates a fresh contract so they are not referenced by the new audit log.

### Production Mode (in-app, your keys)

> **Not yet available.** Production Mode is implemented but disabled in the UI ("Coming soon"). The following documents the intended workflow for when it ships.

Runs the in-app audit pipeline with **your own Stellar keypair** and contract. Choose testnet or mainnet — this is the "double check" that an audit system you deployed elsewhere works end to end. No daemon, no Docker.

**Use this when:** you want to audit your real data with keys and a contract you control — no Docker, no background daemons.

**What you need:**
- Your Stellar secret key (`S…`). On **testnet** the key alone is enough: the first commit funds your account via Friendbot and deploys a fresh commitment contract owned by that key. On **mainnet** you also need your deployed contract ID (`C…`) and an RPC URL.
- A MongoDB **replica set** connection (change streams and oplog require it; a standalone `mongod` won't work).

**Steps:**

1. Open the app, go to the Audit tab, select **Production Mode**.

2. Choose a network: **Testnet** (auto-funded contract) or **Mainnet** (your contract ID + RPC URL).

3. Import your Stellar secret key (S... strkey). It's stored in the OS keychain and never leaves your machine.

4. If mainnet: enter your contract ID (C...) and RPC URL.

5. The live view shows: event feed, epoch progress, on-chain root, verify integrity, per-event proofs, and advanced details — committing via your keypair on your chosen network.

6. Click **Commit Batch** to commit a sealed batch. On testnet the first commit also provisions your contract (see below); pin to IPFS, commit the root on-chain via native signing, then self-attest the root. The batch shows **Verified 1/1** once attested.

**Switching modes:** Production Mode is not yet available in the UI. When it ships, you'll click **Settings** in the audit panel to toggle between Dev and Production.

#### Testnet contract provisioning, persistence, and demo attestation

Production Mode on **testnet** is self-contained: you never run the `stellar` CLI, deploy a contract by hand, or paste a contract ID. The app handles it on your first commit.

**Automatic contract deploy (first testnet commit).** `commit_root*` is admin-gated on-chain, so a commit signed by a key that isn't the contract admin passes simulation but traps on apply. To avoid that, the first testnet commit provisions a commitment contract **owned by your imported key**:

1. Funds your account via Friendbot if needed (a no-op if it's already funded).
2. Uploads the bundled `zk_audit_commitment.wasm` (shipped in `src-tauri/resources/contract/`) and creates a contract instance, signing natively (ed25519 + Soroban RPC), with no `stellar` CLI.
3. Calls `initialize`, which sets the contract **admin = your key**, so your later commits are authorized.

The same key deploys, initializes, commits, and attests, so the admin check always passes. The step is **idempotent**: subsequent commits detect that your key already owns the contract and reuse it instead of redeploying.

**Per-network contract ID persistence.** The deployed contract ID is saved to the app's global settings, keyed by network (`testnet` vs `mainnet`). It survives restarts and overrides the bundled testnet default, so every future testnet commit targets *your* contract. You can see the active contract ID under **Advanced → Your contract** in the Audit tab. Mainnet uses the contract ID you supply; nothing is auto-deployed there.

**Single-attestor demo verification (K=1).** Full K-of-N threshold attestation (the Dev Mode model) requires independent attesters. Production Mode's in-app trial registers your key as the sole attester and sets the threshold to **K=1**, so after each commit the app signs the batch root and the batch immediately shows **Verified 1/1**. This demonstrates the attestation surface end to end with one identity; a real deployment registers independent attesters and raises K (see [Dev Mode](#dev-mode-full-stack-locally) and the [oplog completeness protocol](#oplog-completeness-protocol)).

### Audit domains and selective disclosure

Beyond the single global Merkle tree, the desktop app segments the audit log into **domains** so you can disclose one tenant's history without exposing everyone else's. A domain is the pair `(deployment, database)` — for example `rs:rs0 · sales`. Events recorded before segmentation (with no deployment identity) form a backward-compatible **unattributed** domain.

These features live in the **Audit tab → Change Feed**: domain filter chips, a per-domain status panel (root, event count, legal-hold and pruned badges), and actions to prove inclusion and manage lifecycle. They apply to both Dev and Production mode.

**Per-domain roots & selective disclosure.** Each domain has its own secondary Merkle root, computed deterministically from that domain's leaves in the (tamper-verified) global log. You can generate an inclusion proof for a single record against its domain root — proving that one event is in the log without revealing any other domain's records.

**Aggregation super-root.** A second Merkle tree is built over all per-domain roots (one leaf per domain, each leaf binding `deployment | database | domainRoot`). Its **super-root** commits to the *set* of domain roots, and a short inclusion proof can show that a given domain's root is part of the committed state. The super-root is anchored on-chain by being written into the epoch's commit metadata (`domainSuperRoot=<hex>`), so no contract redeploy is needed. The "Prove in super-root" action returns a proof that is cryptographically verifiable (the leaf hashes up to the super-root through its authentication path).

**Legal hold.** A domain can be placed under legal hold, which blocks pruning/retention until the hold is lifted. Holds are persisted and survive restarts.

**Retention / pruning.** A domain can be *logically* pruned: its active event metadata is dropped from the live view, but a compact **retained Merkle commitment** (root, event count, last index, timestamp) is kept. The append-only global tree, sled state, and on-chain anchor are never modified — so the anchored history stays complete and verifiable, and the pruned domain still participates in the super-root via its retained root. Pruning is refused while a legal hold is active.

> **Guarantee layers.** Integrity (events weren't altered) and inclusion (a proof that an event is in the log) are provided per domain. Completeness (no writes were omitted) is currently a global guarantee via the oplog protocol below; per-domain completeness is not yet sliced out.

## Standalone audit service (`nosqlbuddy-audit`)

> **Advanced / reference.** Most users don't need this — [Dev Mode](#dev-mode-full-stack-locally) above covers the common workflow. This section documents running the audit daemon directly (CLI flags, HTTP API, and the end-to-end protocol).

The audit service runs as a separate process from the desktop app. It captures MongoDB writes via change streams, builds a tamper-evident Poseidon Merkle tree, batches events into epochs, publishes batches to IPFS, and commits Merkle roots to a Soroban contract on Stellar.

> **Role separation.** As described in [Roles: publisher vs attester](#roles-publisher-vs-attester) above, production deployments split the publisher and attester across separate servers. This section documents the standalone binary that each party runs independently.

### Build

```bash
cd src-tauri
cargo build --bin nosqlbuddy-audit
```

### Run via Docker Compose (single-machine demo)

> This runs the publisher, attester, and reader together on one machine. This is fine for demos and Dev Mode, but **not for production** -- see [Deploying the audit service to a server](#deploying-the-audit-service-to-a-server) for the separated deployment model.

If you don't want to install the Rust toolchain, you can run the full audit stack (publisher + attester + reader) via Docker. This uses the same binary, containerized with the Dockerfile at `audit-service/Dockerfile.audit`.

**Prerequisites:** Docker Desktop + the 3-node MongoDB replica set running.

1. Start the 3-node MongoDB replica set (from the project root):
   ```bash
   docker compose -f docker-compose.audit-db.yml up -d
   ```

2. Run the setup wizard (interactive, in Docker). It generates the publisher + attester keypairs, funds them on testnet, deploys a fresh audit contract (publisher becomes admin), generates and authorizes the attester's ed25519 oplog key, and writes `./attester.key` + `.env.audit` into the project root:
   ```bash
   docker compose -f docker-compose.audit.yml run --build --rm setup
   ```
   Accept the defaults (choose **deploy** when asked about the contract) and enter your Pinata API key/secret when prompted. See [Dev Mode](#dev-mode-full-stack-locally) for more detail.

   > **Non-interactive (CI / scripted):** Pass `DEPLOY_CHOICE=deploy` as an env var so the wizard deploys a fresh contract instead of falling back to the shared bundled one:
   > ```bash
   > docker compose -f docker-compose.audit.yml run --rm -e DEPLOY_CHOICE=deploy setup setup --non-interactive
   > ```

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

The `setup` subcommand is a role-aware wizard that generates Stellar keypairs, optionally deploys the contract, and writes `.env.audit`.

> **Dev Mode vs production.** For Dev Mode, run the full wizard once (`--role all`, the default) to generate both keys. For production, the **operator** runs `--role publisher` to generate their publisher key and deploy the contract, then the **auditor** runs `--role attester` independently on their own server. No single party holds both keys.

| Role | Who runs it | What it generates | On which server |
|---|---|---|---|
| `--role all` (default) | Developer | Both keys, contract deploy, attester authorization | One machine (Dev Mode) |
| `--role publisher` | Operator | Publisher key, contract deploy | Company server |
| `--role attester` | Auditor | Attester Stellar key + ed25519 oplog key | Auditor's server |

**Dev Mode (both keys, one machine):**

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

**Production — operator (publisher key + contract deploy):**

```bash
cargo run --bin nosqlbuddy-audit -- setup --role publisher
# Writes .env.audit with STELLAR_SECRET_KEY + CONTRACT_ID. No attester key.
# Prints the contract ID — share it with the auditor.
```

**Production — auditor (attester keys, independently):**

```bash
cargo run --bin nosqlbuddy-audit -- setup --role attester
# Prompts for the contract ID (from the operator).
# Generates the attester Stellar key + ed25519 oplog key.
# Prints the attester public address + ed25519 pubkey — send to the operator.
```

**Attester authorization (operator, after receiving auditor's public keys):**

```bash
cargo run --bin nosqlbuddy-audit -- authorize-attester \
  --contract-id <C...> \
  --secret-key <publisher S...> \
  --attester-address <auditor G...> \
  --attester-pubkey <auditor ed25519 hex>
```

The `--role all` wizard (Dev Mode) walks through:
1. Choosing a network (testnet or mainnet)
2. Generating (or importing) the publisher Stellar keypair
3. Generating (or importing) the attester Stellar keypair
4. Deploying a new contract or using an existing one
5. Initializing the contract (sets the admin = publisher)
6. Generating the attester's ed25519 oplog signing key
7. Authorizing the attester on the contract
8. Entering Pinata IPFS credentials (optional)
9. Writing `.env.audit` with all values

The `--role publisher` wizard does steps 1, 2, 4-5, 8-9 (skips attester key + ed25519 key + authorization).
The `--role attester` wizard does steps 1, 3, 6, 9 (skips publisher key + contract deploy + authorization). It prompts for the contract ID instead.

> **Contract deployment.** The audit Docker image bundles the `stellar` CLI and a prebuilt contract WASM, so the wizard deploys a fresh per-user contract by default (your publisher becomes its admin). Pass an existing `CONTRACT_ID` only if you want to reuse a contract you already control. Running the wizard from a full source checkout works too and additionally lets it build the WASM from source.
>
> **Setting up Dev Mode?** This is the recommended one-command path — see [Dev Mode](#dev-mode-full-stack-locally).

### Start the service

In production, each role runs on a separate server, run by a separate party. Each party only has access to their own secret key.

**Publisher mode** (run by the operator on the company server) — captures writes, manages epochs, publishes to IPFS, commits roots on-chain:

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

**Attester mode** (run by the auditor on the auditor's server) — independent attester that connects to the independent replica member, watches for new epoch commitments on-chain, independently computes the oplog hash, and submits attestations to the contract:

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
| `--proving-key <path>` | — | Pre-generated proving key (from trusted setup ceremony; speeds up proof generation) |
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
| `--role <all\|publisher\|attester>` | `all` | Setup wizard role (see [Setup wizard](#setup-wizard-one-time)). Also reads `SETUP_ROLE` env var |
| `--pinata-api-key <key>` | — | Pinata API key for cloud IPFS pinning |
| `--pinata-api-secret <secret>` | — | Pinata API secret for cloud IPFS pinning |
| `--pinata-gateway-url <url>` | `https://gateway.pinata.cloud` | Pinata gateway URL |
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

## Deploying the audit service to a server

For production, the publisher and attester run on **separate servers, run by separate parties**. The operator (company being audited) runs the publisher; the auditor/regulator runs the attester. Nobody should hold both keys.

Both use the **published Docker image** (`ghcr.io/ronnakamoto/nosqlbuddy-audit`, tagged per release) with `deploy/docker-compose.audit.yml` as the release asset. The Compose file uses Docker profiles (`--profile publisher`, `--profile attester`, `--profile all`) so each party brings up only their own containers.

> **MongoDB is yours to operate.** This stack does not start a database. The publisher connects to the primary; the attester/reader connect to an independent replica member the auditor controls.

### Operator setup (publisher)

1. On the company server, create a working directory and get the deploy assets:
   ```bash
   sudo mkdir -p /opt/nosqlbuddy-audit && cd /opt/nosqlbuddy-audit
   # place docker-compose.audit.yml + audit-stack.env.publisher.example here
   cp audit-stack.env.publisher.example .env.audit
   ```

2. Run the role-aware setup wizard to generate the publisher keypair, deploy the contract, and write `.env.audit`:
   ```bash
   docker compose --env-file .env.audit run --rm setup -- --role publisher
   ```
   This writes `STELLAR_SECRET_KEY` and `CONTRACT_ID` into `.env.audit`. The wizard prints the contract ID -- share it with the auditor.

3. Edit `.env.audit`: set `PUBLISHER_MONGO_URI` to your replica set and pin `AUDIT_IMAGE_TAG` to a release.

4. Start the publisher and reader:
   ```bash
   docker compose --env-file .env.audit --profile publisher up -d
   ```

5. After the auditor sends their public keys (from their setup), authorize the attester on-chain:
   ```bash
   docker compose --env-file .env.audit run --rm setup -- authorize-attester \
     --contract-id <C...> \
     --secret-key <publisher S...> \
     --attester-address <auditor G...> \
     --attester-pubkey <auditor ed25519 hex> \
     --network testnet
   ```

### Auditor setup (attester)

1. On the auditor's server, create a working directory with the same deploy assets:
   ```bash
   sudo mkdir -p /opt/nosqlbuddy-audit && cd /opt/nosqlbuddy-audit
   # place docker-compose.audit.yml + audit-stack.env.attester.example here
   cp audit-stack.env.attester.example .env.audit
   ```

2. Run the role-aware setup wizard to generate the attester keys independently:
   ```bash
   docker compose --env-file .env.audit run --rm setup -- --role attester
   ```
   Enter the contract ID provided by the operator. This writes `ATTESTER_SECRET_KEY` into `.env.audit` and generates `./attester.key` (ed25519 oplog key).

3. The wizard prints the attester public address and ed25519 pubkey. Send these to the operator for on-chain authorization.

4. Once authorized, edit `.env.audit`: set `ATTESTER_MONGO_URI` to the independent replica member.

5. Start the attester and reader:
   ```bash
   docker compose --env-file .env.audit --profile attester up -d
   ```

### Managing the stack

Each party manages their own containers independently:

```bash
docker compose --env-file .env.audit --profile publisher ps   # or --profile attester
docker compose --env-file .env.audit --profile publisher logs -f
docker compose --env-file .env.audit --profile publisher down
```

> **Run every command with `--env-file .env.audit`** and the appropriate `--profile` flag. The `publisher` profile starts publisher + reader; `attester` starts attester + reader; `all` starts everything (Dev Mode only). Daemon state persists in named volumes; secrets are injected via `environment:` and are never baked into the image.

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
