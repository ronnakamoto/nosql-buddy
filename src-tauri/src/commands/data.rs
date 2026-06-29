//! Data-domain command handlers.

use crate::error::{AppError, AppResult};
use crate::events::emit_data_updated;
use crate::state::{AppState, DataItem};
use tauri::{AppHandle, State};

/// Return all data items.
#[tauri::command]
pub async fn get_data(state: State<'_, AppState>) -> AppResult<Vec<DataItem>> {
    let items = state
        .items
        .lock()
        .map_err(|e| AppError::Internal(format!("items mutex poisoned: {e}")))?;
    Ok(items.clone())
}

/// Save a data item and emit a `data-updated` event.
#[tauri::command]
pub async fn save_data(
    item: DataItem,
    state: State<'_, AppState>,
    app: AppHandle,
) -> AppResult<()> {
    if item.name.trim().is_empty() {
        return Err(AppError::Validation("item name must not be empty".into()));
    }
    let count = {
        let mut items = state
            .items
            .lock()
            .map_err(|e| AppError::Internal(format!("items mutex poisoned: {e}")))?;
        items.push(item);
        items.len()
    };
    emit_data_updated(&app, count)?;
    Ok(())
}
