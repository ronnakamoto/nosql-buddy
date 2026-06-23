//! Typed application state managed by `tauri::State`.
//!
//! State is shared across command handlers via `tauri::State<'_, AppState>`.
//! The Mongo domain owns its own sub-state (profile repository, secret store,
//! client registry) so the in-memory scaffolding is no longer a free-for-all.

use std::sync::Arc;

use crate::audit::AuditLog;
use crate::mongo::client_registry::ClientRegistry;
use crate::mongo::credentials::{InMemorySecretStore, KeyringSecretStore, SecretStore};
use crate::mongo::profiles::ProfileRepository;
use crate::mongo::shell::ShellRegistry;

pub struct AppState {
    pub profiles: Arc<ProfileRepository>,
    pub secrets: Arc<dyn SecretStore>,
    pub clients: ClientRegistry,
    pub shell_registry: ShellRegistry,
    pub audit_log: Arc<AuditLog>,
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
        Self {
            profiles,
            secrets,
            clients: ClientRegistry::new(),
            shell_registry: ShellRegistry::new(),
            audit_log: Arc::new(AuditLog::new().expect("failed to create audit log")),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
