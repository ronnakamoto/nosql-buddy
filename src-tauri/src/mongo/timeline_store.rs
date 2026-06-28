//! Persistent store for Data Timeline — a chronological ledger of database
//! operations performed through NoSQLBuddy.
//!
//! Follows the same JSONL append-on-write pattern as `JobStore`.
//! All entries are scoped by `profile_id` so history survives app restarts
//! even though `connection_id` is ephemeral.

use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// Classification of an operation that can appear in the timeline.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum OperationKind {
    // Read operations
    Find,
    Aggregate,
    Sql,
    Explain,

    // Write operations
    InsertOne,
    InsertMany,
    UpdateOne,
    UpdateMany,
    DeleteOne,
    DeleteMany,
    ReplaceOne,
    AggregationWrite, // $merge / $out

    // Schema operations
    IndexCreate,
    IndexDrop,
    CollectionCreate,
    CollectionDrop,

    // Bulk operations
    Import,
    Export,
    Dump,
    Restore,
}

/// How thoroughly rollback state was captured for this operation.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RollbackLevel {
    /// No rollback data stored (metadata only).
    #[default]
    None,
    /// Captured a representative sample of affected documents.
    Sample,
    /// Captured previous values for changed fields only.
    ChangedFields,
    /// Full document pre-images for every affected document.
    Full,
}

/// Approval status for operations that went through a review workflow.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalStatus {
    /// No approval workflow was used.
    #[default]
    NotRequired,
    /// Pending reviewer approval.
    Pending,
    /// Approved and executed.
    Approved,
    /// Rejected; operation was not executed.
    Rejected,
}

/// One entry in the Data Timeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntry {
    /// Unique identifier (UUID v4).
    pub id: String,

    /// Stable profile identifier (survives app restarts).
    pub profile_id: String,

    /// Ephemeral connection identifier for the session.
    pub connection_id: String,

    /// What kind of operation was performed.
    pub kind: OperationKind,

    /// Target database.
    pub database: String,

    /// Target collection (empty string for database-level ops).
    pub collection: String,

    /// Who performed the operation ("local-user" until SSO is added).
    pub actor: String,

    /// User-defined environment tag (e.g. "production", "staging").
    pub environment_tag: String,

    /// The query / filter JSON, if applicable.
    pub query_json: Option<String>,

    /// The update / replacement JSON, if applicable.
    pub update_json: Option<String>,

    /// Number of documents matched by the filter.
    pub matched_count: Option<u64>,

    /// Number of documents actually modified.
    pub modified_count: Option<u64>,

    /// Number of documents inserted.
    pub inserted_count: Option<u64>,

    /// Number of documents deleted.
    pub deleted_count: Option<u64>,

    /// Risk score 0–100 (higher = more dangerous).
    pub risk_score: Option<u8>,

    /// Human-readable risk reasons.
    pub risk_reasons: Option<Vec<String>>,

    /// Approval workflow status.
    pub approval_status: ApprovalStatus,

    /// Reviewer identities (email or username).
    pub reviewers: Option<Vec<String>>,

    /// How much rollback data we captured.
    pub rollback_level: RollbackLevel,

    /// Serialized rollback script (JSON) or inverse operation.
    pub rollback_script: Option<String>,

    /// Path to an archive file containing pre-images (for large ops).
    pub rollback_archive_path: Option<String>,

    /// Free-text note added by the user after the fact.
    pub notes: Option<String>,

    /// When the entry was first created (before execution).
    pub created_at: String,

    /// When the operation was actually executed.
    pub executed_at: Option<String>,

    /// How long the operation took in milliseconds.
    pub execution_ms: Option<u64>,

    /// Whether the operation errored out.
    pub errored: bool,

    /// Error message if errored.
    pub error_message: Option<String>,

    /// For read ops: how many docs were returned.
    pub returned_count: Option<u64>,
}

impl TimelineEntry {
    pub fn builder(id: String, profile_id: String, kind: OperationKind) -> TimelineEntryBuilder {
        TimelineEntryBuilder {
            id,
            profile_id,
            connection_id: String::new(),
            kind,
            database: String::new(),
            collection: String::new(),
            actor: "local-user".to_string(),
            environment_tag: String::new(),
            query_json: None,
            update_json: None,
            matched_count: None,
            modified_count: None,
            inserted_count: None,
            deleted_count: None,
            risk_score: None,
            risk_reasons: None,
            approval_status: ApprovalStatus::NotRequired,
            reviewers: None,
            rollback_level: RollbackLevel::None,
            rollback_script: None,
            rollback_archive_path: None,
            notes: None,
            created_at: chrono_now(),
            executed_at: None,
            execution_ms: None,
            errored: false,
            error_message: None,
            returned_count: None,
        }
    }
}

