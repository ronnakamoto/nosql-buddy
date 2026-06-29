//! Export command: stream documents from a find/aggregate query or an explicit
//! document set to a JSON or CSV destination (file or clipboard).
//!
//! Exports never reuse `find_documents` (which caps at `MAX_LIMIT` and collects
//! everything in memory). Instead they drive a streaming cursor through the
//! `import_export` pipeline, so a multi-million-document collection exports with
//! bounded memory, throttled progress events, and cooperative cancellation.

use bson::Document;
use futures_util::stream::TryStreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::events::notify_job_completed;
use crate::mongo::bson_json::{parse_filter, parse_optional_doc};
use crate::mongo::import_export::bson_sink::BsonSink;
use crate::mongo::import_export::collection_sink::CollectionSink;
use crate::mongo::import_export::core::{run_pipeline, DocumentSink, DocumentSource, JobContext};
use crate::mongo::import_export::csv::CsvSink;
use crate::mongo::import_export::io_util::{validate_target_path, AtomicFileWriter, CompressionKind, CompressedWriter, WriteSink, WriteTarget};
use crate::mongo::import_export::json::{JsonShape, JsonSink};
use crate::mongo::import_export::mapping::{FieldMappingEntry, FieldMappingTransform};
use crate::mongo::import_export::placeholders::{resolve_path, PlaceholderContext};
use crate::mongo::import_export::source_cursor::CursorSource;
use crate::mongo::import_export::source_mem::VecSource;
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::mongo::operation_recorder::RecordContext;
use crate::state::AppState;

