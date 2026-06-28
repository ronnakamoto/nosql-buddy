//! Data-model scan, cache, and edge-override commands.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::mongo::data_model::{
    apply_edge_override, load_graph_cache, save_graph_cache, scan_database_model, DataModelGraph,
    ScanConfig, SignalConfig,
};
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
    /// `$lookup` signals extracted from query history on the frontend (the
    /// history lives in browser localStorage, so the frontend owns parsing).
    #[serde(default)]
    pub lookup_signals: Vec<LookupSignalDto>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSignalDto {
    pub from_collection: String,
    pub to_collection: String,
    pub local_field: String,
    pub foreign_field: String,
    pub count: u32,
}

impl From<LookupSignalDto> for LookupSignal {
    fn from(d: LookupSignalDto) -> Self {
        LookupSignal {
            from_collection: d.from_collection,
            to_collection: d.to_collection,
            local_field: d.local_field,
            foreign_field: d.foreign_field,
            count: d.count,
        }
    }
}

#[tauri::command]
pub async fn scan_data_model(
    request: ScanDataModelRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<DataModelGraph> {
    let entry = state.clients.get(&request.connection_id).await?;
    let config = ScanConfig {
        database: request.database.clone(),
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

    // The frontend parses query history (localStorage) into lookup signals.
    // App-schema overlay is Phase 4; empty for now.
    let lookup_signals: Vec<LookupSignal> =
        request.lookup_signals.into_iter().map(Into::into).collect();
    let app_schema_signals: Vec<AppSchemaSignal> = Vec::new();

    let graph = scan_database_model(
        &entry.client,
        &config,
        &lookup_signals,
        &app_schema_signals,
        Some(&app),
    )
    .await?;

    // Best-effort cache; a cache write failure should not mask a successful scan.
    if let Err(e) = save_graph_cache(&app, &graph) {
        tracing::warn!(error = %e, database = %graph.database, "data-model cache save failed");
    }
    Ok(graph)
}

/// Return the most recently cached `DataModelGraph` for a database, or `null`
/// if no scan has been run yet. Does not touch MongoDB.
#[tauri::command]
pub async fn get_data_model(
    database: String,
    app: tauri::AppHandle,
) -> AppResult<Option<DataModelGraph>> {
    load_graph_cache(&app, &database)
}

/// Apply a user override (confirm / hide) to one relationship edge of the
/// cached graph. Persists the override and returns the updated graph.
#[tauri::command]
pub async fn update_relationship(
    database: String,
    edge_id: String,
    confirmed: Option<bool>,
    hidden: Option<bool>,
    app: tauri::AppHandle,
) -> AppResult<DataModelGraph> {
    if confirmed.is_none() && hidden.is_none() {
        return Err(AppError::Validation(
            "provide at least one of 'confirmed' or 'hidden'".into(),
        ));
    }
    apply_edge_override(&app, &database, &edge_id, confirmed, hidden)
}
