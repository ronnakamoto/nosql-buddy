//! nosqlbuddy-audit — standalone ZK audit service for NoSQLBuddy.
//!
//! One binary, subcommands:
//!   setup   Interactive wizard: generate keys, deploy contract, authorize attester
//!   start   Start the audit service (publisher / reader / attester mode)
//!   stop    Stop a running service
//!   status  Check if the service is running
//!
//! Usage:
//!   nosqlbuddy-audit setup
//!   nosqlbuddy-audit start --mode publish --mongo-uri mongodb://localhost:27017
//!   nosqlbuddy-audit start --mode attest --mongo-uri mongodb://localhost:27019
//!   nosqlbuddy-audit start --mode read
//!   nosqlbuddy-audit stop
//!   nosqlbuddy-audit status
//!
//! The service reuses the same audit modules as the NoSQLBuddy Tauri app.
//! No code duplication — it links against `audit_service` directly.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use audit_service::audit::attestation::AttestationManager;
use audit_service::audit::change_stream::{fetch_deployment_id, ChangeStreamRegistry};
use audit_service::audit::epoch::EpochManager;
use audit_service::audit::ipfs::IpfsConfig;
use audit_service::audit::sled_store::SledTreeStore;
use audit_service::audit::AuditLog;
use audit_service::auditd::{DaemonConfig, DaemonMode, DaemonState};
use mongo_uri::force_direct_connection;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,audit_service=info".into()),
        )
        .try_init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print_help();
        std::process::exit(1);
    }

    let subcommand = &args[1];
    let rest: Vec<String> = args[2..].to_vec();

    match subcommand.as_str() {
        "setup" => cmd_setup(&rest).await,
        "start" => cmd_start(&rest).await,
        "stop" => cmd_stop(&rest),
        "status" => cmd_status(&rest).await,
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        other => {
            eprintln!("error: unknown subcommand '{other}'");
            eprintln!();
            print_help();
            std::process::exit(1);
        }
    }
}

// ─── Subcommand: setup ────────────────────────────────────────────────

