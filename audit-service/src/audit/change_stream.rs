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

use crate::audit::epoch::EpochManager;
use crate::audit::interceptor;
use crate::audit::AuditLog;
use crate::error::{AuditError, AuditResult};

/// Derive a stable audit deployment identity from a `hello`/`isMaster`
/// response document.
///
/// The deployment id is the durable boundary an audit domain is keyed on
/// (`(deploymentId, database)`). It must be **stable across reconnects** —
/// the per-session `connectionId` is deliberately NOT used.
///
/// - Replica set → `rs:{setName}` (the set name is stable for the life of
///   the deployment and survives elections/reconnects).
/// - Sharded cluster (`msg == "isdbgrid"`) → `sharded:{fingerprint}` where
///   the fingerprint is a short hash of the sorted host list.
/// - Standalone → `standalone:{fingerprint}`.
///
/// When no host information is available at all, the fingerprint is
/// `unknown`, which keeps the function total (it never panics and always
/// returns a non-empty id).
pub fn derive_deployment_id(hello: &bson::Document) -> String {
    if let Ok(set_name) = hello.get_str("setName") {
        if !set_name.is_empty() {
            return format!("rs:{set_name}");
        }
    }

    let is_sharded = hello
        .get_str("msg")
        .map(|m| m == "isdbgrid")
        .unwrap_or(false);

    // Build a stable host fingerprint from the advertised member list,
    // falling back to the `me` field for sharded/standalone topologies.
    let mut hosts: Vec<String> = hello
        .get_array("hosts")
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if hosts.is_empty() {
        if let Ok(me) = hello.get_str("me") {
            if !me.is_empty() {
                hosts.push(me.to_string());
            }
        }
    }
    hosts.sort();
    hosts.dedup();

    let fingerprint = if hosts.is_empty() {
        "unknown".to_string()
    } else {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(hosts.join(",").as_bytes());
        hex::encode(&hasher.finalize()[..8])
    };

    if is_sharded {
        format!("sharded:{fingerprint}")
    } else {
        format!("standalone:{fingerprint}")
    }
}

/// Run a `hello` command against the deployment and derive a stable
/// [`derive_deployment_id`]. Returns an empty string only if the command
/// fails (e.g. the server is unreachable) so callers can treat it as the
/// backward-compatible "unattributed" domain.
pub async fn fetch_deployment_id(client: &Client) -> String {
    match client
        .database("admin")
        .run_command(bson::doc! { "hello": 1 })
        .await
    {
        Ok(doc) => derive_deployment_id(&doc),
        Err(e) => {
            log::warn!("could not derive audit deployment id from hello: {e}");
            String::new()
        }
    }
}

/// A handle to a running change stream listener. Dropping this
/// does NOT stop the listener — use `stop()` to cancel it.
pub struct ChangeStreamHandle {
    cancel: Arc<tokio::sync::Notify>,
}

/// Whether a deployment (identified by its stable audit deployment id)
/// supports MongoDB change streams.
///
/// Change streams require a replica set or sharded cluster; a standalone
/// `mongod` does not support them. This is the single source of truth for
/// the capture strategy, so the two capture paths never both record the
/// same write:
///
/// - change-stream-capable (`rs:*`, `sharded:*`) → the change-stream
///   listener is the authoritative capture path; the per-IPC interceptor
///   hooks are skipped to avoid double-recording app-originated writes.
/// - otherwise (`standalone:*` or an empty/unattributed id) → no change
///   stream is started (it would only error and retry forever); the IPC
///   interceptor hooks are the capture path.
pub fn supports_change_streams(deployment_id: &str) -> bool {
    deployment_id.starts_with("rs:") || deployment_id.starts_with("sharded:")
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
    deployment_id: String,
    client: Client,
    audit_log: Arc<AuditLog>,
    epoch_manager: Option<Arc<EpochManager>>,
) -> ChangeStreamHandle {
    let cancel = Arc::new(tokio::sync::Notify::new());
    let cancel_clone = cancel.clone();

    tokio::spawn(async move {
        run_listener(
            connection_id,
            deployment_id,
            client,
            audit_log,
            epoch_manager,
            cancel_clone,
        )
        .await;
    });

    ChangeStreamHandle { cancel }
}

