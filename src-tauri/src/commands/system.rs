//! App-system commands. `app_info` returns the current platform + version
//! for the status bar. `reveal_path` opens the data directory in the OS
//! file manager so the user can inspect saved queries, profiles, etc.

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_os::platform;

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub platform: String,
    pub arch: String,
    pub tauri_version: String,
    pub app_name: String,
    pub app_version: String,
}

#[tauri::command]
pub async fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    let pkg = app.package_info();
    Ok(AppInfo {
        platform: platform().to_string(),
        arch: std::env::consts::ARCH.to_string(),
        tauri_version: tauri::VERSION.to_string(),
        app_name: pkg.name.clone(),
        app_version: pkg.version.to_string(),
    })
}