/// Fluent builder for `TimelineEntry`.
pub struct TimelineEntryBuilder {
    id: String,
    profile_id: String,
    connection_id: String,
    kind: OperationKind,
    database: String,
    collection: String,
    actor: String,
    environment_tag: String,
    query_json: Option<String>,
    update_json: Option<String>,
    matched_count: Option<u64>,
    modified_count: Option<u64>,
    inserted_count: Option<u64>,
    deleted_count: Option<u64>,
    risk_score: Option<u8>,
    risk_reasons: Option<Vec<String>>,
    approval_status: ApprovalStatus,
    reviewers: Option<Vec<String>>,
    rollback_level: RollbackLevel,
    rollback_script: Option<String>,
    rollback_archive_path: Option<String>,
    notes: Option<String>,
    created_at: String,
    executed_at: Option<String>,
    execution_ms: Option<u64>,
    errored: bool,
    error_message: Option<String>,
    returned_count: Option<u64>,
}

#[allow(dead_code)]
impl TimelineEntryBuilder {
    pub fn connection_id(mut self, v: String) -> Self {
        self.connection_id = v;
        self
    }
    pub fn database(mut self, v: String) -> Self {
        self.database = v;
        self
    }
    pub fn collection(mut self, v: String) -> Self {
        self.collection = v;
        self
    }
    pub fn actor(mut self, v: String) -> Self {
        self.actor = v;
        self
    }
    pub fn environment_tag(mut self, v: String) -> Self {
        self.environment_tag = v;
        self
    }
    pub fn query_json(mut self, v: Option<String>) -> Self {
        self.query_json = v;
        self
    }
    pub fn update_json(mut self, v: Option<String>) -> Self {
        self.update_json = v;
        self
    }
    pub fn matched_count(mut self, v: u64) -> Self {
        self.matched_count = Some(v);
        self
    }
    pub fn modified_count(mut self, v: u64) -> Self {
        self.modified_count = Some(v);
        self
    }
    pub fn inserted_count(mut self, v: u64) -> Self {
        self.inserted_count = Some(v);
        self
    }
    pub fn deleted_count(mut self, v: u64) -> Self {
        self.deleted_count = Some(v);
        self
    }
    pub fn risk_score(mut self, v: u8) -> Self {
        self.risk_score = Some(v);
        self
    }
    pub fn risk_reasons(mut self, v: Vec<String>) -> Self {
        self.risk_reasons = Some(v);
        self
    }
    pub fn approval_status(mut self, v: ApprovalStatus) -> Self {
        self.approval_status = v;
        self
    }
    pub fn reviewers(mut self, v: Vec<String>) -> Self {
        self.reviewers = Some(v);
        self
    }
    pub fn rollback_level(mut self, v: RollbackLevel) -> Self {
        self.rollback_level = v;
        self
    }
    pub fn rollback_script(mut self, v: Option<String>) -> Self {
        self.rollback_script = v;
        self
    }
    pub fn rollback_archive_path(mut self, v: Option<String>) -> Self {
        self.rollback_archive_path = v;
        self
    }
    pub fn notes(mut self, v: Option<String>) -> Self {
        self.notes = v;
        self
    }
    pub fn executed_at(mut self, v: String) -> Self {
        self.executed_at = Some(v);
        self
    }
    pub fn execution_ms(mut self, v: u64) -> Self {
        self.execution_ms = Some(v);
        self
    }
    pub fn errored(mut self, v: bool) -> Self {
        self.errored = v;
        self
    }
    pub fn error_message(mut self, v: Option<String>) -> Self {
        self.error_message = v;
        self
    }
    pub fn returned_count(mut self, v: u64) -> Self {
        self.returned_count = Some(v);
        self
    }

