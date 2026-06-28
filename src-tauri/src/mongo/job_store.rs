//! Unified job store for tracking dump, restore, export, and import jobs.
//!
//! Replaces the ephemeral `JobRegistry` with a store that keeps history,
//! per-job logs, and cancellation flags. All operations are async-safe
//! via `tokio::sync::RwLock`.

use std::collections::HashMap;
use std::path::PathBuf;
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
    #[serde(default)]
    /// When this job was spawned by the scheduler, the id of the schedule
    /// template it originated from. `None` for user-initiated jobs and for
    /// templates themselves.
    pub parent_job_id: Option<String>,
    pub processed: u64,
    pub total: Option<u64>,
    pub errors: u64,
    pub message: String,
    /// Opaque JSON blob storing the original request so the scheduler
    /// can reconstruct and re-run the job later.
    pub config_json: Option<String>,
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
            parent_job_id: None,
            processed: 0,
            total: None,
            errors: 0,
            message: String::new(),
            config_json: None,
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
    persist_path: Option<PathBuf>,
}

impl JobStore {
    pub fn new() -> Self {
        Self::with_path(None)
    }

    pub fn with_path(path: Option<PathBuf>) -> Self {
        let mut store = Self {
            active: RwLock::new(HashMap::new()),
            jobs: RwLock::new(Vec::new()),
            logs: RwLock::new(HashMap::new()),
            persist_path: path,
        };
        if let Some(path) = &store.persist_path {
            if let Ok(text) = std::fs::read_to_string(path) {
                let loaded: Vec<JobMeta> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
                store.jobs = RwLock::new(loaded);
            }
        }
        store
    }

    async fn save(&self) {
        if let Some(path) = &self.persist_path {
            let jobs = self.jobs.read().await;
            let lines: Vec<String> = jobs.iter().map(|j| serde_json::to_string(j).unwrap_or_default()).collect();
            let _ = std::fs::write(path, lines.join("\n"));
        }
    }

