//! Unified job store for tracking dump, restore, export, and import jobs.
//!
//! Replaces the ephemeral `JobRegistry` with a store that keeps history,
//! per-job logs, and cancellation flags. All operations are async-safe
//! via `tokio::sync::RwLock`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Classification of a job.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    Dump,
    Restore,
    Export,
    Import,
}

/// Current status of a job.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

/// Lightweight schedule descriptor attached to a job.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleConfig {
    pub cron: String,
    pub enabled: bool,
    pub retention_count: Option<u32>,
    pub next_run_at: Option<String>,
}

/// Metadata record for a single job. Serialised to the frontend verbatim.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobMeta {
    pub job_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub connection_id: String,
    pub database: String,
    pub collections: Vec<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub output_path: Option<String>,
    pub source_path: Option<String>,
    pub schedule: Option<ScheduleConfig>,
    pub processed: u64,
    pub total: Option<u64>,
    pub errors: u64,
    pub message: String,
}

impl JobMeta {
    pub fn new(job_id: String, kind: JobKind, connection_id: String, database: String) -> Self {
        Self {
            job_id,
            kind,
            status: JobStatus::Queued,
            connection_id,
            database,
            collections: Vec::new(),
            created_at: chrono_now(),
            started_at: None,
            finished_at: None,
            output_path: None,
            source_path: None,
            schedule: None,
            processed: 0,
            total: None,
            errors: 0,
            message: String::new(),
        }
    }
}

/// One line in a job's log tail.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobLogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// Filter passed to `JobStore::list`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobFilter {
    pub connection_id: Option<String>,
    pub database: Option<String>,
    pub kind: Option<JobKind>,
    pub status: Option<JobStatus>,
    pub limit: Option<usize>,
}

/// Shared, async-safe job store.
pub struct JobStore {
    active: RwLock<HashMap<String, Arc<AtomicBool>>>,
    jobs: RwLock<Vec<JobMeta>>,
    logs: RwLock<HashMap<String, Vec<JobLogEntry>>>,
}