    pub fn build(self) -> TimelineEntry {
        TimelineEntry {
            id: self.id,
            profile_id: self.profile_id,
            connection_id: self.connection_id,
            kind: self.kind,
            database: self.database,
            collection: self.collection,
            actor: self.actor,
            environment_tag: self.environment_tag,
            query_json: self.query_json,
            update_json: self.update_json,
            matched_count: self.matched_count,
            modified_count: self.modified_count,
            inserted_count: self.inserted_count,
            deleted_count: self.deleted_count,
            risk_score: self.risk_score,
            risk_reasons: self.risk_reasons,
            approval_status: self.approval_status,
            reviewers: self.reviewers,
            rollback_level: self.rollback_level,
            rollback_script: self.rollback_script,
            rollback_archive_path: self.rollback_archive_path,
            notes: self.notes,
            created_at: self.created_at,
            executed_at: self.executed_at,
            execution_ms: self.execution_ms,
            errored: self.errored,
            error_message: self.error_message,
            returned_count: self.returned_count,
        }
    }
}

/// Filter passed to `TimelineStore::list`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineFilter {
    pub profile_id: Option<String>,
    pub database: Option<String>,
    pub collection: Option<String>,
    pub kind: Option<OperationKind>,
    /// ISO-8601 RFC3339; inclusive.
    pub from: Option<String>,
    /// ISO-8601 RFC3339; inclusive.
    pub to: Option<String>,
    pub limit: Option<usize>,
    pub errored: Option<bool>,
}

/// Async-safe persistent timeline store.
pub struct TimelineStore {
    entries: RwLock<Vec<TimelineEntry>>,
    persist_path: Option<PathBuf>,
    // In-memory index: profile_id -> [entry indices] for fast filtering.
    // Rebuilt on load, kept in sync on mutation.
    index: RwLock<HashMap<String, Vec<usize>>>,
}

impl TimelineStore {
    pub fn new() -> Self {
        Self::with_path(None)
    }

    pub fn with_path(path: Option<PathBuf>) -> Self {
        let mut entries: Vec<TimelineEntry> = Vec::new();
        if let Some(ref path) = path {
            if let Ok(text) = std::fs::read_to_string(path) {
                entries = text
                    .lines()
                    .filter_map(|line| serde_json::from_str(line).ok())
                    .collect();
            }
        }

        let index = build_index(&entries);

        Self {
            entries: RwLock::new(entries),
            persist_path: path,
            index: RwLock::new(index),
        }
    }

    async fn save(&self) {
        if let Some(path) = &self.persist_path {
            let entries = self.entries.read().await;
            let lines: Vec<String> = entries
                .iter()
                .map(|e| serde_json::to_string(e).unwrap_or_default())
                .collect();
            let _ = std::fs::write(path, lines.join("\n"));
        }
    }

    /// Append a new entry and persist.
    pub async fn append(&self, entry: TimelineEntry) {
        let mut entries = self.entries.write().await;
        let idx = entries.len();
        entries.push(entry.clone());
        drop(entries);

        // Update index.
        let mut index = self.index.write().await;
        index
            .entry(entry.profile_id.clone())
            .or_default()
            .push(idx);
        drop(index);

        self.save().await;
    }

    /// List entries, newest first, optionally filtered.
    pub async fn list(&self, filter: TimelineFilter) -> Vec<TimelineEntry> {
        let entries = self.entries.read().await;

        // Fast path: if we have a profile_id filter, use the index to narrow
        // the candidate set before applying remaining filters.
        let candidate_indices: Vec<usize> = if let Some(ref pid) = filter.profile_id {
            let index = self.index.read().await;
            index.get(pid).cloned().unwrap_or_default()
        } else {
            (0..entries.len()).collect()
        };

        let mut out: Vec<TimelineEntry> = candidate_indices
            .into_iter()
            .filter_map(|idx| entries.get(idx).cloned())
            .filter(|e| {
                filter.database.as_ref().map_or(true, |db| &e.database == db)
                    && filter
                        .collection
                        .as_ref()
                        .map_or(true, |col| &e.collection == col)
                    && filter.kind.map_or(true, |k| e.kind == k)
                    && filter.errored.map_or(true, |er| e.errored == er)
                    && filter.from.as_ref().map_or(true, |from| &e.created_at >= from)
                    && filter.to.as_ref().map_or(true, |to| &e.created_at <= to)
            })
            .collect();

        // Newest first (by created_at, descending).
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        if let Some(limit) = filter.limit {
            out.truncate(limit);
        }
        out
    }

