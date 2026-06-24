//! nosqlbuddy-auditd — standalone audit daemon.
//!
//! One binary, two modes:
//!   --mode publish   MongoDB change stream → Merkle tree → IPFS → Stellar
//!   --mode read      Read on-chain commitments → verify local log
//!
//! Usage:
//!   nosqlbuddy-auditd --mode publish --mongo-uri mongodb://localhost:27017
//!   nosqlbuddy-auditd --mode read --data-dir ~/.nosqlbuddy/auditd
//!
//! The daemon reuses the same audit modules as the NoSQLBuddy Tauri app.
//! No code duplication — it links against `app_lib` directly.

use std::path::PathBuf;
use std::sync::Arc;

use app_lib::audit::attestation::AttestationManager;
use app_lib::audit::change_stream::ChangeStreamRegistry;
use app_lib::audit::epoch::EpochManager;
use app_lib::audit::ipfs::IpfsConfig;
use app_lib::audit::sled_store::SledTreeStore;
use app_lib::audit::AuditLog;
use app_lib::auditd::{DaemonConfig, DaemonMode, DaemonState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,app_lib=info".into()),
        )
        .try_init();

    let config = parse_args();
    log::info!(
        "nosqlbuddy-auditd starting in {:?} mode, data dir: {}, port: {}",
        config.mode,
        config.data_dir.display(),
        config.port
    );

    // Ensure the data directory exists.
    std::fs::create_dir_all(&config.data_dir)?;

    // Initialize the audit log with persistence.
    let audit_log = Arc::new(AuditLog::new()?);
    audit_log.set_persistence_dir(&config.data_dir)?;
    log::info!(
        "audit log initialized: {} events, root: {}",
        audit_log.event_count(),
        audit_log.root_hex()?
    );

    // Initialize the attestation manager with a sled store.
    // Uses a separate sled path from the audit log's tree store to avoid lock conflicts.
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

    let epoch_manager = EpochManager::new(app_lib::audit::epoch::EpochConfig {
        event_threshold: config.epoch_threshold,
        time_threshold_secs: config.epoch_time_secs,
    });
    log::info!(
        "epoch manager configured: threshold={} events, time={}s",
        config.epoch_threshold, config.epoch_time_secs
    );
    let change_streams = ChangeStreamRegistry::new();

    // Publisher and attester modes both need a MongoDB connection.
    // Publisher: connects to the primary to run the change stream listener
    //   and compute oplog hashes.
    // Attester: connects to the independent replica member to independently
    //   compute oplog hashes and submit attestations.
    let mut mongo_client: Option<mongodb::Client> = None;
    if config.mode == DaemonMode::Publish {
        let mongo_uri = config.mongo_uri.as_deref().ok_or_else(|| {
            "publisher mode requires --mongo-uri".to_string()
        })?;

        log::info!("connecting to MongoDB: {}", redact_uri(mongo_uri));
        let mongo_uri = app_lib::mongo::client_registry::ensure_direct_connection(mongo_uri);
        let client = mongodb::Client::with_uri_str(&mongo_uri).await?;
        let connection_id = "auditd".to_string();

        // Start the change stream listener.
        change_streams
            .start_for(connection_id.clone(), client.clone(), audit_log.clone())
            .await;
        log::info!("change stream listener started for connection {connection_id}");

        mongo_client = Some(client);
    } else if config.mode == DaemonMode::Attest {
        let mongo_uri = config.mongo_uri.as_deref().ok_or_else(|| {
            "attester mode requires --mongo-uri (connect to the independent replica member)".to_string()
        })?;

        log::info!("attester: connecting to independent replica: {}", redact_uri(mongo_uri));
        let mongo_uri = app_lib::mongo::client_registry::ensure_direct_connection(mongo_uri);
        let client = mongodb::Client::with_uri_str(&mongo_uri).await?;
        log::info!("attester: connected to independent replica member");
        mongo_client = Some(client);

        if config.attester_identity.is_none() {
            eprintln!("error: --mode attest requires --attester-identity");
            std::process::exit(1);
        }
        if config.attester_address.is_none() {
            eprintln!("error: --mode attest requires --attester-address");
            std::process::exit(1);
        }
    }

    // Load or generate the attester signing key for attester mode.
    let mut attester_key: Option<ed25519_dalek::SigningKey> = None;
    let mut attester_address: Option<String> = None;
    if config.mode == DaemonMode::Attest {
        let key_file = config.attester_key_file.clone().unwrap_or_else(|| {
            config.data_dir.join("audit").join("attester.key")
        });
        let key = app_lib::auditd::attester::load_or_generate_attester_key(&key_file)
            .map_err(|e| format!("failed to load attester key: {e}"))?;
        let public_key_hex = hex::encode(key.verifying_key().to_bytes());
        log::info!("attester: loaded key {key_file:?}; public key: {public_key_hex}");
        attester_key = Some(key);
        attester_address = config.attester_address.clone();
    }

    // Build the daemon state.
    let state = Arc::new(DaemonState {
        mode: config.mode,
        audit_log: audit_log.clone(),
        epoch_manager,
        attestation_manager,
        change_streams,
        data_dir: config.data_dir.clone(),
        circuit_dir: config.circuit_dir.clone(),
        ipfs_config: IpfsConfig {
            api_url: config.ipfs_api_url.clone(),
            cid_version: 1,
        },
        rpc_url: config.rpc_url.clone(),
        mongo_client,
        attester_key,
        attester_identity: config.attester_identity.clone(),
        attester_address,
        oplog_hash_required: config.oplog_hash_required,
    });

    // Start the HTTP server.
    app_lib::auditd::run_server(state, config.port).await?;

    Ok(())
}

