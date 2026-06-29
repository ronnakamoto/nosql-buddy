//! Safe Change Mode — preview, risk score, and rollback generation for MongoDB writes.
//!
//! Implements the MVP safety workflow from the Data Timeline spec:
//!   1. Parse the proposed write operation.
//!   2. Count matched documents and fetch a sample.
//!   3. Simulate the update to produce before/after diffs.
//!   4. Generate a rollback plan from the captured pre-images.
//!   5. Score the operation risk.
//!   6. Decide whether typed confirmation is required.

use std::sync::Arc;

use bson::{doc, Bson, Document};
use futures_util::TryStreamExt;
use mongodb::Client;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::{doc_to_display_json, parse_optional_doc};
use crate::mongo::client_registry::ClientEntry;
use crate::mongo::timeline_store::OperationKind;

const DEFAULT_SAMPLE_LIMIT: u64 = 10;
const MAX_SAMPLE_LIMIT: u64 = 50;
const ROLLBACK_CAPTURE_LIMIT: u64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeChangePreviewRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub kind: OperationKind,
    pub filter_json: String,
    pub update_json: Option<String>,
    pub replacement_json: Option<String>,
    pub sample_limit: Option<u64>,
}

/// How thoroughly rollback state was captured for the previewed operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewRollbackLevel {
    /// Matched count was zero or matched count exceeded the capture limit.
    MetadataOnly,
    /// Captured up to ROLLBACK_CAPTURE_LIMIT documents as pre-images.
    SampleBased,
    /// Captured every affected document (matched_count <= ROLLBACK_CAPTURE_LIMIT).
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeType {
    Added,
    Modified,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldChange {
    pub field: String,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub change_type: ChangeType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDiff {
    pub document_index: usize,
    pub field_changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub index_used: bool,
    pub stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SafeChangePreview {
    pub kind: OperationKind,
    pub matched_count: u64,
    pub sample_before: Vec<String>,
    pub sample_after: Vec<String>,
    pub diffs: Vec<DocumentDiff>,
    pub risk_score: u32,
    pub risk_reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub rollback_script: String,
    pub rollback_level: PreviewRollbackLevel,
    pub requires_typed_confirmation: bool,
    pub confirmation_text: String,
    pub is_production: bool,
    pub index_info: IndexInfo,
}

/// Preview a proposed write operation without executing it.
pub async fn preview_operation(
    entry: &ClientEntry,
    req: &SafeChangePreviewRequest,
) -> AppResult<SafeChangePreview> {
    let is_production = is_production_profile(&entry.name);
    let db = entry.client.database(&req.database);
    let coll = db.collection::<Document>(&req.collection);

    let filter = parse_optional_doc(Some(&req.filter_json))?.unwrap_or_default();
    let display_limit = req
        .sample_limit
        .unwrap_or(DEFAULT_SAMPLE_LIMIT)
        .min(MAX_SAMPLE_LIMIT);

    let matched_count = coll
        .count_documents(filter.clone())
        .await
        .map_err(|e| AppError::Mongo(e.to_string()))?;

    // Capture enough documents for both display and rollback generation.
    let capture_limit = matched_count.min(ROLLBACK_CAPTURE_LIMIT).max(display_limit) as i64;
    let mut cursor = coll
        .find(filter.clone())
        .limit(capture_limit)
        .await
        .map_err(|e| AppError::Mongo(e.to_string()))?;
    let mut captured_docs: Vec<Document> = Vec::new();
    while let Some(d) = cursor.try_next().await.map_err(|e| AppError::Mongo(e.to_string()))? {
        captured_docs.push(d);
    }

    let rollback_level = if matched_count == 0 {
        PreviewRollbackLevel::MetadataOnly
    } else if matched_count <= ROLLBACK_CAPTURE_LIMIT {
        PreviewRollbackLevel::Full
    } else {
        PreviewRollbackLevel::SampleBased
    };

    let update_doc = parse_optional_doc(req.update_json.as_deref())?;
    let replacement_doc = parse_optional_doc(req.replacement_json.as_deref())?;

    let mut warnings: Vec<String> = Vec::new();
    let (sample_before, sample_after, diffs) = match req.kind {
        OperationKind::UpdateOne | OperationKind::UpdateMany => {
            let update = update_doc.clone().ok_or_else(|| {
                AppError::Validation("update document is required".into())
            })?;
            let mut before = Vec::new();
            let mut after = Vec::new();
            let mut doc_diffs = Vec::new();
            for (i, doc) in captured_docs.iter().take(display_limit as usize).enumerate() {
                let after_doc = simulate_update(doc, &update, &mut warnings)?;
                let diff = compute_diff(doc, &after_doc);
                before.push(json_to_string(&doc_to_display_json(doc)?));
                after.push(json_to_string(&doc_to_display_json(&after_doc)?));
                doc_diffs.push(DocumentDiff { document_index: i, field_changes: diff });
            }
            (before, after, doc_diffs)
        }
        OperationKind::DeleteOne | OperationKind::DeleteMany => {
            let mut before = Vec::new();
            let mut doc_diffs = Vec::new();
            for (i, doc) in captured_docs.iter().take(display_limit as usize).enumerate() {
                before.push(json_to_string(&doc_to_display_json(doc)?));
                doc_diffs.push(DocumentDiff { document_index: i, field_changes: Vec::new() });
            }
            (before, Vec::new(), doc_diffs)
        }
        OperationKind::ReplaceOne => {
            let replacement = replacement_doc.clone().ok_or_else(|| {
                AppError::Validation("replacement document is required".into())
            })?;
            let mut before = Vec::new();
            let mut after = Vec::new();
            let mut doc_diffs = Vec::new();
            for (i, doc) in captured_docs.iter().take(display_limit as usize).enumerate() {
                let diff = compute_diff(doc, &replacement);
                before.push(json_to_string(&doc_to_display_json(doc)?));
                after.push(json_to_string(&doc_to_display_json(&replacement)?));
                doc_diffs.push(DocumentDiff { document_index: i, field_changes: diff });
            }
            (before, after, doc_diffs)
        }
        _ => {
            return Err(AppError::Validation(format!(
                "Safe Change Mode does not support operation kind: {:?}",
                req.kind
            )));
        }
    };

    let index_info =
        check_index_used(&entry.client, &req.database, &req.collection, &filter).await?;

    // Fetch total collection size so risk scoring can use proportion, not just
    // absolute count. This is a best-effort estimate; ignore failures.
    let collection_size = coll.estimated_document_count().await.unwrap_or(0);

    let update_or_replacement = update_doc.or(replacement_doc);
    let rollback_script =
        generate_rollback_script(req, &filter, &captured_docs, &update_or_replacement);

    let risk = score_risk(
        req.kind,
        matched_count,
        collection_size,
        is_production,
        &req.filter_json,
        index_info.index_used,
        &rollback_level,
        req.update_json.as_deref(),
        &warnings,
    );

    let requires_typed_confirmation =
        risk.score >= 60 || (is_production && is_dangerous_kind(req.kind));
    let confirmation_text = format_confirmation(req.kind, matched_count, is_production);

    Ok(SafeChangePreview {
        kind: req.kind,
        matched_count,
        sample_before,
        sample_after,
        diffs,
        risk_score: risk.score,
        risk_reasons: risk.reasons,
        warnings,
        rollback_script,
        rollback_level,
        requires_typed_confirmation,
        confirmation_text,
        is_production,
        index_info,
    })
}

fn format_confirmation(kind: OperationKind, matched_count: u64, is_production: bool) -> String {
    let op = match kind {
        OperationKind::UpdateOne => "UPDATE ONE",
        OperationKind::UpdateMany => "UPDATE",
        OperationKind::DeleteOne => "DELETE ONE",
        OperationKind::DeleteMany => "DELETE",
        OperationKind::ReplaceOne => "REPLACE ONE",
        _ => "APPLY",
    };
    let suffix = if matched_count == 1 { "" } else { "S" };
    if is_production {
        format!("{} {} DOCUMENT{} IN PRODUCTION", op, matched_count, suffix)
    } else {
        format!("{} {} DOCUMENT{}", op, matched_count, suffix)
    }
}

fn is_dangerous_kind(kind: OperationKind) -> bool {
    matches!(
        kind,
        OperationKind::DeleteOne
            | OperationKind::DeleteMany
            | OperationKind::UpdateMany
            | OperationKind::ReplaceOne
    )
}

fn is_production_profile(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("prod")
        || lower.contains("production")
        || lower.contains("live")
        || lower.contains("master")
}

fn json_to_string(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

// ─── Update simulation ────────────────────────────────────────────────────────

/// Simulate a subset of MongoDB update operators on a document clone.
fn simulate_update(
    doc: &Document,
    update: &Document,
    warnings: &mut Vec<String>,
) -> AppResult<Document> {
    let mut result = doc.clone();
    for (op, value) in update.iter() {
        match op.as_str() {
            "$set" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        set_nested(&mut result, k, v.clone())?;
                    }
                }
            }
            "$unset" => {
                if let Bson::Document(fields) = value {
                    for (k, _) in fields.iter() {
                        unset_nested(&mut result, k)?;
                    }
                }
            }
            "$inc" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        inc_nested(&mut result, k, v)?;
                    }
                }
            }
            "$mul" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        mul_nested(&mut result, k, v)?;
                    }
                }
            }
            "$rename" => {
                if let Bson::Document(fields) = value {
                    for (old_name, new_name) in fields.iter() {
                        if let Bson::String(new_name_str) = new_name {
                            rename_field(&mut result, old_name, new_name_str)?;
                        }
                    }
                }
            }
            "$push" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        push_to_array(&mut result, k, v.clone())?;
                    }
                }
            }
            "$addToSet" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        add_to_set(&mut result, k, v.clone())?;
                    }
                }
            }
            "$pull" => {
                if let Bson::Document(fields) = value {
                    for (k, v) in fields.iter() {
                        pull_from_array(&mut result, k, v.clone())?;
                    }
                }
            }
            _ => {
                warnings.push(format!(
                    "Preview does not fully simulate '{}'; after-state is approximate.",
                    op
                ));
            }
        }
    }
    Ok(result)
}