impl JobStore {
    pub fn new() -> Self {
        Self {
            active: RwLock::new(HashMap::new()),
            jobs: RwLock::new(Vec::new()),
            logs: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a new job meta record and return the job id.
    pub async fn create_job(&self, meta: JobMeta) -> String {
        let id = meta.job_id.clone();
        let mut jobs = self.jobs.write().await;
        // If a job with the same id already exists, remove it.
        jobs.retain(|j| j.job_id != id);
        jobs.push(meta);
        id
    }

    /// Register a cancel flag for a job. Backward-compatible with the old
    /// `JobRegistry::register`. Does **not** create a `JobMeta`; callers that
    /// want history should call `create_job` first.
    pub async fn register(&self, job_id: String) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.active.write().await.insert(job_id, flag.clone());
        flag
    }

    /// Flag a job for cancellation. Returns `true` if the job was found.
    pub async fn cancel(&self, job_id: &str) -> bool {
        let guard = self.active.read().await;
        if let Some(flag) = guard.get(job_id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Remove the cancel flag for a finished job.
    pub async fn unregister(&self, job_id: &str) {
        self.active.write().await.remove(job_id);
    }

    /// Update the status and message of an existing job.
    pub async fn update_status(&self, job_id: &str, status: JobStatus, message: String) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.status = status;
            job.message = message;
            match status {
                JobStatus::Running if job.started_at.is_none() => {
                    job.started_at = Some(chrono_now());
                }
                JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled => {
                    job.finished_at = Some(chrono_now());
                }
                _ => {}
            }
        }
    }

    /// Update progress counters for a running job.
    pub async fn update_progress(&self, job_id: &str, processed: u64, total: Option<u64>, errors: u64) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.processed = processed;
            if total.is_some() {
                job.total = total;
            }
            job.errors = errors;
        }
    }

    /// Append a log entry to a job's log.
    pub async fn append_log(&self, job_id: &str, entry: JobLogEntry) {
        let mut logs = self.logs.write().await;
        logs.entry(job_id.to_string()).or_default().push(entry);
    }

    /// Convenience: log an info message with the current timestamp.
    pub async fn log_info(&self, job_id: &str, message: impl Into<String>) {
        self.append_log(
            job_id,
            JobLogEntry {
                timestamp: chrono_now(),
                level: LogLevel::Info,
                message: message.into(),
            },
        )
        .await;
    }

    /// Convenience: log a warning.
    pub async fn log_warn(&self, job_id: &str, message: impl Into<String>) {
        self.append_log(
            job_id,
            JobLogEntry {
                timestamp: chrono_now(),
                level: LogLevel::Warn,
                message: message.into(),
            },
        )
        .await;
    }

    /// Convenience: log an error.
    pub async fn log_error(&self, job_id: &str, message: impl Into<String>) {
        self.append_log(
            job_id,
            JobLogEntry {
                timestamp: chrono_now(),
                level: LogLevel::Error,
                message: message.into(),
            },
        )
        .await;
    }

    /// List jobs, newest first, optionally filtered.
    pub async fn list(&self, filter: JobFilter) -> Vec<JobMeta> {
        let jobs = self.jobs.read().await;
        let mut out: Vec<JobMeta> = jobs
            .iter()
            .filter(|j| {
                filter.connection_id.as_ref().map_or(true, |id| &j.connection_id == id)
                    && filter.database.as_ref().map_or(true, |db| &j.database == db)
                    && filter.kind.map_or(true, |k| j.kind == k)
                    && filter.status.map_or(true, |s| j.status == s)
            })
            .cloned()
            .collect();
        // Newest first (by created_at, descending).
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(limit) = filter.limit {
            out.truncate(limit);
        }
        out
    }

    /// Get a single job by id.
    pub async fn get(&self, job_id: &str) -> Option<JobMeta> {
        self.jobs.read().await.iter().find(|j| j.job_id == job_id).cloned()
    }

    /// Get the log tail for a job.
    pub async fn get_log(&self, job_id: &str) -> Vec<JobLogEntry> {
        self.logs.read().await.get(job_id).cloned().unwrap_or_default()
    }

    /// Delete a job and its log.
    pub async fn delete(&self, job_id: &str) -> bool {
        let mut jobs = self.jobs.write().await;
        let before = jobs.len();
        jobs.retain(|j| j.job_id != job_id);
        let removed = jobs.len() < before;
        drop(jobs);
        if removed {
            self.logs.write().await.remove(job_id);
        }
        removed
    }
}

impl Default for JobStore {
    fn default() -> Self {
        Self::new()
    }
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_meta(kind: JobKind, id: &str) -> JobMeta {
        JobMeta {
            job_id: id.to_string(),
            kind,
            status: JobStatus::Queued,
            connection_id: "conn-1".into(),
            database: "test-db".into(),
            collections: vec!["col-a".into()],
            created_at: "2024-01-01T00:00:00+00:00".into(),
            started_at: None,
            finished_at: None,
            output_path: None,
            source_path: None,
            schedule: None,
            processed: 0,
            total: None,
            errors: 0,
            message: String::new(),
        }
    }

