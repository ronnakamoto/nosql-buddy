//! Data-model scan and snapshot commands.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppResult;
use crate::mongo::data_model::{scan_database_model, DataModelGraph, ScanConfig, SignalConfig};
use crate::mongo::relationship::{AppSchemaSignal, LookupSignal};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanDataModelRequest {
    pub connection_id: String,
    pub database: String,
    pub collections: Vec<String>,
    pub sample_size: u32,
    pub signals: SignalFlags,
    pub confidence_threshold: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalFlags {
    pub object_id_match: bool,
    pub naming: bool,
    pub lookup: bool,
    pub index: bool,
    pub app_schema: bool,
}

#[tauri::command]
pub async fn scan_data_model(
    request: ScanDataModelRequest,
    state: State<'_, AppState>,
) -> AppResult<DataModelGraph> {
    let entry = state.clients.get(&request.connection_id).await?;
    let config = ScanConfig {
        database: request.database,
        collections: request.collections,
        sample_size: request.sample_size,
        signals: SignalConfig {
            object_id_match: request.signals.object_id_match,
            naming: request.signals.naming,
            lookup: request.signals.lookup,
            index: request.signals.index,
            app_schema: request.signals.app_schema,
        },
        confidence_threshold: request.confidence_threshold,
    };

    // TODO: populate lookup_signals from query history and views, and
    // app_schema_signals from an optional schema file. Both are empty for v1.
    let lookup_signals: Vec<LookupSignal> = Vec::new();
    let app_schema_signals: Vec<AppSchemaSignal> = Vec::new();

    let graph = scan_database_model(
        &entry.client,
        &config,
        &lookup_signals,
        &app_schema_signals,
    )
    .await?;
    Ok(graph)
}