/// Interactive setup wizard.
///
/// Generates Stellar keypairs, optionally deploys the contract, initializes
/// it, authorizes the attester, and writes `.env.audit`.
async fn cmd_setup(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use audit_service::audit::stellar_native;

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  nosqlbuddy-audit setup                                      ║");
    println!("║  Interactive wizard — get the full audit stack running       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Non-interactive mode (used by the desktop app's "Set up" button): take
    // sensible defaults and read any operator-provided values from the
    // environment instead of prompting on stdin.
    let non_interactive = args.iter().any(|a| a == "--non-interactive" || a == "-y");
    let ask = |question: &str, default: &str| -> String {
        if non_interactive {
            default.to_string()
        } else {
            prompt(question, default)
        }
    };
    if non_interactive {
        println!("Running in non-interactive mode (defaults + environment).");
        println!();
    }

    // 1. Choose network.
    let network = ask(
        "Stellar network [testnet/mainnet] (default: testnet)",
        &env_default(non_interactive, "STELLAR_NETWORK", "testnet"),
    );
    let is_mainnet = network == "mainnet";

    let (rpc_url, horizon_url, passphrase, default_contract_id) = if is_mainnet {
        (
            ask("Mainnet RPC URL", "https://soroban.stellar.org:443"),
            "https://horizon.stellar.org".to_string(),
            stellar_native::MAINNET_PASSPHRASE.to_string(),
            String::new(),
        )
    } else {
        (
            "https://soroban-testnet.stellar.org:443".to_string(),
            "https://horizon-testnet.stellar.org".to_string(),
            stellar_native::TESTNET_PASSPHRASE.to_string(),
            audit_service::audit::stellar::CONTRACT_ID.to_string(),
        )
    };

    // 2. Publisher keypair.
    println!();
    println!("── Publisher key (controlled by the operator) ──");
    let publisher_secret = ask(
        "Paste publisher Stellar secret key (S...), or press Enter to generate one",
        &env_default(non_interactive, "STELLAR_SECRET_KEY", ""),
    );
    let publisher_kp = if publisher_secret.is_empty() {
        let kp = stellar_native::generate_keypair();
        println!("  Generated publisher keypair:");
        println!("    Account: {}", kp.account_id());
        println!("    Secret:  {}", kp.secret_key_str());
        println!("    ⚠ Save this secret key — it won't be shown again.");
        kp
    } else {
        audit_service::auditd::load_keypair_from_secret_key(&publisher_secret)
            .map_err(|e| format!("invalid publisher secret key: {e}"))?
    };

    // 3. Attester keypair.
    println!();
    println!("── Attester key (controlled by the auditor/regulator) ──");
    println!("  This is a SEPARATE key from the publisher. The trust model requires");
    println!("  the attester to be independent from the operator.");
    let attester_secret = ask(
        "Paste attester Stellar secret key (S...), or press Enter to generate one",
        &env_default(non_interactive, "ATTESTER_SECRET_KEY", ""),
    );
    let attester_kp = if attester_secret.is_empty() {
        let kp = stellar_native::generate_keypair();
        println!("  Generated attester keypair:");
        println!("    Account: {}", kp.account_id());
        println!("    Secret:  {}", kp.secret_key_str());
        println!("    ⚠ Save this secret key — it won't be shown again.");
        kp
    } else {
        audit_service::auditd::load_keypair_from_secret_key(&attester_secret)
            .map_err(|e| format!("invalid attester secret key: {e}"))?
    };

    // 4. Contract: deploy new or use existing?
    println!();
    println!("── Soroban contract ──");
    let deploy_choice = ask(
        if is_mainnet {
            "Deploy a new contract or use existing? [deploy/existing] (default: existing)"
        } else {
            "Deploy a new contract or use the bundled testnet contract? [deploy/existing] (default: existing)"
        },
        &env_default(non_interactive, "DEPLOY_CHOICE", "existing"),
    );

    // 5. Fund accounts on testnet via Friendbot (auto-fund for testnet only).
    // This must happen BEFORE contract deployment, since deployment requires
    // the publisher account to exist and have XLM for fees.
    if !is_mainnet {
        println!();
        println!("── Funding accounts on testnet (Friendbot) ──");
        fund_and_wait_for_testnet_account("publisher", &publisher_kp.account_id(), &horizon_url)
            .await?;
        fund_and_wait_for_testnet_account("attester", &attester_kp.account_id(), &horizon_url)
            .await?;
        // Give Soroban RPC/account sequence caches a short settling window
        // after Horizon confirms both funded accounts exist.
        println!("  Funding confirmed; waiting 3s for RPC propagation...");
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }

    // 6. Deploy or select the contract.
    let contract_id = if deploy_choice == "deploy" {
        // Deploy via stellar CLI (one-time operation).
        println!();
        println!("  Deploying contract via stellar CLI...");
        println!("  (This requires the stellar CLI installed: https://docs.stellar.org/tools/developer-tools/cli/install)");
        eprintln!("  Building WASM...");
        let wasm_path = build_contract_wasm()?;
        eprintln!("  Deploying to {network}...");
        let cid = deploy_contract(&wasm_path, &network, &publisher_kp.secret_key_str())?;
        println!("  Contract deployed: {cid}");
        // Give the freshly created contract instance time to propagate to the
        // RPC node before we invoke `initialize` on it — otherwise the
        // simulation can fail because the contract isn't visible yet.
        println!("  Waiting 12s for the contract to be visible on RPC...");
        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        cid
    } else if is_mainnet {
        ask(
            "Enter your mainnet contract ID (C...)",
            &env_default(non_interactive, "CONTRACT_ID", ""),
        )
    } else {
        println!("  Using bundled testnet contract: {default_contract_id}");
        default_contract_id.clone()
    };

    if contract_id.is_empty() {
        return Err("contract ID is required".into());
    }

    // 7. Initialize the contract (if deploying new).
    if deploy_choice == "deploy" {
        println!();
        println!("  Initializing contract (set admin = publisher)...");
        initialize_contract(&contract_id, &publisher_kp, &rpc_url, &passphrase).await?;
        println!(
            "  Contract initialized. Admin: {}",
            publisher_kp.account_id()
        );
    }

    // 8. Generate attester ed25519 oplog signing key.
    println!();
    println!("── Attester ed25519 oplog signing key ──");
    println!("  This is a separate key from the Stellar key. It signs the oplog hash.");
    let attester_key_file = ask(
        "Path to save the attester ed25519 key (default: ./attester.key)",
        &env_default(non_interactive, "ATTESTER_KEY_FILE", "./attester.key"),
    );
    let attester_key_path = PathBuf::from(&attester_key_file);
    let ed25519_key =
        audit_service::auditd::attester::load_or_generate_attester_key(&attester_key_path)
            .map_err(|e| format!("failed to generate attester key: {e}"))?;
    let abs_attester_key_path = absolute_path(&attester_key_path);
    let ed25519_pubkey_hex = hex::encode(ed25519_key.verifying_key().to_bytes());
    println!("  Attester ed25519 public key: {ed25519_pubkey_hex}");
    println!(
        "  Attester ed25519 key written to: {}",
        abs_attester_key_path.display()
    );

    // Wait for the initialize transaction to be processed by the network.
    if deploy_choice == "deploy" {
        println!();
        println!("  Waiting 10s for initialize transaction to be confirmed...");
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }

    // 9. Authorize the attester on the contract.
    println!();
    println!("  Authorizing attester on the contract...");
    authorize_attester(
        &contract_id,
        &publisher_kp,
        &attester_kp.account_id(),
        &ed25519_pubkey_hex,
        &rpc_url,
        &passphrase,
    )
    .await?;
    println!(
        "  Attester authorized: {} (pubkey: {})",
        attester_kp.account_id(),
        &ed25519_pubkey_hex[..16]
    );

    // 10. Pinata credentials (optional).
    println!();
    println!("── Pinata IPFS credentials (optional) ──");
    let pinata_api_key = ask(
        "Pinata API key (press Enter to skip)",
        &env_default(non_interactive, "PINATA_API_KEY", ""),
    );
    let pinata_api_secret = ask(
        "Pinata API secret (press Enter to skip)",
        &env_default(non_interactive, "PINATA_API_SECRET", ""),
    );
    let pinata_gateway = ask(
        "Pinata gateway URL (default: https://gateway.pinata.cloud)",
        &env_default(
            non_interactive,
            "PINATA_GATEWAY_URL",
            "https://gateway.pinata.cloud",
        ),
    );

    // 11. Write .env.audit.
    println!();
    let env_path = ask(
        "Write .env.audit to (default: .env.audit)",
        &env_default(non_interactive, "ENV_AUDIT_PATH", ".env.audit"),
    );
    write_env_file(
        &env_path,
        &contract_id,
        &publisher_kp.secret_key_str(),
        &attester_kp.secret_key_str(),
        &pinata_api_key,
        &pinata_api_secret,
        &pinata_gateway,
    )?;
    let abs_env_path = absolute_path(&PathBuf::from(&env_path));
    println!("  Wrote {}", abs_env_path.display());

    // 12. Summary.
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  Setup complete!                                             ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Network:      {network:<47}║");
    println!("║  Contract:     {contract_id:<47}║");
    println!("║  Publisher:    {pub:<47}║", pub = publisher_kp.account_id());
    println!(
        "║  Attester:     {att:<47}║",
        att = attester_kp.account_id()
    );
    println!(
        "║  Ed25519 key:  {abs_attester:<47}║",
        abs_attester = abs_attester_key_path.display()
    );
    println!(
        "║  Env file:     {abs_env:<47}║",
        abs_env = abs_env_path.display()
    );
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Next steps:");
    if is_mainnet {
        println!(
            "  1. Fund the publisher and attester accounts on mainnet (XLM required for tx fees)"
        );
        println!("     Publisher: {}", publisher_kp.account_id());
        println!("     Attester:  {}", attester_kp.account_id());
    } else {
        println!("  1. Accounts funded on testnet via Friendbot (10,000 XLM each)");
    }
    println!("  2. Start the audit stack:");
    println!("     docker compose -f docker-compose.audit.yml up -d");
    println!("  3. Or start individual services:");
    println!("     nosqlbuddy-audit start --mode publish --mongo-uri mongodb://localhost:27017");
    println!("     nosqlbuddy-audit start --mode attest --mongo-uri mongodb://localhost:27019");

    Ok(())
}

/// Resolve a (possibly relative) path to an absolute, normalized path for
/// display, so users see exactly where files were written. Falls back to a
/// best-effort `current_dir`-joined path if the file can't be canonicalized.
fn absolute_path(path: &PathBuf) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    if path.is_absolute() {
        return path.clone();
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| path.clone())
}