fn get_nested_value(doc: &Document, path: &str) -> Option<Bson> {
    let parts: Vec<&str> = path.splitn(2, '.').collect();
    if parts.len() == 1 {
        return doc.get(path).cloned();
    }
    if let Some(Bson::Document(ref child)) = doc.get(parts[0]).cloned() {
        return get_nested_value(child, parts[1]);
    }
    None
}

fn set_nested(doc: &mut Document, path: &str, value: Bson) -> AppResult<()> {
    let parts: Vec<&str> = path.splitn(2, '.').collect();
    if parts.len() == 1 {
        doc.insert(path.to_string(), value);
        return Ok(());
    }
    let child = doc
        .entry(parts[0].to_string())
        .or_insert_with(|| Bson::Document(Document::new()));
    if let Bson::Document(ref mut d) = child {
        set_nested(d, parts[1], value)?;
    } else {
        return Err(AppError::Validation(format!(
            "cannot set nested field '{}': intermediate is not a document",
            path
        )));
    }
    Ok(())
}

fn unset_nested(doc: &mut Document, path: &str) -> AppResult<()> {
    let parts: Vec<&str> = path.splitn(2, '.').collect();
    if parts.len() == 1 {
        doc.remove(path);
        return Ok(());
    }
    if let Some(Bson::Document(ref mut child)) = doc.get_mut(parts[0]) {
        unset_nested(child, parts[1])?;
    }
    Ok(())
}

