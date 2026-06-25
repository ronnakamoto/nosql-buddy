//! MongoDB change stream listener for the ZK audit log.
//!
//! This module watches a MongoDB deployment for write operations and
//! feeds them into the audit log. Unlike the interceptor approach
//! (which hooks individual IPC calls), the change stream listener
//! captures ALL writes regardless of how they arrive — shell, IPC
//! commands, external clients, or migration scripts.
//!
//! ## Architecture
//!
//! The listener runs as a Tokio task spawned from `start()`. It:
//! 1. Opens a change stream on the MongoDB client with
//!    `showExpandedEvents: true` to capture insert/update/replace/
//!    delete/drop/rename operations.
//! 2. For each event, extracts the operation type, namespace, and
//!    document payload, then calls the appropriate interceptor
//!    function on the audit log.
//! 3. Persists the resume token after each event so the listener can
//!    resume gaplessly after an app restart (Phase 3).
//!
//! ## Resume token persistence (Phase 3)
//!
//! After processing each event, the listener calls
//! `stream.resume_token()` and saves the token to the sled store via
//! `AuditLog::save_resume_token()`. On startup, the token is loaded
//! and passed to `watch().resume_after()`, so the stream resumes from
//! the last processed event rather than the current server state.
//! This ensures no writes are missed while the app was offline.
//!
//! ## Limitations
//!
//! - No filtering by database/collection (watches everything)
//! - The change stream requires a replica set or sharded cluster
//!   (standalone mongod doesn't support change streams)
//! - Expanded events (createIndex, dropIndex) come through as
//!   `OperationType::Other(String)` and are handled generically

use std::sync::Arc;

use futures_util::StreamExt;
use mongodb::change_stream::event::{ChangeStreamEvent, OperationType};
use mongodb::Client;
use tokio::sync::Mutex;

use crate::audit::interceptor;
use crate::audit::AuditLog;
use crate::error::{AuditError, AuditResult};

/// A handle to a running change stream listener. Dropping this
/// does NOT stop the listener — use `stop()` to cancel it.
pub struct ChangeStreamHandle {
    cancel: Arc<tokio::sync::Notify>,
}

impl ChangeStreamHandle {
    /// Signal the listener to stop. The listener will exit at the
    /// next iteration of its event loop.
    pub fn stop(&self) {
        self.cancel.notify_waiters();
    }
}

/// Start a change stream listener on the given MongoDB client.
///
/// The listener watches all databases and collections for write
/// operations and records them in the audit log. It runs as a
/// detached Tokio task — the returned handle can be used to stop it.
///
/// If a resume token was previously saved for this `connection_id`,
/// the stream resumes from that point, ensuring no writes are missed
/// while the app was offline.
pub fn start(
    connection_id: String,
    client: Client,
    audit_log: Arc<AuditLog>,
) -> ChangeStreamHandle {
    let cancel = Arc::new(tokio::sync::Notify::new());
    let cancel_clone = cancel.clone();

    tokio::spawn(async move {
        run_listener(connection_id, client, audit_log, cancel_clone).await;
    });

    ChangeStreamHandle { cancel }
}

