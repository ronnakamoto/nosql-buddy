//! App-level settings: theme, last-used connection, etc. Stored in
//! tauri-plugin-store so they survive restarts.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_store::StoreExt;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

const STORE_FILE: &str = "nosqlbuddy.settings.json";
const THEME_KEY: &str = "theme";
const LAST_CONNECTION_KEY: &str = "lastConnectionId";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: Theme,
    pub last_connection_id: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            last_connection_id: None,
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
    let last_connection_id = store.get(LAST_CONNECTION_KEY).and_then(|v| {
        v.as_str().map(|s| s.to_string())
    });
    Ok(AppSettings {
        theme,
        last_connection_id,
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
    store
        .save()
        .map_err(|e| AppError::Internal(format!("settings store save: {e}")))?;
    Ok(())
}
