//! Connection profile persistence. Profile metadata is stored as JSON via
//! `tauri-plugin-store`; secrets are stored in the OS keychain. The
//! `ProfileRepository` orchestrates both. The frontend never sees the raw
//! secret; it sees a summary with `hasSecret: true` plus a masked URI.

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri_plugin_store::StoreExt;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::mongo::credentials::SecretStore;
use crate::mongo::types::{AuthMechanism, ConnectionProfile, ProfileSummary};

const STORE_FILE: &str = "nosqlbuddy.profiles.json";
const PROFILES_KEY: &str = "profiles";

/// On-disk representation of a profile. Never contains the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredProfile {
    id: String,
    name: String,
    uri: String,
    auth_mechanism: AuthMechanism,
    has_secret: bool,
    group: Option<String>,
    color: Option<String>,
    notes: Option<String>,
    ssh_tunnel: Option<crate::mongo::types::SshTunnelConfig>,
    socks5: Option<crate::mongo::types::Socks5Config>,
}

impl From<&ConnectionProfile> for StoredProfile {
    fn from(p: &ConnectionProfile) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            uri: p.uri.clone(),
            auth_mechanism: p.auth_mechanism,
            has_secret: p.secret.is_some(),
            group: p.group.clone(),
            color: p.color.clone(),
            notes: p.notes.clone(),
            ssh_tunnel: p.ssh_tunnel.clone(),
            socks5: p.socks5.clone(),
        }
    }
}

impl From<StoredProfile> for ProfileSummary {
    fn from(s: StoredProfile) -> Self {
        ProfileSummary::from_stored(
            s.id,
            s.name,
            s.uri,
            s.auth_mechanism,
            s.has_secret,
            s.group,
            s.color,
            s.notes,
            s.ssh_tunnel,
            s.socks5,
        )
    }
}

/// Repository for connection profiles. Splits secret vs metadata storage so
/// the secret never lives on disk in plaintext.
pub struct ProfileRepository {
    #[allow(dead_code)]
    store_path: PathBuf,
    secrets: Arc<dyn SecretStore>,
}

impl ProfileRepository {
    pub fn new(store_path: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            store_path,
            secrets,
        }
    }

    fn load_all(&self, app: &tauri::AppHandle) -> AppResult<Vec<StoredProfile>> {
        let store = app
            .store(STORE_FILE)
            .map_err(|e| AppError::Internal(format!("profile store open: {e}")))?;
        let value = store.get(PROFILES_KEY);
        match value {
            Some(v) => Ok(serde_json::from_value(v)
                .map_err(|e| AppError::Internal(format!("profile decode: {e}")))?),
            None => Ok(Vec::new()),
        }
    }

    fn save_all(
        &self,
        app: &tauri::AppHandle,
        profiles: &[StoredProfile],
    ) -> AppResult<()> {
        let store = app
            .store(STORE_FILE)
            .map_err(|e| AppError::Internal(format!("profile store open: {e}")))?;
        let value = serde_json::to_value(profiles)?;
        store.set(PROFILES_KEY, value);
        store
            .save()
            .map_err(|e| AppError::Internal(format!("profile store save: {e}")))?;
        Ok(())
    }

    pub fn list_summaries(
        &self,
        app: &tauri::AppHandle,
    ) -> AppResult<Vec<ProfileSummary>> {
        let stored = self.load_all(app)?;
        let summaries: Vec<ProfileSummary> = stored
            .iter()
            .map(|p| ProfileSummary::from(p.clone()))
            .collect();
        Ok(summaries)
    }

    pub fn upsert(
        &self,
        app: &tauri::AppHandle,
        mut profile: ConnectionProfile,
    ) -> AppResult<ConnectionProfile> {
        if profile.id.is_empty() {
            profile.id = Uuid::new_v4().to_string();
        }
        let mut profiles = self.load_all(app)?;
        if let Some(existing) = profiles.iter().find(|p| p.id == profile.id).cloned() {
            if existing.name != profile.name
                && profiles.iter().any(|p| p.id != profile.id && p.name == profile.name)
            {
                return Err(AppError::ProfileExists(profile.name));
            }
        } else if profiles.iter().any(|p| p.name == profile.name) {
            return Err(AppError::ProfileExists(profile.name));
        }
        // Persist secret to keychain, then strip from the profile before storing
        if let Some(secret) = profile.secret.take() {
            if secret.is_empty() {
                let _ = self.secrets.delete(&profile.id);
            } else {
                self.secrets.put(&profile.id, &secret)?;
            }
        }
        let stored = StoredProfile::from(&profile);
        profiles.retain(|p| p.id != stored.id);
        profiles.push(stored);
        self.save_all(app, &profiles)?;
        Ok(profile)
    }

    pub fn get(
        &self,
        app: &tauri::AppHandle,
        id: &str,
    ) -> AppResult<ConnectionProfile> {
        let stored = self
            .load_all(app)?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| AppError::ProfileNotFound(id.to_string()))?;
        let secret = self.secrets.get(id)?;
        Ok(ConnectionProfile {
            id: stored.id,
            name: stored.name,
            uri: stored.uri,
            auth_mechanism: stored.auth_mechanism,
            secret,
            group: stored.group,
            color: stored.color,
            notes: stored.notes,
            ssh_tunnel: stored.ssh_tunnel,
            socks5: stored.socks5,
        })
    }

    pub fn delete(&self, app: &tauri::AppHandle, id: &str) -> AppResult<()> {
        let mut profiles = self.load_all(app)?;
        let before = profiles.len();
        profiles.retain(|p| p.id != id);
        if profiles.len() == before {
            return Err(AppError::ProfileNotFound(id.to_string()));
        }
        self.save_all(app, &profiles)?;
        let _ = self.secrets.delete(id);
        Ok(())
    }
}
