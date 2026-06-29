//! App-level settings: theme, last-used connection, Safe Change Mode config.
//! Stored in tauri-plugin-store so they survive restarts.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const STORE_FILE: &str = "nosqlbuddy.settings.json";
const THEME_KEY: &str = "theme";
const LAST_CONNECTION_KEY: &str = "lastConnectionId";
const SAFE_CHANGE_KEY: &str = "safeChange";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Self::System
    }
}

/// Safe Change Mode configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeChangeSettings {
    /// Enable the Safe Change preview flow for write operations.
    pub enabled: bool,
    /// Risk score threshold above which typed confirmation is always required
    /// (regardless of the per-operation heuristic). Range 0–100.
    pub require_typed_confirmation_threshold: u32,
    /// Always show the preview modal even for low-risk write operations when
    /// the target connection is detected as production.
    pub always_preview_on_production: bool,
}

impl Default for SafeChangeSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            require_typed_confirmation_threshold: 60,
            always_preview_on_production: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: Theme,
    pub last_connection_id: Option<String>,
    pub safe_change: SafeChangeSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            last_connection_id: None,
            safe_change: SafeChangeSettings::default(),
        }
    }
}

#[tauri::command]
pub async fn get_settings(app: AppHandle) -> AppResult<AppSettings> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Internal(format!("settings store open: {e}")))?;
    let theme = match store.get(THEME_KEY) {
        Some(v) => serde_json::from_value(v).unwrap_or(Theme::System),
        None => Theme::System,
    };
    let last_connection_id = store
        .get(LAST_CONNECTION_KEY)
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    let safe_change = match store.get(SAFE_CHANGE_KEY) {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => SafeChangeSettings::default(),
    };
    Ok(AppSettings {
        theme,
        last_connection_id,
        safe_change,
    })
}

#[tauri::command]
pub async fn update_settings(
    settings: AppSettings,
    app: AppHandle,
    _state: State<'_, AppState>,
) -> AppResult<()> {
    let store = app
        .store(STORE_FILE)
        .map_err(|e| AppError::Internal(format!("settings store open: {e}")))?;
    store.set(THEME_KEY, serde_json::json!(settings.theme));
    if let Some(id) = &settings.last_connection_id {
        store.set(LAST_CONNECTION_KEY, serde_json::json!(id));
    } else {
        store.delete(LAST_CONNECTION_KEY);
    }
    store.set(SAFE_CHANGE_KEY, serde_json::json!(settings.safe_change));
    store
        .save()
        .map_err(|e| AppError::Internal(format!("settings store save: {e}")))?;
    Ok(())
}
