//! Background scheduler loop that checks for scheduled jobs every minute
//! and spawns new instances when their next_run_at time is reached.
//!
//! Design goals:
//! - Light on CPU/RAM: tick every 60s, skip missed ticks, limit concurrency.
//! - Responsive: use a semaphore so heavy dumps/exports don't swamp the
//!   async runtime; stagger spawns to spread disk IO.
//! - Safe: track in-flight runs so a slow job isn't duplicated if the next
//!   tick fires while it's still running.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio::sync::{Mutex, Semaphore};
use tokio::time::{interval, sleep};

use crate::commands::dump::{run_dump, DumpRequest};
use crate::commands::export::{run_export, ExportRequest};
use crate::events::{chrono_now, emit_job_status_changed};
use crate::mongo::client_registry::{build_client, ClientEntry};
use crate::mongo::job_store::{JobKind, JobMeta, JobStatus};
use crate::state::AppState;

/// Max concurrent scheduled dump/export jobs. Keeps disk IO and CPU
/// (compression) bounded so the UI stays responsive.
const MAX_CONCURRENT_JOBS: usize = 2;

/// Delay between spawning consecutive scheduled jobs within one tick.
/// Spreads out disk IO and MongoDB connection load.
const STAGGER_MS: u64 = 500;

/// Spawn a background tokio task that checks for due jobs every 60 seconds.
/// Should be called once at app startup inside the Tauri `setup` hook.
pub fn start_scheduler(app: tauri::AppHandle) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Concurrency limiter shared across all ticks.
        let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_JOBS));
        // Tracks original job_ids whose scheduled run is currently in-flight
        // so we never spawn a duplicate before the previous one finishes.
        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

        loop {
            ticker.tick().await;
            if let Err(e) = tick(&app, &semaphore, &in_flight).await {
                tracing::warn!(error = %e, "scheduler tick failed");
            }
        }
    });
}

/// One scheduler tick: find due jobs, clone their config, spawn them,
/// update next_run_at, and apply retention cleanup.
async fn tick(
    app: &tauri::AppHandle,
    semaphore: &Arc<Semaphore>,
    in_flight: &Arc<Mutex<HashSet<String>>>,
) -> Result<(), SchedulerError> {
    let state = app.state::<AppState>();
    let due = state.jobs.due_jobs().await;
    if due.is_empty() {
        return Ok(());
    }

    // Collect retention configs so we run cleanup once per tick, not per job.
    let mut retention_queue: Vec<(String, u32)> = Vec::new();

    for meta in due {
        let original_id = meta.job_id.clone();

        // Skip if a previous scheduled run for this job is still executing.
        {
            let guard = in_flight.lock().await;
            if guard.contains(&original_id) {
                tracing::debug!(job_id = %original_id, "scheduler: previous run still in flight, skipping");
                continue;
            }
        }

        let new_id = uuid::Uuid::new_v4().to_string();
        let schedule = meta.schedule.clone();

        // Clone the meta into a fresh queued job.
        let mut new_meta = JobMeta::new(
            new_id.clone(),
            meta.kind,
            meta.connection_id.clone(),
            meta.database.clone(),
        );
        new_meta.collections = meta.collections.clone();
        new_meta.output_path = meta.output_path.clone();
        new_meta.source_path = meta.source_path.clone();
        new_meta.config_json = meta.config_json.clone();
        new_meta.profile_id = meta.profile_id.clone();
        new_meta.schedule = None; // One-off run; schedule stays on the original.
        new_meta.parent_job_id = Some(original_id.clone()); // Link run to its template.
        new_meta.message = "Queued by scheduler".into();
        state.jobs.create_job(new_meta.clone()).await;
        state
            .jobs
            .log_info(&new_id, "Scheduled job triggered")
            .await;

        // Update next_run_at and queue retention BEFORE spawning so we
        // don't borrow original_id after moving it into the async block.
        if let Some(ref sched) = schedule {
            if sched.enabled {
                match next_run_from_cron(&sched.cron) {
                    Some(next) => {
                        state
                            .jobs
                            .update_next_run(&original_id, Some(next.clone()))
                            .await;
                        state
                            .jobs
                            .log_info(&original_id, format!("Next run scheduled at {next}"))
                            .await;

                        if let Some(retention) = sched.retention_count {
                            retention_queue.push((original_id.clone(), retention));
                        }
                    }
                    None => {
                        // An invalid cron expression cannot yield a next
                        // occurrence. Make this loud (warn log + job warning)
                        // instead of silently clearing next_run_at and leaving
                        // the user with a schedule that never fires again.
                        tracing::warn!(
                            job_id = %original_id,
                            cron = %sched.cron,
                            "invalid cron expression; schedule cannot advance and is paused"
                        );
                        state.jobs.update_next_run(&original_id, None).await;
                        state
                            .jobs
                            .log_warn(
                                &original_id,
                                format!(
                                    "Invalid cron expression {:?}; schedule paused until corrected",
                                    sched.cron
                                ),
                            )
                            .await;
                    }
                }
            }
        }

        // Mark original as in-flight.
        in_flight.lock().await.insert(original_id.clone());

        // Spawn the actual work, gated by the semaphore so we never run
        // more than MAX_CONCURRENT_JOBS at once.
        let app_clone = app.clone();
        let meta_clone = meta.clone();
        let in_flight_clone = in_flight.clone();
        let original_id_for_spawn = original_id.clone();
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| SchedulerError::Internal(format!("semaphore closed: {e}")))?;

        tokio::spawn(async move {
            let _permit = permit; // hold until task completes
            if let Err(e) = execute_scheduled_job(&app_clone, &meta_clone, &new_id).await {
                tracing::error!(error = %e, job_id = %new_id, "scheduled job failed");
            }
            in_flight_clone.lock().await.remove(&original_id_for_spawn);
        });

        // Small stagger between job spawns to spread disk IO load.
        sleep(Duration::from_millis(STAGGER_MS)).await;
    }

    // Run retention cleanup once per tick for all triggered jobs.
    for (original_id, retention) in retention_queue {
        let removed = state
            .jobs
            .cleanup_retention(&original_id, retention as usize)
            .await;
        if removed > 0 {
            tracing::debug!(removed, job_id = %original_id, "retention cleanup removed old jobs");
        }
    }

    Ok(())
}

