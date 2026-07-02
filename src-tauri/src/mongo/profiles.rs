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

/// Split a connection URI into a password-less URI and the effective secret to
/// persist, so a password embedded in the URI never lands on disk in plaintext.
///
/// Precedence for the resulting secret:
/// 1. An explicit, non-empty `secret` field (the dedicated password input).
/// 2. A password extracted from the URI userinfo (percent-decoded).
/// 3. The original `secret` value (`None` = keep existing, `Some("")` = clear),
///    preserved only when the URI carries no password.
///
/// The username, hosts, database, and query options in the URI are preserved.
fn split_uri_and_secret(uri: String, secret: Option<String>) -> (String, Option<String>) {
    let stripped = mongo_uri::strip_password(&uri);
    let secret = match secret {
        Some(s) if !s.is_empty() => Some(s),
        other => match stripped.password {
            Some(pw) => Some(pw),
            None => other,
        },
    };
    (stripped.uri, secret)
}

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
    #[serde(default)]
    tls: Option<crate::mongo::types::TlsConfig>,
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
            tls: p.tls.clone(),
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
            s.tls,
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

    fn save_all(&self, app: &tauri::AppHandle, profiles: &[StoredProfile]) -> AppResult<()> {
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

    pub fn list_summaries(&self, app: &tauri::AppHandle) -> AppResult<Vec<ProfileSummary>> {
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
        // Never let a password embedded in the URI reach on-disk metadata:
        // move it into the secret store and persist a password-less URI.
        let (sanitized_uri, secret) = split_uri_and_secret(profile.uri, profile.secret);
        profile.uri = sanitized_uri;
        profile.secret = secret;
        let mut profiles = self.load_all(app)?;
        if let Some(existing) = profiles.iter().find(|p| p.id == profile.id).cloned() {
            if existing.name != profile.name
                && profiles
                    .iter()
                    .any(|p| p.id != profile.id && p.name == profile.name)
            {
                return Err(AppError::ProfileExists(profile.name));
            }
        } else if profiles.iter().any(|p| p.name == profile.name) {
            return Err(AppError::ProfileExists(profile.name));
        }
        // Persist secret to keychain, then strip from the profile before
        // storing. `persist_secret` returns whether a secret exists for this
        // profile *after* the operation, which is the value we must persist as
        // `has_secret` — computing it from `profile.secret` after `take()`
        // would always be `false`.
        let has_secret = self.persist_secret(&mut profile)?;
        let mut stored = StoredProfile::from(&profile);
        stored.has_secret = has_secret;
        profiles.retain(|p| p.id != stored.id);
        profiles.push(stored);
        self.save_all(app, &profiles)?;
        Ok(profile)
    }

    /// Persist (or clear) the profile's secret in the secret store and report
    /// whether a secret exists for this profile afterwards. The secret is
    /// taken out of `profile` so it never reaches on-disk metadata.
    ///
    /// Rules:
    /// - `Some(non-empty)` → store it, `has_secret = true`.
    /// - `Some(empty)`     → clear it, `has_secret = false`.
    /// - `None`            → leave the store untouched and preserve the current
    ///   state (an existing keychain entry must keep `has_secret = true` across
    ///   metadata-only edits).
    fn persist_secret(&self, profile: &mut ConnectionProfile) -> AppResult<bool> {
        match profile.secret.take() {
            Some(secret) if secret.is_empty() => {
                let _ = self.secrets.delete(&profile.id);
                Ok(false)
            }
            Some(secret) => {
                self.secrets.put(&profile.id, &secret)?;
                Ok(true)
            }
            None => Ok(self.secrets.get(&profile.id)?.is_some()),
        }
    }

    pub fn get(&self, app: &tauri::AppHandle, id: &str) -> AppResult<ConnectionProfile> {
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
            tls: stored.tls,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::credentials::InMemorySecretStore;
    use crate::mongo::types::AuthMechanism;

    fn repo() -> (ProfileRepository, Arc<InMemorySecretStore>) {
        let secrets = Arc::new(InMemorySecretStore::new());
        let repo = ProfileRepository::new(PathBuf::from("/tmp/test.json"), secrets.clone());
        (repo, secrets)
    }

    fn profile(id: &str, secret: Option<&str>) -> ConnectionProfile {
        ConnectionProfile {
            id: id.to_string(),
            name: "p".to_string(),
            uri: "mongodb://127.0.0.1:27017".to_string(),
            auth_mechanism: AuthMechanism::None,
            secret: secret.map(|s| s.to_string()),
            group: None,
            color: None,
            notes: None,
            ssh_tunnel: None,
            socks5: None,
            tls: None,
        }
    }

    // ── persist_secret: the has_secret invariant (regression for the bug where
    //    has_secret was always persisted as false) ───────────────────────────

    // ── split_uri_and_secret: URI passwords never persist in plaintext ───────

    #[test]
    fn split_moves_uri_password_into_secret_and_sanitizes_uri() {
        let (uri, secret) = split_uri_and_secret(
            "mongodb://alice:hunter2@127.0.0.1:27017/db?authSource=admin".to_string(),
            None,
        );
        assert_eq!(uri, "mongodb://alice@127.0.0.1:27017/db?authSource=admin");
        assert_eq!(secret.as_deref(), Some("hunter2"));
    }

    #[test]
    fn split_explicit_secret_field_beats_uri_password() {
        let (uri, secret) = split_uri_and_secret(
            "mongodb://alice:uri-pw@127.0.0.1:27017".to_string(),
            Some("field-pw".to_string()),
        );
        assert_eq!(uri, "mongodb://alice@127.0.0.1:27017");
        assert_eq!(secret.as_deref(), Some("field-pw"));
    }

    #[test]
    fn split_no_password_preserves_keep_existing_semantics() {
        // Metadata-only edit: no field secret, no URI password -> keep existing.
        let (uri, secret) =
            split_uri_and_secret("mongodb://alice@127.0.0.1:27017".to_string(), None);
        assert_eq!(uri, "mongodb://alice@127.0.0.1:27017");
        assert_eq!(secret, None);
    }

    #[test]
    fn split_no_password_preserves_clear_semantics() {
        // Explicit empty secret with no URI password -> clear the secret.
        let (uri, secret) = split_uri_and_secret(
            "mongodb://127.0.0.1:27017".to_string(),
            Some(String::new()),
        );
        assert_eq!(uri, "mongodb://127.0.0.1:27017");
        assert_eq!(secret.as_deref(), Some(""));
    }

    #[test]
    fn split_uri_password_overrides_empty_clear_field() {
        // A password typed into the URI is an explicit credential and wins over
        // an empty (clearing) secret field.
        let (uri, secret) = split_uri_and_secret(
            "mongodb://alice:hunter2@127.0.0.1:27017".to_string(),
            Some(String::new()),
        );
        assert_eq!(uri, "mongodb://alice@127.0.0.1:27017");
        assert_eq!(secret.as_deref(), Some("hunter2"));
    }

    #[test]
    fn persist_secret_stores_and_reports_true() {
        let (repo, secrets) = repo();
        let mut p = profile("id-1", Some("hunter2"));
        let has_secret = repo.persist_secret(&mut p).expect("persist");
        assert!(has_secret, "a non-empty secret must yield has_secret = true");
        assert!(p.secret.is_none(), "secret must be stripped from the profile");
        assert_eq!(secrets.get("id-1").expect("get"), Some("hunter2".to_string()));
    }

    #[test]
    fn persist_secret_empty_clears_and_reports_false() {
        let (repo, secrets) = repo();
        secrets.put("id-1", "old").expect("seed");
        let mut p = profile("id-1", Some(""));
        let has_secret = repo.persist_secret(&mut p).expect("persist");
        assert!(!has_secret, "an empty secret must yield has_secret = false");
        assert_eq!(secrets.get("id-1").expect("get"), None, "secret must be cleared");
    }

    #[test]
    fn persist_secret_none_preserves_existing_secret() {
        let (repo, secrets) = repo();
        secrets.put("id-1", "kept").expect("seed");
        let mut p = profile("id-1", None);
        let has_secret = repo.persist_secret(&mut p).expect("persist");
        assert!(
            has_secret,
            "a metadata-only edit must preserve has_secret for an existing secret"
        );
        assert_eq!(secrets.get("id-1").expect("get"), Some("kept".to_string()));
    }

    #[test]
    fn persist_secret_none_without_existing_reports_false() {
        let (repo, _secrets) = repo();
        let mut p = profile("id-1", None);
        let has_secret = repo.persist_secret(&mut p).expect("persist");
        assert!(!has_secret, "no secret provided and none stored => has_secret = false");
    }

    // ── StoredProfile::from must never carry the secret, and the upsert path
    //    overrides has_secret with the real value ─────────────────────────────

    #[test]
    fn stored_profile_from_strips_secret_state_to_metadata_only() {
        // StoredProfile derives has_secret from the in-memory option; once the
        // secret has been taken this is false, which is exactly why upsert must
        // override it with the persist_secret result.
        let p = profile("id-1", None);
        let stored = StoredProfile::from(&p);
        assert!(!stored.has_secret);
        assert_eq!(stored.id, "id-1");
        assert_eq!(stored.uri, "mongodb://127.0.0.1:27017");
    }

    #[test]
    fn summary_from_stored_masks_uri_and_carries_has_secret() {
        let stored = StoredProfile {
            id: "id-1".to_string(),
            name: "prod".to_string(),
            uri: "mongodb://alice:pw@db:27017/app".to_string(),
            auth_mechanism: AuthMechanism::ScramSha256,
            has_secret: true,
            group: None,
            color: None,
            notes: None,
            ssh_tunnel: None,
            socks5: None,
            tls: None,
        };
        let summary = ProfileSummary::from(stored);
        assert!(summary.has_secret);
        assert!(!summary.masked_uri.contains("alice"), "user must be masked");
        assert!(!summary.masked_uri.contains("pw"), "password must be masked");
        assert!(summary.masked_uri.contains("***:***@"));
    }
}