/// The core listener loop. Opens a change stream and processes events
/// until cancelled or until an unrecoverable error occurs.
async fn run_listener(
    connection_id: String,
    deployment_id: String,
    client: Client,
    audit_log: Arc<AuditLog>,
    epoch_manager: Option<Arc<EpochManager>>,
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
            result = watch_once(&client, &deployment_id, &audit_log, epoch_manager.as_ref(), &cancel, &connection_id, resume_token) => {
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
    deployment_id: &str,
    audit_log: &Arc<AuditLog>,
    epoch_manager: Option<&Arc<EpochManager>>,
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
                        process_event(event, deployment_id, audit_log, epoch_manager);

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
fn process_event(
    event: ChangeStreamEvent<bson::Document>,
    deployment_id: &str,
    audit_log: &Arc<AuditLog>,
    epoch_manager: Option<&Arc<EpochManager>>,
) {
    let ns = match event.ns {
        Some(ns) => ns,
        None => return,
    };

    let db = &ns.db;
    let coll = ns.coll.as_deref().unwrap_or("");

    // Advance the open epoch (batch) for each recorded leaf so the UI's
    // "Batch · filling" counter and the Seal action track captured writes in
    // real time. The change-stream listener is the authoritative capture path
    // on replica sets / sharded clusters, so without this the open batch would
    // only catch up on app restart or after a reset. `None` keeps the legacy
    // behavior for callers (e.g. the daemon) that reconcile epochs elsewhere.
    let advance = |res: AuditResult<u64>| {
        if let (Ok(index), Some(em)) = (res, epoch_manager) {
            if let Err(e) = em.record_event(index, audit_log) {
                log::warn!("failed to advance epoch for change-stream event: {e}");
            }
        }
    };

    match event.operation_type {
        OperationType::Insert => {
            if let Some(doc) = event.full_document {
                if let Ok(json) = serde_json::to_string(&doc) {
                    advance(interceptor::record_insert(
                        audit_log,
                        deployment_id,
                        db,
                        coll,
                        &json,
                    ));
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
                let updated =
                    serde_json::to_string(&ud.updated_fields).unwrap_or_else(|_| "{}".to_string());
                let removed =
                    serde_json::to_string(&ud.removed_fields).unwrap_or_else(|_| "[]".to_string());
                format!(
                    r#"{{"updatedFields":{},"removedFields":{}}}"#,
                    updated, removed
                )
            } else if let Some(doc) = event.full_document {
                serde_json::to_string(&doc).unwrap_or_else(|_| "{}".to_string())
            } else {
                "{}".to_string()
            };
            advance(interceptor::record_update(
                audit_log,
                deployment_id,
                db,
                coll,
                &filter_json,
                &update_json,
            ));
        }
        OperationType::Delete => {
            let filter_json = match &event.document_key {
                Some(key) => serde_json::to_string(key).unwrap_or_default(),
                None => "{}".to_string(),
            };
            advance(interceptor::record_delete(
                audit_log,
                deployment_id,
                db,
                coll,
                &filter_json,
            ));
        }
        OperationType::Drop => {
            advance(interceptor::record_drop_collection(
                audit_log,
                deployment_id,
                db,
                coll,
            ));
        }
        OperationType::DropDatabase => {
            advance(interceptor::record_drop_database(
                audit_log,
                deployment_id,
                db,
            ));
        }
        OperationType::Rename => {
            // The `to` field contains the new namespace.
            let new_name = event
                .to
                .as_ref()
                .and_then(|t| t.coll.as_deref())
                .unwrap_or("renamed");
            advance(interceptor::record_rename_collection(
                audit_log,
                deployment_id,
                db,
                coll,
                new_name,
            ));
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
                    advance(interceptor::record_create_index(
                        audit_log,
                        deployment_id,
                        db,
                        coll,
                        &keys_json,
                        "{}",
                    ));
                }
                "dropIndexes" => {
                    let name = event
                        .document_key
                        .as_ref()
                        .and_then(|d| d.get_str("name").ok())
                        .unwrap_or("unknown");
                    advance(interceptor::record_drop_index(
                        audit_log,
                        deployment_id,
                        db,
                        coll,
                        name,
                    ));
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
        deployment_id: String,
        client: Client,
        audit_log: Arc<AuditLog>,
        epoch_manager: Option<Arc<EpochManager>>,
    ) {
        let mut handles = self.handles.lock().await;
        if let Some(old) = handles.remove(&connection_id) {
            old.stop();
        }
        let handle = start(
            connection_id.clone(),
            deployment_id,
            client,
            audit_log,
            epoch_manager,
        );
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
    use crate::audit::epoch::EpochConfig;

    /// Build a synthetic insert change event for the given document, so the
    /// capture path can be exercised without a live MongoDB.
    fn insert_event(doc: bson::Document) -> ChangeStreamEvent<bson::Document> {
        let event_doc = bson::doc! {
            "_id": { "_data": "resume-token" },
            "operationType": "insert",
            "ns": { "db": "appdb", "coll": "events" },
            "documentKey": { "_id": 1 },
            "fullDocument": doc,
        };
        bson::from_document(event_doc).expect("construct change stream event")
    }

    // Regression guard for the capture→epoch seam: on replica sets the change
    // stream is the authoritative capture path, so a captured write must
    // advance BOTH the audit log and the open batch (epoch). Before this was
    // wired, the batch counter stayed at 0 and "Seal Batch" never enabled.
    #[test]
    fn process_event_advances_open_epoch() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let epoch_manager = Arc::new(EpochManager::new(EpochConfig {
            event_threshold: 100,
            time_threshold_secs: 0,
        }));

        process_event(
            insert_event(bson::doc! { "x": 1 }),
            "rs:rs0",
            &audit,
            Some(&epoch_manager),
        );

        assert_eq!(audit.event_count(), 1, "audit log must record the capture");
        assert_eq!(
            epoch_manager.current_epoch().event_count,
            1,
            "open batch must track the captured write"
        );
    }

    // A captured write should auto-close the batch once the threshold is hit,
    // exactly as the manual/IPC path does.
    #[test]
    fn process_event_auto_closes_batch_at_threshold() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let epoch_manager = Arc::new(EpochManager::new(EpochConfig {
            event_threshold: 2,
            time_threshold_secs: 0,
        }));

        process_event(
            insert_event(bson::doc! { "n": 0 }),
            "rs:rs0",
            &audit,
            Some(&epoch_manager),
        );
        process_event(
            insert_event(bson::doc! { "n": 1 }),
            "rs:rs0",
            &audit,
            Some(&epoch_manager),
        );

        let epochs = epoch_manager.list_epochs();
        assert_eq!(
            epochs.len(),
            2,
            "threshold reached should open a fresh batch"
        );
        assert!(!epochs[0].is_open(), "first batch should have sealed");
        assert_eq!(epochs[0].event_count, 2);
        assert!(epochs[1].is_open());
    }

    // Backward-compat: callers that reconcile epochs elsewhere (e.g. the
    // daemon's auto_commit_loop) pass None and must NOT have the epoch touched
    // here, avoiding double-counting.
    #[test]
    fn process_event_without_epoch_manager_only_records_log() {
        let audit = Arc::new(AuditLog::new().unwrap());
        let epoch_manager = EpochManager::new(EpochConfig::default());

        process_event(insert_event(bson::doc! { "x": 1 }), "rs:rs0", &audit, None);

        assert_eq!(audit.event_count(), 1);
        assert_eq!(
            epoch_manager.current_epoch().event_count,
            0,
            "epoch must be left untouched when no manager is supplied"
        );
    }

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

    #[test]
    fn derive_deployment_id_uses_replica_set_name() {
        let hello = bson::doc! {
            "setName": "rs0",
            "hosts": ["a:27017", "b:27017"],
            "me": "a:27017",
        };
        assert_eq!(derive_deployment_id(&hello), "rs:rs0");
    }

    #[test]
    fn derive_deployment_id_is_stable_regardless_of_host_order() {
        let a = bson::doc! { "setName": "rs0", "hosts": ["a:27017", "b:27017"] };
        let b = bson::doc! { "setName": "rs0", "hosts": ["b:27017", "a:27017"] };
        assert_eq!(derive_deployment_id(&a), derive_deployment_id(&b));
    }

    #[test]
    fn derive_deployment_id_detects_sharded_cluster() {
        let hello = bson::doc! { "msg": "isdbgrid", "me": "mongos:27017" };
        let id = derive_deployment_id(&hello);
        assert!(id.starts_with("sharded:"), "got {id}");
    }

    #[test]
    fn derive_deployment_id_sharded_fingerprint_is_host_order_invariant() {
        let a = bson::doc! { "msg": "isdbgrid", "hosts": ["m1:27017", "m2:27017"] };
        let b = bson::doc! { "msg": "isdbgrid", "hosts": ["m2:27017", "m1:27017"] };
        assert_eq!(derive_deployment_id(&a), derive_deployment_id(&b));
    }

    #[test]
    fn derive_deployment_id_falls_back_to_standalone() {
        let hello = bson::doc! { "me": "localhost:27017" };
        let id = derive_deployment_id(&hello);
        assert!(id.starts_with("standalone:"), "got {id}");
    }

    #[test]
    fn derive_deployment_id_is_total_without_host_info() {
        let hello = bson::doc! {};
        assert_eq!(derive_deployment_id(&hello), "standalone:unknown");
    }

    #[test]
    fn derive_deployment_id_ignores_empty_set_name() {
        let hello = bson::doc! { "setName": "", "me": "localhost:27017" };
        assert!(derive_deployment_id(&hello).starts_with("standalone:"));
    }

    #[test]
    fn supports_change_streams_matches_topology() {
        // Replica set and sharded deployments support change streams.
        assert!(supports_change_streams("rs:rs0"));
        assert!(supports_change_streams("sharded:abc123"));
        // Standalone and the empty/unattributed id do not.
        assert!(!supports_change_streams("standalone:abc123"));
        assert!(!supports_change_streams(""));
    }
}
