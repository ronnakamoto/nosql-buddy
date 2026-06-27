//! Job management commands: list, get detail, cancel, rerun, delete, and
//! schedule updates. All state is held in the shared `JobStore`.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::AppResult;
use crate::mongo::job_store::{JobFilter, JobLogEntry, JobMeta, JobStatus};
use crate::state::AppState;
use std::str::FromStr;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsRequest {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListJobsResponse {
    pub jobs: Vec<JobMeta>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDetailResponse {
    #[serde(flatten)]
    pub meta: JobMeta,
    pub logs: Vec<JobLogEntry>,
}

#[tauri::command]
pub async fn list_jobs(request: ListJobsRequest, state: State<'_, AppState>) -> AppResult<ListJobsResponse> {
    let filter = JobFilter {
        connection_id: request.connection_id,
        database: request.database,
        kind: request.kind.and_then(|k| match k.as_str() {
            "dump" => Some(crate::mongo::job_store::JobKind::Dump),
            "restore" => Some(crate::mongo::job_store::JobKind::Restore),
            "export" => Some(crate::mongo::job_store::JobKind::Export),
            "import" => Some(crate::mongo::job_store::JobKind::Import),
            _ => None,
        }),
        status: request.status.and_then(|s| match s.as_str() {
            "queued" => Some(JobStatus::Queued),
            "running" => Some(JobStatus::Running),
            "done" => Some(JobStatus::Done),
            "failed" => Some(JobStatus::Failed),
            "cancelled" => Some(JobStatus::Cancelled),
            _ => None,
        }),
        limit: request.limit,
    };
    let jobs = state.jobs.list(filter).await;
    Ok(ListJobsResponse { jobs })
}

#[tauri::command]
pub async fn get_job(job_id: String, state: State<'_, AppState>) -> AppResult<JobDetailResponse> {
    let meta = state
        .jobs
        .get(&job_id)
        .await
        .ok_or_else(|| crate::error::AppError::NotFound(format!("job not found: {job_id}")))?;
    let logs = state.jobs.get_log(&job_id).await;
    Ok(JobDetailResponse { meta, logs })
}

#[tauri::command]
pub async fn cancel_job(job_id: String, state: State<'_, AppState>) -> AppResult<bool> {
    let ok = state.jobs.cancel(&job_id).await;
    if ok {
        state
            .jobs
            .update_status(&job_id, JobStatus::Cancelled, "Cancelled by user".into())
            .await;
    }
    Ok(ok)
}

#[tauri::command]
pub async fn delete_job(job_id: String, state: State<'_, AppState>) -> AppResult<bool> {
    Ok(state.jobs.delete(&job_id).await)
}

#[tauri::command]
pub async fn rerun_job(job_id: String, state: State<'_, AppState>) -> AppResult<JobMeta> {
    let meta = state
        .jobs
        .get(&job_id)
        .await
        .ok_or_else(|| crate::error::AppError::NotFound(format!("job not found: {job_id}")))?;

    // Clone the meta, reset status, and queue a new job.
    let mut new_meta = meta.clone();
    new_meta.job_id = uuid::Uuid::new_v4().to_string();
    new_meta.status = JobStatus::Queued;
    new_meta.created_at = chrono::Local::now().to_rfc3339();
    new_meta.started_at = None;
    new_meta.finished_at = None;
    new_meta.processed = 0;
    new_meta.total = None;
    new_meta.errors = 0;
    new_meta.message = "Queued for rerun".into();

    state.jobs.create_job(new_meta.clone()).await;
    state.jobs.log_info(&new_meta.job_id, "Queued for rerun").await;

    Ok(new_meta)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateScheduleRequest {
    pub job_id: String,
    pub cron: String,
    pub enabled: bool,
    pub retention_count: Option<u32>,
}

#[tauri::command]
pub async fn update_schedule(
    request: UpdateScheduleRequest,
    state: State<'_, AppState>,
) -> AppResult<JobMeta> {
    let mut meta = state
        .jobs
        .get(&request.job_id)
        .await
        .ok_or_else(|| crate::error::AppError::NotFound(format!("job not found: {}", request.job_id)))?;

    let next = if request.enabled {
        let schedule = cron::Schedule::from_str(&request.cron)
            .map_err(|e| crate::error::AppError::Validation(format!("invalid cron: {e}")))?;
        schedule.upcoming(chrono::Local).next().map(|dt| dt.to_rfc3339())
    } else {
        None
    };

    meta.schedule = Some(crate::mongo::job_store::ScheduleConfig {
        cron: request.cron,
        enabled: request.enabled,
        retention_count: request.retention_count,
        next_run_at: next,
    });
    state.jobs.create_job(meta.clone()).await;
    Ok(meta)
}
