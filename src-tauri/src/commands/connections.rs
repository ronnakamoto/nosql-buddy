//! Connection lifecycle commands: list, save, delete, open, close, test.
//!
//! All profile operations run through the `ProfileRepository`, which keeps
//! secrets in the OS keychain and metadata in the tauri-plugin-store. The
//! `connection_test` command opens a short-lived client so the user can
//! verify credentials without polluting the active connection registry.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::events::{emit_connection_closed, emit_connection_opened};
use crate::mongo::client_registry::{build_client_with_auth, describe_connection, ClientEntry};
use crate::mongo::types::{ConnectionHandle, ConnectionProfile, ProfileSummary};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileRequest {
    pub id: Option<String>,
    pub name: String,
    pub uri: String,
    pub auth_mechanism: crate::mongo::types::AuthMechanism,
    /// Plaintext secret, sent once. Never logged. Never returned.
    pub secret: Option<String>,
    pub group: Option<String>,
    pub color: Option<String>,
    pub notes: Option<String>,
    pub ssh_tunnel: Option<crate::mongo::types::SshTunnelConfig>,
    pub socks5: Option<crate::mongo::types::Socks5Config>,
    #[serde(default)]
    pub tls: Option<crate::mongo::types::TlsConfig>,
}

#[tauri::command]
pub async fn list_profiles(
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<Vec<ProfileSummary>> {
    state.profiles.list_summaries(&app)
}

#[tauri::command]
pub async fn save_profile(
    request: SaveProfileRequest,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<ProfileSummary> {
    let id = request.id.unwrap_or_default();
    if request.name.trim().is_empty() {
        return Err(AppError::Validation(
            "profile name must not be empty".into(),
        ));
    }
    if request.uri.trim().is_empty() {
        return Err(AppError::Validation(
            "connection URI must not be empty".into(),
        ));
    }
    let profile = ConnectionProfile {
        id,
        name: request.name,
        uri: request.uri,
        auth_mechanism: request.auth_mechanism,
        secret: request.secret,
        group: request.group,
        color: request.color,
        notes: request.notes,
        ssh_tunnel: request.ssh_tunnel,
        socks5: request.socks5,
        tls: request.tls,
    };
    let saved = state.profiles.upsert(&app, profile)?;
    let has_secret = saved.secret.is_some() || state.secrets.get(&saved.id)?.is_some();
    Ok(ProfileSummary::from_stored(
        saved.id,
        saved.name,
        saved.uri,
        saved.auth_mechanism,
        has_secret,
        saved.group,
        saved.color,
        saved.notes,
        saved.ssh_tunnel,
        saved.socks5,
        saved.tls,
    ))
}

#[tauri::command]
pub async fn delete_profile(
    id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    state.profiles.delete(&app, &id)
}

/// Return the full (unmasked) URI for a profile. Used by the driver
/// code generator so the user's real connection string ends up in
/// the generated snippet. The caller (renderer) is trusted here —
/// the renderer already has the ability to call `open_connection`
/// with an arbitrary profile id, so leaking the URI back to itself
/// is not a privilege escalation. We deliberately do NOT include
/// the URI in `ProfileSummary` (which is fetched on app start and
/// logged into logs) — it's only fetched on demand.
#[tauri::command]
pub async fn resolve_profile_uri(
    id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<String> {
    let profile = state.profiles.get(&app, &id)?;
    Ok(profile.uri)
}

#[tauri::command]
pub async fn test_profile(
    request: SaveProfileRequest,
    _state: State<'_, AppState>,
) -> AppResult<TestResult> {
    let profile = ConnectionProfile {
        id: request.id.unwrap_or_default(),
        name: request.name,
        uri: request.uri,
        auth_mechanism: request.auth_mechanism,
        secret: request.secret,
        group: request.group,
        color: request.color,
        notes: request.notes,
        ssh_tunnel: request.ssh_tunnel,
        socks5: request.socks5,
        tls: request.tls,
    };
    let client = build_client_with_auth(
        &profile.uri,
        "NoSQLBuddy-test",
        profile.auth_mechanism,
        profile.secret.as_deref(),
        profile.tls.as_ref(),
    )
    .await?;
    let database = client.database("admin");
    let started = std::time::Instant::now();
    // ping and connectionStatus do NOT require authentication on some proxies
    // (e.g. Atlas SQL gateway), so a missing credential still passes. We use
    // listDatabases instead: it forces the driver to actually authenticate,
    // so missing or wrong credentials surface here instead of later during
    // normal use. If the user is authenticated but lacks the listDatabases
    // privilege, we still report success — auth itself worked.
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        database.run_command(bson::doc! { "listDatabases": 1, "nameOnly": true }),
    )
    .await;
    let latency_ms = Some(started.elapsed().as_millis() as u64);
    match result {
        Ok(Ok(_)) => Ok(TestResult {
            ok: true,
            message: "Connection successful. The server responded.".into(),
            latency_ms,
        }),
        Ok(Err(e)) => {
            let raw = e.to_string();
            let is_auth_failure = raw.contains("auth required")
                || raw.contains("Unauthorized")
                || raw.contains("Authentication failed");
            let has_credentials = profile.secret.as_deref().is_some_and(|s| !s.is_empty())
                || mongo_uri::has_username(&profile.uri);
            let message = if is_auth_failure && !has_credentials {
                "No username or password provided. Add credentials to your connection URI \
                 (e.g. mongodb://username:password@host/...) or set a password in the auth settings."
                    .into()
            } else if is_auth_failure {
                "Authentication failed. Please check that your username and password are correct."
                    .into()
            } else if raw.contains("not authorized") {
                // Authenticated successfully, but the user lacks the
                // listDatabases privilege. The connection itself is valid.
                "Authentication successful. Your user lacks the listDatabases privilege, but the connection is valid."
                    .into()
            } else {
                raw
            };
            Ok(TestResult {
                ok: !is_auth_failure,
                message,
                latency_ms,
            })
        }
        Err(_) => Ok(TestResult {
            ok: false,
            message: "Connection timed out after 8s".into(),
            latency_ms,
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
    pub latency_ms: Option<u64>,
}

#[tauri::command]
pub async fn open_connection(
    profile_id: String,
    secret_override: Option<String>,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<ConnectionHandle> {
    let mut profile = state.profiles.get(&app, &profile_id)?;
    if let Some(override_secret) = secret_override {
        if !override_secret.is_empty() {
            profile.secret = Some(override_secret);
        }
    }
    let client = build_client_with_auth(
        &profile.uri,
        "NoSQLBuddy",
        profile.auth_mechanism,
        profile.secret.as_deref(),
        profile.tls.as_ref(),
    )
    .await?;
    // Confirm we can actually reach and authenticate with the server before
    // publishing the handle. listDatabases requires authentication (unlike
    // ping/connectionStatus which can pass without auth on some proxies).
    tokio::time::timeout(
        std::time::Duration::from_secs(8),
        client
            .database("admin")
            .run_command(bson::doc! { "listDatabases": 1, "nameOnly": true }),
    )
    .await
    .map_err(|_| AppError::Timeout("listDatabases".into()))??;
    let connection_id = Uuid::new_v4().to_string();
    // Derive a stable per-deployment identity so audit events are segmented
    // by the deployment they originate from. Resolved once at connect time.
    let deployment_id = crate::audit::change_stream::fetch_deployment_id(&client).await;
    let entry = ClientEntry {
        client: client.clone(),
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        deployment_id: deployment_id.clone(),
        opened_at: chrono::Utc::now(),
    };
    state.clients.insert(connection_id.clone(), entry).await;
    emit_connection_opened(&app, &connection_id, &profile.id, &profile.name)?;
    // Start a change stream listener only for deployments that support change
    // streams (replica sets / sharded clusters). There it is the authoritative
    // capture path: it records all writes regardless of origin (shell, IPC,
    // external clients), and the per-IPC interceptor hooks are skipped to avoid
    // double-recording (see `commands::mongo`). On standalone deployments
    // change streams are unsupported, so we do NOT start a listener (it would
    // only error and retry forever) and rely on the IPC interceptor hooks
    // instead.
    if crate::audit::change_stream::supports_change_streams(&deployment_id) {
        state
            .change_streams
            .start_for(
                connection_id.clone(),
                deployment_id,
                (*client).clone(),
                state.audit_log.clone(),
                Some(state.epoch_manager.clone()),
            )
            .await;
    }
    let handle = describe_connection(&client, &connection_id, &profile.id, &profile.name).await?;
    // Drop the secret from local memory now that the client is up. The
    // driver keeps a pool internally; we don't need the string anymore.
    drop(profile);
    Ok(handle)
}

#[tauri::command]
pub async fn close_connection(
    connection_id: String,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    let entry = state.clients.remove(&connection_id).await?;
    // Stop the change stream listener for this connection.
    state.change_streams.stop_for(&connection_id).await;
    emit_connection_closed(&app, &connection_id, &entry.profile_id)?;
    Ok(())
}

#[tauri::command]
pub async fn list_active_connections(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::mongo::client_registry::ConnectionDescriptor>> {
    Ok(state.clients.list().await)
}