/// The core listener loop. Opens a change stream and processes events
/// until cancelled or until an unrecoverable error occurs.
async fn run_listener(
    connection_id: String,
    client: Client,
    audit_log: Arc<AuditLog>,
    cancel: Arc<tokio::sync::Notify>,
) {
    log::info!("audit change stream listener starting for connection {connection_id}");

    // Load the saved resume token (if any) so we can resume gaplessly.
    let initial_token = audit_log.load_resume_token(&connection_id).ok().flatten();
    if let Some(ref _token) = initial_token {
        log::info!("change stream resuming from saved token for connection {connection_id}");
    } else {
        log::info!("change stream starting fresh for connection {connection_id}");
    }

    // Retry loop: if the change stream drops (e.g. MongoDB restart,
    // network blip), we reconnect after a backoff. On reconnect, we
    // use the latest saved resume token (not the initial one) so we
    // resume from the last successfully processed event.
    let mut backoff = std::time::Duration::from_secs(1);
    let max_backoff = std::time::Duration::from_secs(30);

    loop {
        // Load the latest resume token before each (re)connection attempt.
        let resume_token = audit_log.load_resume_token(&connection_id).ok().flatten();

        tokio::select! {
            _ = cancel.notified() => {
                log::info!("audit change stream listener stopped");
                return;
            }
            result = watch_once(&client, &audit_log, &cancel, &connection_id, resume_token) => {
                match result {
                    Ok(()) => {
                        // watch_once returned normally — this means the
                        // stream ended or was cancelled.
                        log::info!("change stream ended normally");
                        return;
                    }
                    Err(e) => {
                        log::warn!("change stream error: {e}, retrying in {backoff:?}");
                        tokio::select! {
                            _ = cancel.notified() => {
                                log::info!("audit change stream listener stopped during backoff");
                                return;
                            }
                            _ = tokio::time::sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(max_backoff);
                    }
                }
            }
        }
    }
}

/// Open a change stream and process events until the stream ends
/// or the cancel signal is received.
async fn watch_once(
    client: &Client,
    audit_log: &Arc<AuditLog>,
    cancel: &Arc<tokio::sync::Notify>,
    connection_id: &str,
    resume_token: Option<mongodb::change_stream::event::ResumeToken>,
) -> AuditResult<()> {
    // Use the builder API: client.watch() returns a Watch struct,
    // chain .resume_after() if we have a token, then .show_expanded_events(true).
    let mut watch_builder = client.watch().show_expanded_events(true);
    if let Some(token) = resume_token {
        watch_builder = watch_builder.resume_after(token);
    }
    let mut stream = watch_builder
        .await
        .map_err(|e| AuditError::Mongo(format!("failed to open change stream: {e}")))?;

    log::info!("change stream opened, listening for events (connection {connection_id})");

    loop {
        tokio::select! {
            _ = cancel.notified() => {
                return Ok(());
            }
            result = stream.next() => {
                match result {
                    Some(Ok(event)) => {
                        process_event(event, audit_log);

                        // Persist the resume token after each event so
                        // we can resume gaplessly after a restart.
                        if let Some(token) = stream.resume_token() {
                            if let Err(e) = audit_log.save_resume_token(connection_id, &token) {
                                log::warn!("failed to save resume token: {e}");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        return Err(AuditError::Mongo(format!(
                            "change stream error: {e}"
                        )));
                    }
                    None => {
                        // Stream exhausted (server closed it).
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Process a single change event and record it in the audit log.
fn process_event(event: ChangeStreamEvent<bson::Document>, audit_log: &Arc<AuditLog>) {
    let ns = match event.ns {
        Some(ns) => ns,
        None => return,
    };

    let db = &ns.db;
    let coll = ns.coll.as_deref().unwrap_or("");

    match event.operation_type {
        OperationType::Insert => {
            if let Some(doc) = event.full_document {
                if let Ok(json) = serde_json::to_string(&doc) {
                    let _ = interceptor::record_insert(audit_log, db, coll, &json);
                }
            }
        }
        OperationType::Update | OperationType::Replace => {
            // For update events, use the document_key as the filter
            // and the update_description as the update payload.
            let filter_json = match &event.document_key {
                Some(key) => serde_json::to_string(key).unwrap_or_default(),
                None => "{}".to_string(),
            };
            let update_json = if let Some(ref ud) = event.update_description {
                // Build a JSON object with updatedFields and removedFields.
                let updated = serde_json::to_string(&ud.updated_fields)
                    .unwrap_or_else(|_| "{}".to_string());
                let removed = serde_json::to_string(&ud.removed_fields)
                    .unwrap_or_else(|_| "[]".to_string());
                format!(r#"{{"updatedFields":{},"removedFields":{}}}"#, updated, removed)
            } else if let Some(doc) = event.full_document {
                serde_json::to_string(&doc).unwrap_or_else(|_| "{}".to_string())
            } else {
                "{}".to_string()
            };
            let _ = interceptor::record_update(audit_log, db, coll, &filter_json, &update_json);
        }
        OperationType::Delete => {
            let filter_json = match &event.document_key {
                Some(key) => serde_json::to_string(key).unwrap_or_default(),
                None => "{}".to_string(),
            };
            let _ = interceptor::record_delete(audit_log, db, coll, &filter_json);
        }
        OperationType::Drop => {
            let _ = interceptor::record_drop_collection(audit_log, db, coll);
        }
        OperationType::DropDatabase => {
            let _ = interceptor::record_drop_database(audit_log, db);
        }
        OperationType::Rename => {
            // The `to` field contains the new namespace.
            let new_name = event
                .to
                .as_ref()
                .and_then(|t| t.coll.as_deref())
                .unwrap_or("renamed");
            let _ = interceptor::record_rename_collection(audit_log, db, coll, new_name);
        }
        OperationType::Invalidate => {
            // The namespace was invalidated (e.g. collection dropped).
            // We already handle Drop above, so this is a no-op.
        }
        OperationType::Other(ref op) => {
            // Expanded events (createIndex, dropIndex, create, modify,
            // shardCollection, refineCollectionShardKey) come through
            // as Other(String). We handle the ones we care about.
            match op.as_str() {
                "createIndexes" => {
                    // The full_document contains the index spec.
                    let keys_json = match &event.full_document {
                        Some(doc) => doc
                            .get("key")
                            .and_then(|v| serde_json::to_string(v).ok())
                            .unwrap_or_default(),
                        None => "{}".to_string(),
                    };
                    let _ = interceptor::record_create_index(
                        audit_log,
                        db,
                        coll,
                        &keys_json,
                        "{}",
                    );
                }
                "dropIndexes" => {
                    let name = event
                        .document_key
                        .as_ref()
                        .and_then(|d| d.get_str("name").ok())
                        .unwrap_or("unknown");
                    let _ = interceptor::record_drop_index(audit_log, db, coll, name);
                }
                _ => {
                    // Unknown expanded event — log but don't audit.
                    log::debug!("unhandled expanded event: {op}");
                }
            }
        }
        _ => {
            // The OperationType enum is #[non_exhaustive], so future
            // variants may be added. We silently ignore unknown ones.
        }
    }
}

/// A registry of active change stream listeners, keyed by connection ID.
/// This allows starting/stopping listeners per MongoDB connection.
pub struct ChangeStreamRegistry {
    handles: Mutex<std::collections::HashMap<String, ChangeStreamHandle>>,
}

impl ChangeStreamRegistry {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Start a change stream listener for the given connection.
    /// If one is already running, it is stopped first.
    /// The connection_id is used to key the resume token in sled so
    /// the stream can resume gaplessly after an app restart.
    pub async fn start_for(
        &self,
        connection_id: String,
        client: Client,
        audit_log: Arc<AuditLog>,
    ) {
        let mut handles = self.handles.lock().await;
        if let Some(old) = handles.remove(&connection_id) {
            old.stop();
        }
        let handle = start(connection_id.clone(), client, audit_log);
        handles.insert(connection_id, handle);
    }

    /// Stop the change stream listener for the given connection.
    /// The resume token is preserved in sled so the stream can resume
    /// gaplessly if the user reconnects to the same deployment later.
    pub async fn stop_for(&self, connection_id: &str) {
        let mut handles = self.handles.lock().await;
        if let Some(handle) = handles.remove(connection_id) {
            handle.stop();
        }
    }

    /// Stop all change stream listeners.
    pub async fn stop_all(&self) {
        let mut handles = self.handles.lock().await;
        for (_, handle) in handles.drain() {
            handle.stop();
        }
    }
}

impl Default for ChangeStreamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_stream_registry_starts_empty() {
        let registry = ChangeStreamRegistry::new();
        let handles = registry.handles.try_lock();
        assert!(handles.is_ok());
        assert!(handles.unwrap().is_empty());
    }

    #[test]
    fn change_stream_handle_can_be_stopped() {
        let cancel = Arc::new(tokio::sync::Notify::new());
        let handle = ChangeStreamHandle { cancel };
        handle.stop(); // should not panic
    }
}