/// Parse CLI arguments into a DaemonConfig.
///
/// Supported flags:
///   --mode <publish|read>         Daemon mode (default: publish)
///   --mongo-uri <uri>             MongoDB connection URI (required for publish)
///   --data-dir <dir>              Data directory (default: OS data dir)
///   --port <port>                 HTTP port (default: 9173)
///   --circuit-dir <dir>           Circuit artifacts directory (for proof generation)
///   --ipfs-api <url>              IPFS Kubo HTTP API URL (default: http://127.0.0.1:5001)
///   --rpc-url <url>               Stellar Soroban RPC URL (default: testnet)
///   --epoch-threshold <n>         Auto-close epoch after N events (default: 100, 0=disabled)
///   --epoch-time-secs <s>         Auto-close epoch after S seconds (default: 0=disabled)
///   --help, -h                    Show help
fn parse_args() -> DaemonConfig {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
            "--attester-identity" => {
                i += 1;
                if i < args.len() {
                    config.attester_identity = Some(args[i].clone());
                }
            }
            "--attester-address" => {
                i += 1;
                if i < args.len() {
                    config.attester_address = Some(args[i].clone());
                }
            }
            "--oplog-hash-required" => {
                config.oplog_hash_required = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("error: unknown argument '{other}'");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // Validate: publisher and attester modes require --mongo-uri.
    if config.mode == DaemonMode::Publish && config.mongo_uri.is_none() {
        eprintln!("error: --mode publish requires --mongo-uri");
        eprintln!("  Example: nosqlbuddy-auditd --mode publish --mongo-uri mongodb://localhost:27017");
        std::process::exit(1);
    }
    if config.mode == DaemonMode::Attest && config.mongo_uri.is_none() {
        eprintln!("error: --mode attest requires --mongo-uri (connect to the independent replica member)");
        eprintln!("  Example: nosqlbuddy-auditd --mode attest --mongo-uri mongodb://localhost:27019");
        std::process::exit(1);
    }

    config
}

fn print_help() {
    eprintln!(
        "nosqlbuddy-auditd — standalone ZK audit daemon for NoSQLBuddy\n\
        \n\
        Usage:\n\
          nosqlbuddy-auditd --mode publish --mongo-uri <uri> [options]\n\
          nosqlbuddy-auditd --mode read [options]\n\
          nosqlbuddy-auditd --mode attest --mongo-uri <uri> [options]\n\
        \n\
        Options:\n\
          --mode <publish|read|attest>  Daemon mode (default: publish)\n\
          --mongo-uri <uri>             MongoDB connection URI (required for publish/attest)\n\
          --data-dir <dir>              Data directory (default: OS data dir)\n\
          --port <port>                 HTTP API port (default: 9173)\n\
          --circuit-dir <dir>           Circuit artifacts directory (for proof generation)\n\
          --ipfs-api <url>              IPFS Kubo HTTP API URL (default: http://127.0.0.1:5001)\n\
          --rpc-url <url>               Stellar Soroban RPC URL (default: testnet)\n\
          --epoch-threshold <n>         Auto-close epoch after N events (default: 100, 0=disabled)\n\
          --epoch-time-secs <s>         Auto-close epoch after S seconds (default: 0=disabled)\n\
          --attester-key-file <path>    Path to the ed25519 attester signing key (attest mode; generated if missing)\n\
          --attester-identity <name>    Stellar CLI identity for attester transactions (attest mode)\n\
          --attester-address <addr>     Stellar account address of the attester (attest mode)\n\
          --oplog-hash-required         Fail epoch close if oplog hash computation fails\n\
          --help, -h                    Show this help message\n\
        \n\
        Publisher mode endpoints (localhost:9173):\n\
          GET  /status             Audit log status\n\
          GET  /events             List audit events\n\
          GET  /root               Current Merkle root\n\
          POST /proof/:index       Generate Groth16 inclusion proof\n\
          GET  /epochs             List all epochs\n\
          GET  /epoch/current      Current open epoch\n\
          POST /epoch/close        Close current epoch (freeze root + oplog hash)\n\
          POST /epoch/:n/commit    Commit epoch root to Stellar (with oplog hash)\n\
          POST /epoch/:n/publish-ipfs  Publish epoch events to IPFS\n\
          GET  /epoch/:n/ipfs-cid  Get IPFS CID for an epoch\n\
          GET  /onchain-root       Latest committed root (via RPC)\n\
          GET  /ipfs/check         Check if IPFS daemon is reachable\n\
          GET  /publishers         List registered publishers\n\
          POST /publishers         Register a publisher\n\
          DELETE /publishers/:key  Remove a publisher\n\
          POST /attestations       Submit an attestation\n\
          GET  /attestations/:n    List attestations for an epoch\n\
          GET  /attestations/:n/status  Attestation threshold status\n\
          GET  /threshold          Get K-of-N threshold\n\
          POST /threshold          Set K-of-N threshold\n\
        \n\
        Reader mode endpoints:\n\
          GET  /status             Audit log status\n\
          GET  /events             List audit events\n\
          GET  /root               Current Merkle root\n\
          POST /proof/:index       Generate Groth16 inclusion proof\n\
          GET  /reader/verify      Verify local log against on-chain root\n\
          GET  /reader/onchain-root    Get on-chain root (via RPC)\n\
          POST /reader/rebuild     Rebuild/verify from chain + IPFS\n\
        \n\
        Attester mode endpoints:\n\
          GET  /attest/status      Attester daemon status\n\
          POST /attest/scan        Scan for unattested epochs and submit attestations\n\
          GET  /attest/attestations/:n  List attestations for an epoch"
    );
}

/// Redact credentials from a MongoDB URI for logging.
fn redact_uri(uri: &str) -> String {
    // Simple redaction: replace password in mongodb://user:pass@host
    if let Some(at_pos) = uri.rfind('@') {
        if let Some(scheme_end) = uri.find("://") {
            let creds_start = scheme_end + 3;
            if creds_start < at_pos {
                let (before, after) = uri.split_at(at_pos + 1);
                let _ = before; // includes scheme://user:pass@
                let scheme = &uri[..creds_start];
                return format!("{scheme}***@{after}");
            }
        }
    }
    uri.to_string()
}
