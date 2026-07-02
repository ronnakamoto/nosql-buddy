//! IPC wire-shape tests.
//!
//! Every type that crosses the Tauri IPC boundary (returned from a
//! `#[tauri::command]` or sent as an event payload) is serialized to JSON
//! by serde before the TypeScript frontend sees it.  The frontend then reads
//! fields by name (e.g. `entry.matchedCount`, `op.kind`).
//!
//! These tests serialize each type to `serde_json::Value` — exactly as the
//! IPC layer does — and assert the *exact wire shape* the frontend depends on:
//!
//!  - `#[serde(rename_all = "camelCase")]`  →  snake_case fields must be absent
//!  - `#[serde(tag = "kind")]`              →  enum variant names must be inline
//!  - Enum variant strings (camelCase / kebab-case / custom renames)
//!  - `Option<T>` fields serialize as JSON `null`, not absent
//!
//! A failure here means silent data loss or `undefined` values in the UI with
//! no compile-time or runtime error to point at the cause.

use serde_json::json;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn ser<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).expect("serialize")
}

/// Assert that a JSON object has exactly this string at this dotted key path.
macro_rules! assert_str {
    ($json:expr, $path:expr, $expected:expr) => {{
        let parts: Vec<&str> = $path.split('.').collect();
        let mut cur: &serde_json::Value = &$json;
        for part in &parts {
            cur = cur.get(*part).unwrap_or_else(|| {
                panic!("key {:?} missing in: {}", part, cur)
            });
        }
        assert_eq!(
            cur.as_str().unwrap_or_else(|| panic!("not a string at {:?}: {}", $path, cur)),
            $expected,
            "wrong value at {:?}",
            $path
        );
    }};
}

/// Assert that a JSON object has a null value at this dotted key path.
macro_rules! assert_null {
    ($json:expr, $path:expr) => {{
        let parts: Vec<&str> = $path.split('.').collect();
        let mut cur = &$json;
        for part in &parts {
            cur = cur.get(part).unwrap_or_else(|| {
                panic!("key {:?} missing (expected null) in: {}", part, $json)
            });
        }
        assert!(cur.is_null(), "expected null at {:?}, got: {}", $path, cur);
    }};
}

/// Assert that a key is ABSENT from a JSON object (not even null).
macro_rules! assert_absent {
    ($json:expr, $key:expr) => {{
        assert!(
            $json.get($key).is_none(),
            "key {:?} should be absent but found: {}",
            $key,
            $json.get($key).unwrap()
        );
    }};
}

// ─── OperationKind ───────────────────────────────────────────────────────────

use app_lib::mongo::timeline_store::OperationKind;

#[test]
fn operation_kind_read_variants_are_camel_case() {
    assert_eq!(ser(&OperationKind::Find), json!("find"));
    assert_eq!(ser(&OperationKind::Aggregate), json!("aggregate"));
    assert_eq!(ser(&OperationKind::Sql), json!("sql"));
    assert_eq!(ser(&OperationKind::Explain), json!("explain"));
}

#[test]
fn operation_kind_write_variants_are_camel_case() {
    assert_eq!(ser(&OperationKind::InsertOne), json!("insertOne"));
    assert_eq!(ser(&OperationKind::InsertMany), json!("insertMany"));
    assert_eq!(ser(&OperationKind::UpdateOne), json!("updateOne"));
    assert_eq!(ser(&OperationKind::UpdateMany), json!("updateMany"));
    assert_eq!(ser(&OperationKind::DeleteOne), json!("deleteOne"));
    assert_eq!(ser(&OperationKind::DeleteMany), json!("deleteMany"));
    assert_eq!(ser(&OperationKind::ReplaceOne), json!("replaceOne"));
    assert_eq!(ser(&OperationKind::AggregationWrite), json!("aggregationWrite"));
}

#[test]
fn operation_kind_schema_variants_are_camel_case() {
    assert_eq!(ser(&OperationKind::IndexCreate), json!("indexCreate"));
    assert_eq!(ser(&OperationKind::IndexDrop), json!("indexDrop"));
    assert_eq!(ser(&OperationKind::CollectionCreate), json!("collectionCreate"));
    assert_eq!(ser(&OperationKind::CollectionDrop), json!("collectionDrop"));
    assert_eq!(ser(&OperationKind::CollectionRename), json!("collectionRename"));
}