    /// Insert a new job meta record and return the job id.
    pub async fn create_job(&self, meta: JobMeta) -> String {
        let id = meta.job_id.clone();
        let mut jobs = self.jobs.write().await;
        let existing = jobs.iter().find(|j| j.job_id == id).cloned();
        let mut meta = meta;
        if let Some(existing) = existing {
            // Core runners create fresh JobMeta records from request payloads.
            // Preserve scheduler/template fields that live outside those
            // requests when a record is re-registered with the same id.
            if meta.schedule.is_none() {
                meta.schedule = existing.schedule;
            }
            if meta.parent_job_id.is_none() {
                meta.parent_job_id = existing.parent_job_id;
            }
        }
        // If a job with the same id already exists, replace it.
        jobs.retain(|j| j.job_id != id);
        jobs.push(meta);
        drop(jobs);
        self.save().await;
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
        drop(jobs);
        self.save().await;
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
        drop(jobs);
        self.save().await;
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
            self.save().await;
        }
        removed
    }

    /// Find jobs with an enabled schedule whose next_run_at is in the past.
    pub async fn due_jobs(&self) -> Vec<JobMeta> {
        let now = chrono::Local::now();
        let jobs = self.jobs.read().await;
        jobs.iter()
            .filter(|j| {
                // Only schedule templates (never the runs they spawn) are due,
                // and only when not currently executing. A failed/cancelled
                // template still fires on its next occurrence so a transient
                // error doesn't silently disable the schedule.
                j.parent_job_id.is_none()
                    && j.status != JobStatus::Running
                    && j.status != JobStatus::Queued
                    && j.schedule.as_ref().map_or(false, |s| {
                        s.enabled
                            && s.next_run_at.as_ref().map_or(false, |t| {
                                chrono::DateTime::parse_from_rfc3339(t)
                                    .map_or(false, |dt| dt.with_timezone(&chrono::Local) <= now)
                            })
                    })
            })
            .cloned()
            .collect()
    }

    /// Update the next_run_at field for a scheduled job.
    pub async fn update_next_run(&self, job_id: &str, next_run_at: Option<String>) {
        let mut jobs = self.jobs.write().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.job_id == job_id) {
            if let Some(ref mut schedule) = job.schedule {
                schedule.next_run_at = next_run_at;
            }
        }
        drop(jobs);
        self.save().await;
    }

    /// Remove old finished runs spawned by a schedule template, keeping only
    /// the most recent `retention_count`. The template itself is never removed.
    /// Returns the number of deleted runs.
    pub async fn cleanup_retention(&self, template_job_id: &str, retention_count: usize) -> usize {
        let mut jobs = self.jobs.write().await;
        // Finished runs that originated from this template, newest-first.
        let mut runs: Vec<(String, String)> = jobs
            .iter()
            .filter(|j| {
                j.parent_job_id.as_deref() == Some(template_job_id)
                    && j.status != JobStatus::Running
                    && j.status != JobStatus::Queued
            })
            .map(|j| (j.job_id.clone(), j.created_at.clone()))
            .collect();
        runs.sort_by(|a, b| b.1.cmp(&a.1));

        let to_remove: Vec<String> = runs
            .iter()
            .skip(retention_count)
            .map(|(id, _)| id.clone())
            .collect();

        let before = jobs.len();
        jobs.retain(|j| !to_remove.contains(&j.job_id));
        let removed = before - jobs.len();
        drop(jobs);
        if removed > 0 {
            let mut logs = self.logs.write().await;
            for id in &to_remove {
                logs.remove(id);
            }
            self.save().await;
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
            parent_job_id: None,
            processed: 0,
            total: None,
            errors: 0,
            message: String::new(),
            config_json: None,
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

    #[tokio::test]
    async fn persistence_round_trip_saves_and_restores_jobs() {
        let path = std::env::temp_dir().join(format!("mongo-buddy-job-store-test-{}", uuid::Uuid::new_v4()));
        {
            let store = JobStore::with_path(Some(path.clone()));
            let mut meta = sample_meta(JobKind::Dump, "persist-job-1");
            meta.status = JobStatus::Done;
            meta.message = "all good".into();
            meta.processed = 42;
            store.create_job(meta).await;
            store.update_status("persist-job-1", JobStatus::Failed, "boom".into()).await;
        }

        // Re-open and verify
        let store2 = JobStore::with_path(Some(path.clone()));
        let jobs = store2.list(JobFilter::default()).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "persist-job-1");
        assert_eq!(jobs[0].status, JobStatus::Failed);
        assert_eq!(jobs[0].message, "boom");
        assert_eq!(jobs[0].processed, 42);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn persistence_deletes_are_flushed() {
        let path = std::env::temp_dir().join(format!("mongo-buddy-job-store-delete-test-{}", uuid::Uuid::new_v4()));
        {
            let store = JobStore::with_path(Some(path.clone()));
            store.create_job(sample_meta(JobKind::Dump, "del-1")).await;
            store.create_job(sample_meta(JobKind::Export, "del-2")).await;
            store.delete("del-1").await;
        }

        let store2 = JobStore::with_path(Some(path.clone()));
        let jobs = store2.list(JobFilter::default()).await;
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "del-2");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn due_jobs_returns_only_enabled_past_next_run() {
        let store = JobStore::new();
        let past = chrono::Local::now() - chrono::Duration::hours(1);
        let future = chrono::Local::now() + chrono::Duration::hours(1);

        // Job A: done, enabled, next_run in past → due
        let mut a = sample_meta(JobKind::Dump, "due-a");
        a.status = JobStatus::Done;
        a.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(a).await;

        // Job B: done, enabled, next_run in future → not due
        let mut b = sample_meta(JobKind::Dump, "due-b");
        b.status = JobStatus::Done;
        b.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(future.to_rfc3339()),
        });
        store.create_job(b).await;

        // Job C: running, enabled, next_run in past → not due (still running)
        let mut c = sample_meta(JobKind::Dump, "due-c");
        c.status = JobStatus::Running;
        c.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(c).await;

        // Job D: done, disabled, next_run in past → not due
        let mut d = sample_meta(JobKind::Dump, "due-d");
        d.status = JobStatus::Done;
        d.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: false,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(d).await;

        let due = store.due_jobs().await;
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].job_id, "due-a");
    }

    #[tokio::test]
    async fn update_next_run_sets_next_run_at() {
        let store = JobStore::new();
        let mut meta = sample_meta(JobKind::Dump, "next-run");
        meta.status = JobStatus::Done;
        meta.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some("2024-01-01T00:00:00+00:00".into()),
        });
        store.create_job(meta).await;

        let new_time = "2025-06-15T10:00:00+00:00".to_string();
        store.update_next_run("next-run", Some(new_time.clone())).await;

        let job = store.get("next-run").await.unwrap();
        assert_eq!(job.schedule.as_ref().unwrap().next_run_at, Some(new_time));
    }

    #[tokio::test]
    async fn cleanup_retention_keeps_only_n_most_recent() {
        let store = JobStore::new();
        // The template that owns the schedule.
        let mut template = sample_meta(JobKind::Dump, "tmpl-1");
        template.status = JobStatus::Done;
        template.schedule = Some(ScheduleConfig {
            cron: "0 2 * * *".into(),
            enabled: true,
            retention_count: Some(2),
            next_run_at: None,
        });
        store.create_job(template).await;
        // Five generated runs (children) of that template.
        for i in 0..5 {
            let mut meta = sample_meta(JobKind::Dump, &format!("ret-{i}"));
            meta.status = JobStatus::Done;
            meta.created_at = format!("2024-01-0{}T00:00:00+00:00", 5 - i); // newest first
            meta.parent_job_id = Some("tmpl-1".into());
            store.create_job(meta).await;
        }

        let deleted = store.cleanup_retention("tmpl-1", 2).await;
        assert_eq!(deleted, 3);

        let remaining = store.list(JobFilter::default()).await;
        // Two most-recent runs survive, plus the template itself.
        assert_eq!(remaining.len(), 3);
        assert!(remaining.iter().any(|j| j.job_id == "ret-0"));
        assert!(remaining.iter().any(|j| j.job_id == "ret-1"));
        assert!(remaining.iter().any(|j| j.job_id == "tmpl-1"));
    }
}