/// In non-interactive mode, read a value from the environment (falling back to
/// `fallback` when unset/empty); in interactive mode always return `fallback`
/// as the prompt default so existing behavior is unchanged.
fn env_default(non_interactive: bool, var: &str, fallback: &str) -> String {
    if non_interactive {
        std::env::var(var)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| fallback.to_string())
    } else {
        fallback.to_string()
    }
}

/// Fund a testnet account, then wait until Horizon can read it.
async fn fund_and_wait_for_testnet_account(
    role: &str,
    account_id: &str,
    horizon_url: &str,
) -> Result<(), String> {
    print!("  Funding {role} account {account_id}... ");
    let _ = io::stdout().flush();
    let friendbot_error = match audit_service::audit::stellar_native::fund_account(account_id).await
    {
        Ok(()) => {
            println!("OK");
            None
        }
        Err(e) => {
            println!("SKIP ({e})");
            Some(e.to_string())
        }
    };

    print!("  Waiting for {role} account to be visible on Horizon... ");
    let _ = io::stdout().flush();
    match wait_for_horizon_account(
        horizon_url,
        account_id,
        20,
        std::time::Duration::from_secs(3),
    )
    .await
    {
        Ok(()) => {
            println!("OK");
            Ok(())
        }
        Err(last_horizon_error) => {
            println!("FAILED");
            let friendbot_hint = friendbot_error
                .map(|e| format!(" Friendbot said: {e}."))
                .unwrap_or_default();
            Err(format!(
                "testnet {role} account {account_id} was not visible after Friendbot funding.\
                 {friendbot_hint} Last Horizon check: {last_horizon_error}.\
                 Fund it manually with: curl -fsS \"https://friendbot.stellar.org?addr={account_id}\" and rerun setup."
            ))
        }
    }
}

async fn wait_for_horizon_account(
    horizon_url: &str,
    account_id: &str,
    attempts: usize,
    delay: std::time::Duration,
) -> Result<(), String> {
    let url = format!(
        "{}/accounts/{}",
        horizon_url.trim_end_matches('/'),
        account_id
    );
    let client = reqwest::Client::new();
    let mut last_error = "not checked".to_string();

    for attempt in 1..=attempts {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let text = text.trim();
                last_error = if text.is_empty() {
                    format!("HTTP {status}")
                } else {
                    format!("HTTP {status}: {text}")
                };
            }
            Err(e) => {
                last_error = e.to_string();
            }
        }

        if attempt < attempts {
            tokio::time::sleep(delay).await;
        }
    }

    Err(last_error)
}