#[test]
fn operation_kind_bulk_variants_are_camel_case() {
    assert_eq!(ser(&OperationKind::Import), json!("import"));
    assert_eq!(ser(&OperationKind::Export), json!("export"));
    assert_eq!(ser(&OperationKind::Dump), json!("dump"));
    assert_eq!(ser(&OperationKind::Restore), json!("restore"));
}

// ─── RollbackLevel / ApprovalStatus ──────────────────────────────────────────

use app_lib::mongo::timeline_store::{ApprovalStatus, RollbackLevel};

#[test]
fn rollback_level_variants_are_camel_case() {
    assert_eq!(ser(&RollbackLevel::None), json!("none"));
    assert_eq!(ser(&RollbackLevel::Sample), json!("sample"));
    assert_eq!(ser(&RollbackLevel::ChangedFields), json!("changedFields"));
    assert_eq!(ser(&RollbackLevel::Full), json!("full"));
}

#[test]
fn approval_status_variants_are_camel_case() {
    assert_eq!(ser(&ApprovalStatus::NotRequired), json!("notRequired"));
    assert_eq!(ser(&ApprovalStatus::Pending), json!("pending"));
    assert_eq!(ser(&ApprovalStatus::Approved), json!("approved"));
    assert_eq!(ser(&ApprovalStatus::Rejected), json!("rejected"));
}

// ─── TimelineEntry ───────────────────────────────────────────────────────────

use app_lib::mongo::timeline_store::TimelineEntry;

fn sample_timeline_entry() -> TimelineEntry {
    TimelineEntry::builder("e1".into(), "p1".into(), OperationKind::UpdateMany)
        .database("shopkeeper".into())
        .collection("products".into())
        .matched_count(4)
        .modified_count(4)
        .risk_score(45)
        .risk_reasons(vec!["updateMany operation".into()])
        .execution_ms(12)
        .build()
}

#[test]
fn timeline_entry_top_level_fields_are_camel_case() {
    let j = ser(&sample_timeline_entry());
    // Spot-check a selection of camelCase field names.
    assert!(j.get("profileId").is_some(),   "profileId missing");
    assert!(j.get("connectionId").is_some(), "connectionId missing");
    assert!(j.get("environmentTag").is_some(), "environmentTag missing");
    assert!(j.get("queryJson").is_some(),    "queryJson missing");
    assert!(j.get("updateJson").is_some(),   "updateJson missing");
    assert!(j.get("matchedCount").is_some(), "matchedCount missing");
    assert!(j.get("modifiedCount").is_some(),"modifiedCount missing");
    assert!(j.get("insertedCount").is_some(),"insertedCount missing");
    assert!(j.get("deletedCount").is_some(), "deletedCount missing");
    assert!(j.get("riskScore").is_some(),    "riskScore missing");
    assert!(j.get("riskReasons").is_some(),  "riskReasons missing");
    assert!(j.get("approvalStatus").is_some(),"approvalStatus missing");
    assert!(j.get("rollbackLevel").is_some(), "rollbackLevel missing");
    assert!(j.get("rollbackScript").is_some(),"rollbackScript missing");
    assert!(j.get("rollbackArchivePath").is_some(), "rollbackArchivePath missing");
    assert!(j.get("createdAt").is_some(),    "createdAt missing");
    assert!(j.get("executedAt").is_some(),   "executedAt missing");
    assert!(j.get("executionMs").is_some(),  "executionMs missing");
    assert!(j.get("errorMessage").is_some(), "errorMessage missing");
    assert!(j.get("returnedCount").is_some(),"returnedCount missing");
}

