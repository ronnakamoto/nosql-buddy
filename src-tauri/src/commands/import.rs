//! Import command: preview and stream JSON/CSV rows into a MongoDB collection.

use bson::{Bson, Document};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::doc_to_display_json;
use crate::mongo::import_export::collection_sink::CollectionSink;
use crate::mongo::import_export::core::{
    run_pipeline, DocumentSink, DocumentSource, JobContext, RowError, RowResult,
};
use crate::mongo::import_export::csv_source::CsvSource;
use crate::mongo::import_export::io_util::validate_source_path;
use crate::mongo::import_export::json_source::{JsonImportShape, JsonSource};
use crate::mongo::import_export::mapping::{FieldMappingEntry, FieldMappingTransform};
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::state::AppState;

const DEFAULT_BATCH_SIZE: usize = 1000;
const DEFAULT_PREVIEW_ROWS: usize = 20;
const MAX_PREVIEW_ROWS: usize = 100;
const MAX_ERROR_SAMPLES: usize = 100;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportFormat {
    Json,
    Csv,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportSourceKind {
    File,
    Clipboard,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSourceDto {
    pub kind: ImportSourceKind,
    pub path: Option<String>,
    pub clipboard_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub json_shape: JsonImportShape,
    pub csv_delimiter: Option<String>,
    pub csv_headers: bool,
    pub batch_size: Option<usize>,
    pub preview_rows: Option<usize>,
    /// Optional field-mapping table applied as a Transform before the
    /// collection sink. When present and non-empty, only the declared
    /// `target` fields are written; source columns not in the table are
    /// dropped, and type overrides coerce the parsed cell values.
    pub field_mapping: Option<Vec<FieldMappingEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub job_id: String,
    pub source: ImportSourceDto,
    pub format: ImportFormat,
    pub options: ImportOptions,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInference {
    pub name: String,
    pub bson_type: String,
    pub nullable: bool,
    pub samples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewImportResult {
    pub rows: Vec<serde_json::Value>,
    pub fields: Vec<FieldInference>,
    pub errors: Vec<RowError>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub job_id: String,
    pub processed: u64,
    pub inserted: u64,
    pub errors: u64,
    pub cancelled: bool,
    pub row_errors: Vec<RowError>,
}

#[tauri::command]
pub async fn preview_import(request: ImportRequest) -> AppResult<PreviewImportResult> {
    let mut source = build_source(&request)?;
    let limit = request
        .options
        .preview_rows
        .unwrap_or(DEFAULT_PREVIEW_ROWS)
        .clamp(1, MAX_PREVIEW_ROWS);
    let mut docs = Vec::new();
    let mut rows = Vec::new();
    let mut errors = Vec::new();

    while docs.len() < limit {
        match source.next_doc().await? {
            Some(RowResult::Doc(doc)) => {
                rows.push(doc_to_display_json(&doc)?);
                docs.push(doc);
            }
            Some(RowResult::Skipped) => {}
            Some(RowResult::Error(err)) => {
                if errors.len() < MAX_ERROR_SAMPLES {
                    errors.push(err);
                }
            }
            None => break,
        }
    }

    Ok(PreviewImportResult {
        fields: infer_fields(&docs),
        rows,
        errors,
    })
}

#[tauri::command]
pub async fn run_import(
    request: ImportRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<ImportResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let collection = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let source = build_source(&request)?;
    let inserted = Arc::new(AtomicU64::new(0));
    let sink: Box<dyn DocumentSink> = Box::new(CollectionSink::new(
        collection,
        request.options.batch_size.unwrap_or(DEFAULT_BATCH_SIZE),
        inserted.clone(),
    ));

    // Compose the transform chain: currently just the optional field mapping.
    let mut transforms: Vec<Box<dyn crate::mongo::import_export::core::Transform>> = Vec::new();
    if let Some(entries) = request.options.field_mapping.clone() {
        if !entries.is_empty() {
            transforms.push(Box::new(FieldMappingTransform::new(entries)?));
        }
    }

    let mut meta = JobMeta::new(
        request.job_id.clone(),
        JobKind::Import,
        request.connection_id.clone(),
        request.database.clone(),
    );
    meta.collections = vec![request.collection.clone()];
    meta.source_path = request.source.path.clone();
    state.jobs.create_job(meta).await;
    state.jobs.update_status(&request.job_id, JobStatus::Running, "Import started".into()).await;

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
        max_error_samples: MAX_ERROR_SAMPLES,
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
    let msg = format!("Imported {} documents, {} errors", inserted.load(Ordering::Relaxed), report.errors);
    state.jobs.update_status(&request.job_id, status, msg).await;

    Ok(ImportResult {
        job_id: report.job_id,
        processed: report.processed,
        inserted: inserted.load(Ordering::Relaxed),
        errors: report.errors,
        cancelled: report.cancelled,
        row_errors: report.row_errors,
    })
}

fn build_source(request: &ImportRequest) -> AppResult<Box<dyn DocumentSource>> {
    let delimiter = resolve_delimiter(request.options.csv_delimiter.as_deref())?;
    match (request.format, request.source.kind) {
        (ImportFormat::Json, ImportSourceKind::File) => {
            let path =
                request.source.path.as_deref().ok_or_else(|| {
                    AppError::Validation("file import requires a source path".into())
                })?;
            let path = validate_source_path(path)?;
            Ok(Box::new(JsonSource::from_path(
                &path,
                request.options.json_shape,
            )?))
        }
        (ImportFormat::Json, ImportSourceKind::Clipboard) => {
            let text = request.source.clipboard_text.clone().ok_or_else(|| {
                AppError::Validation("clipboard import requires clipboard text".into())
            })?;
            Ok(Box::new(JsonSource::from_text(
                text,
                request.options.json_shape,
            )?))
        }
        (ImportFormat::Csv, ImportSourceKind::File) => {
            let path =
                request.source.path.as_deref().ok_or_else(|| {
                    AppError::Validation("file import requires a source path".into())
                })?;
            let path = validate_source_path(path)?;
            Ok(Box::new(CsvSource::from_path(
                &path,
                delimiter,
                request.options.csv_headers,
            )?))
        }
        (ImportFormat::Csv, ImportSourceKind::Clipboard) => {
            let text = request.source.clipboard_text.clone().ok_or_else(|| {
                AppError::Validation("clipboard import requires clipboard text".into())
            })?;
            Ok(Box::new(CsvSource::from_text(
                text,
                delimiter,
                request.options.csv_headers,
            )?))
        }
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

fn infer_fields(docs: &[Document]) -> Vec<FieldInference> {
    #[derive(Default)]
    struct Acc {
        seen: usize,
        bson_type: Option<String>,
        samples: Vec<String>,
    }

    let mut fields = BTreeMap::<String, Acc>::new();
    for doc in docs {
        for (key, value) in doc {
            let acc = fields.entry(key.clone()).or_default();
            acc.seen += 1;
            let kind = bson_kind(value).to_string();
            acc.bson_type = Some(match &acc.bson_type {
                None => kind,
                Some(existing) if existing == &kind => existing.clone(),
                Some(_) => "mixed".to_string(),
            });
            if acc.samples.len() < 3 {
                acc.samples.push(sample_value(value));
            }
        }
    }

    fields
        .into_iter()
        .map(|(name, acc)| FieldInference {
            name,
            bson_type: acc.bson_type.unwrap_or_else(|| "unknown".into()),
            nullable: acc.seen < docs.len(),
            samples: acc.samples,
        })
        .collect()
}

fn bson_kind(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "object",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::ObjectId(_) => "objectId",
        Bson::DateTime(_) => "date",
        Bson::Binary(_) => "binary",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        Bson::Decimal128(_) => "decimal128",
        _ => "bson",
    }
}

fn sample_value(value: &Bson) -> String {
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => dt
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        Bson::Decimal128(d) => d.to_string(),
        Bson::Null => "null".into(),
        other => serde_json::to_string(&other.clone().into_relaxed_extjson()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::import_export::core::{DocumentSink, RowResult};
    use crate::mongo::import_export::source_mem::VecSource;
    use async_trait::async_trait;
    use bson::doc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn infers_field_types_and_nullability() {
        let fields = infer_fields(&[doc! { "a": 1i32, "b": "x" }, doc! { "a": 2i64 }]);
        let a = fields.iter().find(|f| f.name == "a").unwrap();
        assert_eq!(a.bson_type, "mixed");
        assert!(!a.nullable);
        let b = fields.iter().find(|f| f.name == "b").unwrap();
        assert!(b.nullable);
        assert_eq!(b.samples, vec!["x"]);
    }

    /// A test-only sink that collects written documents, mirroring how
    /// `CollectionSink` consumes the pipeline output.
    struct CollectSink {
        docs: Arc<Mutex<Vec<Document>>>,
        finished: Arc<AtomicBool>,
    }

    #[async_trait]
    impl DocumentSink for CollectSink {
        async fn start(&mut self) -> AppResult<()> {
            Ok(())
        }
        async fn write(&mut self, doc: Document) -> AppResult<()> {
            self.docs.lock().unwrap().push(doc);
            Ok(())
        }
        async fn finish(self: Box<Self>) -> AppResult<()> {
            self.finished.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn abort(self: Box<Self>) -> AppResult<()> {
            Ok(())
        }
    }

    fn job_ctx(id: &str) -> JobContext {
        JobContext {
            job_id: id.into(),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            app_handle: None,
            progress_observer: None,
            throttle_ms: 0,
            max_errors: 100,
            max_error_samples: 100,
        }
    }

    #[tokio::test]
    async fn import_with_field_mapping_renames_drops_and_coerces() {
        // Phase 4 wiring: an import-style pipeline (VecSource -> field mapping
        // -> collecting sink) renames source columns, drops skipped ones, and
        // applies a type override. This mirrors what `run_import` does without
        // needing a live Mongo collection.
        let source_docs = vec![
            doc! { "name": "Ada", "n": "42", "secret": "pw" },
            doc! { "name": "Grace", "n": "2", "secret": "pw2" },
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
                type_override: Some(crate::mongo::import_export::mapping::TypeOverride::Int32),
            },
            FieldMappingEntry {
                source: "secret".into(),
                target: "secret".into(),
                skip: true,
                type_override: None,
            },
        ];
        let transform = FieldMappingTransform::new(entries).unwrap();

        let collected: Arc<Mutex<Vec<Document>>> = Arc::default();
        let finished = Arc::new(AtomicBool::new(false));
        let sink = CollectSink {
            docs: collected.clone(),
            finished: finished.clone(),
        };

        let report = run_pipeline(
            Box::new(VecSource::new(source_docs)),
            vec![Box::new(transform)],
            Box::new(sink),
            job_ctx("import-mapping"),
        )
        .await
        .unwrap();

        assert_eq!(report.processed, 2);
        assert_eq!(report.errors, 0);
        assert!(finished.load(Ordering::Relaxed));

        let docs = collected.lock().unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].get_str("fullName").unwrap(), "Ada");
        assert_eq!(docs[0].get_i32("count").unwrap(), 42); // string -> int32
        assert!(docs[0].get("secret").is_none());
        assert!(docs[0].get("name").is_none());
        assert!(docs[0].get("n").is_none());
        assert_eq!(docs[1].get_str("fullName").unwrap(), "Grace");
        assert_eq!(docs[1].get_i32("count").unwrap(), 2);
    }

    #[tokio::test]
    async fn import_with_empty_field_mapping_is_a_no_op_transform() {
        // An empty mapping table means "no transform" — the pipeline passes
        // documents through unchanged. `run_import` skips building a transform
        // when entries is empty, so this test documents that contract.
        let docs = vec![doc! { "a": 1i32 }];
        let collected: Arc<Mutex<Vec<Document>>> = Arc::default();
        let sink = CollectSink {
            docs: collected.clone(),
            finished: Arc::new(AtomicBool::new(false)),
        };
        let report = run_pipeline(
            Box::new(VecSource::new(docs)),
            Vec::new(),
            Box::new(sink),
            job_ctx("import-no-mapping"),
        )
        .await
        .unwrap();
        assert_eq!(report.processed, 1);
        assert_eq!(collected.lock().unwrap()[0].get_i32("a").unwrap(), 1);
    }

    // Silence the unused-import warning for RowResult when only some tests use it.
    #[allow(dead_code)]
    fn _row_result_is_imported(_r: RowResult) {}
}