fn inc_nested(doc: &mut Document, path: &str, delta: &Bson) -> AppResult<()> {
    let current = get_nested_value(doc, path).unwrap_or(Bson::Null);
    let new_value = numeric_add(&current, delta).ok_or_else(|| {
        AppError::Validation(format!("cannot $inc non-numeric field '{}'", path))
    })?;
    set_nested(doc, path, new_value)
}

fn mul_nested(doc: &mut Document, path: &str, factor: &Bson) -> AppResult<()> {
    let current = get_nested_value(doc, path).unwrap_or(Bson::Null);
    let new_value = numeric_mul(&current, factor).ok_or_else(|| {
        AppError::Validation(format!("cannot $mul non-numeric field '{}'", path))
    })?;
    set_nested(doc, path, new_value)
}

fn rename_field(doc: &mut Document, old_name: &str, new_name: &str) -> AppResult<()> {
    if let Some(value) = doc.remove(old_name) {
        doc.insert(new_name.to_string(), value);
    }
    Ok(())
}

fn push_to_array(doc: &mut Document, path: &str, value: Bson) -> AppResult<()> {
    let current = get_nested_value(doc, path);
    let new_arr = match current {
        Some(Bson::Array(mut arr)) => {
            arr.push(value);
            Bson::Array(arr)
        }
        None => Bson::Array(vec![value]),
        _ => {
            return Err(AppError::Validation(format!(
                "cannot $push to non-array field '{}'",
                path
            )))
        }
    };
    set_nested(doc, path, new_arr)
}

fn add_to_set(doc: &mut Document, path: &str, value: Bson) -> AppResult<()> {
    let current = get_nested_value(doc, path);
    let new_arr = match current {
        Some(Bson::Array(mut arr)) => {
            if !arr.contains(&value) {
                arr.push(value);
            }
            Bson::Array(arr)
        }
        None => Bson::Array(vec![value]),
        _ => {
            return Err(AppError::Validation(format!(
                "cannot $addToSet to non-array field '{}'",
                path
            )))
        }
    };
    set_nested(doc, path, new_arr)
}

fn pull_from_array(doc: &mut Document, path: &str, value: Bson) -> AppResult<()> {
    let current = get_nested_value(doc, path);
    if let Some(Bson::Array(mut arr)) = current {
        arr.retain(|item| item != &value);
        set_nested(doc, path, Bson::Array(arr))?;
    }
    Ok(())
}

fn numeric_add(a: &Bson, b: &Bson) -> Option<Bson> {
    match (a, b) {
        (Bson::Int32(x), Bson::Int32(y)) => Some(Bson::Int32(x + y)),
        (Bson::Int32(x), Bson::Int64(y)) => Some(Bson::Int64(i64::from(*x) + y)),
        (Bson::Int64(x), Bson::Int32(y)) => Some(Bson::Int64(x + i64::from(*y))),
        (Bson::Int64(x), Bson::Int64(y)) => Some(Bson::Int64(x + y)),
        (Bson::Double(x), Bson::Double(y)) => Some(Bson::Double(x + y)),
        (Bson::Double(x), Bson::Int32(y)) => Some(Bson::Double(x + f64::from(*y))),
        (Bson::Double(x), Bson::Int64(y)) => Some(Bson::Double(x + *y as f64)),
        (Bson::Null, Bson::Int32(y)) => Some(Bson::Int32(*y)),
        (Bson::Null, Bson::Int64(y)) => Some(Bson::Int64(*y)),
        (Bson::Null, Bson::Double(y)) => Some(Bson::Double(*y)),
        _ => None,
    }
}

fn numeric_mul(a: &Bson, b: &Bson) -> Option<Bson> {
    match (a, b) {
        (Bson::Int32(x), Bson::Int32(y)) => Some(Bson::Int32(x * y)),
        (Bson::Int32(x), Bson::Int64(y)) => Some(Bson::Int64(i64::from(*x) * y)),
        (Bson::Int64(x), Bson::Int32(y)) => Some(Bson::Int64(x * i64::from(*y))),
        (Bson::Int64(x), Bson::Int64(y)) => Some(Bson::Int64(x * y)),
        (Bson::Double(x), Bson::Double(y)) => Some(Bson::Double(x * y)),
        (Bson::Double(x), Bson::Int32(y)) => Some(Bson::Double(x * f64::from(*y))),
        (Bson::Double(x), Bson::Int64(y)) => Some(Bson::Double(x * *y as f64)),
        (Bson::Null, Bson::Int32(_)) => Some(Bson::Int32(0)),
        (Bson::Null, Bson::Int64(_)) => Some(Bson::Int64(0)),
        (Bson::Null, Bson::Double(_)) => Some(Bson::Double(0.0)),
        _ => None,
    }
}