#[test]
fn timeline_entry_snake_case_field_names_must_not_appear() {
    let j = ser(&sample_timeline_entry());
    assert_absent!(j, "profile_id");
    assert_absent!(j, "connection_id");
    assert_absent!(j, "environment_tag");
    assert_absent!(j, "query_json");
    assert_absent!(j, "update_json");
    assert_absent!(j, "matched_count");
    assert_absent!(j, "modified_count");
    assert_absent!(j, "inserted_count");
    assert_absent!(j, "deleted_count");
    assert_absent!(j, "risk_score");
    assert_absent!(j, "risk_reasons");
    assert_absent!(j, "approval_status");
    assert_absent!(j, "rollback_level");
    assert_absent!(j, "rollback_script");
    assert_absent!(j, "rollback_archive_path");
    assert_absent!(j, "created_at");
    assert_absent!(j, "executed_at");
    assert_absent!(j, "execution_ms");
    assert_absent!(j, "error_message");
    assert_absent!(j, "returned_count");
}

#[test]
fn timeline_entry_none_options_serialize_as_null_not_absent() {
    // The frontend reads `entry.queryJson` expecting either a string or null.
    // If the field is absent the frontend would see `undefined`, which
    // behaves differently to `null` in JS conditionals and display logic.
    let j = ser(&sample_timeline_entry());
    assert_null!(j, "queryJson");
    assert_null!(j, "updateJson");
    assert_null!(j, "executedAt");
    assert_null!(j, "errorMessage");
    assert_null!(j, "rollbackScript");
    assert_null!(j, "rollbackArchivePath");
    assert_null!(j, "returnedCount");
    assert_null!(j, "insertedCount");
    assert_null!(j, "deletedCount");
}

#[test]
fn timeline_entry_nested_enum_kinds_are_camel_case_strings() {
    let j = ser(&sample_timeline_entry());
    assert_str!(j, "kind", "updateMany");
    assert_str!(j, "approvalStatus", "notRequired");
    assert_str!(j, "rollbackLevel", "none");
}

#[test]
fn timeline_entry_numeric_fields_serialize_correctly() {
    let j = ser(&sample_timeline_entry());
    assert_eq!(j["matchedCount"], json!(4u64));
    assert_eq!(j["modifiedCount"], json!(4u64));
    assert_eq!(j["riskScore"], json!(45u8));
    assert_eq!(j["executionMs"], json!(12u64));
}

// ─── SafeChangePreview ───────────────────────────────────────────────────────

use app_lib::mongo::safe_change::{
    ChangeType, DocumentDiff, FieldChange, IndexInfo, PreviewRollbackLevel, SafeChangePreview,
};

