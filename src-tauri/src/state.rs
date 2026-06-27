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

pub struct AppState {
    pub profiles: Arc<ProfileRepository>,
    pub secrets: Arc<dyn SecretStore>,
    pub clients: ClientRegistry,
    pub shell_registry: ShellRegistry,
    pub jobs: JobStore,
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
        let jobs_path = dirs::data_dir().map(|d| d.join("nosqlbuddy").join("jobs.jsonl"));
        if let Some(ref p) = jobs_path {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
        }
        Self {
            profiles,
            secrets,
            clients: ClientRegistry::new(),
            shell_registry: ShellRegistry::new(),
            jobs: JobStore::with_path(jobs_path),
            audit_log: Arc::new(AuditLog::new().expect("failed to create audit log")),
            change_streams: ChangeStreamRegistry::new(),
            epoch_manager: EpochManager::default(),
            attestation_manager: AttestationManager::default(),
            verification_store: VerificationStore::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