    /// Get a single entry by id.
    pub async fn get(&self, id: &str) -> Option<TimelineEntry> {
        self.entries
            .read()
            .await
            .iter()
            .find(|e| e.id == id)
            .cloned()
    }

    /// Update the notes field of an existing entry.
    pub async fn update_notes(&self, id: &str, notes: String) -> bool {
        let mut entries = self.entries.write().await;
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            entry.notes = Some(notes);
            drop(entries);
            self.save().await;
            true
        } else {
            false
        }
    }

    /// Delete an entry by id.
    pub async fn delete(&self, id: &str) -> bool {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|e| e.id != id);
        let removed = entries.len() < before;
        drop(entries);
        if removed {
            // Rebuild index since indices shifted.
            let entries_guard = self.entries.read().await;
            let new_index = build_index(&*entries_guard);
            drop(entries_guard);
            *self.index.write().await = new_index;
            self.save().await;
        }
        removed
    }

    /// Total number of entries across all profiles.
    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }

    /// Remove entries older than the cutoff (RFC3339 string), keeping the
    /// most recent `retention_count` entries per profile regardless of age.
    pub async fn prune(&self, cutoff: &str, retention_count: usize) -> usize {
        let mut entries = self.entries.write().await;
        let before = entries.len();

        // Build a set of ids to keep: the most recent `retention_count` per profile.
        let mut by_profile: HashMap<String, Vec<&TimelineEntry>> = HashMap::new();
        for e in entries.iter() {
            by_profile.entry(e.profile_id.clone()).or_default().push(e);
        }

        let mut keep_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (_pid, list) in by_profile.iter_mut() {
            // Sort newest first.
            list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            for e in list.iter().take(retention_count) {
                keep_ids.insert(e.id.clone());
            }
        }

        entries.retain(|e| {
            e.created_at >= cutoff.to_string() || keep_ids.contains(&e.id)
        });

        let removed = before - entries.len();
        drop(entries);
        if removed > 0 {
            let entries_guard = self.entries.read().await;
            let new_index = build_index(&*entries_guard);
            drop(entries_guard);
            *self.index.write().await = new_index;
            self.save().await;
        }
        removed
    }
}

impl Default for TimelineStore {
    fn default() -> Self {
        Self::new()
    }
}

fn build_index(entries: &[TimelineEntry]) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        index.entry(e.profile_id.clone()).or_default().push(i);
    }
    index
}