fn sample_safe_change_preview() -> SafeChangePreview {
    SafeChangePreview {
        kind: OperationKind::UpdateMany,
        matched_count: 4,
        sample_before: vec![r#"{"sku":"WH-1000XM5"}"#.into()],
        sample_after: vec![r#"{"sku":"WH-1000XM5","discontinued":true}"#.into()],
        diffs: vec![DocumentDiff {
            document_index: 0,
            field_changes: vec![
                FieldChange {
                    field: "discontinued".into(),
                    old_value: None,
                    new_value: Some(serde_json::json!(true)),
                    change_type: ChangeType::Added,
                },
                FieldChange {
                    field: "stock".into(),
                    old_value: Some(serde_json::json!(145)),
                    new_value: Some(serde_json::json!(0)),
                    change_type: ChangeType::Modified,
                },
            ],
        }],
        risk_score: 40,
        risk_reasons: vec!["updateMany operation".into()],
        warnings: vec![],
        rollback_script: r#"{"op":"bulkWrite"}"#.into(),
        rollback_level: PreviewRollbackLevel::Full,
        requires_typed_confirmation: false,
        confirmation_text: "UPDATE 4 DOCUMENTS".into(),
        is_production: false,
        index_info: IndexInfo { index_used: false, stage: "COLLSCAN".into() },
    }
}

#[test]
fn safe_change_preview_top_level_fields_are_camel_case() {
    let j = ser(&sample_safe_change_preview());
    assert!(j.get("matchedCount").is_some(),           "matchedCount missing");
    assert!(j.get("sampleBefore").is_some(),           "sampleBefore missing");
    assert!(j.get("sampleAfter").is_some(),            "sampleAfter missing");
    assert!(j.get("riskScore").is_some(),              "riskScore missing");
    assert!(j.get("riskReasons").is_some(),            "riskReasons missing");
    assert!(j.get("rollbackScript").is_some(),         "rollbackScript missing");
    assert!(j.get("rollbackLevel").is_some(),          "rollbackLevel missing");
    assert!(j.get("requiresTypedConfirmation").is_some(), "requiresTypedConfirmation missing");
    assert!(j.get("confirmationText").is_some(),       "confirmationText missing");
    assert!(j.get("isProduction").is_some(),           "isProduction missing");
    assert!(j.get("indexInfo").is_some(),              "indexInfo missing");
}

#[test]
fn safe_change_preview_snake_case_must_not_appear() {
    let j = ser(&sample_safe_change_preview());
    assert_absent!(j, "matched_count");
    assert_absent!(j, "sample_before");
    assert_absent!(j, "sample_after");
    assert_absent!(j, "risk_score");
    assert_absent!(j, "risk_reasons");
    assert_absent!(j, "rollback_script");
    assert_absent!(j, "rollback_level");
    assert_absent!(j, "requires_typed_confirmation");
    assert_absent!(j, "confirmation_text");
    assert_absent!(j, "is_production");
    assert_absent!(j, "index_info");
}

#[test]
fn safe_change_preview_rollback_level_variants_are_camel_case() {
    assert_eq!(ser(&PreviewRollbackLevel::MetadataOnly), json!("metadataOnly"));
    assert_eq!(ser(&PreviewRollbackLevel::SampleBased),  json!("sampleBased"));
    assert_eq!(ser(&PreviewRollbackLevel::Full),          json!("full"));
}

#[test]
fn safe_change_preview_change_type_variants_are_camel_case() {
    assert_eq!(ser(&ChangeType::Added),    json!("added"));
    assert_eq!(ser(&ChangeType::Modified), json!("modified"));
    assert_eq!(ser(&ChangeType::Removed),  json!("removed"));
}

#[test]
fn safe_change_preview_nested_diff_fields_are_camel_case() {
    let j = ser(&sample_safe_change_preview());
    let diff = &j["diffs"][0];
    assert!(diff.get("documentIndex").is_some(), "documentIndex missing");
    assert!(diff.get("fieldChanges").is_some(),  "fieldChanges missing");
    assert_absent!(diff, "document_index");
    assert_absent!(diff, "field_changes");

    let fc = &diff["fieldChanges"][0];
    assert!(fc.get("field").is_some(),      "field missing");
    assert!(fc.get("oldValue").is_some(),   "oldValue missing");
    assert!(fc.get("newValue").is_some(),   "newValue missing");
    assert!(fc.get("changeType").is_some(), "changeType missing");
    assert_absent!(fc, "old_value");
    assert_absent!(fc, "new_value");
    assert_absent!(fc, "change_type");
    assert_str!(fc, "changeType", "added");
}

#[test]
fn safe_change_preview_index_info_fields_are_camel_case() {
    let j = ser(&sample_safe_change_preview());
    let ii = &j["indexInfo"];
    assert!(ii.get("indexUsed").is_some(), "indexUsed missing");
    assert!(ii.get("stage").is_some(),     "stage missing");
    assert_absent!(ii, "index_used");
}

// ─── SafeChangeMeta ──────────────────────────────────────────────────────────

use app_lib::commands::mongo::SafeChangeMeta;

#[test]
fn safe_change_meta_is_camel_case_and_optional_fields_work() {
    use app_lib::mongo::timeline_store::RollbackLevel;

    let m = SafeChangeMeta {
        risk_score: Some(75),
        risk_reasons: Some(vec!["production".into()]),
        rollback_script: Some("db.c.insertMany([])".into()),
        rollback_level: RollbackLevel::Full,
    };
    let j = serde_json::to_value(&m).unwrap();
    assert_eq!(j["riskScore"], json!(75));
    assert_eq!(j["rollbackLevel"], json!("full"));
    assert!(j.get("risk_score").is_none());
    assert!(j.get("rollback_level").is_none());

    // Default (no safe-change data) serializes with nulls / "none"
    let d = SafeChangeMeta::default();
    let jd = serde_json::to_value(&d).unwrap();
    assert!(jd["riskScore"].is_null());
    assert_eq!(jd["rollbackLevel"], json!("none"));
}

// ─── ShellOutput ─────────────────────────────────────────────────────────────

use app_lib::mongo::shell::{ShellOutput, ShellTable};

#[test]
fn shell_output_uses_internally_tagged_kind_field() {
    let text = ShellOutput::Text { value: "hello".into() };
    let j = ser(&text);
    assert_str!(j, "kind", "text");
    assert!(j.get("value").is_some(), "value missing from ShellOutput::Text");
    // Must NOT be externally tagged: { "text": { "value": ... } }
    assert_absent!(j, "text");

    let err = ShellOutput::Error { value: "boom".into() };
    let j = ser(&err);
    assert_str!(j, "kind", "error");
    assert_absent!(j, "error");

    let tbl = ShellOutput::Table {
        value: ShellTable { columns: vec!["a".into()], rows: vec![], execution_ms: 5 },
    };
    let j = ser(&tbl);
    assert_str!(j, "kind", "table");
    assert_absent!(j, "table");
}

#[test]
fn shell_table_fields_are_camel_case() {
    let t = ShellTable { columns: vec!["col1".into()], rows: vec![], execution_ms: 42 };
    let j = ser(&t);
    assert!(j.get("executionMs").is_some(), "executionMs missing");
    assert_absent!(j, "execution_ms");
}

// ─── ShellOperation ──────────────────────────────────────────────────────────

use app_lib::mongo::shell::ShellOperation;

#[test]
fn shell_operation_fields_are_camel_case() {
    let op = ShellOperation {
        kind: OperationKind::UpdateOne,
        database: "shopkeeper".into(),
        collection: "products".into(),
        query_json: Some(r#"{"sku":"X"}"#.into()),
        update_json: Some(r#"{"$set":{"price":9}}"#.into()),
        matched_count: Some(1),
        modified_count: Some(1),
        inserted_count: None,
        deleted_count: None,
        execution_ms: Some(3),
        errored: false,
        error_message: None,
    };
    let j = ser(&op);
    assert!(j.get("queryJson").is_some(),    "queryJson missing");
    assert!(j.get("updateJson").is_some(),   "updateJson missing");
    assert!(j.get("matchedCount").is_some(), "matchedCount missing");
    assert!(j.get("modifiedCount").is_some(),"modifiedCount missing");
    assert!(j.get("insertedCount").is_some(),"insertedCount missing");
    assert!(j.get("deletedCount").is_some(), "deletedCount missing");
    assert!(j.get("executionMs").is_some(),  "executionMs missing");
    assert!(j.get("errorMessage").is_some(), "errorMessage missing");
    assert_absent!(j, "query_json");
    assert_absent!(j, "update_json");
    assert_absent!(j, "matched_count");
    assert_absent!(j, "modified_count");
    assert_absent!(j, "execution_ms");
    assert_absent!(j, "error_message");
    assert_str!(j, "kind", "updateOne");
    // None fields must be null, not absent.
    assert_null!(j, "insertedCount");
    assert_null!(j, "deletedCount");
    assert_null!(j, "errorMessage");
}

// ─── JobKind / JobStatus / JobMeta ───────────────────────────────────────────

use app_lib::mongo::job_store::{JobKind, JobMeta, JobStatus};

#[test]
fn job_kind_variants_are_camel_case() {
    assert_eq!(ser(&JobKind::Dump),    json!("dump"));
    assert_eq!(ser(&JobKind::Restore), json!("restore"));
    assert_eq!(ser(&JobKind::Export),  json!("export"));
    assert_eq!(ser(&JobKind::Import),  json!("import"));
}

#[test]
fn job_status_variants_are_camel_case() {
    assert_eq!(ser(&JobStatus::Queued),    json!("queued"));
    assert_eq!(ser(&JobStatus::Running),   json!("running"));
    assert_eq!(ser(&JobStatus::Done),      json!("done"));
    assert_eq!(ser(&JobStatus::Failed),    json!("failed"));
    assert_eq!(ser(&JobStatus::Cancelled), json!("cancelled"));
}

#[test]
fn job_meta_fields_are_camel_case() {
    let m = JobMeta::new("j1".into(), JobKind::Dump, "c1".into(), "shopkeeper".into());
    let j = ser(&m);
    assert!(j.get("jobId").is_some(),      "jobId missing");
    assert!(j.get("connectionId").is_some(),"connectionId missing");
    assert!(j.get("profileId").is_some(),  "profileId missing");
    assert!(j.get("createdAt").is_some(),  "createdAt missing");
    assert!(j.get("startedAt").is_some(),  "startedAt missing");
    assert!(j.get("finishedAt").is_some(), "finishedAt missing");
    assert!(j.get("outputPath").is_some(), "outputPath missing");
    assert!(j.get("sourcePath").is_some(), "sourcePath missing");
    assert!(j.get("parentJobId").is_some(),"parentJobId missing");
    assert!(j.get("configJson").is_some(), "configJson missing");
    assert_absent!(j, "job_id");
    assert_absent!(j, "connection_id");
    assert_absent!(j, "profile_id");
    assert_absent!(j, "created_at");
    assert_absent!(j, "started_at");
    assert_absent!(j, "finished_at");
    assert_absent!(j, "output_path");
    assert_absent!(j, "source_path");
    assert_absent!(j, "parent_job_id");
    assert_absent!(j, "config_json");
    // Nested enum values
    assert_str!(j, "kind", "dump");
    assert_str!(j, "status", "queued");
    // None options serialize as null
    assert_null!(j, "startedAt");
    assert_null!(j, "finishedAt");
    assert_null!(j, "outputPath");
    assert_null!(j, "sourcePath");
    assert_null!(j, "parentJobId");
    assert_null!(j, "configJson");
}

// ─── AuthMechanism ───────────────────────────────────────────────────────────

use app_lib::mongo::types::AuthMechanism;

#[test]
fn auth_mechanism_variants_are_kebab_case() {
    assert_eq!(ser(&AuthMechanism::None),         json!("none"));
    assert_eq!(ser(&AuthMechanism::ScramSha1),    json!("scram-sha-1"));
    assert_eq!(ser(&AuthMechanism::ScramSha256),  json!("scram-sha-256"));
    assert_eq!(ser(&AuthMechanism::X509),         json!("x509"));
    assert_eq!(ser(&AuthMechanism::Ldap),         json!("ldap"));
    // Kerberos remains in the enum for backward compatibility with stored
    // profiles but is hidden from the UI (no gssapi-auth build feature).
    assert_eq!(ser(&AuthMechanism::Kerberos),     json!("kerberos"));
    assert_eq!(ser(&AuthMechanism::AwsIam),       json!("aws-iam"));
}

// ─── CollectionKind ──────────────────────────────────────────────────────────

use app_lib::mongo::types::CollectionKind;

#[test]
fn collection_kind_variants_are_kebab_case() {
    assert_eq!(ser(&CollectionKind::Collection), json!("collection"));
    assert_eq!(ser(&CollectionKind::View),       json!("view"));
    assert_eq!(ser(&CollectionKind::TimeSeries), json!("time-series"));
    assert_eq!(ser(&CollectionKind::Sharded),    json!("sharded"));
    assert_eq!(ser(&CollectionKind::Bucketed),   json!("bucketed"));
}

// ─── RelationshipKind / SignalKind ────────────────────────────────────────────

use app_lib::mongo::relationship::{RelationshipKind, SignalKind};

#[test]
fn relationship_kind_uses_custom_hyphenated_renames() {
    assert_eq!(ser(&RelationshipKind::OneToOne),   json!("one-to-one"));
    assert_eq!(ser(&RelationshipKind::OneToMany),  json!("one-to-many"));
    assert_eq!(ser(&RelationshipKind::ManyToOne),  json!("many-to-one"));
    assert_eq!(ser(&RelationshipKind::ManyToMany), json!("many-to-many"));
}

#[test]
fn signal_kind_uses_custom_camel_case_renames() {
    assert_eq!(ser(&SignalKind::ObjectIdMatch),     json!("objectIdMatch"));
    assert_eq!(ser(&SignalKind::NamingConvention),  json!("namingConvention"));
    assert_eq!(ser(&SignalKind::Lookup),            json!("lookup"));
    assert_eq!(ser(&SignalKind::Index),             json!("index"));
    assert_eq!(ser(&SignalKind::AppSchema),         json!("appSchema"));
}