    #[tokio::test]
    async fn create_job_inserts_and_returns_id() {
        let store = JobStore::new();
        let meta = sample_meta(JobKind::Dump, "job-1");
        let id = store.create_job(meta.clone()).await;
        assert_eq!(id, "job-1");
        let jobs = store.list(JobFilter::default()).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "job-1");
    }

    #[tokio::test]
    async fn duplicate_job_id_replaces_existing() {
        let store = JobStore::new();
        let mut meta1 = sample_meta(JobKind::Dump, "job-1");
        meta1.database = "db-a".into();
        store.create_job(meta1).await;

        let mut meta2 = sample_meta(JobKind::Export, "job-1");
        meta2.database = "db-b".into();
        store.create_job(meta2).await;

        let jobs = store.list(JobFilter::default()).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].kind, JobKind::Export);
        assert_eq!(jobs[0].database, "db-b");
    }

    #[tokio::test]
    async fn list_filters_by_kind_and_status() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "d1")).await;
        let mut m2 = sample_meta(JobKind::Export, "e1");
        m2.status = JobStatus::Running;
        store.create_job(m2).await;

        let all = store.list(JobFilter::default()).await;
        assert_eq!(all.len(), 2);

        let dumps = store.list(JobFilter { kind: Some(JobKind::Dump), ..Default::default() }).await;
        assert_eq!(dumps.len(), 1);
        assert_eq!(dumps[0].job_id, "d1");

        let running = store.list(JobFilter { status: Some(JobStatus::Running), ..Default::default() }).await;
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].job_id, "e1");
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let store = JobStore::new();
        for i in 0..5 {
            let mut meta = sample_meta(JobKind::Dump, &format!("job-{i}"));
            meta.created_at = format!("2024-01-0{}T00:00:00+00:00", 5 - i);
            store.create_job(meta).await;
        }
        let jobs = store.list(JobFilter { limit: Some(2), ..Default::default() }).await;
        assert_eq!(jobs.len(), 2);
        // Newest first: job-0 was created last (2024-01-05) because 5-0=5
        assert_eq!(jobs[0].job_id, "job-0");
        assert_eq!(jobs[1].job_id, "job-1");
    }

    #[tokio::test]
    async fn update_status_sets_running_timestamp() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.update_status("job-1", JobStatus::Running, "Started".into()).await;
        let job = store.get("job-1").await.unwrap();
        assert_eq!(job.status, JobStatus::Running);
        assert_eq!(job.message, "Started");
        assert!(job.started_at.is_some());
        assert!(job.finished_at.is_none());
    }

    #[tokio::test]
    async fn update_status_sets_finished_timestamp_on_done() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.update_status("job-1", JobStatus::Done, "Finished".into()).await;
        let job = store.get("job-1").await.unwrap();
        assert!(job.finished_at.is_some());
    }

    #[tokio::test]
    async fn update_progress_updates_counters() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.update_progress("job-1", 100, Some(200), 3).await;
        let job = store.get("job-1").await.unwrap();
        assert_eq!(job.processed, 100);
        assert_eq!(job.total, Some(200));
        assert_eq!(job.errors, 3);
    }

    #[tokio::test]
    async fn update_progress_does_not_overwrite_total_with_none() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.update_progress("job-1", 50, Some(100), 0).await;
        store.update_progress("job-1", 60, None, 1).await;
        let job = store.get("job-1").await.unwrap();
        assert_eq!(job.processed, 60);
        assert_eq!(job.total, Some(100)); // should retain previous total
        assert_eq!(job.errors, 1);
    }

    #[tokio::test]
    async fn log_append_and_retrieve() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.log_info("job-1", "step 1").await;
        store.log_warn("job-1", "step 2").await;
        store.log_error("job-1", "step 3").await;

        let logs = store.get_log("job-1").await;
        assert_eq!(logs.len(), 3);
        assert!(matches!(logs[0].level, LogLevel::Info));
        assert!(matches!(logs[1].level, LogLevel::Warn));
        assert!(matches!(logs[2].level, LogLevel::Error));
    }

    #[tokio::test]
    async fn delete_removes_job_and_logs() {
        let store = JobStore::new();
        store.create_job(sample_meta(JobKind::Dump, "job-1")).await;
        store.log_info("job-1", "msg").await;
        assert!(store.delete("job-1").await);
        assert!(store.get("job-1").await.is_none());
        assert!(store.get_log("job-1").await.is_empty());
        assert!(!store.delete("job-1").await); // idempotent
    }

    #[tokio::test]
    async fn cancel_flag_registration_and_unregistration() {
        let store = JobStore::new();
        let flag = store.register("job-1".into()).await;
        assert!(!flag.load(Ordering::SeqCst));
        assert!(store.cancel("job-1").await);
        assert!(flag.load(Ordering::SeqCst));
        store.unregister("job-1").await;
        assert!(!store.cancel("job-1").await); // no longer registered
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_job() {
        let store = JobStore::new();
        assert!(store.get("missing").await.is_none());
    }

    #[tokio::test]
    async fn list_returns_empty_for_no_jobs() {
        let store = JobStore::new();
        let jobs = store.list(JobFilter::default()).await;
        assert!(jobs.is_empty());
    }
}