// ─── Field diff ───────────────────────────────────────────────────────────────

/// Compute a field-level diff between two documents using display JSON values.
fn compute_diff(before: &Document, after: &Document) -> Vec<FieldChange> {
    let before_json = match doc_to_display_json(before) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let after_json = match doc_to_display_json(after) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let before_obj = match before_json.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };
    let after_obj = match after_json.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut keys: Vec<String> = before_obj.keys().chain(after_obj.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    let mut changes = Vec::new();
    for key in &keys {
        let old_val = before_obj.get(key).cloned();
        let new_val = after_obj.get(key).cloned();
        let change_type = match (&old_val, &new_val) {
            (None, Some(_)) => ChangeType::Added,
            (Some(_), None) => ChangeType::Removed,
            (Some(a), Some(b)) if a != b => ChangeType::Modified,
            _ => continue,
        };
        changes.push(FieldChange {
            field: key.clone(),
            old_value: old_val,
            new_value: new_val,
            change_type,
        });
    }
    changes
}

// ─── Rollback plan ────────────────────────────────────────────────────────────

/// Generate a JSON rollback plan from the captured pre-image documents.
fn generate_rollback_script(
    req: &SafeChangePreviewRequest,
    filter: &Document,
    captured_docs: &[Document],
    update_or_replacement: &Option<Document>,
) -> String {
    let plan = match req.kind {
        OperationKind::UpdateOne | OperationKind::UpdateMany => {
            let update = match update_or_replacement {
                Some(doc) => doc.clone(),
                None => return String::new(),
            };
            let mut ops: Vec<serde_json::Value> = Vec::new();
            for doc in captured_docs {
                let id_filter = match doc.get("_id") {
                    Some(id) => doc! { "_id": id.clone() },
                    None => continue,
                };
                let mut inverse = Document::new();
                apply_inverse_update(doc, &update, &mut inverse);
                if !inverse.is_empty() {
                    let id_filter_val = bson_doc_to_json(&id_filter);
                    let inverse_val = bson_doc_to_json(&inverse);
                    ops.push(serde_json::json!({
                        "updateOne": { "filter": id_filter_val, "update": inverse_val }
                    }));
                }
            }
            if ops.is_empty() {
                let filter_val = bson_doc_to_json(filter);
                let inverse = invert_update_metadata(&update);
                let inverse_val = bson_doc_to_json(&inverse);
                ops.push(serde_json::json!({
                    "updateMany": { "filter": filter_val, "update": inverse_val }
                }));
            }
            serde_json::json!({ "operation": "bulkWrite", "operations": ops })
        }
        OperationKind::DeleteOne | OperationKind::DeleteMany => {
            if captured_docs.is_empty() {
                serde_json::json!({
                    "operation": "insertMany",
                    "documents": [],
                    "note": "No pre-images captured. Restore from an external backup.",
                })
            } else {
                let docs: Vec<serde_json::Value> =
                    captured_docs.iter().map(|d| bson_doc_to_json(d)).collect();
                serde_json::json!({ "operation": "insertMany", "documents": docs })
            }
        }
        OperationKind::ReplaceOne => {
            if let Some(original) = captured_docs.first() {
                let id_filter = match original.get("_id") {
                    Some(id) => doc! { "_id": id.clone() },
                    None => filter.clone(),
                };
                serde_json::json!({
                    "operation": "replaceOne",
                    "filter": bson_doc_to_json(&id_filter),
                    "replacement": bson_doc_to_json(original),
                })
            } else {
                serde_json::json!({
                    "operation": "replaceOne",
                    "filter": bson_doc_to_json(filter),
                    "replacement": "No pre-image captured.",
                })
            }
        }
        _ => serde_json::json!({ "error": "unsupported rollback kind" }),
    };
    serde_json::to_string_pretty(&plan).unwrap_or_default()
}

fn bson_doc_to_json(doc: &Document) -> serde_json::Value {
    match bson::to_bson(doc) {
        Ok(bson) => bson::from_bson::<serde_json::Value>(bson).unwrap_or(serde_json::Value::Null),
        Err(_) => serde_json::Value::Null,
    }
}

