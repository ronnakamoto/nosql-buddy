//! Core traits and pipeline runner for streaming import/export.

use async_trait::async_trait;
use bson::Document;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tauri::AppHandle;

use crate::error::{AppError, AppResult};
use crate::events::{emit_import_export_progress, ImportExportProgressPayload};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowError {
    pub row: Option<u64>,
    pub message: String,
}

#[derive(Debug)]
pub enum RowResult {
    Doc(Document),
    Skipped,
    Error(RowError),
}

#[async_trait]
pub trait DocumentSource: Send {
    /// Total count if cheaply known (e.g. from `collStats` for a whole collection
    /// export, or `None` if it's a complex query or unknown CSV size).
    fn size_hint(&self) -> Option<u64>;

    /// Pull the next row. Returns `Ok(None)` on EOF.
    async fn next_doc(&mut self) -> AppResult<Option<RowResult>>;
}

pub trait Transform: Send {
    /// Apply a transformation. Returning `Ok(None)` drops the document.
    fn apply(&self, doc: Document) -> AppResult<Option<Document>>;
}

#[async_trait]
pub trait DocumentSink: Send {
    /// Called before the first document is written. Useful for CSV headers
    /// or JSON array opening brackets.
    async fn start(&mut self) -> AppResult<()>;

    /// Write one document to the sink.
    async fn write(&mut self, doc: Document) -> AppResult<()>;

    /// Called on successful completion. Must be infallible once it succeeds
    /// (e.g. fsync, close array brackets, atomic rename of a .part file).
    async fn finish(self: Box<Self>) -> AppResult<()>;

    /// Called if the pipeline is cancelled or errors out. Cleans up temp
    /// resources (e.g. deletes the `.part` file so we don't leave garbage).
    async fn abort(self: Box<Self>) -> AppResult<()>;
}

pub struct JobContext {
    pub job_id: String,
    pub cancel_flag: Arc<AtomicBool>,
    /// Optional so the pipeline can be unit-tested without a Tauri runtime.
    pub app_handle: Option<AppHandle>,
    pub progress_observer: Option<ProgressObserver>,
    /// Emit progress at most every N milliseconds.
    pub throttle_ms: u64,
    /// Abort the entire job if error count exceeds this.
    pub max_errors: u64,
    /// Keep at most this many row-level errors in the final report.
    pub max_error_samples: usize,
}

pub type ProgressObserver = Arc<dyn Fn(ImportExportProgressPayload) + Send + Sync>;

