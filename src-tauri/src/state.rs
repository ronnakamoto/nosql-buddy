//! Typed application state managed by `tauri::State`.
//!
//! State is shared across command handlers via `tauri::State<'_, AppState>`.
//! The Mongo domain owns its own sub-state (profile repository, secret store,
//! client registry) so the in-memory scaffolding is no longer a free-for-all.

use std::sync::Arc;

use crate::audit::attestation::AttestationManager;
use crate::audit::change_stream::ChangeStreamRegistry;
use crate::audit::epoch::EpochManager;
use crate::audit::verification_store::VerificationStore;
use crate::audit::AuditLog;
use crate::mongo::client_registry::ClientRegistry;
use crate::mongo::credentials::{InMemorySecretStore, KeyringSecretStore, SecretStore};
use crate::mongo::job_store::JobStore;
use crate::mongo::profiles::ProfileRepository;
use crate::mongo::shell::ShellRegistry;
use crate::mongo::timeline_store::TimelineStore;

pub struct AppState {
    pub profiles: Arc<ProfileRepository>,
    pub secrets: Arc<dyn SecretStore>,
    pub clients: ClientRegistry,
    pub shell_registry: ShellRegistry,
    pub jobs: JobStore,
    pub timeline: Arc<TimelineStore>,
    pub audit_log: Arc<AuditLog>,
    pub change_streams: ChangeStreamRegistry,
    pub epoch_manager: EpochManager,
    pub attestation_manager: AttestationManager,
    pub verification_store: VerificationStore,
}

impl AppState {
    pub fn new() -> Self {
        Self::with_secrets(Arc::new(KeyringSecretStore))
    }

    pub fn with_in_memory_secrets() -> Self {
        Self::with_secrets(Arc::new(InMemorySecretStore::new()))
    }

    fn with_secrets(secrets: Arc<dyn SecretStore>) -> Self {
        // The store path is resolved per-app; the actual store handle is
        // obtained lazily inside the profile repository so we can hand
        // `tauri::AppHandle` to it. We use a placeholder here.
        let store_path = std::env::temp_dir().join("nosqlbuddy");
        let profiles = Arc::new(ProfileRepository::new(store_path, secrets.clone()));
        let data_dir = dirs::data_dir().map(|d| d.join("nosqlbuddy"));
        if let Some(ref p) = data_dir {
            let _ = std::fs::create_dir_all(p);
        }
        let jobs_path = data_dir.as_ref().map(|d| d.join("jobs.jsonl"));
        let timeline_path = data_dir.map(|d| d.join("timeline.jsonl"));
        Self {
            profiles,
            secrets,
            clients: ClientRegistry::new(),
            shell_registry: ShellRegistry::new(),
            jobs: JobStore::with_path(jobs_path),
            timeline: Arc::new(TimelineStore::with_path(timeline_path)),
            audit_log: Arc::new(build_audit_log()),
            change_streams: ChangeStreamRegistry::new(),
            epoch_manager: EpochManager::default(),
            attestation_manager: AttestationManager::default(),
            verification_store: VerificationStore::default(),
        }
    }
}

/// Construct the audit log, failing fast with an actionable, logged message.
///
/// `AuditLog::new()` only fails if the underlying Poseidon hasher cannot be
/// initialized, which is a deterministic, environment-level fault (not a
/// transient one), so retrying or shrinking the Merkle tree cannot recover it.
/// The whole audit subsystem (and every command that records an audit event)
/// depends on this handle, so we treat a failure as fatal — but we log the
/// real cause first instead of surfacing a bare `expect` panic with no context.
fn build_audit_log() -> AuditLog {
    match AuditLog::new() {
        Ok(log) => log,
        Err(e) => {
            tracing::error!(
                error = %e,
                "fatal: could not initialize the audit log (Poseidon/Merkle init failed); \
                 the audit subsystem cannot start"
            );
            panic!("audit log initialization failed: {e}");
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