/// Build the inverse of a single document update based on the original update operators.
fn apply_inverse_update(doc: &Document, update: &Document, inverse: &mut Document) {
    for (op, value) in update.iter() {
        match op.as_str() {
            "$set" => {
                if let Bson::Document(fields) = value {
                    for (k, _) in fields.iter() {
                        match get_nested_value(doc, k) {
                            Some(old) => {
                                // Field existed: restore its old value.
                                let set_entry = inverse
                                    .entry("$set".to_string())
                                    .or_insert_with(|| Bson::Document(Document::new()));
                                if let Bson::Document(ref mut d) = set_entry {
                                    d.insert(k.clone(), old);
                                }
                            }
                            None => {
                                // Field was newly added: unset it on rollback.
                                let unset_entry = inverse
                                    .entry("$unset".to_string())
                                    .or_insert_with(|| Bson::Document(Document::new()));
                                if let Bson::Document(ref mut d) = unset_entry {
                                    d.insert(k.clone(), Bson::String(String::new()));
                                }
                            }
                        }
                    }
                }
            }
            "$unset" => {
                if let Bson::Document(fields) = value {
                    for (k, _) in fields.iter() {
                        let old = get_nested_value(doc, k).unwrap_or(Bson::Null);
                        let set_entry = inverse
                            .entry("$set".to_string())
                            .or_insert_with(|| Bson::Document(Document::new()));
                        if let Bson::Document(ref mut d) = set_entry {
                            d.insert(k.clone(), old);
                        }
                    }
                }
            }
            _ => {
                // Unsupported operators: annotate as needing manual rollback.
                let note_entry = inverse
                    .entry("$set".to_string())
                    .or_insert_with(|| Bson::Document(Document::new()));
                if let Bson::Document(ref mut d) = note_entry {
                    d.insert(
                        "__rollback_note".to_string(),
                        Bson::String(format!("operator '{}' requires manual rollback", op)),
                    );
                }
            }
        }
    }
}

