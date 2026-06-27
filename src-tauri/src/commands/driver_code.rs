//! IPC handler that translates a (potentially Run'd) aggregation
//! pipeline into a driver-code snippet for one of six languages.
//!
//! The Rust-side `query_code::generate_with` is connection-aware:
//! when a `ConnectionInfo` is supplied, the snippet embeds the user's
//! real Mongo URI (and a leading comment identifying the profile
//! and auth mechanism). When the URI is empty (e.g. the editor
//! was opened against a stale handle), the command falls back to
//! the `mongodb://127.0.0.1:27017` placeholder so callers always
//! get usable code.

use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::error::{AppError, AppResult};
use crate::mongo::query_code::{self, ConnectionInfo, Language};

/// Frontend-facing request payload.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratePipelineCodeRequest {
    /// Database the pipeline runs against.
    pub database: String,
    /// Collection the pipeline runs against.
    pub collection: String,
    /// The pipeline itself (array of stage documents).
    pub pipeline: Vec<JsonValue>,
    /// Target language (kebab-case enum variant name).
    pub language: String,
    /// Optional profile metadata. Both fields are optional so the
    /// caller can supply either, both, or neither; missing fields
    /// just trim the leading comment line in the generated snippet.
    pub profile_name: Option<String>,
    pub auth_mechanism: Option<String>,
    /// Full Mongo URI. Empty string falls back to the placeholder.
    pub uri: String,
}

/// Generate driver code for a previously-run aggregation pipeline.
#[tauri::command]
pub async fn generate_pipeline_code(request: GeneratePipelineCodeRequest) -> AppResult<String> {
    let conn = ConnectionInfo {
        uri: request.uri,
        database: request.database.clone(),
        profile_name: request.profile_name,
        auth_mechanism: request.auth_mechanism,
    };

    let lang = parse_language(&request.language)?;
    let code = query_code::generate_with(
        lang,
        &request.database,
        &request.collection,
        &request.pipeline,
        &conn,
    );
    Ok(code)
}

fn parse_language(s: &str) -> AppResult<Language> {
    match s {
        "node-js" => Ok(Language::NodeJs),
        "python" => Ok(Language::Python),
        "java" => Ok(Language::Java),
        "c-sharp" => Ok(Language::CSharp),
        "ruby" => Ok(Language::Ruby),
        "shell" => Ok(Language::Shell),
        other => Err(AppError::Validation(format!("unknown language: {other}"))),
    }
}
