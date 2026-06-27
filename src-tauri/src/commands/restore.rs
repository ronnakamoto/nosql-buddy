//! Database restore command: read BSON files and insert into collections.

use bson::Document;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppResult;
use crate::events::{emit_job_log_entry, emit_job_status_changed, notify_job_completed};
use crate::mongo::import_export::bson_source::BsonSource;
use crate::mongo::import_export::collection_sink::{CollectionSink, InsertMode};
use crate::mongo::import_export::core::{run_pipeline, JobContext};
use crate::mongo::import_export::io_util::validate_source_dir;
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::state::AppState;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictStrategy {
    Drop,
    Skip,
    Upsert,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub connection_id: String,
    pub source_dir: String,
    pub target_database: String,
    pub create_database: bool,
    pub collection_map: Vec<CollectionMapping>,
    pub conflict_strategy: ConflictStrategy,
    pub job_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionMapping {
    pub source: String,
    pub target: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub job_id: String,
    pub processed: u64,
    pub inserted: u64,
    pub errors: u64,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePreviewEntry {
    pub source_name: String,
    pub target_name: String,
    pub approximate_count: u64,
    pub size_bytes: u64,
}

#[tauri::command]
pub async fn preview_archive(source_dir: String) -> AppResult<Vec<ArchivePreviewEntry>> {
    let dir = validate_source_dir(&source_dir)?;
    let mut entries = Vec::new();

    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");

        let is_bson = ext == "bson" || (ext == "gz" && stem.ends_with(".bson")) || (ext == "zst" && stem.ends_with(".bson"));
        if !is_bson {
            continue;
        }

        // Extract the collection name consistently:
        //  - categories.bson       -> "categories"
        //  - categories.bson.gz  -> "categories"  (stem is "categories.bson", strip ".bson")
        //  - categories.bson.zst -> "categories"
        let name = if ext == "gz" || ext == "zst" {
            stem.strip_suffix(".bson").unwrap_or(stem).to_string()
        } else {
            stem.to_string()
        };
        let size = entry.metadata()?.len();
        // Rough estimate: 500 bytes average document size.
        let approx = size / 500;
        entries.push(ArchivePreviewEntry {
            source_name: name.clone(),
            target_name: name,
            approximate_count: approx,
            size_bytes: size,
        });
    }

    entries.sort_by(|a, b| a.source_name.cmp(&b.source_name));
    Ok(entries)
}

#[tauri::command]
pub async fn restore_database(
    request: RestoreRequest,
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> AppResult<RestoreResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let db = entry.client.database(&request.target_database);

    let dir = validate_source_dir(&request.source_dir)?;

    // Register job.
    let mut meta = JobMeta::new(
        request.job_id.clone(),
        JobKind::Restore,
        request.connection_id.clone(),
        request.target_database.clone(),
    );
    meta.source_path = Some(request.source_dir.clone());
    let collections: Vec<String> = request
        .collection_map
        .iter()
        .filter(|m| m.enabled)
        .map(|m| m.target.clone())
        .collect();
    meta.collections = collections;
    state.jobs.create_job(meta).await;
    state
        .jobs
        .update_status(&request.job_id, JobStatus::Running, "Restore started".into())
        .await;
    emit_job_status_changed(&app, &request.job_id, "running", "Restore started", None);

    let cancel_flag = state.jobs.register(request.job_id.clone()).await;

    let mut total_processed: u64 = 0;
    let mut total_inserted: u64 = 0;
    let mut total_errors: u64 = 0;
    let mut cancelled = false;

    for mapping in &request.collection_map {
        if !mapping.enabled {
            continue;
        }
        if cancel_flag.load(std::sync::atomic::Ordering::Relaxed) {
            cancelled = true;
            break;
        }

        // Resolve the source file, trying plain, gzip, and zstd variants.
        let base = dir.join(format!("{}.bson", mapping.source));
        let gz = dir.join(format!("{}.bson.gz", mapping.source));
        let zst = dir.join(format!("{}.bson.zst", mapping.source));
        let source_path = if base.is_file() {
            base
        } else if gz.is_file() {
            gz
        } else if zst.is_file() {
            zst
        } else {
            let msg = format!("Source file not found: {}", base.display());
            state.jobs.log_warn(&request.job_id, &msg).await;
            emit_job_log_entry(&app, &request.job_id, &chrono_now(), "warn", &msg);
            continue;
        };

        let collection = db.collection::<Document>(&mapping.target);

        // Apply conflict strategy.
        match request.conflict_strategy {
            ConflictStrategy::Drop => {
                let _ = collection.drop().await;
            }
            ConflictStrategy::Skip => {
                let count = collection.estimated_document_count().await.unwrap_or(1);
                if count > 0 {
                    let msg = format!("Skipping {}: already exists", mapping.target);
                    state.jobs.log_info(&request.job_id, &msg).await;
                    emit_job_log_entry(&app, &request.job_id, &chrono_now(), "info", &msg);
                    continue;
                }
            }
            ConflictStrategy::Upsert => {
                // No pre-action needed; CollectionSink will replace_one with upsert:true.
            }
        }

        let source = BsonSource::from_path(&source_path)?;
        let inserted = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let sink = CollectionSink::new(collection, 1000, inserted.clone())
            .with_mode(if matches!(request.conflict_strategy, ConflictStrategy::Upsert) {
                InsertMode::Upsert
            } else {
                InsertMode::Insert
            });

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
                total_inserted += inserted.load(std::sync::atomic::Ordering::Relaxed);
                total_errors += r.errors;
                let msg = format!("Restored {}: {} docs", mapping.target, r.processed);
                state.jobs.log_info(&request.job_id, &msg).await;
                emit_job_log_entry(&app, &request.job_id, &chrono_now(), "info", &msg);
            }
            Err(e) => {
                total_errors += 1;
                let msg = format!("Failed to restore {}: {e}", mapping.target);
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
    let msg = format!("Restored {} documents into {} collections", total_inserted, request.collection_map.len());
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
    notify_job_completed(&app, &request.job_id, "Restore", &msg);

    Ok(RestoreResult {
        job_id: request.job_id,
        processed: total_processed,
        inserted: total_inserted,
        errors: total_errors,
        cancelled,
    })
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir_with_files(files: &[(&str, &[u8])]) -> PathBuf {
        let dir = dirs::home_dir()
            .unwrap()
            .join(format!("mongo-buddy-restore-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, content) in files {
            std::fs::write(dir.join(name), content).unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn preview_archive_finds_plain_bson_files() {
        let dir = temp_dir_with_files(&[
            ("users.bson", b"xxx"),
            ("orders.bson", b"yyy"),
            ("readme.txt", b"zzz"),
        ]);
        let entries = preview_archive(dir.to_string_lossy().to_string()).await.unwrap();
        let names: Vec<String> = entries.iter().map(|e| e.source_name.clone()).collect();
        assert!(names.contains(&"users".to_string()));
        assert!(names.contains(&"orders".to_string()));
        assert!(!names.contains(&"readme".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_archive_finds_gzip_bson_files() {
        let dir = temp_dir_with_files(&[
            ("users.bson.gz", b"xxx"),
            ("orders.bson.gz", b"yyy"),
        ]);
        let entries = preview_archive(dir.to_string_lossy().to_string()).await.unwrap();
        let names: Vec<String> = entries.iter().map(|e| e.source_name.clone()).collect();
        assert!(names.contains(&"users".to_string()));
        assert!(names.contains(&"orders".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_archive_finds_zstd_bson_files() {
        let dir = temp_dir_with_files(&[
            ("products.bson.zst", b"xxx"),
        ]);
        let entries = preview_archive(dir.to_string_lossy().to_string()).await.unwrap();
        let names: Vec<String> = entries.iter().map(|e| e.source_name.clone()).collect();
        assert!(names.contains(&"products".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn preview_archive_mixed_compression_strips_bson_suffix_consistently() {
        let dir = temp_dir_with_files(&[
            ("categories.bson", b"a"),
            ("orders.bson.gz", b"b"),
            ("products.bson.zst", b"c"),
        ]);
        let entries = preview_archive(dir.to_string_lossy().to_string()).await.unwrap();
        let names: Vec<String> = entries.iter().map(|e| e.source_name.clone()).collect();
        // All should be bare collection names, no trailing ".bson".
        assert!(names.contains(&"categories".to_string()));
        assert!(names.contains(&"orders".to_string()));
        assert!(names.contains(&"products".to_string()));
        assert!(!names.iter().any(|n| n.ends_with(".bson")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