/// Build the Soroban contract WASM using `stellar contract build`.
///
/// This uses the Stellar CLI's built-in build command which handles
/// WASM compatibility (reference-types, optimization, etc.) correctly.
fn build_contract_wasm() -> Result<PathBuf, String> {
    // Prefer a prebuilt WASM shipped with the image (CONTRACT_WASM_PATH). The
    // slim runtime container has the stellar CLI for deploy but no Rust
    // toolchain to compile the contract, so the WASM is built ahead of time and
    // bundled. Falls back to `stellar contract build` when running from a full
    // source checkout (e.g. a developer's machine).
    if let Ok(prebuilt) = std::env::var("CONTRACT_WASM_PATH") {
        let path = PathBuf::from(prebuilt);
        if path.exists() {
            eprintln!("  Using prebuilt contract WASM: {}", path.display());
            return Ok(path);
        }
    }

    // The contract is at <project-root>/zk-audit/soroban-contract/.
    // Try a few candidate locations relative to the current directory.
    let candidates = [
        PathBuf::from("zk-audit/soroban-contract/Cargo.toml"),
        PathBuf::from("../zk-audit/soroban-contract/Cargo.toml"),
        PathBuf::from("../../zk-audit/soroban-contract/Cargo.toml"),
    ];
    let manifest = candidates.iter().find(|p| p.exists()).ok_or_else(|| {
        format!(
            "contract manifest not found in any of: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    let contract_dir = manifest.parent().unwrap();

    eprintln!("  Building WASM from {}...", manifest.display());
    let output = std::process::Command::new("stellar")
        .args(["contract", "build", "--manifest-path"])
        .arg(manifest)
        .arg("--profile")
        .arg("release")
        .output()
        .map_err(|e| {
            format!(
                "failed to run `stellar contract build` — is the stellar CLI installed?\n\
             Install: https://docs.stellar.org/tools/developer-tools/cli/install\n\
             Error: {e}"
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "stellar contract build failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // `stellar contract build` outputs to target/wasm32v1-none/release/
    // (not wasm32-unknown-unknown like plain cargo build).
    let wasm = contract_dir.join("target/wasm32v1-none/release/zk_audit_commitment.wasm");
    if !wasm.exists() {
        // Fall back to the standard target dir.
        let wasm_fallback =
            contract_dir.join("target/wasm32-unknown-unknown/release/zk_audit_commitment.wasm");
        if !wasm_fallback.exists() {
            return Err(format!(
                "WASM not found at {} (or {}) — check the build output",
                wasm.display(),
                wasm_fallback.display()
            ));
        }
        return Ok(wasm_fallback);
    }
    Ok(wasm)
}

/// Deploy the contract via the stellar CLI (one-time operation).
///
/// Performed as two explicit transactions — upload the WASM, then instantiate
/// from its hash — with a short wait in between. The CLI's combined
/// `deploy --wasm` builds both transactions back-to-back, which races on the
/// source account's sequence number right after funding and fails with
/// `TxBadSeq` or `TxNoAccount`. Splitting the steps lets each transaction fetch
/// a fresh sequence number; the deploy step also retries on transient RPC lag.
fn deploy_contract(
    wasm_path: &PathBuf,
    network: &str,
    source_secret: &str,
) -> Result<String, String> {
    let wasm_hash = upload_contract_wasm(wasm_path, network, source_secret)?;
    eprintln!("  Uploaded WASM (hash {wasm_hash}); waiting for the account sequence to settle...");
    std::thread::sleep(std::time::Duration::from_secs(6));
    deploy_from_wasm_hash(&wasm_hash, network, source_secret)
}

/// Upload the contract WASM (its own transaction) and return its 64-char hex
/// hash. Idempotent: re-uploading an already-installed WASM is a no-op on-chain.
fn upload_contract_wasm(
    wasm_path: &PathBuf,
    network: &str,
    source_secret: &str,
) -> Result<String, String> {
    let mut last_stderr = String::new();
    for attempt in 1..=3 {
        let output = std::process::Command::new("stellar")
            .args(["contract", "upload"])
            .arg("--wasm")
            .arg(wasm_path)
            .arg("--source")
            .arg(source_secret)
            .arg("--network")
            .arg(network)
            .output()
            .map_err(|e| {
                format!(
                    "failed to run `stellar contract upload` — is the stellar CLI installed?\n\
                 Install: https://docs.stellar.org/tools/developer-tools/cli/install\n\
                 Error: {e}"
                )
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // The CLI prints the 64-char hex WASM hash on the last line.
            let hash = stdout.lines().last().unwrap_or("").trim().to_string();
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!(
                    "unexpected stellar CLI output (expected 64-char wasm hash):\n{stdout}"
                ));
            }
            return Ok(hash);
        }

        last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if last_stderr.contains("Account not found") && attempt < 3 {
            eprintln!(
                "  Upload attempt {attempt} could not see the funded account; retrying in 5s..."
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        break;
    }

    Err(format!("stellar contract upload failed:\n{last_stderr}"))
}

/// Instantiate a contract from an already-uploaded WASM hash, retrying on
/// transient RPC lag while the account or sequence number catches up.
fn deploy_from_wasm_hash(
    wasm_hash: &str,
    network: &str,
    source_secret: &str,
) -> Result<String, String> {
    let mut last_stderr = String::new();
    for attempt in 1..=3 {
        let output = std::process::Command::new("stellar")
            .args(["contract", "deploy"])
            .arg("--wasm-hash")
            .arg(wasm_hash)
            .arg("--source")
            .arg(source_secret)
            .arg("--network")
            .arg(network)
            .output()
            .map_err(|e| {
                format!(
                    "failed to run `stellar contract deploy` — is the stellar CLI installed?\n\
                 Install: https://docs.stellar.org/tools/developer-tools/cli/install\n\
                 Error: {e}"
                )
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // The CLI prints the contract ID (C...) on the last line.
            let cid = stdout.lines().last().unwrap_or("").trim().to_string();
            if !cid.starts_with('C') {
                return Err(format!(
                    "unexpected stellar CLI output (expected C... contract ID):\n{stdout}"
                ));
            }
            return Ok(cid);
        }

        last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let retryable = last_stderr.contains("TxBadSeq")
            || last_stderr.contains("TxNoAccount")
            || last_stderr.contains("Account not found");
        if retryable && attempt < 3 {
            eprintln!(
                "  Deploy attempt {attempt} hit a transient RPC/account error; retrying in 5s..."
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
            continue;
        }
        break;
    }

    Err(format!("stellar contract deploy failed:\n{last_stderr}"))
}

/// Call `initialize(admin)` on the contract via native signing.
async fn initialize_contract(
    contract_id: &str,
    admin_keypair: &audit_service::audit::stellar_native::StellarKeypair,
    rpc_url: &str,
    passphrase: &str,
) -> Result<(), String> {
    audit_service::audit::stellar_native::initialize_contract_native(
        contract_id,
        admin_keypair,
        rpc_url,
        passphrase,
    )
    .await
    .map_err(|e| format!("initialize_contract: {e}"))
}

/// Call `authorize_attester(address, pubkey)` on the contract via native signing.
async fn authorize_attester(
    contract_id: &str,
    admin_keypair: &audit_service::audit::stellar_native::StellarKeypair,
    attester_address: &str,
    attester_ed25519_pubkey_hex: &str,
    rpc_url: &str,
    passphrase: &str,
) -> Result<(), String> {
    audit_service::audit::stellar_native::authorize_attester_native(
        contract_id,
        admin_keypair,
        attester_address,
        attester_ed25519_pubkey_hex,
        rpc_url,
        passphrase,
    )
    .await
    .map_err(|e| format!("authorize_attester: {e}"))
}

/// Write the .env.audit file with all credentials.
fn write_env_file(
    path: &str,
    contract_id: &str,
    publisher_secret: &str,
    attester_secret: &str,
    pinata_api_key: &str,
    pinata_api_secret: &str,
    pinata_gateway: &str,
) -> Result<(), String> {
    let content = format!(
        "# ─── Audit stack environment (generated by nosqlbuddy-audit setup) ───\n\n\
         # Soroban contract ID (deployed by this setup)\n\
         CONTRACT_ID={contract_id}\n\n\
         # Publisher's Stellar secret key (operator)\n\
         STELLAR_SECRET_KEY={publisher_secret}\n\n\
         # Attester's Stellar secret key (auditor/regulator)\n\
         ATTESTER_SECRET_KEY={attester_secret}\n\n\
         # Pinata IPFS credentials\n\
         PINATA_API_KEY={pinata_api_key}\n\
         PINATA_API_SECRET={pinata_api_secret}\n\
         PINATA_GATEWAY_URL={pinata_gateway}\n"
    );
    std::fs::write(path, content).map_err(|e| format!("failed to write {path}: {e}"))
}

/// Prompt for input with a default value. Returns the user's input or the default.
fn prompt(question: &str, default: &str) -> String {
    if default.is_empty() {
        print!("{question}: ");
    } else {
        print!("{question} [{default}]: ");
    }
    let _ = io::stdout().flush();
    let stdin = io::stdin();
    let line = stdin.lock().lines().next();
    match line {
        Some(Ok(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => default.to_string(),
    }
}

// ─── Subcommand: start ────────────────────────────────────────────────

/// Start the audit service.
async fn cmd_start(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_start_args(args);

    log::info!(
        "nosqlbuddy-audit starting in {:?} mode, data dir: {}, port: {}",
        config.mode,
        config.data_dir.display(),
        config.port
    );

    // Ensure the data directory exists.
    std::fs::create_dir_all(&config.data_dir)?;

    // Write PID file.
    let pid_file = pid_file_path(&config.data_dir, config.port);
    if pid_file.exists() {
        let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
        let pid: i32 = pid_str.trim().parse().unwrap_or(0);
        let current_pid = std::process::id() as i32;
        // In containers the previous run's PID file often contains the same
        // PID as the current process (PID 1), so treat our own PID as stale.
        if pid > 0 && pid != current_pid && is_process_running(pid) {
            eprintln!("error: nosqlbuddy-audit is already running (PID {pid})");
            eprintln!("  Use 'nosqlbuddy-audit stop' to stop it first.");
            std::process::exit(1);
        }
    }
    std::fs::write(&pid_file, std::process::id().to_string())?;

    // Initialize the audit log with persistence.
    let audit_log = Arc::new(AuditLog::new()?);
    audit_log.set_persistence_dir(&config.data_dir)?;
    log::info!(
        "audit log initialized: {} events, root: {}",
        audit_log.event_count(),
        audit_log.root_hex()?
    );

    // Initialize the attestation manager with a sled store.
    let attestation_manager = AttestationManager::default();
    let sled_path = config.data_dir.join("audit").join("attestation.sled");
    if sled_path.exists() || config.data_dir.join("audit").exists() {
        match SledTreeStore::open(&sled_path) {
            Ok(store) => {
                attestation_manager.set_store(store);
                log::info!("attestation manager sled store initialized");
            }
            Err(e) => {
                log::warn!("failed to open sled store for attestation: {e}");
            }
        }
    }

    let epoch_file = config.data_dir.join("audit").join("epochs.json");
    let epoch_manager = EpochManager::new_with_persistence(
        audit_service::audit::epoch::EpochConfig {
            event_threshold: config.epoch_threshold,
            time_threshold_secs: config.epoch_time_secs,
        },
        &epoch_file,
    );
    log::info!(
        "epoch manager configured: threshold={} events, time={}s, state={}",
        config.epoch_threshold,
        config.epoch_time_secs,
        epoch_file.display()
    );
    let change_streams = ChangeStreamRegistry::new();

    // Publisher and attester modes both need a MongoDB connection.
    // Reader mode optionally connects to an independent replica member so it
    // can independently compute the oplog hash for completeness verification.
    let mut mongo_client: Option<mongodb::Client> = None;
    if config.mode == DaemonMode::Publish {
        let mongo_uri = config
            .mongo_uri
            .as_deref()
            .ok_or_else(|| "publisher mode requires --mongo-uri".to_string())?;

        log::info!("connecting to MongoDB: {}", redact_uri(mongo_uri));
        // Pass the URI through untouched: the publisher watches change streams,
        // which require talking to the replica set (the primary). Forcing
        // directConnection would pin to one seed and break that. Operators who
        // really want a pinned connection add ?directConnection=true themselves.
        let client = mongodb::Client::with_uri_str(mongo_uri).await?;
        let connection_id = "audit".to_string();

        // Derive a stable per-deployment identity so audit events are
        // segmented by the deployment they originate from.
        let deployment_id = fetch_deployment_id(&client).await;
        log::info!("resolved deployment identity: {deployment_id}");

        change_streams
            .start_for(
                connection_id.clone(),
                deployment_id,
                client.clone(),
                audit_log.clone(),
                None,
            )
            .await;
        log::info!("change stream listener started for connection {connection_id}");

        mongo_client = Some(client);
    } else if config.mode == DaemonMode::Attest {
        let mongo_uri = config.mongo_uri.as_deref().ok_or_else(|| {
            "attester mode requires --mongo-uri (connect to the independent replica member)"
                .to_string()
        })?;

        log::info!(
            "attester: connecting to independent replica: {}",
            redact_uri(mongo_uri)
        );
        let mongo_uri = force_direct_connection(mongo_uri);
        let client = mongodb::Client::with_uri_str(&mongo_uri).await?;
        log::info!("attester: connected to independent replica member");
        mongo_client = Some(client);
    } else if config.mode == DaemonMode::Read {
        if let Some(mongo_uri) = config.mongo_uri.as_deref() {
            log::info!(
                "reader: connecting to independent replica for oplog verification: {}",
                redact_uri(mongo_uri)
            );
            let mongo_uri = force_direct_connection(mongo_uri);
            let client = mongodb::Client::with_uri_str(&mongo_uri).await?;
            log::info!("reader: connected to independent replica member");
            mongo_client = Some(client);
        } else {
            log::warn!("reader: no --mongo-uri provided; oplog verification will be in degraded mode (on-chain root only)");
        }
    }

    // Load or generate the attester signing key for attester mode.
    let mut attester_key: Option<ed25519_dalek::SigningKey> = None;
    let mut attester_address: Option<String> = None;
    if config.mode == DaemonMode::Attest {
        let key_file = config
            .attester_key_file
            .clone()
            .unwrap_or_else(|| config.data_dir.join("audit").join("attester.key"));
        let key = audit_service::auditd::attester::load_or_generate_attester_key(&key_file)
            .map_err(|e| format!("failed to load attester key: {e}"))?;
        let public_key_hex = hex::encode(key.verifying_key().to_bytes());
        log::info!("attester: loaded ed25519 key {key_file:?}; public key: {public_key_hex}");
        attester_key = Some(key);
    }

    // Load the publisher's Stellar keypair for native signing.
    let signing_keypair: Option<audit_service::audit::stellar_native::StellarKeypair> =
        if config.mode == DaemonMode::Publish {
            let sk = config
                .secret_key
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    std::env::var("STELLAR_SECRET_KEY")
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                })
                .or_else(|| {
                    std::env::var("PUBLISHER_SECRET_KEY")
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                });
            match sk {
                Some(s) => {
                    let kp = audit_service::auditd::load_keypair_from_secret_key(&s)
                        .map_err(|e| format!("failed to load publisher keypair: {e}"))?;
                    log::info!(
                        "publisher: loaded Stellar keypair (account: {})",
                        kp.account_id()
                    );
                    Some(kp)
                }
                None => {
                    return Err(
                        "publisher mode requires --secret-key or STELLAR_SECRET_KEY env var"
                            .to_string()
                            .into(),
                    );
                }
            }
        } else {
            None
        };

    // Load the attester's Stellar keypair for native transaction signing.
    let attester_stellar_keypair: Option<audit_service::audit::stellar_native::StellarKeypair> =
        if config.mode == DaemonMode::Attest {
            let sk = config
                .attester_secret_key
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    std::env::var("ATTESTER_SECRET_KEY")
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                });
            match sk {
                Some(s) => {
                    let kp = audit_service::auditd::load_keypair_from_secret_key(&s)
                        .map_err(|e| format!("failed to load attester Stellar keypair: {e}"))?;
                    log::info!(
                        "attester: loaded Stellar keypair (account: {})",
                        kp.account_id()
                    );
                    if attester_address.is_none() {
                        attester_address = Some(kp.account_id());
                    }
                    Some(kp)
                }
                None => {
                    return Err("attester mode requires --attester-secret-key or ATTESTER_SECRET_KEY env var".to_string().into());
                }
            }
        } else {
            None
        };

    // Build the daemon state.
    let state = Arc::new(DaemonState {
        mode: config.mode,
        audit_log: audit_log.clone(),
        epoch_manager,
        attestation_manager,
        change_streams,
        data_dir: config.data_dir.clone(),
        circuit_dir: config.circuit_dir.clone(),
        proving_key_path: config.proving_key_path.clone(),
        ipfs_config: IpfsConfig {
            api_url: config.ipfs_api_url.clone(),
            cid_version: 1,
        },
        pinata_config: config.pinata_config.clone(),
        rpc_url: config.rpc_url.clone(),
        mongo_client,
        attester_key,
        attester_address,
        oplog_hash_required: config.oplog_hash_required,
        signing_keypair,
        attester_stellar_keypair,
        chain: config.chain.clone(),
    });

    // Set up cleanup to remove the PID file on exit.
    let pid_file_cleanup = pid_file.clone();
    let cleanup = move || {
        let _ = std::fs::remove_file(&pid_file_cleanup);
    };

    // Start the HTTP server.
    log::info!(
        "nosqlbuddy-audit listening on http://0.0.0.0:{}",
        config.port
    );
    let result = audit_service::auditd::run_server(state, config.port).await;

    // Cleanup PID file on exit.
    cleanup();

    result?;
    Ok(())
}

/// Parse `start` subcommand arguments into a DaemonConfig.
fn parse_start_args(args: &[String]) -> DaemonConfig {
    let mut config = DaemonConfig::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    config.mode = match args[i].as_str() {
                        "publish" => DaemonMode::Publish,
                        "read" => DaemonMode::Read,
                        "attest" => DaemonMode::Attest,
                        _ => {
                            eprintln!("error: --mode must be 'publish', 'read', or 'attest'");
                            std::process::exit(1);
                        }
                    };
                }
            }
            "--mongo-uri" => {
                i += 1;
                if i < args.len() {
                    config.mongo_uri = Some(args[i].clone());
                }
            }
            "--data-dir" => {
                i += 1;
                if i < args.len() {
                    config.data_dir = PathBuf::from(&args[i]);
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    config.port = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("error: --port must be a number");
                        std::process::exit(1);
                    });
                }
            }
            "--circuit-dir" => {
                i += 1;
                if i < args.len() {
                    config.circuit_dir = Some(PathBuf::from(&args[i]));
                }
            }
            "--proving-key" => {
                i += 1;
                if i < args.len() {
                    config.proving_key_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--ipfs-api" => {
                i += 1;
                if i < args.len() {
                    config.ipfs_api_url = args[i].clone();
                }
            }
            "--rpc-url" => {
                i += 1;
                if i < args.len() {
                    config.rpc_url = args[i].clone();
                    config.chain.rpc_url = args[i].clone();
                }
            }
            "--epoch-threshold" => {
                i += 1;
                if i < args.len() {
                    config.epoch_threshold = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("error: --epoch-threshold must be a number");
                        std::process::exit(1);
                    });
                }
            }
            "--epoch-time-secs" => {
                i += 1;
                if i < args.len() {
                    config.epoch_time_secs = args[i].parse().unwrap_or_else(|_| {
                        eprintln!("error: --epoch-time-secs must be a number");
                        std::process::exit(1);
                    });
                }
            }
            "--attester-key-file" => {
                i += 1;
                if i < args.len() {
                    config.attester_key_file = Some(PathBuf::from(&args[i]));
                }
            }
            "--secret-key" => {
                i += 1;
                if i < args.len() {
                    config.secret_key = Some(args[i].clone());
                }
            }
            "--attester-secret-key" => {
                i += 1;
                if i < args.len() {
                    config.attester_secret_key = Some(args[i].clone());
                }
            }
            "--pinata-api-key" => {
                i += 1;
                if i < args.len() {
                    let mut pc = config.pinata_config.clone().unwrap_or_default();
                    pc.api_key = args[i].clone();
                    config.pinata_config = Some(pc);
                }
            }
            "--pinata-api-secret" => {
                i += 1;
                if i < args.len() {
                    let mut pc = config.pinata_config.clone().unwrap_or_default();
                    pc.api_secret = args[i].clone();
                    config.pinata_config = Some(pc);
                }
            }
            "--pinata-gateway-url" => {
                i += 1;
                if i < args.len() {
                    let mut pc = config.pinata_config.clone().unwrap_or_default();
                    pc.gateway_url = args[i].clone();
                    config.pinata_config = Some(pc);
                }
            }
            "--network" => {
                i += 1;
                if i < args.len() {
                    config.chain = match args[i].as_str() {
                        "testnet" => audit_service::auditd::DaemonChainConfig::testnet(),
                        "mainnet" => audit_service::auditd::DaemonChainConfig::mainnet(
                            config.rpc_url.clone(),
                            String::new(),
                        ),
                        _ => {
                            eprintln!("error: --network must be 'testnet' or 'mainnet'");
                            std::process::exit(1);
                        }
                    };
                }
            }
            "--contract-id" => {
                i += 1;
                if i < args.len() {
                    config.chain.contract_id = args[i].clone();
                }
            }
            "--horizon-url" => {
                i += 1;
                if i < args.len() {
                    config.chain.horizon_url = args[i].clone();
                }
            }
            "--oplog-hash-required" => {
                config.oplog_hash_required = true;
            }
            "--help" | "-h" => {
                print_start_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("error: unknown argument '{other}'");
                print_start_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Read Pinata credentials from environment if not provided via CLI.
    let pinata_api_key = std::env::var("PINATA_API_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let pinata_api_secret = std::env::var("PINATA_API_SECRET")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let pinata_gateway_url = std::env::var("PINATA_GATEWAY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if pinata_api_key.is_some() || pinata_api_secret.is_some() || pinata_gateway_url.is_some() {
        let mut pc = config.pinata_config.clone().unwrap_or_default();
        if let Some(key) = pinata_api_key {
            pc.api_key = key;
        }
        if let Some(secret) = pinata_api_secret {
            pc.api_secret = secret;
        }
        if let Some(gateway) = pinata_gateway_url {
            pc.gateway_url = gateway;
        }
        config.pinata_config = Some(pc);
    }

    // Validate: publisher and attester modes require --mongo-uri.
    if config.mode == DaemonMode::Publish && config.mongo_uri.is_none() {
        eprintln!("error: --mode publish requires --mongo-uri");
        eprintln!("  Example: nosqlbuddy-audit start --mode publish --mongo-uri mongodb://localhost:27017");
        std::process::exit(1);
    }
    if config.mode == DaemonMode::Attest && config.mongo_uri.is_none() {
        eprintln!(
            "error: --mode attest requires --mongo-uri (connect to the independent replica member)"
        );
        eprintln!(
            "  Example: nosqlbuddy-audit start --mode attest --mongo-uri mongodb://localhost:27019"
        );
        std::process::exit(1);
    }

    config
}

// ─── Subcommand: stop ─────────────────────────────────────────────────

/// Stop a running service.
fn cmd_stop(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = parse_data_dir_arg(args);
    let port = parse_port_arg(args, 9173);
    let pid_file = pid_file_path(&data_dir, port);

    if !pid_file.exists() {
        eprintln!(
            "nosqlbuddy-audit is not running (no PID file at {})",
            pid_file.display()
        );
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
    let pid: i32 = pid_str.trim().parse().unwrap_or(0);

    if pid <= 0 {
        eprintln!("error: invalid PID in {}", pid_file.display());
        std::fs::remove_file(&pid_file).ok();
        std::process::exit(1);
    }

    if !is_process_running(pid) {
        eprintln!("nosqlbuddy-audit (PID {pid}) is not running — removing stale PID file");
        std::fs::remove_file(&pid_file).ok();
        std::process::exit(0);
    }

    // Send SIGTERM (Unix) or TerminateProcess (Windows).
    let result = kill_process(pid);
    match result {
        Ok(()) => {
            // Wait for the process to exit.
            for _ in 0..50 {
                if !is_process_running(pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            std::fs::remove_file(&pid_file).ok();
            println!("nosqlbuddy-audit (PID {pid}) stopped");
        }
        Err(e) => {
            eprintln!("error: failed to stop PID {pid}: {e}");
            std::process::exit(1);
        }
    }

    Ok(())
}

// ─── Subcommand: status ───────────────────────────────────────────────

/// Check if the service is running.
async fn cmd_status(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = parse_data_dir_arg(args);
    let port = parse_port_arg(args, 9173);
    let pid_file = pid_file_path(&data_dir, port);

    if !pid_file.exists() {
        println!("nosqlbuddy-audit: not running (no PID file)");
        std::process::exit(1);
    }

    let pid_str = std::fs::read_to_string(&pid_file).unwrap_or_default();
    let pid: i32 = pid_str.trim().parse().unwrap_or(0);

    if pid <= 0 || !is_process_running(pid) {
        println!("nosqlbuddy-audit: not running (stale PID file)");
        std::fs::remove_file(&pid_file).ok();
        std::process::exit(1);
    }

    println!("nosqlbuddy-audit: running (PID {pid})");
    println!("  PID file: {}", pid_file.display());

    // Health check: hit the /status endpoint.
    let url = format!("http://localhost:{port}/status");
    match reqwest::get(&url).await {
        Ok(resp) if resp.status().is_success() => {
            println!("  Health:   OK (HTTP {} at {url})", resp.status());
        }
        Ok(resp) => {
            println!("  Health:   DEGRADED (HTTP {} at {url})", resp.status());
        }
        Err(e) => {
            println!("  Health:   UNREACHABLE ({url}: {e})");
        }
    }

    Ok(())
}

// ─── Helpers ──────────────────────────────────────────────────────────

/// Get the PID file path for a given data dir + port.
fn pid_file_path(data_dir: &PathBuf, port: u16) -> PathBuf {
    data_dir.join(format!("nosqlbuddy-audit-{port}.pid"))
}

/// Check if a process is running.
fn is_process_running(pid: i32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) returns 0 if the process exists, -1 otherwise.
        unsafe { libc::kill(pid, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        // On Windows, check if the process handle is valid.
        let handle =
            unsafe { windows::Win32::System::Threading::OpenProcess(0, false, pid as u32) };
        !handle.is_invalid()
    }
}

/// Send SIGTERM to a process.
fn kill_process(pid: i32) -> Result<(), String> {
    #[cfg(unix)]
    {
        let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
        if rc == 0 {
            Ok(())
        } else {
            Err(format!(
                "kill({pid}, SIGTERM) failed: errno {}",
                std::io::Error::last_os_error()
            ))
        }
    }
    #[cfg(not(unix))]
    {
        // On Windows, use TerminateProcess.
        let handle =
            unsafe { windows::Win32::System::Threading::OpenProcess(0x0001, false, pid as u32) };
        if handle.is_invalid() {
            return Err("OpenProcess failed".to_string());
        }
        let rc = unsafe { windows::Win32::System::Threading::TerminateProcess(handle, 1) };
        if rc.is_ok() {
            Ok(())
        } else {
            Err("TerminateProcess failed".to_string())
        }
    }
}

/// Parse --data-dir from args (shared by stop/status).
fn parse_data_dir_arg(args: &[String]) -> PathBuf {
    let mut i = 0;
    let mut data_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("nosqlbuddy-audit");
    while i < args.len() {
        if args[i] == "--data-dir" && i + 1 < args.len() {
            data_dir = PathBuf::from(&args[i + 1]);
        }
        i += 1;
    }
    data_dir
}

/// Parse --port from args (shared by stop/status).
fn parse_port_arg(args: &[String], default: u16) -> u16 {
    let mut i = 0;
    let mut port = default;
    while i < args.len() {
        if args[i] == "--port" && i + 1 < args.len() {
            port = args[i + 1].parse().unwrap_or(default);
        }
        i += 1;
    }
    port
}

/// Redact credentials from a MongoDB URI for logging.
fn redact_uri(uri: &str) -> String {
    if let Some(at_pos) = uri.rfind('@') {
        if let Some(scheme_end) = uri.find("://") {
            let creds_start = scheme_end + 3;
            if creds_start < at_pos {
                let (before, after) = uri.split_at(at_pos + 1);
                let _ = before;
                let scheme = &uri[..creds_start];
                return format!("{scheme}***@{after}");
            }
        }
    }
    uri.to_string()
}

// ─── Help ─────────────────────────────────────────────────────────────

fn print_help() {
    eprintln!(
        "nosqlbuddy-audit — standalone ZK audit service for NoSQLBuddy\n\
        \n\
        Usage:\n\
          nosqlbuddy-audit <subcommand> [options]\n\
        \n\
        Subcommands:\n\
          setup    Interactive wizard: generate keys, deploy contract, authorize attester\n\
          start    Start the audit service (publisher / reader / attester mode)\n\
          stop     Stop a running service\n\
          status   Check if the service is running\n\
        \n\
        Run 'nosqlbuddy-audit <subcommand> --help' for subcommand-specific options.\n\
        \n\
        Examples:\n\
          nosqlbuddy-audit setup\n\
          nosqlbuddy-audit start --mode publish --mongo-uri mongodb://localhost:27017\n\
          nosqlbuddy-audit start --mode attest --mongo-uri mongodb://localhost:27019\n\
          nosqlbuddy-audit start --mode read\n\
          nosqlbuddy-audit stop\n\
          nosqlbuddy-audit status"
    );
}

fn print_start_help() {
    eprintln!(
        "nosqlbuddy-audit start — start the audit service\n\
        \n\
        Usage:\n\
          nosqlbuddy-audit start --mode <publish|read|attest> [options]\n\
        \n\
        Options:\n\
          --mode <publish|read|attest>  Service mode (default: publish)\n\
          --mongo-uri <uri>             MongoDB connection URI (required for publish/attest)\n\
          --data-dir <dir>              Data directory (default: OS data dir)\n\
          --port <port>                 HTTP API port (default: 9173)\n\
          --circuit-dir <dir>           Circuit artifacts directory (for proof generation)\n\
          --proving-key <path>          Path to pre-generated proving key (from ceremony; speeds up proofs)\n\
          --ipfs-api <url>              IPFS Kubo HTTP API URL (default: http://127.0.0.1:5001)\n\
          --rpc-url <url>               Stellar Soroban RPC URL (default: testnet)\n\
          --epoch-threshold <n>         Auto-close epoch after N events (default: 100, 0=disabled)\n\
          --epoch-time-secs <s>         Auto-close epoch after S seconds (default: 0=disabled)\n\
          --attester-key-file <path>    Path to the ed25519 attester signing key (attest mode; generated if missing)\n\
          --attester-identity <name>    Deprecated. Use --attester-secret-key instead\n\
          --attester-address <addr>     Stellar account address of the attester (derived from keypair if not set)\n\
          --secret-key <S...>           Stellar secret key for the publisher (native signing)\n\
          --attester-secret-key <S...>  Stellar secret key for the attester's account (native signing)\n\
          --pinata-api-key <key>        Pinata API key for cloud IPFS pinning\n\
          --pinata-api-secret <secret>  Pinata API secret for cloud IPFS pinning\n\
          --pinata-gateway-url <url>    Pinata gateway URL (default: https://gateway.pinata.cloud)\n\
          --network <testnet|mainnet>   Stellar network (default: testnet)\n\
          --contract-id <C...>          Soroban contract ID (default: testnet contract)\n\
          --horizon-url <url>           Horizon API URL for account lookups (default: testnet)\n\
          --oplog-hash-required         Fail epoch close if oplog hash computation fails\n\
          --help, -h                    Show this help message"
    );
}