const COLUMN_SAMPLE: u32 = 200;
const MAX_COLUMNS: usize = 256;
const DEFAULT_COPY_BATCH_SIZE: usize = 1000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceMode {
    Find,
    Aggregate,
    Documents,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Json,
    Csv,
    Bson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DestinationKind {
    File,
    Clipboard,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JsonShapeDto {
    Array,
    Ndjson,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSourceDto {
    pub mode: SourceMode,
    pub filter_json: Option<String>,
    pub projection_json: Option<String>,
    pub sort_json: Option<String>,
    pub pipeline_json: Option<String>,
    pub documents_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportDestinationDto {
    pub kind: DestinationKind,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub json_shape: JsonShapeDto,
    pub canonical: bool,
    pub csv_delimiter: Option<String>,
    pub csv_headers: bool,
    pub csv_columns: Option<Vec<String>>,
    pub compression: CompressionKind,
    /// Optional field-mapping table applied as a Transform before the sink.
    /// When present and non-empty, the mapping is the complete output schema:
    /// undeclared fields are dropped. For CSV, the sink columns are derived
    /// from the non-skipped `target` names in declared order, overriding
    /// `csv_columns` and the schema sample.
    pub field_mapping: Option<Vec<FieldMappingEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub job_id: String,
    pub source: ExportSourceDto,
    pub format: ExportFormat,
    pub destination: ExportDestinationDto,
    pub options: ExportOptions,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub job_id: String,
    pub processed: u64,
    pub errors: u64,
    pub cancelled: bool,
    pub path: Option<String>,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyTargetDto {
    pub database: String,
    pub collection: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub job_id: String,
    pub source: ExportSourceDto,
    pub target: CopyTargetDto,
    pub batch_size: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyResult {
    pub job_id: String,
    pub processed: u64,
    pub inserted: u64,
    pub errors: u64,
    pub cancelled: bool,
}

#[tauri::command]
pub async fn export_documents(
    request: ExportRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<ExportResult> {
    run_export(&request, state.inner(), &app).await
}

/// Core export logic callable from both the command handler and the scheduler.
pub async fn run_export(request: &ExportRequest, state: &AppState, app: &tauri::AppHandle) -> AppResult<ExportResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let db = entry.client.database(&request.database);
    let coll = db.collection::<Document>(&request.collection);
    let started = std::time::Instant::now();

    // Resolve the field-mapping transform once, up front, so both the column
    // derivation and the pipeline see the same validated table.
    let mapping_entries = request.options.field_mapping.clone().unwrap_or_default();
    let field_mapping = if mapping_entries.is_empty() {
        None
    } else {
        Some(FieldMappingTransform::new(mapping_entries.clone())?)
    };

    // Resolve CSV columns up front (deterministic, streamable output). A
    // field mapping, when present, is the authoritative column set: the
    // non-skipped `target` names in declared order override both an explicit
    // `csv_columns` list and the schema sample.
    let columns = if matches!(request.format, ExportFormat::Csv) {
        if field_mapping.is_some() {
            FieldMappingTransform::csv_columns(&mapping_entries)
        } else if let Some(cols) = &request.options.csv_columns {
            if cols.is_empty() {
                sample_columns(&coll, &request.source).await?
            } else {
                cols.clone()
            }
        } else {
            sample_columns(&coll, &request.source).await?
        }
    } else {
        Vec::new()
    };

    // Build the streaming source.
    let source: Box<dyn DocumentSource> = build_source(&coll, &request.source).await?;

    // Resolve path placeholders against the connection context before
    // validation, so an invalid expanded path is still rejected by
    // `validate_target_path`. Clipboard exports have no path to resolve.
    let profile_name = state
        .profiles
        .get(app, &request.connection_id)
        .map(|p| p.name)
        .unwrap_or_default();
    let resolved_destination = resolve_destination_placeholders(
        &request.destination,
        &PlaceholderContext {
            database: &request.database,
            collection: &request.collection,
            profile: &profile_name,
        },
    );

    // Build the write target (validated file path, or in-memory buffer).
    let (target, file_path) = build_target(&resolved_destination, request.options.compression)?;
    let output_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Build the sink for the chosen format.
    let sink: Box<dyn DocumentSink> = match request.format {
        ExportFormat::Json => {
            let shape = match request.options.json_shape {
                JsonShapeDto::Array => JsonShape::Array,
                JsonShapeDto::Ndjson => JsonShape::Ndjson,
            };
            Box::new(JsonSink::new(
                target,
                output_slot.clone(),
                shape,
                request.options.canonical,
            ))
        }
        ExportFormat::Csv => {
            let delimiter = resolve_delimiter(request.options.csv_delimiter.as_deref())?;
            Box::new(CsvSink::new(
                target,
                output_slot.clone(),
                columns,
                delimiter,
                request.options.csv_headers,
            ))
        }
        ExportFormat::Bson => {
            Box::new(BsonSink::new(target))
        }
    };

    // Compose the transform chain: currently just the optional field mapping.
    let mut transforms: Vec<Box<dyn crate::mongo::import_export::core::Transform>> = Vec::new();
    if let Some(m) = field_mapping {
        transforms.push(Box::new(m));
    }

    // Track in the global job store.
    let mut meta = JobMeta::new(
        request.job_id.clone(),
        JobKind::Export,
        request.connection_id.clone(),
        request.database.clone(),
    );
    meta.collections = vec![request.collection.clone()];
    meta.output_path = file_path.clone();
    meta.config_json = Some(serde_json::to_string(request).unwrap_or_default());
    meta.profile_id = entry.profile_id.clone();
    state.jobs.create_job(meta).await;
    state.jobs.update_status(&request.job_id, JobStatus::Running, "Export started".into()).await;

    // Register the job for cancellation, run, then unregister.
    let cancel_flag = state
        .jobs
        .register(request.job_id.clone())
        .await;
    let ctx = JobContext {
        job_id: request.job_id.clone(),
        cancel_flag,
        app_handle: Some(app.clone()),
        progress_observer: None,
        throttle_ms: 250,
        max_errors: 10_000,
        max_error_samples: 100,
    };

    let report = run_pipeline(source, transforms, sink, ctx).await;
    state.jobs.unregister(&request.job_id).await;
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            state.jobs.update_status(&request.job_id, JobStatus::Failed, e.to_string()).await;
            return Err(e);
        }
    };

    let status = if report.cancelled {
        JobStatus::Cancelled
    } else {
        JobStatus::Done
    };
    state.jobs.update_status(&request.job_id, status, format!("Exported {} documents", report.processed)).await;

    let clipboard_text = output_slot
        .lock()
        .map_err(|_| AppError::Internal("export output slot mutex poisoned".into()))?
        .take();

    notify_job_completed(app, &request.job_id, "Export", &format!("Exported {} documents", report.processed));

    let elapsed = started.elapsed().as_millis() as u64;
    let ctx = RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    let source_summary = serde_json::json!({
        "mode": request.source.mode,
        "format": request.format,
        "destinationKind": request.destination.kind,
    });
    crate::mongo::operation_recorder::record_export(
        &state.timeline,
        &ctx,
        &serde_json::to_string(&source_summary).unwrap_or_default(),
        report.processed,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(ExportResult {
        job_id: report.job_id,
        processed: report.processed,
        errors: report.errors,
        cancelled: report.cancelled,
        path: if report.cancelled { None } else { file_path },
        clipboard_text: if report.cancelled {
            None
        } else {
            clipboard_text
        },
    })
}

/// Resolve path placeholders in a file destination. Clipboard destinations are
/// returned unchanged. The returned dto keeps the (now-expanded) path so the
/// downstream `validate_target_path` still enforces the allowed-root rule.
fn resolve_destination_placeholders(
    dest: &ExportDestinationDto,
    ctx: &PlaceholderContext<'_>,
) -> ExportDestinationDto {
    match dest.kind {
        DestinationKind::Clipboard => dest.clone(),
        DestinationKind::File => {
            let resolved = dest
                .path
                .as_deref()
                .map(|p| resolve_path(p, ctx));
            ExportDestinationDto {
                kind: dest.kind,
                path: resolved,
            }
        }
    }
}

/// Cancel a running import/export job by id.
#[tauri::command]
pub async fn cancel_import_export(job_id: String, state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.jobs.cancel(&job_id).await)
}

#[tauri::command]
pub async fn copy_documents(
    request: CopyRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<CopyResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let source_collection = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let target_collection = entry
        .client
        .database(&request.target.database)
        .collection::<Document>(&request.target.collection);

    let source = build_source(&source_collection, &request.source).await?;
    let inserted = Arc::new(AtomicU64::new(0));
    let sink: Box<dyn DocumentSink> = Box::new(CollectionSink::new(
        target_collection,
        request.batch_size.unwrap_or(DEFAULT_COPY_BATCH_SIZE),
        inserted.clone(),
    ));

    let mut meta = JobMeta::new(
        request.job_id.clone(),
        JobKind::Export,
        request.connection_id.clone(),
        request.database.clone(),
    );
    meta.collections = vec![request.collection.clone(), request.target.collection.clone()];
    meta.profile_id = entry.profile_id.clone();
    state.jobs.create_job(meta).await;
    state.jobs.update_status(&request.job_id, JobStatus::Running, "Copy started".into()).await;

    let cancel_flag = state
        .jobs
        .register(request.job_id.clone())
        .await;
    let ctx = JobContext {
        job_id: request.job_id.clone(),
        cancel_flag,
        app_handle: Some(app.clone()),
        progress_observer: None,
        throttle_ms: 250,
        max_errors: 10_000,
        max_error_samples: 100,
    };

    let report = run_pipeline(source, Vec::new(), sink, ctx).await;
    state.jobs.unregister(&request.job_id).await;
    let report = match report {
        Ok(r) => r,
        Err(e) => {
            state.jobs.update_status(&request.job_id, JobStatus::Failed, e.to_string()).await;
            return Err(e);
        }
    };

    let status = if report.cancelled {
        JobStatus::Cancelled
    } else {
        JobStatus::Done
    };
    state.jobs.update_status(&request.job_id, status, format!("Copied {} documents", report.processed)).await;

    Ok(CopyResult {
        job_id: report.job_id,
        processed: report.processed,
        inserted: inserted.load(Ordering::Relaxed),
        errors: report.errors,
        cancelled: report.cancelled,
    })
}

async fn build_source(
    coll: &mongodb::Collection<Document>,
    source: &ExportSourceDto,
) -> AppResult<Box<dyn DocumentSource>> {
    match source.mode {
        SourceMode::Find => {
            let filter = parse_optional_doc(source.filter_json.as_deref())?.unwrap_or_default();
            let projection = parse_optional_doc(source.projection_json.as_deref())?;
            let sort = parse_optional_doc(source.sort_json.as_deref())?;
            let total = coll.count_documents(filter.clone()).await.ok();
            let mut find = coll.find(filter).batch_size(1000);
            if let Some(p) = projection {
                find = find.projection(p);
            }
            if let Some(s) = sort {
                find = find.sort(s);
            }
            let cursor = find.await?;
            Ok(Box::new(CursorSource::new(cursor, total)))
        }
        SourceMode::Aggregate => {
            let pipeline_json = source.pipeline_json.as_deref().ok_or_else(|| {
                AppError::Validation("aggregate export requires a pipeline".into())
            })?;
            let pipeline: Vec<Document> = serde_json::from_str(pipeline_json)?;
            let cursor = coll.aggregate(pipeline).batch_size(1000).await?;
            Ok(Box::new(CursorSource::new(cursor, None)))
        }
        SourceMode::Documents => {
            let docs = parse_documents(source.documents_json.as_deref())?;
            Ok(Box::new(VecSource::new(docs)))
        }
    }
}

/// Sample a few documents to derive the union of top-level field names, in
/// first-seen order, for CSV column headers.
async fn sample_columns(
    coll: &mongodb::Collection<Document>,
    source: &ExportSourceDto,
) -> AppResult<Vec<String>> {
    let docs: Vec<Document> = match source.mode {
        SourceMode::Find => {
            let filter = parse_optional_doc(source.filter_json.as_deref())?.unwrap_or_default();
            let cursor = coll.find(filter).limit(COLUMN_SAMPLE as i64).await?;
            cursor.try_collect().await?
        }
        SourceMode::Aggregate => {
            let pipeline_json = source.pipeline_json.as_deref().ok_or_else(|| {
                AppError::Validation("aggregate export requires a pipeline".into())
            })?;
            let mut pipeline: Vec<Document> = serde_json::from_str(pipeline_json)?;
            pipeline.push(bson::doc! { "$limit": COLUMN_SAMPLE as i64 });
            let cursor = coll.aggregate(pipeline).await?;
            cursor.try_collect().await?
        }
        SourceMode::Documents => parse_documents(source.documents_json.as_deref())?,
    };

    let mut seen = std::collections::HashSet::new();
    let mut columns = Vec::new();
    for doc in &docs {
        for key in doc.keys() {
            if seen.insert(key.clone()) {
                columns.push(key.clone());
                if columns.len() >= MAX_COLUMNS {
                    return Ok(columns);
                }
            }
        }
    }
    if columns.is_empty() {
        return Err(AppError::Validation(
            "could not determine CSV columns: the query returned no documents to sample".into(),
        ));
    }
    Ok(columns)
}

fn parse_documents(documents_json: Option<&str>) -> AppResult<Vec<Document>> {
    let raw = documents_json.ok_or_else(|| {
        AppError::Validation("documents export requires a documents array".into())
    })?;
    let value: serde_json::Value = serde_json::from_str(raw)?;
    let array = value
        .as_array()
        .ok_or_else(|| AppError::Validation("documents must be a JSON array".into()))?;
    let mut docs = Vec::with_capacity(array.len());
    for item in array {
        let text = serde_json::to_string(item)?;
        docs.push(parse_filter(&text)?);
    }
    Ok(docs)
}

fn build_target(
    dest: &ExportDestinationDto,
    compression: CompressionKind,
) -> AppResult<(WriteSink, Option<String>)> {
    match dest.kind {
        DestinationKind::File => {
            let path = dest
                .path
                .as_deref()
                .ok_or_else(|| AppError::Validation("file export requires a target path".into()))?;
            let validated = validate_target_path(path)?;
            let writer = AtomicFileWriter::create(validated.clone())?;
            let target = WriteTarget::File(writer);
            let sink = if compression == CompressionKind::None {
                WriteSink::Plain(target)
            } else {
                WriteSink::Compressed(CompressedWriter::new(target, compression)?)
            };
            Ok((
                sink,
                Some(validated.to_string_lossy().to_string()),
            ))
        }
        DestinationKind::Clipboard => Ok((WriteSink::Plain(WriteTarget::Buffer(Vec::new())), None)),
    }
}

fn resolve_delimiter(delimiter: Option<&str>) -> AppResult<u8> {
    match delimiter {
        None => Ok(b','),
        Some(s) => {
            let bytes = s.as_bytes();
            if bytes.len() == 1 {
                Ok(bytes[0])
            } else {
                Err(AppError::Validation(
                    "CSV delimiter must be a single ASCII character".into(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::import_export::json::JsonShape;
    use bson::doc;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_job_context(job_id: &str) -> JobContext {
        JobContext {
            job_id: job_id.to_string(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            app_handle: None,
            progress_observer: None,
            throttle_ms: 0,
            max_errors: 100,
            max_error_samples: 100,
        }
    }

    fn unique_name(prefix: &str) -> String {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        format!("{prefix}_{}_{}", std::process::id(), millis)
    }

    fn home_export_path(file_name: &str) -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap()).join(file_name)
    }

    #[tokio::test]
    #[ignore = "requires MongoDB at MONGO_BUDDY_LIVE_MONGO_URI or localhost:27017 replica set"]
    async fn live_phase1_validates_find_aggregate_sql_and_clipboard_exports(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let uri = std::env::var("MONGO_BUDDY_LIVE_MONGO_URI")
            .unwrap_or_else(|_| "mongodb://localhost:27017/?replicaSet=rs0".to_string());
        let client = mongodb::Client::with_uri_str(&uri).await?;
        let db_name = unique_name("mongo_buddy_phase1_export");
        let db = client.database(&db_name);
        let coll = db.collection::<Document>("widgets");

        let json_path = home_export_path(&format!("{db_name}_find.json"));
        let csv_path = home_export_path(&format!("{db_name}_aggregate.csv"));
        let outcome = async {
            coll.insert_many(vec![
                doc! { "category": "book", "name": "Atlas", "n": 1, "nested": { "city": "NYC" } },
                doc! { "category": "book", "name": "Compass", "n": 2, "nested": { "city": "SFO" } },
                doc! { "category": "tool", "name": "Shell", "n": 3, "nested": { "city": "LON" } },
            ])
            .await?;

            let find_source = ExportSourceDto {
                mode: SourceMode::Find,
                filter_json: Some(r#"{"category":"book"}"#.into()),
                projection_json: Some(r#"{"_id":0,"name":1,"n":1}"#.into()),
                sort_json: Some(r#"{"n":1}"#.into()),
                pipeline_json: None,
                documents_json: None,
            };
            let json_slot = Arc::new(Mutex::new(None));
            let json_path_text = json_path.to_string_lossy();
            let json_target = WriteSink::Plain(WriteTarget::File(AtomicFileWriter::create(validate_target_path(
                json_path_text.as_ref(),
            )?)?));
            let json_report = run_pipeline(
                build_source(&coll, &find_source).await?,
                Vec::new(),
                Box::new(JsonSink::new(
                    json_target,
                    json_slot,
                    JsonShape::Array,
                    false,
                )),
                test_job_context("live-find-json-file"),
            )
            .await?;
            assert_eq!(json_report.processed, 2);

            let json_text = std::fs::read_to_string(&json_path)?;
            let exported: Vec<serde_json::Value> = serde_json::from_str(&json_text)?;
            assert_eq!(exported.len(), 2);
            assert_eq!(exported[0]["name"], "Atlas");
            assert_eq!(exported[1]["name"], "Compass");

            let aggregate_source = ExportSourceDto {
                mode: SourceMode::Aggregate,
                filter_json: None,
                projection_json: None,
                sort_json: None,
                pipeline_json: Some(
                    r#"[
                        {"$match":{"category":"book"}},
                        {"$sort":{"n":1}},
                        {"$project":{"_id":0,"name":1,"n":1,"nested.city":1}}
                    ]"#
                    .into(),
                ),
                documents_json: None,
            };
            let columns = sample_columns(&coll, &aggregate_source).await?;
            let csv_slot = Arc::new(Mutex::new(None));
            let csv_path_text = csv_path.to_string_lossy();
            let csv_target = WriteSink::Plain(WriteTarget::File(AtomicFileWriter::create(validate_target_path(
                csv_path_text.as_ref(),
            )?)?));
            let csv_report = run_pipeline(
                build_source(&coll, &aggregate_source).await?,
                Vec::new(),
                Box::new(CsvSink::new(csv_target, csv_slot, columns, b',', true)),
                test_job_context("live-aggregate-csv-file"),
            )
            .await?;
            assert_eq!(csv_report.processed, 2);

            let csv_text = std::fs::read_to_string(&csv_path)?;
            let mut reader = csv::Reader::from_reader(csv_text.as_bytes());
            let rows: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>()?;
            assert_eq!(rows.len(), 2);
            assert!(csv_text.contains("Atlas"));
            assert!(csv_text.contains("Compass"));

            let sql_translated_source = ExportSourceDto {
                mode: SourceMode::Aggregate,
                filter_json: None,
                projection_json: None,
                sort_json: None,
                pipeline_json: Some(
                    r#"[
                        {"$match":{"category":"tool"}},
                        {"$project":{"_id":0,"name":1,"n":1}}
                    ]"#
                    .into(),
                ),
                documents_json: None,
            };
            let clipboard_slot = Arc::new(Mutex::new(None));
            let clipboard_report = run_pipeline(
                build_source(&coll, &sql_translated_source).await?,
                Vec::new(),
                Box::new(JsonSink::new(
                    WriteSink::Plain(WriteTarget::Buffer(Vec::new())),
                    clipboard_slot.clone(),
                    JsonShape::Ndjson,
                    false,
                )),
                test_job_context("live-sql-translated-clipboard"),
            )
            .await?;
            assert_eq!(clipboard_report.processed, 1);
            let clipboard_text = clipboard_slot.lock().unwrap().take().unwrap();
            assert!(clipboard_text.contains("Shell"));
            assert!(clipboard_text.ends_with('\n'));

            let documents_source = ExportSourceDto {
                mode: SourceMode::Documents,
                filter_json: None,
                projection_json: None,
                sort_json: None,
                pipeline_json: None,
                documents_json: Some(
                    r#"[{"kind":"visible-row","n":1},{"kind":"visible-row","n":2}]"#.into(),
                ),
            };
            let documents_slot = Arc::new(Mutex::new(None));
            let documents_report = run_pipeline(
                build_source(&coll, &documents_source).await?,
                Vec::new(),
                Box::new(CsvSink::new(
                    WriteSink::Plain(WriteTarget::Buffer(Vec::new())),
                    documents_slot.clone(),
                    vec!["kind".into(), "n".into()],
                    b',',
                    true,
                )),
                test_job_context("live-documents-csv-clipboard"),
            )
            .await?;
            assert_eq!(documents_report.processed, 2);
            let documents_text = documents_slot.lock().unwrap().take().unwrap();
            assert!(documents_text.contains("visible-row"));

            Ok::<(), Box<dyn std::error::Error>>(())
        }
        .await;

        let _ = db.drop().await;
        let _ = std::fs::remove_file(&json_path);
        let _ = std::fs::remove_file(&csv_path);
        outcome
    }

    #[test]
    fn resolve_destination_placeholders_passes_clipboard_through_unchanged() {
        let dest = ExportDestinationDto {
            kind: DestinationKind::Clipboard,
            path: None,
        };
        let ctx = PlaceholderContext {
            database: "shop",
            collection: "orders",
            profile: "Local",
        };
        let resolved = resolve_destination_placeholders(&dest, &ctx);
        assert!(matches!(resolved.kind, DestinationKind::Clipboard));
        assert!(resolved.path.is_none());
    }

    #[test]
    fn resolve_destination_placeholders_expands_file_path_tokens() {
        let dest = ExportDestinationDto {
            kind: DestinationKind::File,
            path: Some("/home/user/${db}_${collection}.json".into()),
        };
        let ctx = PlaceholderContext {
            database: "shop",
            collection: "orders",
            profile: "Local",
        };
        let resolved = resolve_destination_placeholders(&dest, &ctx);
        assert!(matches!(resolved.kind, DestinationKind::File));
        assert!(resolved.path.as_deref().unwrap().ends_with("shop_orders.json"));
    }

    #[tokio::test]
    async fn csv_export_with_field_mapping_uses_mapping_columns_and_renames() {
        // Exercises the Phase 4 wiring end-to-end without a live Mongo: a
        // VecSource feeds two docs through a field-mapping transform into a
        // CSV clipboard sink, and we assert the headers come from the mapping
        // (not the source field names) and the row cells are renamed.
        use crate::mongo::import_export::mapping::FieldMappingEntry;
        use crate::mongo::import_export::source_mem::VecSource;

        let docs = vec![
            doc! { "name": "Ada", "n": 1i32, "secret": "pw" },
            doc! { "name": "Grace", "n": 2i32, "secret": "pw2" },
        ];
        let entries = vec![
            FieldMappingEntry {
                source: "name".into(),
                target: "fullName".into(),
                skip: false,
                type_override: None,
            },
            FieldMappingEntry {
                source: "n".into(),
                target: "count".into(),
                skip: false,
                type_override: None,
            },
            FieldMappingEntry {
                source: "secret".into(),
                target: "secret".into(),
                skip: true,
                type_override: None,
            },
        ];
        let columns = FieldMappingTransform::csv_columns(&entries);
        assert_eq!(columns, vec!["fullName".to_string(), "count".to_string()]);

        let transform = FieldMappingTransform::new(entries).unwrap();
        let slot = Arc::new(Mutex::new(None));
        let sink: Box<dyn DocumentSink> = Box::new(CsvSink::new(
            WriteSink::Plain(WriteTarget::Buffer(Vec::new())),
            slot.clone(),
            columns,
            b',',
            true,
        ));
        let report = run_pipeline(
            Box::new(VecSource::new(docs)),
            vec![Box::new(transform)],
            sink,
            test_job_context("csv-mapping"),
        )
        .await
        .unwrap();
        assert_eq!(report.processed, 2);
        assert_eq!(report.errors, 0);

        let text = slot.lock().unwrap().take().unwrap();
        let mut reader = csv::Reader::from_reader(text.as_bytes());
        let headers: Vec<&str> = reader.headers().unwrap().iter().collect();
        assert_eq!(headers, vec!["fullName", "count"]);
        let rows: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get(0), Some("Ada"));
        assert_eq!(rows[0].get(1), Some("1"));
        assert_eq!(rows[1].get(0), Some("Grace"));
        assert_eq!(rows[1].get(1), Some("2"));
        // The skipped `secret` field must not appear anywhere in the output.
        assert!(!text.contains("secret"));
        assert!(!text.contains("pw"));
    }

    #[tokio::test]
    async fn json_export_with_field_mapping_drops_undeclared_fields() {
        use crate::mongo::import_export::mapping::FieldMappingEntry;
        use crate::mongo::import_export::source_mem::VecSource;

        let docs = vec![doc! { "name": "Ada", "n": 1i32, "secret": "pw" }];
        let entries = vec![FieldMappingEntry {
            source: "name".into(),
            target: "fullName".into(),
            skip: false,
            type_override: None,
        }];
        let transform = FieldMappingTransform::new(entries).unwrap();
        let slot = Arc::new(Mutex::new(None));
        let sink: Box<dyn DocumentSink> = Box::new(JsonSink::new(
            WriteSink::Plain(WriteTarget::Buffer(Vec::new())),
            slot.clone(),
            JsonShape::Array,
            false,
        ));
        let report = run_pipeline(
            Box::new(VecSource::new(docs)),
            vec![Box::new(transform)],
            sink,
            test_job_context("json-mapping"),
        )
        .await
        .unwrap();
        assert_eq!(report.processed, 1);

        let text = slot.lock().unwrap().take().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let obj = parsed.get(0).unwrap();
        assert_eq!(obj["fullName"], "Ada");
        assert!(obj.get("name").is_none());
        assert!(obj.get("n").is_none());
        assert!(obj.get("secret").is_none());
    }
}
