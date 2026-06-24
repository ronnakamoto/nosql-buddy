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
    });

    // Publisher mode: connect to MongoDB and start the change stream listener.
    if config.mode == DaemonMode::Publish {
        let mongo_uri = config.mongo_uri.as_deref().ok_or_else(|| {
            "publisher mode requires --mongo-uri".to_string()
        })?;

        log::info!("connecting to MongoDB: {}", redact_uri(mongo_uri));
        let client = mongodb::Client::with_uri_str(mongo_uri).await?;
        let connection_id = "auditd".to_string();

        // Start the change stream listener.
        state
            .change_streams
            .start_for(connection_id.clone(), client, audit_log.clone())
            .await;
        log::info!("change stream listener started for connection {connection_id}");
    }

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
                        _ => {
                            eprintln!("error: --mode must be 'publish' or 'read'");
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

    // Validate: publisher mode requires --mongo-uri.
    if config.mode == DaemonMode::Publish && config.mongo_uri.is_none() {
        eprintln!("error: --mode publish requires --mongo-uri");
        eprintln!("  Example: nosqlbuddy-auditd --mode publish --mongo-uri mongodb://localhost:27017");
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
        \n\
        Options:\n\
          --mode <publish|read>    Daemon mode (default: publish)\n\
          --mongo-uri <uri>        MongoDB connection URI (required for publish)\n\
          --data-dir <dir>         Data directory (default: OS data dir)\n\
          --port <port>            HTTP API port (default: 9173)\n\
          --circuit-dir <dir>      Circuit artifacts directory (for proof generation)\n\
          --ipfs-api <url>         IPFS Kubo HTTP API URL (default: http://127.0.0.1:5001)\n\
          --rpc-url <url>          Stellar Soroban RPC URL (default: testnet)\n\
          --epoch-threshold <n>    Auto-close epoch after N events (default: 100, 0=disabled)\n\
          --epoch-time-secs <s>    Auto-close epoch after S seconds (default: 0=disabled)\n\
          --help, -h               Show this help message\n\
        \n\
        Publisher mode endpoints (localhost:9173):\n\
          GET  /status             Audit log status\n\
          GET  /events             List audit events\n\
          GET  /root               Current Merkle root\n\
          POST /proof/:index       Generate Groth16 inclusion proof\n\
          GET  /epochs             List all epochs\n\
          GET  /epoch/current      Current open epoch\n\
          POST /epoch/close        Close current epoch (freeze root)\n\
          POST /epoch/:n/commit    Commit epoch root to Stellar\n\
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
          POST /reader/rebuild     Rebuild/verify from chain + IPFS"
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
