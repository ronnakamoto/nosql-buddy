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

#[cfg(test)]
mod tests {
    use super::*;

    // ── Theme serialization ──────────────────────────────────────────────────

    #[test]
    fn theme_variants_are_lowercase() {
        assert_eq!(serde_json::to_value(Theme::System).unwrap(), serde_json::json!("system"));
        assert_eq!(serde_json::to_value(Theme::Light).unwrap(),  serde_json::json!("light"));
        assert_eq!(serde_json::to_value(Theme::Dark).unwrap(),   serde_json::json!("dark"));
    }

    #[test]
    fn theme_deserializes_from_lowercase() {
        let v: Theme = serde_json::from_str(r#""system""#).unwrap();
        assert!(matches!(v, Theme::System));
        let v: Theme = serde_json::from_str(r#""light""#).unwrap();
        assert!(matches!(v, Theme::Light));
        let v: Theme = serde_json::from_str(r#""dark""#).unwrap();
        assert!(matches!(v, Theme::Dark));
    }

    #[test]
    fn theme_default_is_system() {
        assert!(matches!(Theme::default(), Theme::System));
    }

    // ── SafeChangeSettings wire shape ────────────────────────────────────────

    #[test]
    fn safe_change_settings_fields_are_camel_case() {
        let s = SafeChangeSettings::default();
        let j = serde_json::to_value(&s).unwrap();
        assert!(j.get("requireTypedConfirmationThreshold").is_some(),
            "requireTypedConfirmationThreshold missing");
        assert!(j.get("alwaysPreviewOnProduction").is_some(),
            "alwaysPreviewOnProduction missing");
        assert!(j.get("enabled").is_some(), "enabled missing");
        // Snake-case must be absent.
        assert!(j.get("require_typed_confirmation_threshold").is_none(),
            "snake_case must not appear");
        assert!(j.get("always_preview_on_production").is_none(),
            "snake_case must not appear");
    }

    #[test]
    fn safe_change_settings_defaults_are_correct() {
        let s = SafeChangeSettings::default();
        assert!(s.enabled, "safe change should be enabled by default");
        assert_eq!(s.require_typed_confirmation_threshold, 60);
        assert!(s.always_preview_on_production);
    }

    #[test]
    fn safe_change_settings_round_trips_through_json() {
        let original = SafeChangeSettings {
            enabled: false,
            require_typed_confirmation_threshold: 80,
            always_preview_on_production: false,
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: SafeChangeSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.enabled, false);
        assert_eq!(back.require_typed_confirmation_threshold, 80);
        assert_eq!(back.always_preview_on_production, false);
    }

    // ── AppSettings wire shape ───────────────────────────────────────────────

    #[test]
    fn app_settings_fields_are_camel_case() {
        let s = AppSettings::default();
        let j = serde_json::to_value(&s).unwrap();
        assert!(j.get("theme").is_some(),            "theme missing");
        assert!(j.get("lastConnectionId").is_some(), "lastConnectionId missing");
        assert!(j.get("safeChange").is_some(),       "safeChange missing");
        // Snake-case absent
        assert!(j.get("last_connection_id").is_none(), "snake_case must not appear");
        assert!(j.get("safe_change").is_none(),        "snake_case must not appear");
    }

    #[test]
    fn app_settings_default_has_correct_values() {
        let s = AppSettings::default();
        let j = serde_json::to_value(&s).unwrap();
        assert_eq!(j["theme"], serde_json::json!("system"));
        assert!(j["lastConnectionId"].is_null(),
            "lastConnectionId should be null when None");
        assert_eq!(j["safeChange"]["enabled"], serde_json::json!(true));
        assert_eq!(j["safeChange"]["requireTypedConfirmationThreshold"],
            serde_json::json!(60u32));
    }

    #[test]
    fn app_settings_round_trips_through_json() {
        let original = AppSettings {
            theme: Theme::Dark,
            last_connection_id: Some("conn-abc".into()),
            safe_change: SafeChangeSettings {
                enabled: true,
                require_typed_confirmation_threshold: 75,
                always_preview_on_production: false,
            },
        };
        let json = serde_json::to_string(&original).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.theme, Theme::Dark));
        assert_eq!(back.last_connection_id, Some("conn-abc".into()));
        assert_eq!(back.safe_change.require_typed_confirmation_threshold, 75);
    }

    #[test]
    fn app_settings_none_last_connection_id_serializes_as_null() {
        let s = AppSettings::default();
        let j = serde_json::to_value(&s).unwrap();
        // The field must be present as null — not absent — so the frontend
        // can distinguish "no connection saved" from "field missing".
        assert!(j.get("lastConnectionId").is_some(),
            "lastConnectionId key must be present");
        assert!(j["lastConnectionId"].is_null(),
            "lastConnectionId must be null when None");
    }
}