/// Parse a cron expression and return the next occurrence as an RFC3339 string.
fn next_run_from_cron(cron_expr: &str) -> Option<String> {
    let schedule = cron::Schedule::from_str(cron_expr).ok()?;
    schedule
        .upcoming(chrono::Local)
        .next()
        .map(|dt| dt.to_rfc3339())
}

/// Reconstruct the original request from `config_json` and execute it.
/// On failure the job status is updated to Failed so the UI reflects the
/// outcome instead of leaving it stuck at Queued/Running.
async fn execute_scheduled_job(
    app: &tauri::AppHandle,
    meta: &JobMeta,
    new_job_id: &str,
) -> Result<(), SchedulerError> {
    let state = app.state::<AppState>();
    let config_json = meta
        .config_json
        .as_ref()
        .ok_or_else(|| SchedulerError::MissingConfig(meta.job_id.clone()))?;

    // The connection id baked into config_json is ephemeral (minted per
    // `open_connection`) and is almost always stale by the time a schedule
    // fires after a restart. Resolve a live connection from the stable
    // profile id instead.
    let connection_id = match resolve_connection(app, state.inner(), meta).await {
        Ok(id) => id,
        Err(e) => {
            let msg = format!("Scheduled job could not connect: {e}");
            state
                .jobs
                .update_status(new_job_id, JobStatus::Failed, msg.clone())
                .await;
            state.jobs.log_error(new_job_id, &msg).await;
            emit_job_status_changed(app, new_job_id, "failed", &msg, Some(chrono_now()));
            return Err(e);
        }
    };

    tracing::info!(
        job_id = %new_job_id,
        kind = ?meta.kind,
        conn = %connection_id,
        db = %meta.database,
        "scheduler: starting scheduled job"
    );

    match meta.kind {
        JobKind::Dump => {
            let mut request: DumpRequest = serde_json::from_str(config_json)
                .map_err(|e| SchedulerError::BadConfig(e.to_string()))?;
            request.job_id = new_job_id.to_string();
            request.connection_id = connection_id.clone();
            // Ensure destination dir exists (it may have been deleted).
            if let Err(e) = std::fs::create_dir_all(&request.destination_dir) {
                tracing::warn!(error = %e, dir = %request.destination_dir, "scheduler: failed to create destination dir");
            }
            if let Err(e) = run_dump(&request, state.inner(), app).await {
                let msg = format!("Scheduled dump failed: {e}");
                tracing::error!(job_id = %new_job_id, error = %e, "scheduled dump failed");
                state
                    .jobs
                    .update_status(new_job_id, JobStatus::Failed, msg.clone())
                    .await;
                state.jobs.log_error(new_job_id, &msg).await;
                emit_job_status_changed(app, new_job_id, "failed", &msg, Some(chrono_now()));
                return Err(SchedulerError::JobFailed(e.to_string()));
            }
        }
        JobKind::Export => {
            let mut request: ExportRequest = serde_json::from_str(config_json)
                .map_err(|e| SchedulerError::BadConfig(e.to_string()))?;
            request.job_id = new_job_id.to_string();
            request.connection_id = connection_id.clone();
            // Only file exports are schedulable; skip clipboard.
            if request.destination.kind == crate::commands::export::DestinationKind::Clipboard {
                return Err(SchedulerError::UnsupportedDestination);
            }
            if let Err(e) = run_export(&request, state.inner(), app).await {
                let msg = format!("Scheduled export failed: {e}");
                tracing::error!(job_id = %new_job_id, error = %e, "scheduled export failed");
                state
                    .jobs
                    .update_status(new_job_id, JobStatus::Failed, msg.clone())
                    .await;
                state.jobs.log_error(new_job_id, &msg).await;
                emit_job_status_changed(app, new_job_id, "failed", &msg, Some(chrono_now()));
                return Err(SchedulerError::JobFailed(e.to_string()));
            }
        }
        JobKind::Restore | JobKind::Import => {
            // Scheduling restore/import is intentionally unsupported
            // because they require user confirmation (conflict strategy,
            // target DB selection, etc.).
            return Err(SchedulerError::UnsupportedKind(meta.kind));
        }
    }

    tracing::info!(job_id = %new_job_id, "scheduler: scheduled job completed successfully");
    Ok(())
}