impl JobContext {
    fn emit(&self, phase: &str, processed: u64, total: Option<u64>, message: String) {
        let payload = ImportExportProgressPayload {
            job_id: self.job_id.clone(),
            phase: phase.to_string(),
            processed,
            total,
            message,
        };
        if let Some(app) = &self.app_handle {
            emit_import_export_progress(app, payload.clone());
        }
        if let Some(observer) = &self.progress_observer {
            observer(payload);
        }
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobReport {
    pub job_id: String,
    pub processed: u64,
    pub errors: u64,
    pub cancelled: bool,
    pub row_errors: Vec<RowError>,
}

/// Run a streaming pipeline. Drives the source, applies transforms, and feeds
/// the sink while throttling progress updates and honoring cancellation.
pub async fn run_pipeline(
    mut src: Box<dyn DocumentSource>,
    transforms: Vec<Box<dyn Transform>>,
    mut sink: Box<dyn DocumentSink>,
    ctx: JobContext,
) -> AppResult<JobReport> {
    sink.start().await?;

    let mut processed: u64 = 0;
    let mut errors: u64 = 0;
    let mut row_errors: Vec<RowError> = Vec::new();
    let mut last_progress = Instant::now();
    let size_hint = src.size_hint();
    let mut cancelled = false;

    while !ctx.cancel_flag.load(Ordering::Relaxed) {
        let row = match src.next_doc().await {
            Ok(Some(r)) => r,
            Ok(None) => break, // EOF
            Err(e) => {
                sink.abort().await?;
                return Err(e);
            }
        };

        match row {
            RowResult::Doc(doc) => {
                let mut current = Some(doc);
                for t in &transforms {
                    let Some(d) = current.take() else { break };
                    match t.apply(d) {
                        Ok(next) => current = next, // None drops the doc
                        Err(e) => {
                            errors += 1;
                            if row_errors.len() < ctx.max_error_samples {
                                row_errors.push(RowError {
                                    row: Some(processed + 1),
                                    message: e.to_string(),
                                });
                            }
                            current = None;
                            break;
                        }
                    }
                }

                if let Some(doc) = current {
                    if let Err(e) = sink.write(doc).await {
                        sink.abort().await?;
                        return Err(e);
                    }
                }
            }
            RowResult::Skipped => {
                // Do nothing, just iterate
            }
            RowResult::Error(err) => {
                errors += 1;
                if row_errors.len() < ctx.max_error_samples {
                    row_errors.push(err);
                }
            }
        }

        processed += 1;
        if errors > ctx.max_errors {
            sink.abort().await?;
            return Err(AppError::Validation(format!(
                "Job aborted: error count exceeded maximum of {}",
                ctx.max_errors
            )));
        }

        let now = Instant::now();
        if now.duration_since(last_progress).as_millis() as u64 >= ctx.throttle_ms {
            ctx.emit(
                "processing",
                processed,
                size_hint,
                format!("Processed {}", processed),
            );
            last_progress = now;
        }
    }

    if ctx.cancel_flag.load(Ordering::Relaxed) {
        cancelled = true;
        sink.abort().await?;
        ctx.emit("cancelled", processed, size_hint, "Cancelled".into());
    } else {
        sink.finish().await?;
        ctx.emit("done", processed, size_hint, "Completed".into());
    }

    Ok(JobReport {
        job_id: ctx.job_id,
        processed,
        errors,
        cancelled,
        row_errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::events::ImportExportProgressPayload;
    use crate::mongo::import_export::source_mem::VecSource;
    use async_trait::async_trait;
    use bson::{doc, Document};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Mutex;

    #[derive(Default)]
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

    fn ctx(job_id: &str, cancel_flag: Arc<AtomicBool>) -> JobContext {
        JobContext {
            job_id: job_id.to_string(),
            cancel_flag,
            app_handle: None,
            progress_observer: None,
            throttle_ms: 0,
            max_errors: 100,
            max_error_samples: 100,
        }
    }

    fn ctx_with_events(
        job_id: &str,
        cancel_flag: Arc<AtomicBool>,
        throttle_ms: u64,
    ) -> (JobContext, Arc<Mutex<Vec<ImportExportProgressPayload>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        (
            JobContext {
                job_id: job_id.to_string(),
                cancel_flag,
                app_handle: None,
                progress_observer: Some(Arc::new(move |payload| {
                    observed.lock().unwrap().push(payload);
                })),
                throttle_ms,
                max_errors: 100,
                max_error_samples: 100,
            },
            events,
        )
    }

    #[tokio::test]
    async fn pipeline_streams_all_documents() {
        let docs = vec![doc! { "n": 1 }, doc! { "n": 2 }, doc! { "n": 3 }];
        let collected = Arc::new(Mutex::new(Vec::new()));
        let finished = Arc::new(AtomicBool::new(false));
        let sink = CollectSink {
            docs: collected.clone(),
            finished: finished.clone(),
        };
        let report = run_pipeline(
            Box::new(VecSource::new(docs)),
            Vec::new(),
            Box::new(sink),
            ctx("t1", Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap();

        assert_eq!(report.processed, 3);
        assert_eq!(report.errors, 0);
        assert!(!report.cancelled);
        assert!(finished.load(Ordering::Relaxed));
        assert_eq!(collected.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn pipeline_respects_cancellation() {
        let docs: Vec<Document> = (0..100).map(|n| doc! { "n": n }).collect();
        let flag = Arc::new(AtomicBool::new(true)); // pre-cancelled
        let collected = Arc::new(Mutex::new(Vec::new()));
        let sink = CollectSink {
            docs: collected.clone(),
            finished: Arc::new(AtomicBool::new(false)),
        };
        let report = run_pipeline(
            Box::new(VecSource::new(docs)),
            Vec::new(),
            Box::new(sink),
            ctx("t2", flag),
        )
        .await
        .unwrap();

        assert!(report.cancelled);
        assert_eq!(collected.lock().unwrap().len(), 0);
    }

    #[derive(Default)]
    struct StreamAccounting {
        produced: AtomicU64,
        written: AtomicU64,
        max_gap: AtomicU64,
    }

    struct CountingSource {
        remaining: u64,
        state: Arc<StreamAccounting>,
    }

    #[async_trait]
    impl DocumentSource for CountingSource {
        fn size_hint(&self) -> Option<u64> {
            Some(self.remaining)
        }

        async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
            if self.remaining == 0 {
                return Ok(None);
            }
            self.remaining -= 1;
            let produced = self.state.produced.fetch_add(1, Ordering::Relaxed) + 1;
            let written = self.state.written.load(Ordering::Relaxed);
            self.state
                .max_gap
                .fetch_max(produced - written, Ordering::Relaxed);
            Ok(Some(RowResult::Doc(doc! { "n": produced as i64 })))
        }
    }

    struct TrackingSink {
        state: Arc<StreamAccounting>,
    }

    #[async_trait]
    impl DocumentSink for TrackingSink {
        async fn start(&mut self) -> AppResult<()> {
            Ok(())
        }

        async fn write(&mut self, _doc: Document) -> AppResult<()> {
            self.state.written.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn finish(self: Box<Self>) -> AppResult<()> {
            Ok(())
        }

        async fn abort(self: Box<Self>) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn pipeline_keeps_only_one_document_in_flight() {
        let state = Arc::new(StreamAccounting::default());
        let total = 20_000;
        let report = run_pipeline(
            Box::new(CountingSource {
                remaining: total,
                state: state.clone(),
            }),
            Vec::new(),
            Box::new(TrackingSink {
                state: state.clone(),
            }),
            ctx("streaming", Arc::new(AtomicBool::new(false))),
        )
        .await
        .unwrap();

        assert_eq!(report.processed, total);
        assert_eq!(state.written.load(Ordering::Relaxed), total);
        assert!(state.max_gap.load(Ordering::Relaxed) <= 1);
    }

    #[tokio::test]
    async fn pipeline_delivers_progress_payloads() {
        let docs = vec![doc! { "n": 1 }, doc! { "n": 2 }];
        let (ctx, events) = ctx_with_events("progress", Arc::new(AtomicBool::new(false)), 0);

        run_pipeline(
            Box::new(VecSource::new(docs)),
            Vec::new(),
            Box::new(CollectSink::default()),
            ctx,
        )
        .await
        .unwrap();

        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| {
            event.job_id == "progress"
                && event.phase == "processing"
                && event.processed == 1
                && event.total == Some(2)
                && event.message == "Processed 1"
        }));
        let done = events.last().unwrap();
        assert_eq!(done.phase, "done");
        assert_eq!(done.processed, 2);
        assert_eq!(done.total, Some(2));
    }

    #[tokio::test]
    async fn pipeline_throttles_intermediate_progress() {
        let docs = vec![doc! { "n": 1 }, doc! { "n": 2 }];
        let (ctx, events) = ctx_with_events("throttled", Arc::new(AtomicBool::new(false)), 60_000);

        run_pipeline(
            Box::new(VecSource::new(docs)),
            Vec::new(),
            Box::new(CollectSink::default()),
            ctx,
        )
        .await
        .unwrap();

        let events = events.lock().unwrap();
        assert!(!events.iter().any(|event| event.phase == "processing"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].phase, "done");
    }

    struct ErrorSource {
        remaining: u64,
    }

    #[async_trait]
    impl DocumentSource for ErrorSource {
        fn size_hint(&self) -> Option<u64> {
            Some(self.remaining)
        }

        async fn next_doc(&mut self) -> AppResult<Option<RowResult>> {
            if self.remaining == 0 {
                return Ok(None);
            }
            self.remaining -= 1;
            Ok(Some(RowResult::Error(RowError {
                row: Some(3 - self.remaining),
                message: "bad row".into(),
            })))
        }
    }

    struct AbortTrackingSink {
        aborted: Arc<AtomicBool>,
        finished: Arc<AtomicBool>,
    }

    #[async_trait]
    impl DocumentSink for AbortTrackingSink {
        async fn start(&mut self) -> AppResult<()> {
            Ok(())
        }

        async fn write(&mut self, _doc: Document) -> AppResult<()> {
            Ok(())
        }

        async fn finish(self: Box<Self>) -> AppResult<()> {
            self.finished.store(true, Ordering::Relaxed);
            Ok(())
        }

        async fn abort(self: Box<Self>) -> AppResult<()> {
            self.aborted.store(true, Ordering::Relaxed);
            Ok(())
        }
    }

    #[tokio::test]
    async fn pipeline_aborts_when_error_cap_is_exceeded() {
        let aborted = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let err = run_pipeline(
            Box::new(ErrorSource { remaining: 3 }),
            Vec::new(),
            Box::new(AbortTrackingSink {
                aborted: aborted.clone(),
                finished: finished.clone(),
            }),
            JobContext {
                job_id: "errors".into(),
                cancel_flag: Arc::new(AtomicBool::new(false)),
                app_handle: None,
                progress_observer: None,
                throttle_ms: 0,
                max_errors: 1,
                max_error_samples: 100,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Validation(_)));
        assert!(aborted.load(Ordering::Relaxed));
        assert!(!finished.load(Ordering::Relaxed));
    }
}
