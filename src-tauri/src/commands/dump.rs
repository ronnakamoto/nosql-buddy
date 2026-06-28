//! Database dump command: stream one or more collections to BSON / JSON / CSV files.

use bson::Document;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::events::{emit_job_log_entry, emit_job_status_changed, notify_job_completed};
use crate::mongo::import_export::bson_sink::BsonSink;
use crate::mongo::import_export::core::{run_pipeline, DocumentSink, JobContext};
use crate::mongo::import_export::io_util::{validate_target_path, AtomicFileWriter, CompressionKind, CompressedWriter, WriteSink, WriteTarget};
use crate::mongo::import_export::json::{JsonShape, JsonSink};
use crate::mongo::import_export::placeholders::{resolve_path, PlaceholderContext};
use crate::mongo::import_export::source_cursor::CursorSource;
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DumpFormat {
    Bson,
    Json,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRequest {
    pub connection_id: String,
    pub database: String,
    pub collections: Vec<String>,
    pub destination_dir: String,
    pub path_template: String,
    pub format: DumpFormat,
    pub compression: CompressionKind,
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpResult {
    pub job_id: String,
    pub processed: u64,
    pub errors: u64,
    pub cancelled: bool,
    pub files: Vec<String>,
}

#[tauri::command]
pub async fn dump_database(
    request: DumpRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<DumpResult> {
    run_dump(&request, state.inner(), &app).await
}

/// Core dump logic callable from both the command handler and the scheduler.
pub async fn run_dump(request: &DumpRequest, state: &AppState, app: &tauri::AppHandle) -> AppResult<DumpResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let db = entry.client.database(&request.database);

    // Resolve collections to dump.
    let collections = if request.collections.is_empty() {
        let mut names = db.list_collection_names().await?;
        names.retain(|n| !n.starts_with("system."));
        names
    } else {
        request.collections.clone()
    };

    if collections.is_empty() {
        return Err(AppError::Validation("no collections to dump".into()));
    }

    let dest = validate_target_path(&request.destination_dir)?;
    std::fs::create_dir_all(&dest)?;

    // Register job.
    let mut meta = JobMeta::new(
        request.job_id.clone(),
        JobKind::Dump,
        request.connection_id.clone(),
        request.database.clone(),
    );
    meta.collections = collections.clone();
    meta.output_path = Some(request.destination_dir.clone());
    meta.config_json = Some(serde_json::to_string(request).unwrap_or_default());
    meta.profile_id = entry.profile_id.clone();
    state.jobs.create_job(meta).await;
    state
        .jobs
        .update_status(&request.job_id, JobStatus::Running, "Dump started".into())
        .await;
    emit_job_status_changed(app, &request.job_id, "running", "Dump started", None);

    let cancel_flag = state.jobs.register(request.job_id.clone()).await;

    let mut total_processed: u64 = 0;
    let mut total_errors: u64 = 0;
    let mut files: Vec<String> = Vec::new();
    let mut cancelled = false;
    let mut timeline_entries: Vec<crate::mongo::timeline_store::TimelineEntry> = Vec::with_capacity(collections.len());

    let profile_name = state
        .clients
        .get(&request.connection_id)
        .await
        .map(|e| e.name)
        .unwrap_or_default();

    for coll_name in &collections {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            cancelled = true;
            break;
        }

        let collection = db.collection::<Document>(coll_name);
        let cursor = collection.find(bson::doc! {}).batch_size(1000).await?;
        let total = collection.estimated_document_count().await.ok();
        let source = CursorSource::new(cursor, total);

        let file_path = resolve_dump_path(
            &dest,
            &request.path_template,
            &request.database,
            coll_name,
            &profile_name,
            request.format,
            request.compression,
        )?;
        let writer = AtomicFileWriter::create(file_path.clone())?;
        let target = WriteTarget::File(writer);
        let sink_target = if request.compression == CompressionKind::None {
            WriteSink::Plain(target)
        } else {
            WriteSink::Compressed(CompressedWriter::new(target, request.compression)?)
        };

        let sink: Box<dyn DocumentSink> = build_sink(sink_target, request.format)?;

        let ctx = JobContext {
            job_id: request.job_id.clone(),
            cancel_flag: cancel_flag.clone(),
            app_handle: Some(app.clone()),
            progress_observer: None,
            throttle_ms: 250,
            max_errors: 10_000,
            max_error_samples: 100,
        };

        let coll_start = std::time::Instant::now();
        let report = run_pipeline(Box::new(source), Vec::new(), sink, ctx).await;
        let coll_elapsed = coll_start.elapsed().as_millis() as u64;
        match report {
            Ok(r) => {
                total_processed += r.processed;
                total_errors += r.errors;
                files.push(file_path.to_string_lossy().to_string());
                let msg = format!("Dumped {coll_name}: {} docs", r.processed);
                state.jobs.log_info(&request.job_id, &msg).await;
                emit_job_log_entry(app, &request.job_id, &chrono_now(), "info", &msg);

                timeline_entries.push(
                    crate::mongo::timeline_store::TimelineEntry::builder(
                        uuid::Uuid::new_v4().to_string(),
                        profile_id.clone(),
                        crate::mongo::timeline_store::OperationKind::Dump,
                    )
                    .connection_id(request.connection_id.clone())
                    .database(request.database.clone())
                    .collection(coll_name.clone())
                    .actor("local-user".to_string())
                    .returned_count(r.processed)
                    .execution_ms(coll_elapsed)
                    .executed_at(chrono_now())
                    .build(),
                );
            }
            Err(e) => {
                total_errors += 1;
                let msg = format!("Failed to dump {coll_name}: {e}");
                state.jobs.log_error(&request.job_id, &msg).await;
                emit_job_log_entry(app, &request.job_id, &chrono_now(), "error", &msg);

                timeline_entries.push(
                    crate::mongo::timeline_store::TimelineEntry::builder(
                        uuid::Uuid::new_v4().to_string(),
                        profile_id.clone(),
                        crate::mongo::timeline_store::OperationKind::Dump,
                    )
                    .connection_id(request.connection_id.clone())
                    .database(request.database.clone())
                    .collection(coll_name.clone())
                    .actor("local-user".to_string())
                    .execution_ms(coll_elapsed)
                    .errored(true)
                    .error_message(Some(msg))
                    .executed_at(chrono_now())
                    .build(),
                );
            }
        }
    }

    if !timeline_entries.is_empty() {
        state.timeline.append_batch(timeline_entries).await;
    }

    state.jobs.unregister(&request.job_id).await;

    let status = if cancelled {
        JobStatus::Cancelled
    } else if total_errors > 0 {
        JobStatus::Failed
    } else {
        JobStatus::Done
    };
    let msg = format!("Dumped {} documents from {} collections", total_processed, files.len());
    let finished = Some(chrono_now());
    state
        .jobs
        .update_status(&request.job_id, status, msg.clone())
        .await;
    emit_job_status_changed(
        app,
        &request.job_id,
        &format!("{status:?}").to_lowercase(),
        &msg,
        finished.clone(),
    );
    notify_job_completed(app, &request.job_id, "Dump", &msg);

    Ok(DumpResult {
        job_id: request.job_id.clone(),
        processed: total_processed,
        errors: total_errors,
        cancelled,
        files,
    })
}

fn build_sink(target: WriteSink, format: DumpFormat) -> AppResult<Box<dyn DocumentSink>> {
    match format {
        DumpFormat::Bson => Ok(Box::new(BsonSink::new(target))),
        DumpFormat::Json => Ok(Box::new(JsonSink::new(
            target,
            std::sync::Arc::new(std::sync::Mutex::new(None)),
            JsonShape::Array,
            false,
        ))),
    }
}

fn resolve_dump_path(
    dest: &PathBuf,
    template: &str,
    database: &str,
    collection: &str,
    profile: &str,
    format: DumpFormat,
    compression: CompressionKind,
) -> AppResult<PathBuf> {
    let resolved = if template.trim().is_empty() {
        format!("{collection}")
    } else {
        resolve_path(
            template,
            &PlaceholderContext {
                database,
                collection,
                profile,
            },
        )
    };

    let ext = match format {
        DumpFormat::Bson => "bson",
        DumpFormat::Json => "json",
    };
    let comp_ext = compression.extension();
    let file_name = format!("{resolved}.{ext}{comp_ext}");
    Ok(dest.join(file_name))
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::import_export::io_util::CompressionKind;
    use std::path::PathBuf;

    #[test]
    fn resolve_dump_path_empty_template_uses_collection_name() {
        let dest = PathBuf::from("/tmp");
        let path = resolve_dump_path(&dest, "", "mydb", "users", "prod", DumpFormat::Bson, CompressionKind::None).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "users.bson");
    }

    #[test]
    fn resolve_dump_path_with_collection_token() {
        let dest = PathBuf::from("/tmp");
        let path = resolve_dump_path(&dest, "${collection}_backup", "mydb", "orders", "prod", DumpFormat::Json, CompressionKind::Gzip).unwrap();
        assert!(path.file_name().unwrap().to_str().unwrap().starts_with("orders_backup.json.gz"));
    }

    #[test]
    fn resolve_dump_path_with_db_and_collection_tokens() {
        let dest = PathBuf::from("/tmp");
        let path = resolve_dump_path(&dest, "${db}_${collection}", "staging", "events", "dev", DumpFormat::Bson, CompressionKind::Zstd).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "staging_events.bson.zst");
    }

    #[test]
    fn resolve_dump_path_with_profile_token() {
        let dest = PathBuf::from("/tmp");
        let path = resolve_dump_path(&dest, "${profile}_${collection}", "mydb", "items", "Local RS0", DumpFormat::Bson, CompressionKind::None).unwrap();
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "Local RS0_items.bson");
    }
}