fn chrono_now() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str, profile_id: &str, kind: OperationKind, db: &str, col: &str) -> TimelineEntry {
        TimelineEntry::builder(id.to_string(), profile_id.to_string(), kind)
            .database(db.to_string())
            .collection(col.to_string())
            .build()
    }

    #[tokio::test]
    async fn append_and_list() {
        let store = TimelineStore::new();
        let e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
        store.append(e1.clone()).await;
        let e2 = sample_entry("e2", "p1", OperationKind::UpdateMany, "db1", "col2");
        store.append(e2.clone()).await;

        let all = store.list(TimelineFilter::default()).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "e2"); // newest first
    }

    #[tokio::test]
    async fn list_filters_by_profile_and_kind() {
        let store = TimelineStore::new();
        store.append(sample_entry("e1", "p1", OperationKind::Find, "db1", "col1")).await;
        store.append(sample_entry("e2", "p1", OperationKind::UpdateMany, "db1", "col1")).await;
        store.append(sample_entry("e3", "p2", OperationKind::Find, "db2", "col1")).await;

        let p1 = store.list(TimelineFilter {
            profile_id: Some("p1".into()),
            ..Default::default()
        }).await;
        assert_eq!(p1.len(), 2);

        let finds = store.list(TimelineFilter {
            kind: Some(OperationKind::Find),
            ..Default::default()
        }).await;
        assert_eq!(finds.len(), 2);
    }

    #[tokio::test]
    async fn update_notes_and_delete() {
        let store = TimelineStore::new();
        store.append(sample_entry("e1", "p1", OperationKind::Find, "db1", "col1")).await;

        assert!(store.update_notes("e1", "note-a".into()).await);
        let e = store.get("e1").await.unwrap();
        assert_eq!(e.notes, Some("note-a".into()));

        assert!(store.delete("e1").await);
        assert!(store.get("e1").await.is_none());
    }

    #[tokio::test]
    async fn prune_older_than_cutoff() {
        let store = TimelineStore::new();
        let mut e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
        e1.created_at = "2023-01-01T00:00:00+00:00".into();
        let mut e2 = sample_entry("e2", "p1", OperationKind::Find, "db1", "col1");
        e2.created_at = "2025-01-01T00:00:00+00:00".into();
        store.append(e1).await;
        store.append(e2).await;

        // e1 is older than cutoff and NOT in the retention set (only e2 is kept),
        // so it should be removed.
        let removed = store.prune("2024-01-01T00:00:00+00:00", 1).await;
        assert_eq!(removed, 1);

        let all = store.list(TimelineFilter::default()).await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "e2");

        // With retention_count=2, both entries are kept even though e1 is older.
        let store2 = TimelineStore::new();
        let mut e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
        e1.created_at = "2023-01-01T00:00:00+00:00".into();
        let mut e2 = sample_entry("e2", "p1", OperationKind::Find, "db1", "col1");
        e2.created_at = "2025-01-01T00:00:00+00:00".into();
        store2.append(e1).await;
        store2.append(e2).await;
        let removed2 = store2.prune("2024-01-01T00:00:00+00:00", 2).await;
        assert_eq!(removed2, 0); // both kept by retention

        let all2 = store2.list(TimelineFilter::default()).await;
        assert_eq!(all2.len(), 2);

        let removed3 = store2.prune("2026-01-01T00:00:00+00:00", 0).await;
        assert_eq!(removed3, 2);
    }

    #[tokio::test]
    async fn empty_store_returns_empty_list() {
        let store = TimelineStore::new();
        let all = store.list(TimelineFilter::default()).await;
        assert!(all.is_empty());
        assert_eq!(store.len().await, 0);
    }

    #[tokio::test]
    async fn get_returns_none_for_missing_entry() {
        let store = TimelineStore::new();
        assert!(store.get("nonexistent").await.is_none());
    }

    #[tokio::test]
    async fn update_notes_returns_false_for_missing_entry() {
        let store = TimelineStore::new();
        assert!(!store.update_notes("nonexistent", "note".into()).await);
    }

    #[tokio::test]
    async fn delete_returns_false_for_missing_entry() {
        let store = TimelineStore::new();
        assert!(!store.delete("nonexistent").await);
    }

    #[tokio::test]
    async fn list_filters_by_database_and_collection() {
        let store = TimelineStore::new();
        store.append(sample_entry("e1", "p1", OperationKind::Find, "db1", "col1")).await;
        store.append(sample_entry("e2", "p1", OperationKind::Find, "db1", "col2")).await;
        store.append(sample_entry("e3", "p1", OperationKind::Find, "db2", "col1")).await;

        let db1 = store.list(TimelineFilter {
            database: Some("db1".into()),
            ..Default::default()
        }).await;
        assert_eq!(db1.len(), 2);

        let col1 = store.list(TimelineFilter {
            collection: Some("col1".into()),
            ..Default::default()
        }).await;
        assert_eq!(col1.len(), 2);

        let both = store.list(TimelineFilter {
            database: Some("db1".into()),
            collection: Some("col1".into()),
            ..Default::default()
        }).await;
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].id, "e1");
    }

    #[tokio::test]
    async fn list_filters_by_date_range() {
        let store = TimelineStore::new();
        let mut e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
        e1.created_at = "2023-06-01T00:00:00+00:00".into();
        let mut e2 = sample_entry("e2", "p1", OperationKind::Find, "db1", "col1");
        e2.created_at = "2024-06-01T00:00:00+00:00".into();
        let mut e3 = sample_entry("e3", "p1", OperationKind::Find, "db1", "col1");
        e3.created_at = "2025-06-01T00:00:00+00:00".into();
        store.append(e1).await;
        store.append(e2).await;
        store.append(e3).await;

        let mid = store.list(TimelineFilter {
            from: Some("2024-01-01T00:00:00+00:00".into()),
            to: Some("2024-12-31T23:59:59+00:00".into()),
            ..Default::default()
        }).await;
        assert_eq!(mid.len(), 1);
        assert_eq!(mid[0].id, "e2");
    }

    #[tokio::test]
    async fn list_filters_by_errored() {
        let store = TimelineStore::new();
        let mut e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
        e1.errored = true;
        let e2 = sample_entry("e2", "p1", OperationKind::Find, "db1", "col1");
        store.append(e1).await;
        store.append(e2).await;

        let errors = store.list(TimelineFilter {
            errored: Some(true),
            ..Default::default()
        }).await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].id, "e1");
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let store = TimelineStore::new();
        for i in 0..5 {
            store.append(sample_entry(&format!("e{i}"), "p1", OperationKind::Find, "db1", "col1")).await;
        }
        let limited = store.list(TimelineFilter {
            limit: Some(2),
            ..Default::default()
        }).await;
        assert_eq!(limited.len(), 2);
    }

    #[tokio::test]
    async fn delete_rebuilds_index_correctly() {
        let store = TimelineStore::new();
        store.append(sample_entry("e1", "p1", OperationKind::Find, "db1", "col1")).await;
        store.append(sample_entry("e2", "p1", OperationKind::Find, "db1", "col1")).await;
        store.append(sample_entry("e3", "p2", OperationKind::Find, "db2", "col1")).await;

        assert!(store.delete("e2").await);

        let p1 = store.list(TimelineFilter {
            profile_id: Some("p1".into()),
            ..Default::default()
        }).await;
        assert_eq!(p1.len(), 1);
        assert_eq!(p1[0].id, "e1");

        let p2 = store.list(TimelineFilter {
            profile_id: Some("p2".into()),
            ..Default::default()
        }).await;
        assert_eq!(p2.len(), 1);
        assert_eq!(p2[0].id, "e3");
    }

    #[tokio::test]
    async fn persist_and_load_round_trip() {
        let temp = std::env::temp_dir().join(format!("timeline-test-{}.jsonl", uuid::Uuid::new_v4()));
        {
            let store = TimelineStore::with_path(Some(temp.clone()));
            let mut e1 = sample_entry("e1", "p1", OperationKind::Find, "db1", "col1");
            e1.created_at = "2024-01-01T00:00:00+00:00".into();
            store.append(e1).await;
            let mut e2 = sample_entry("e2", "p1", OperationKind::UpdateMany, "db1", "col2");
            e2.created_at = "2024-02-01T00:00:00+00:00".into();
            store.append(e2).await;
        }

        // Load into a fresh store from the same file.
        let store2 = TimelineStore::with_path(Some(temp.clone()));
        let all = store2.list(TimelineFilter::default()).await;
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "e2"); // newest first
        assert_eq!(all[1].id, "e1");

        // Cleanup.
        let _ = std::fs::remove_file(&temp);
    }

    #[tokio::test]
    async fn builder_produces_correct_entry() {
        let entry = TimelineEntry::builder("id-1".into(), "profile-1".into(), OperationKind::UpdateMany)
            .connection_id("conn-a".into())
            .database("mydb".into())
            .collection("mycol".into())
            .actor("admin".into())
            .environment_tag("production".into())
            .query_json(Some(r#"{"status":"pending"}"#.into()))
            .update_json(Some(r#"{"$set":{"status":"done"}}"#.into()))
            .matched_count(5)
            .modified_count(3)
            .risk_score(42)
            .risk_reasons(vec!["broad filter".into()])
            .rollback_level(RollbackLevel::Full)
            .notes(Some("hotfix".into()))
            .build();

        assert_eq!(entry.id, "id-1");
        assert_eq!(entry.profile_id, "profile-1");
        assert_eq!(entry.connection_id, "conn-a");
        assert_eq!(entry.database, "mydb");
        assert_eq!(entry.collection, "mycol");
        assert_eq!(entry.actor, "admin");
        assert_eq!(entry.environment_tag, "production");
        assert_eq!(entry.query_json, Some(r#"{"status":"pending"}"#.into()));
        assert_eq!(entry.update_json, Some(r#"{"$set":{"status":"done"}}"#.into()));
        assert_eq!(entry.matched_count, Some(5));
        assert_eq!(entry.modified_count, Some(3));
        assert_eq!(entry.risk_score, Some(42));
        assert_eq!(entry.risk_reasons, Some(vec!["broad filter".into()]));
        assert_eq!(entry.rollback_level, RollbackLevel::Full);
        assert_eq!(entry.notes, Some("hotfix".into()));
        assert_eq!(entry.approval_status, ApprovalStatus::NotRequired);
    }
}