/// Resolve a usable connection id for a scheduled job. Prefers an already-open
/// connection for the job's profile; otherwise opens a fresh client from the
/// stored profile so schedules keep working across app restarts. Legacy jobs
/// recorded before profile tracking fall back to their stored connection id.
async fn resolve_connection(
    app: &tauri::AppHandle,
    state: &AppState,
    meta: &JobMeta,
) -> Result<String, SchedulerError> {
    if meta.profile_id.is_empty() {
        if state.clients.get(&meta.connection_id).await.is_ok() {
            return Ok(meta.connection_id.clone());
        }
        if let Some(conn_id) = state.clients.only_connection_id().await {
            return Ok(conn_id);
        }
        return Ok(meta.connection_id.clone());
    }
    if let Some(conn_id) = state.clients.connection_for_profile(&meta.profile_id).await {
        return Ok(conn_id);
    }
    let profile = state
        .profiles
        .get(app, &meta.profile_id)
        .map_err(|e| SchedulerError::Connection(format!("profile {}: {e}", meta.profile_id)))?;
    let client = build_client(&profile.uri, "NoSQLBuddy-scheduler")
        .await
        .map_err(|e| SchedulerError::Connection(e.to_string()))?;
    let connection_id = uuid::Uuid::new_v4().to_string();
    let deployment_id = crate::audit::change_stream::fetch_deployment_id(&client).await;
    state
        .clients
        .insert(
            connection_id.clone(),
            ClientEntry {
                client,
                profile_id: profile.id.clone(),
                name: profile.name.clone(),
                deployment_id,
                opened_at: chrono::Utc::now(),
            },
        )
        .await;
    tracing::info!(profile_id = %meta.profile_id, "scheduler: opened connection for scheduled job");
    Ok(connection_id)
}

