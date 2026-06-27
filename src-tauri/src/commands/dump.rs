//! Database dump command: stream one or more collections to BSON files.

use bson::Document;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::events::{emit_job_log_entry, emit_job_status_changed};
use crate::mongo::import_export::bson_sink::BsonSink;
use crate::mongo::import_export::core::{run_pipeline, JobContext};
use crate::mongo::import_export::io_util::{validate_target_path, AtomicFileWriter};
use crate::mongo::import_export::source_cursor::CursorSource;
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::state::AppState;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpRequest {
    pub connection_id: String,
    pub database: String,
    pub collections: Vec<String>,
    pub destination_dir: String,
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
    let entry = state.clients.get(&request.connection_id).await?;
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
    state.jobs.create_job(meta).await;
    state
        .jobs
        .update_status(&request.job_id, JobStatus::Running, "Dump started".into())
        .await;
    emit_job_status_changed(&app, &request.job_id, "running", "Dump started", None);

    let cancel_flag = state.jobs.register(request.job_id.clone()).await;

    let mut total_processed: u64 = 0;
    let mut total_errors: u64 = 0;
    let mut files: Vec<String> = Vec::new();
    let mut cancelled = false;

    for coll_name in &collections {
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            cancelled = true;
            break;
        }

        let collection = db.collection::<Document>(coll_name);
        let cursor = collection.find(bson::doc! {}).batch_size(1000).await?;
        let total = collection.estimated_document_count().await.ok();
        let file_path = dest.join(format!("{coll_name}.bson"));
        let writer = AtomicFileWriter::create(file_path.clone())?;
        let sink = BsonSink::new(crate::mongo::import_export::io_util::WriteTarget::File(writer));
        let source = CursorSource::new(cursor, total);

        let ctx = JobContext {
            job_id: request.job_id.clone(),
            cancel_flag: cancel_flag.clone(),
            app_handle: Some(app.clone()),
            progress_observer: None,
            throttle_ms: 250,
            max_errors: 10_000,
            max_error_samples: 100,
        };

        let report = run_pipeline(Box::new(source), Vec::new(), Box::new(sink), ctx).await;
        match report {
            Ok(r) => {
                total_processed += r.processed;
                total_errors += r.errors;
                files.push(file_path.to_string_lossy().to_string());
                let msg = format!("Dumped {coll_name}: {} docs", r.processed);
                state.jobs.log_info(&request.job_id, &msg).await;
                emit_job_log_entry(&app, &request.job_id, &chrono_now(), "info", &msg);
            }
            Err(e) => {
                total_errors += 1;
                let msg = format!("Failed to dump {coll_name}: {e}");
                state.jobs.log_error(&request.job_id, &msg).await;
                emit_job_log_entry(&app, &request.job_id, &chrono_now(), "error", &msg);
            }
        }
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
        &app,
        &request.job_id,
        &format!("{status:?}").to_lowercase(),
        &msg,
        finished.clone(),
    );

    Ok(DumpResult {
        job_id: request.job_id,
        processed: total_processed,
        errors: total_errors,
        cancelled,
        files,
    })
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}