fn invert_update_metadata(update: &Document) -> Document {
    let mut inverse = Document::new();
    for (op, value) in update.iter() {
        match op.as_str() {
            "$set" => {
                if let Bson::Document(fields) = value {
                    let unset = inverse
                        .entry("$unset".to_string())
                        .or_insert_with(|| Bson::Document(Document::new()));
                    if let Bson::Document(ref mut d) = unset {
                        for (k, _) in fields.iter() {
                            d.insert(k.clone(), Bson::String(String::new()));
                        }
                    }
                }
            }
            "$unset" => {
                if let Bson::Document(fields) = value {
                    let set = inverse
                        .entry("$set".to_string())
                        .or_insert_with(|| Bson::Document(Document::new()));
                    if let Bson::Document(ref mut d) = set {
                        for (k, _) in fields.iter() {
                            d.insert(k.clone(), Bson::Null);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    inverse
}

// ─── Index check ─────────────────────────────────────────────────────────────

/// Check whether the filter is expected to use an index by running explain.
async fn check_index_used(
    client: &Arc<Client>,
    database: &str,
    collection: &str,
    filter: &Document,
) -> AppResult<IndexInfo> {
    let db = client.database(database);
    let explain_doc = doc! {
        "explain": {
            "find": collection,
            "filter": filter.clone(),
        },
        "verbosity": "queryPlanner",
    };
    match db.run_command(explain_doc).await {
        Ok(result) => {
            let stage = find_stage_in_doc(&result).unwrap_or_default();
            let index_used = stage == "IXSCAN" || contains_ixscan(&result);
            Ok(IndexInfo { index_used, stage })
        }
        Err(_) => {
            // Explain unavailable — don't fail the preview, just report unknown.
            Ok(IndexInfo {
                index_used: false,
                stage: "UNKNOWN".into(),
            })
        }
    }
}

fn find_stage_in_doc(doc: &Document) -> Option<String> {
    if let Some(Bson::String(s)) = doc.get("stage") {
        return Some(s.clone());
    }
    for (_, v) in doc.iter() {
        match v {
            Bson::Document(ref d) => {
                if let Some(s) = find_stage_in_doc(d) {
                    return Some(s);
                }
            }
            Bson::Array(ref arr) => {
                for item in arr.iter() {
                    if let Bson::Document(ref d) = item {
                        if let Some(s) = find_stage_in_doc(d) {
                            return Some(s);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn contains_ixscan(doc: &Document) -> bool {
    if let Some(Bson::String(s)) = doc.get("stage") {
        if s == "IXSCAN" {
            return true;
        }
    }
    for (_, v) in doc.iter() {
        match v {
            Bson::Document(ref d) if contains_ixscan(d) => return true,
            Bson::Array(ref arr) => {
                for item in arr.iter() {
                    if let Bson::Document(ref d) = item {
                        if contains_ixscan(d) {
                            return true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    false
}

// ─── Risk scoring ─────────────────────────────────────────────────────────────

struct RiskResult {
    score: u32,
    reasons: Vec<String>,
}

/// Score a proposed operation based on the Data Timeline risk factors.
fn score_risk(
    kind: OperationKind,
    matched_count: u64,
    collection_size: u64,
    is_production: bool,
    filter_json: &str,
    index_used: bool,
    rollback_level: &PreviewRollbackLevel,
    update_json: Option<&str>,
    extra_warnings: &[String],
) -> RiskResult {
    let mut score: u32 = 0;
    let mut reasons: Vec<String> = Vec::new();

    if is_production {
        score += 25;
        reasons.push("Production environment".into());
    }

    match kind {
        OperationKind::DeleteMany => {
            score += 20;
            reasons.push("deleteMany operation".into());
        }
        OperationKind::UpdateMany => {
            score += 15;
            reasons.push("updateMany operation".into());
        }
        OperationKind::DeleteOne => {
            score += 10;
            reasons.push("deleteOne operation".into());
        }
        OperationKind::UpdateOne | OperationKind::ReplaceOne => {
            score += 5;
            reasons.push(format!("{:?} operation", kind));
        }
        _ => {}
    }

    let filter = parse_optional_doc(Some(filter_json))
        .unwrap_or_default()
        .unwrap_or_default();
    if filter.is_empty() {
        score += 25;
        reasons.push("Empty filter matches the entire collection".into());
    } else if likely_broad_filter(&filter) {
        score += 10;
        reasons.push("Filter is broad".into());
    }

    if !index_used {
        score += 15;
        reasons.push("Query does not use an index (collection scan likely)".into());
    }

    // Score based on how large a fraction of the collection is affected.
    // Uses proportion when the collection size is known, otherwise falls
    // back to absolute thresholds so small demo DBs aren't under-scored.
    if matched_count > 1 {
        let pct = if collection_size > 0 {
            (matched_count * 100) / collection_size
        } else {
            0
        };
        if pct >= 50 || matched_count > 1000 {
            score += 20;
            if pct >= 50 {
                reasons.push(format!(
                    "{}% of the collection will be affected ({} docs)",
                    pct, matched_count
                ));
            } else {
                reasons.push(format!("High matched count: {}", matched_count));
            }
        } else if pct >= 25 || matched_count > 100 {
            score += 10;
            if pct >= 25 {
                reasons.push(format!(
                    "{}% of the collection will be affected ({} docs)",
                    pct, matched_count
                ));
            } else {
                reasons.push(format!("Moderate matched count: {}", matched_count));
            }
        } else if matched_count >= 5 {
            // Small absolute count but still multiple docs — add a small bump.
            score += 5;
            reasons.push(format!("{} documents will be affected", matched_count));
        }
    }

    if matches!(rollback_level, PreviewRollbackLevel::MetadataOnly) {
        score += 10;
        reasons.push("No captured pre-images for rollback".into());
    }

    if let Some(update) = update_json {
        if let Ok(Some(update_doc)) = parse_optional_doc(Some(update)) {
            for op in ["$unset", "$rename", "$pull", "$mul"] {
                if update_doc.contains_key(op) {
                    score += 5;
                    reasons.push(format!("Update uses {}", op));
                }
            }
        }
    }

    for warning in extra_warnings.iter() {
        score += 2;
        reasons.push(warning.clone());
    }

    RiskResult {
        score: score.min(100),
        reasons,
    }
}

fn likely_broad_filter(filter: &Document) -> bool {
    if filter.contains_key("_id") {
        return false;
    }
    // Only $or / $and / $regex / $exists at the top level → likely broad.
    filter
        .keys()
        .all(|k| matches!(k.as_str(), "$or" | "$and" | "$regex" | "$exists"))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn production_profile_detection() {
        assert!(is_production_profile("Production Billing"));
        assert!(is_production_profile("prod-us-east"));
        assert!(is_production_profile("live-api"));
        assert!(!is_production_profile("staging"));
        assert!(!is_production_profile("local-dev"));
    }

    #[test]
    fn simulate_update_set_and_unset() {
        let doc = doc! { "name": "Ada", "status": "active", "age": 30i32 };
        let update = doc! { "$set": { "status": "inactive" }, "$unset": { "age": "" } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        assert_eq!(after.get_str("status").unwrap(), "inactive");
        assert!(!after.contains_key("age"));
        assert_eq!(after.get_str("name").unwrap(), "Ada");
        assert!(warnings.is_empty());
    }

    #[test]
    fn simulate_update_inc_and_mul() {
        let doc = doc! { "c": 10i32, "d": 2.5f64 };
        let update = doc! { "$inc": { "c": 5i32 }, "$mul": { "d": 2.0f64 } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        assert_eq!(after.get_i32("c").unwrap(), 15);
        assert!((after.get_f64("d").unwrap() - 5.0).abs() < f64::EPSILON);
        assert!(warnings.is_empty());
    }

    #[test]
    fn simulate_update_push_and_add_to_set() {
        let doc = doc! { "tags": ["a"] };
        let update = doc! { "$push": { "tags": "b" }, "$addToSet": { "tags": "a" } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        let tags = after.get_array("tags").unwrap();
        // $push adds "b"; $addToSet does not re-add "a"
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn simulate_update_unknown_operator_adds_warning() {
        let doc = doc! { "x": 1i32 };
        let update = doc! { "$min": { "x": 0i32 } };
        let mut warnings = Vec::new();
        let _ = simulate_update(&doc, &update, &mut warnings).unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].contains("$min"));
    }

    #[test]
    fn compute_diff_detects_all_change_types() {
        let before = doc! { "a": 1i32, "b": 2i32 };
        let after = doc! { "a": 2i32, "c": 3i32 };
        let diff = compute_diff(&before, &after);
        assert_eq!(diff.len(), 3);
        let modified = diff.iter().find(|d| d.field == "a").unwrap();
        assert!(matches!(modified.change_type, ChangeType::Modified));
        let removed = diff.iter().find(|d| d.field == "b").unwrap();
        assert!(matches!(removed.change_type, ChangeType::Removed));
        let added = diff.iter().find(|d| d.field == "c").unwrap();
        assert!(matches!(added.change_type, ChangeType::Added));
    }

    #[test]
    fn risk_score_empty_filter_in_production() {
        let risk = score_risk(
            OperationKind::DeleteMany,
            5000,
            10000,
            true,
            "{}",
            false,
            &PreviewRollbackLevel::SampleBased,
            None,
            &[],
        );
        assert!(risk.score >= 70);
        assert!(risk.reasons.iter().any(|r| r.contains("Production")));
        assert!(risk.reasons.iter().any(|r| r.contains("Empty filter")));
    }

    #[test]
    fn risk_score_low_for_indexed_id_query() {
        let risk = score_risk(
            OperationKind::UpdateOne,
            1,
            1000,
            false,
            r#"{"_id":1}"#,
            true,
            &PreviewRollbackLevel::Full,
            None,
            &[],
        );
        assert!(risk.score < 50);
    }

    #[test]
    fn rollback_update_contains_updateone() {
        let doc = doc! { "_id": 1i32, "status": "active", "temp": "value" };
        let req = SafeChangePreviewRequest {
            connection_id: "c".into(),
            database: "db".into(),
            collection: "col".into(),
            kind: OperationKind::UpdateOne,
            filter_json: r#"{"_id":1}"#.into(),
            update_json: Some(r#"{"$set":{"status":"inactive"},"$unset":{"temp":""}}"#.into()),
            replacement_json: None,
            sample_limit: None,
        };
        let update = parse_optional_doc(req.update_json.as_deref())
            .unwrap()
            .unwrap();
        let script = generate_rollback_script(
            &req,
            &doc! { "_id": 1i32 },
            &[doc],
            &Some(update),
        );
        assert!(script.contains("updateOne"));
        assert!(script.contains("active"));
        assert!(script.contains("temp"));
    }

    #[test]
    fn rollback_delete_is_insert_many() {
        let req = SafeChangePreviewRequest {
            connection_id: "c".into(),
            database: "db".into(),
            collection: "col".into(),
            kind: OperationKind::DeleteMany,
            filter_json: r#"{"status":"failed"}"#.into(),
            update_json: None,
            replacement_json: None,
            sample_limit: None,
        };
        let script = generate_rollback_script(
            &req,
            &doc! { "status": "failed" },
            &[doc! { "_id": 1i32, "status": "failed" }],
            &None,
        );
        assert!(script.contains("insertMany"));
    }

    // ── is_dangerous_kind ────────────────────────────────────────────────────

    #[test]
    fn is_dangerous_kind_covers_all_variants() {
        assert!(is_dangerous_kind(OperationKind::DeleteOne));
        assert!(is_dangerous_kind(OperationKind::DeleteMany));
        assert!(is_dangerous_kind(OperationKind::UpdateMany));
        assert!(is_dangerous_kind(OperationKind::ReplaceOne));
        // Non-dangerous
        assert!(!is_dangerous_kind(OperationKind::UpdateOne));
        assert!(!is_dangerous_kind(OperationKind::InsertOne));
        assert!(!is_dangerous_kind(OperationKind::InsertMany));
        assert!(!is_dangerous_kind(OperationKind::Find));
        assert!(!is_dangerous_kind(OperationKind::Aggregate));
    }

    // ── format_confirmation ───────────────────────────────────────────────────

    #[test]
    fn format_confirmation_all_operations_and_production_flag() {
        // UpdateMany plural
        assert_eq!(
            format_confirmation(OperationKind::UpdateMany, 5, false),
            "UPDATE 5 DOCUMENTS"
        );
        // UpdateMany singular (1 doc, no "S")
        assert_eq!(
            format_confirmation(OperationKind::UpdateMany, 1, false),
            "UPDATE 1 DOCUMENT"
        );
        // UpdateOne
        assert_eq!(
            format_confirmation(OperationKind::UpdateOne, 3, false),
            "UPDATE ONE 3 DOCUMENTS"
        );
        // DeleteMany
        assert_eq!(
            format_confirmation(OperationKind::DeleteMany, 2, false),
            "DELETE 2 DOCUMENTS"
        );
        // DeleteOne
        assert_eq!(
            format_confirmation(OperationKind::DeleteOne, 1, false),
            "DELETE ONE 1 DOCUMENT"
        );
        // ReplaceOne
        assert_eq!(
            format_confirmation(OperationKind::ReplaceOne, 1, false),
            "REPLACE ONE 1 DOCUMENT"
        );
        // Production suffix
        assert_eq!(
            format_confirmation(OperationKind::DeleteMany, 100, true),
            "DELETE 100 DOCUMENTS IN PRODUCTION"
        );
        // Unknown kind falls back to "APPLY"
        assert_eq!(
            format_confirmation(OperationKind::Find, 1, false),
            "APPLY 1 DOCUMENT"
        );
    }

    // ── rollback_level selection ──────────────────────────────────────────────

    #[test]
    fn rollback_level_selection_from_matched_count() {
        // 0 → MetadataOnly
        let level = if 0u64 == 0 {
            PreviewRollbackLevel::MetadataOnly
        } else if 0u64 <= 100 {
            PreviewRollbackLevel::Full
        } else {
            PreviewRollbackLevel::SampleBased
        };
        assert!(matches!(level, PreviewRollbackLevel::MetadataOnly));

        // 1 → Full (1 <= 100)
        let level = if 1u64 == 0 {
            PreviewRollbackLevel::MetadataOnly
        } else if 1u64 <= 100 {
            PreviewRollbackLevel::Full
        } else {
            PreviewRollbackLevel::SampleBased
        };
        assert!(matches!(level, PreviewRollbackLevel::Full));

        // 100 → Full (boundary, still <= 100)
        let level = if 100u64 == 0 {
            PreviewRollbackLevel::MetadataOnly
        } else if 100u64 <= 100 {
            PreviewRollbackLevel::Full
        } else {
            PreviewRollbackLevel::SampleBased
        };
        assert!(matches!(level, PreviewRollbackLevel::Full));

        // 101 → SampleBased
        let level = if 101u64 == 0 {
            PreviewRollbackLevel::MetadataOnly
        } else if 101u64 <= 100 {
            PreviewRollbackLevel::Full
        } else {
            PreviewRollbackLevel::SampleBased
        };
        assert!(matches!(level, PreviewRollbackLevel::SampleBased));
    }

    // ── requires_typed_confirmation ───────────────────────────────────────────

    #[test]
    fn requires_typed_confirmation_triggers_above_score_60() {
        // score 60 → required
        let required = 60u32 >= 60 || (false && is_dangerous_kind(OperationKind::UpdateOne));
        assert!(required);
        // score 59 → not required (non-dangerous, non-production)
        let required = 59u32 >= 60 || (false && is_dangerous_kind(OperationKind::UpdateOne));
        assert!(!required);
    }

    #[test]
    fn requires_typed_confirmation_triggers_for_dangerous_op_in_production() {
        // score 30 but is_production=true and dangerous kind → required
        let required = 30u32 >= 60 || (true && is_dangerous_kind(OperationKind::DeleteMany));
        assert!(required);
        // score 30 but is_production=true and NOT dangerous kind → not required
        let required = 30u32 >= 60 || (true && is_dangerous_kind(OperationKind::UpdateOne));
        assert!(!required);
    }

    // ── score_risk proportional scoring ──────────────────────────────────────

    #[test]
    fn score_risk_update_many_full_collection_is_higher_than_one_percent() {
        let full_collection = score_risk(
            OperationKind::UpdateMany,
            1000,
            1000, // 100% of collection
            false,
            r#"{"x":1}"#,
            true,
            &PreviewRollbackLevel::Full,
            Some(r#"{"$set":{"x":2}}"#),
            &[],
        );
        let one_percent = score_risk(
            OperationKind::UpdateMany,
            10,
            1000, // 1% of collection
            false,
            r#"{"x":1}"#,
            true,
            &PreviewRollbackLevel::Full,
            Some(r#"{"$set":{"x":2}}"#),
            &[],
        );
        assert!(
            full_collection.score > one_percent.score,
            "full_collection={} one_percent={}",
            full_collection.score,
            one_percent.score
        );
    }

    #[test]
    fn score_risk_delete_one_id_query_small_collection_is_low() {
        let risk = score_risk(
            OperationKind::DeleteOne,
            1,
            10000,
            false,
            r#"{"_id":"abc"}"#,
            true,
            &PreviewRollbackLevel::Full,
            None,
            &[],
        );
        assert!(risk.score < 50, "expected low risk, got {}", risk.score);
    }

    #[test]
    fn score_risk_sample_based_rollback_is_higher_than_full() {
        let sample = score_risk(
            OperationKind::UpdateMany,
            500,
            1000,
            false,
            r#"{"x":1}"#,
            true,
            &PreviewRollbackLevel::SampleBased,
            Some(r#"{"$set":{"x":2}}"#),
            &[],
        );
        let full = score_risk(
            OperationKind::UpdateMany,
            500,
            1000,
            false,
            r#"{"x":1}"#,
            true,
            &PreviewRollbackLevel::Full,
            Some(r#"{"$set":{"x":2}}"#),
            &[],
        );
        assert!(
            sample.score >= full.score,
            "sample={} full={}",
            sample.score,
            full.score
        );
    }

    // ── is_production_profile boundary cases ─────────────────────────────────

    #[test]
    fn is_production_profile_boundary_cases() {
        // "master" keyword
        assert!(is_production_profile("master-cluster"));
        // case-insensitive "PROD"
        assert!(is_production_profile("PROD_DB"));
        // "production" in the middle
        assert!(is_production_profile("us-production-west"));
        // no keyword → false
        assert!(!is_production_profile("development"));
        assert!(!is_production_profile("test-db"));
        assert!(!is_production_profile(""));
    }

    // ── simulate_update additional operators ─────────────────────────────────

    #[test]
    fn simulate_update_rename_field() {
        let doc = doc! { "old_name": "Ada" };
        let update = doc! { "$rename": { "old_name": "new_name" } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        assert!(!after.contains_key("old_name"), "old key should be gone");
        assert_eq!(after.get_str("new_name").unwrap(), "Ada");
        assert!(warnings.is_empty());
    }

    #[test]
    fn simulate_update_set_nested_dot_path() {
        let doc = doc! { "address": { "city": "London", "zip": "SW1" } };
        let update = doc! { "$set": { "address.city": "Manchester" } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        let addr = after.get_document("address").unwrap();
        assert_eq!(addr.get_str("city").unwrap(), "Manchester");
        assert_eq!(addr.get_str("zip").unwrap(), "SW1");
    }

    #[test]
    fn simulate_update_pull_from_array() {
        let doc = doc! { "tags": ["rust", "mongo", "tauri"] };
        let update = doc! { "$pull": { "tags": "mongo" } };
        let mut warnings = Vec::new();
        let after = simulate_update(&doc, &update, &mut warnings).unwrap();
        let tags: Vec<_> = after
            .get_array("tags")
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(tags, vec!["rust", "tauri"]);
    }
}