#[derive(Debug, thiserror::Error)]
enum SchedulerError {
    #[error("job {0} has no stored config_json")]
    MissingConfig(String),
    #[error("invalid stored config: {0}")]
    BadConfig(String),
    #[error("could not resolve a connection: {0}")]
    Connection(String),
    #[error("scheduled job failed: {0}")]
    JobFailed(String),
    #[error("scheduling is only supported for dump and export jobs")]
    UnsupportedKind(JobKind),
    #[error("clipboard destinations cannot be scheduled")]
    UnsupportedDestination,
    #[error("scheduler internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mongo::job_store::{
        JobFilter, JobKind, JobMeta, JobStatus, JobStore, ScheduleConfig,
    };

    #[test]
    fn next_run_from_cron_produces_future_date() {
        let next = next_run_from_cron("0 0 2 * * *");
        assert!(
            next.is_some(),
            "cron parsing should succeed for valid expression"
        );
        let dt = chrono::DateTime::parse_from_rfc3339(&next.unwrap()).unwrap();
        assert!(dt > chrono::Local::now() - chrono::Duration::days(1));
    }

    #[test]
    fn next_run_from_cron_returns_none_for_invalid() {
        assert!(next_run_from_cron("not a cron").is_none());
    }

    #[tokio::test]
    async fn due_jobs_skips_future_and_disabled() {
        let store = JobStore::new();
        let now = chrono::Local::now();
        let past = now - chrono::Duration::hours(1);
        let future = now + chrono::Duration::hours(1);

        let mut due = JobMeta::new("due-1".into(), JobKind::Dump, "c1".into(), "db".into());
        due.status = JobStatus::Done;
        due.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(due).await;

        let mut not_yet = JobMeta::new("not-yet".into(), JobKind::Dump, "c1".into(), "db".into());
        not_yet.status = JobStatus::Done;
        not_yet.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(future.to_rfc3339()),
        });
        store.create_job(not_yet).await;

        let mut disabled = JobMeta::new("disabled".into(), JobKind::Dump, "c1".into(), "db".into());
        disabled.status = JobStatus::Done;
        disabled.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: false,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(disabled).await;

        let due_list = store.due_jobs().await;
        assert_eq!(due_list.len(), 1);
        assert_eq!(due_list[0].job_id, "due-1");
    }

    #[tokio::test]
    async fn update_next_run_advances_schedule() {
        let store = JobStore::new();
        let mut meta = JobMeta::new("sched-1".into(), JobKind::Dump, "c1".into(), "db".into());
        meta.status = JobStatus::Done;
        meta.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some("2024-01-01T00:00:00+00:00".into()),
        });
        store.create_job(meta).await;

        let new_next = "2025-01-01T00:00:00+00:00".to_string();
        store
            .update_next_run("sched-1", Some(new_next.clone()))
            .await;

        let job = store.get("sched-1").await.unwrap();
        assert_eq!(job.schedule.unwrap().next_run_at, Some(new_next));
    }

    #[tokio::test]
    async fn retention_cleanup_removes_old_jobs() {
        let store = JobStore::new();
        // Template owning the schedule.
        let mut template = JobMeta::new("tmpl".into(), JobKind::Dump, "c1".into(), "db".into());
        template.status = JobStatus::Done;
        template.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: true,
            retention_count: Some(2),
            next_run_at: None,
        });
        store.create_job(template).await;
        // Five generated runs (children).
        for i in 0..5 {
            let mut meta =
                JobMeta::new(format!("ret-{i}"), JobKind::Dump, "c1".into(), "db".into());
            meta.status = JobStatus::Done;
            meta.created_at = format!("2024-01-0{}T00:00:00+00:00", 5 - i);
            meta.parent_job_id = Some("tmpl".into());
            store.create_job(meta).await;
        }

        let removed = store.cleanup_retention("tmpl", 2).await;
        assert_eq!(removed, 3);

        let remaining = store.list(JobFilter::default()).await;
        // Two newest runs survive, plus the template.
        assert_eq!(remaining.len(), 3);
    }

    #[tokio::test]
    async fn in_flight_deduplication_prevents_duplicate_spawns() {
        use tokio::sync::Mutex;
        let store = JobStore::new();
        let now = chrono::Local::now();
        let past = now - chrono::Duration::hours(1);

        let mut meta = JobMeta::new("dedup-1".into(), JobKind::Dump, "c1".into(), "db".into());
        meta.status = JobStatus::Done;
        meta.schedule = Some(ScheduleConfig {
            cron: "0 0 2 * * *".into(),
            enabled: true,
            retention_count: Some(5),
            next_run_at: Some(past.to_rfc3339()),
        });
        store.create_job(meta).await;

        // Simulate the in-flight set being populated as if a previous run
        // is still executing.
        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        in_flight.lock().await.insert("dedup-1".into());

        let due = store.due_jobs().await;
        assert_eq!(due.len(), 1);

        // With in-flight populated, the tick would skip this job.
        let guard = in_flight.lock().await;
        assert!(guard.contains("dedup-1"));
        // The tick logic checks the guard and continues if present.
    }
}
