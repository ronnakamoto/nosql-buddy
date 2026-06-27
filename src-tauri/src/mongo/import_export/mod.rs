//! Import / Export streaming pipeline and registry.
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;

pub mod bson_sink;
pub mod bson_source;
pub mod collection_sink;
pub mod core;
pub mod csv;
pub mod csv_source;
pub mod io_util;
pub mod json;
pub mod json_source;
pub mod mapping;
pub mod placeholders;
pub mod source_cursor;
pub mod source_mem;

#[derive(Default)]
pub struct JobRegistry {
    inner: RwLock<HashMap<String, Arc<AtomicBool>>>,
}

impl JobRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new job, returning its cancellation flag.
    pub async fn register(&self, job_id: String) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.inner.write().await.insert(job_id, flag.clone());
        flag
    }

    /// Cancel an active job. Returns true if the job was found and flagged.
    pub async fn cancel(&self, job_id: &str) -> bool {
        if let Some(flag) = self.inner.read().await.get(job_id) {
            flag.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    /// Remove a finished or cancelled job from the registry.
    pub async fn unregister(&self, job_id: &str) {
        self.inner.write().await.remove(job_id);
    }
}
