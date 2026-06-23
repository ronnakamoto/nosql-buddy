//! Secure credential storage backed by the OS keychain via the `keyring`
//! crate. Connections store non-secret metadata in the profile store, and
//! any password / SCRAM secret lives only in the OS keychain under the
//! profile id. No credential ever lands in the on-disk profile JSON.

use std::sync::Arc;

use keyring::Entry;

use crate::error::{AppError, AppResult};

const SERVICE: &str = "studio.nosqlbuddy.connections";

/// Trait abstraction for the secret store so business logic never depends
/// on the concrete `keyring` crate. Allows unit tests to inject an
/// in-memory implementation.
pub trait SecretStore: Send + Sync {
    fn put(&self, profile_id: &str, secret: &str) -> AppResult<()>;
    fn get(&self, profile_id: &str) -> AppResult<Option<String>>;
    fn delete(&self, profile_id: &str) -> AppResult<()>;
}

/// OS keychain adapter. Each connection profile has at most one secret.
pub struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn put(&self, profile_id: &str, secret: &str) -> AppResult<()> {
        let entry = Entry::new(SERVICE, profile_id)?;
        entry.set_password(secret)?;
        Ok(())
    }

    fn get(&self, profile_id: &str) -> AppResult<Option<String>> {
        let entry = Entry::new(SERVICE, profile_id)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(AppError::from(err)),
        }
    }

    fn delete(&self, profile_id: &str) -> AppResult<()> {
        let entry = Entry::new(SERVICE, profile_id)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(AppError::from(err)),
        }
    }
}

/// In-memory secret store used by tests.
#[derive(Default, Clone)]
pub struct InMemorySecretStore {
    inner: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn put(&self, profile_id: &str, secret: &str) -> AppResult<()> {
        self.inner
            .lock()
            .map_err(|e| AppError::Internal(format!("secret store poisoned: {e}")))?
            .insert(profile_id.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, profile_id: &str) -> AppResult<Option<String>> {
        Ok(self
            .inner
            .lock()
            .map_err(|e| AppError::Internal(format!("secret store poisoned: {e}")))?
            .get(profile_id)
            .cloned())
    }

    fn delete(&self, profile_id: &str) -> AppResult<()> {
        self.inner
            .lock()
            .map_err(|e| AppError::Internal(format!("secret store poisoned: {e}")))?
            .remove(profile_id);
        Ok(())
    }
}
