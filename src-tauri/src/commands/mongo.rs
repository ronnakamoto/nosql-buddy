//! MongoDB data operations: list databases, list collections, find, aggregate,
//! count, indexes, sample schema, insert/update/delete with preview/apply,
//! explain, and stats. All commands return typed DTOs; documents are encoded
//! as MongoDB Extended JSON to preserve BSON fidelity.

use std::time::Instant;

use bson::Document;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::{doc_to_display_json, doc_to_extjson, parse_optional_doc};
use crate::mongo::client_registry::list_collections as registry_list_collections;
use crate::mongo::types::{
    CollationDto, CollectionKind, CollectionStats, CollectionSummary, DatabaseSummary,
    DocumentPage, ExplainResult, IndexInfo, IndexStats,
};
use crate::state::AppState;

/// Convert a driver `Collation` (enum-typed fields) into the flat JSON-friendly
/// `CollationDto` used over IPC. Strength becomes an i32 (1=Primary … 5=Identical);
/// the kebab-case enum strings are passed through verbatim.
fn collation_from_driver(c: &mongodb::options::Collation) -> CollationDto {
    CollationDto {
        locale: c.locale.clone(),
        strength: c.strength.map(|s| u32::from(s) as i32),
        case_level: c.case_level,
        case_first: c.case_first.map(|v| v.as_str().to_string()),
        numeric_ordering: c.numeric_ordering,
        alternate: c.alternate.map(|v| v.as_str().to_string()),
        max_variable: c.max_variable.map(|v| v.as_str().to_string()),
        normalization: c.normalization,
        backwards: c.backwards,
    }
}

/// Convert the IPC `CollationDto` back into a driver `Collation`. Unknown
/// kebab-case strings become `None` rather than erroring, so a malformed
/// payload from the renderer degrades gracefully instead of failing the
/// whole create-index call.
fn collation_to_driver(dto: &CollationDto) -> mongodb::options::Collation {
    use mongodb::options::{
        CollationAlternate, CollationCaseFirst, CollationMaxVariable, CollationStrength,
    };
    let strength = match dto.strength {
        Some(1) => Some(CollationStrength::Primary),
        Some(2) => Some(CollationStrength::Secondary),
        Some(3) => Some(CollationStrength::Tertiary),
        Some(4) => Some(CollationStrength::Quaternary),
        Some(5) => Some(CollationStrength::Identical),
        _ => None,
    };
    let case_first = match dto.case_first.as_deref() {
        Some("upper") => Some(CollationCaseFirst::Upper),
        Some("lower") => Some(CollationCaseFirst::Lower),
        Some("off") => Some(CollationCaseFirst::Off),
        _ => None,
    };
    let alternate = match dto.alternate.as_deref() {
        Some("non-ignorable") => Some(CollationAlternate::NonIgnorable),
        Some("shifted") => Some(CollationAlternate::Shifted),
        _ => None,
    };
    let max_variable = match dto.max_variable.as_deref() {
        Some("punct") => Some(CollationMaxVariable::Punct),
        Some("space") => Some(CollationMaxVariable::Space),
        _ => None,
    };
    mongodb::options::Collation::builder()
        .locale(dto.locale.clone())
        .strength(strength)
        .case_level(dto.case_level)
        .case_first(case_first)
        .numeric_ordering(dto.numeric_ordering)
        .alternate(alternate)
        .max_variable(max_variable)
        .normalization(dto.normalization)
        .backwards(dto.backwards)
        .build()
}

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;
const DEFAULT_SAMPLE: u32 = 200;

/// Per-page bound for the paging commands. Memory per page is bounded by
/// `page_size * avg_doc_size`, never by total collection size, so a generous
/// cap is safe and lets users who want a bigger initial window set one.
const MAX_PAGE_SIZE: u32 = 1000;
const DEFAULT_PAGE_SIZE: u32 = 50;

/// How the backend computes `total_count` for a `DocumentPage`. The default
/// (`Estimated`) hits collection metadata via `estimatedDocumentCount` and is
/// ~constant time regardless of collection size; `Exact` runs a real
/// `countDocuments` against the (optional) filter and is only used when the
/// user explicitly asks, since it is O(scan) on large collections; `None`
/// skips the count entirely for the lowest-latency first paint.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CountMode {
    #[default]
    Estimated,
    Exact,
    None,
}

#[tauri::command]
pub async fn list_databases(
    connection_id: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<DatabaseSummary>> {
    let entry = state.clients.get(&connection_id).await?;
    let names = entry.client.list_database_names().await?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let size = entry
            .client
            .database(&name)
            .run_command(bson::doc! { "dbStats": 1 })
            .await
            .ok()
            .and_then(|d| d.get_i64("dataSize").ok().map(|v| v as u64));
        let collections_count = entry
            .client
            .database(&name)
            .list_collection_names()
            .await
            .ok()
            .map(|c| c.len() as u64);
        out.push(DatabaseSummary {
            name,
            size_on_disk: size,
            collections_count,
        });
    }
    Ok(out)
}

#[tauri::command]
pub async fn list_collections(
    connection_id: String,
    database: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<CollectionSummary>> {
    let entry = state.clients.get(&connection_id).await?;
    registry_list_collections(&entry.client, &database).await
}

#[tauri::command]
pub async fn collection_stats(
    connection_id: String,
    database: String,
    collection: String,
    state: State<'_, AppState>,
) -> AppResult<CollectionStats> {
    let entry = state.clients.get(&connection_id).await?;
    let doc = entry
        .client
        .database(&database)
        .run_command(bson::doc! { "collStats": &collection })
        .await?;
    let count = entry
        .client
        .database(&database)
        .run_command(bson::doc! { "count": &collection })
        .await
        .ok()
        .and_then(|d| {
            d.get_i32("n")
                .ok()
                .map(|v| v as u64)
                .or_else(|| d.get_i64("n").ok().map(|v| v as u64))
        })
        .unwrap_or(0);
    Ok(CollectionStats {
        name: collection,
        document_count: count,
        size_bytes: doc.get_i64("size").map(|v| v as u64).unwrap_or(0),
        storage_size_bytes: doc.get_i64("storageSize").map(|v| v as u64).unwrap_or(0),
        index_count: doc.get_i32("nindexes").unwrap_or(0) as u32,
        total_index_size_bytes: doc.get_i64("totalIndexSize").map(|v| v as u64).unwrap_or(0),
        avg_obj_size_bytes: doc.get_i32("avgObjSize").map(|v| v as u64).unwrap_or(0),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: String,
    pub projection_json: Option<String>,
    pub sort_json: Option<String>,
    pub limit: Option<u32>,
    pub skip: Option<u64>,
}

#[tauri::command]
pub async fn find_documents(
    request: FindRequest,
    state: State<'_, AppState>,
) -> AppResult<DocumentPage> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let skip = request.skip.unwrap_or(0);
    let filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let projection = parse_optional_doc(request.projection_json.as_deref())?;
    let sort = parse_optional_doc(request.sort_json.as_deref())?;

    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let started = Instant::now();
    let mut find = coll.find(filter).limit(limit as i64).skip(skip);
    if let Some(p) = projection {
        find = find.projection(p);
    }
    if let Some(s) = sort {
        find = find.sort(s);
    }
    let cursor = find.await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    let has_more = docs.len() as u32 == limit;
    let elapsed = started.elapsed().as_millis() as u64;
    // Legacy single-shot find keeps an estimated count; the per-find full
    // collection scan (`count_documents({})`) it used to run was a latent
    // perf bug on large collections and the field was barely surfaced.
    let (total, approx) = estimate_count(&coll).await;
    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_find(
        &state.timeline,
        &ctx,
        &request.filter_json,
        docs.len() as u64,
        elapsed,
        false,
    )
    .await;

    Ok(DocumentPage {
        documents,
        limit,
        skip,
        has_more,
        execution_ms: Some(elapsed),
        total_count: total,
        total_count_approx: approx,
    })
}

/// Paged find request. Uses skip/limit paging: `skip = (page - 1) *
/// page_size`. `page_size` bounds one page (default 50, max 1000); memory is
/// bounded per page, not by total collection size. `sort_json` is honored
/// as-is; when absent the driver's natural order is used.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FindPageRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: String,
    pub projection_json: Option<String>,
    pub sort_json: Option<String>,
    /// 1-based page number. `skip = (page - 1) * page_size`. Default 1.
    #[serde(default = "default_page")]
    pub page: u64,
    pub page_size: Option<u32>,
    /// How to compute `total_count`. Defaults to `Estimated` (metadata,
    /// ~constant time).
    #[serde(default)]
    pub count_mode: CountMode,
}

fn default_page() -> u64 {
    1
}

/// Resolve the effective page size, clamped to `[1, MAX_PAGE_SIZE]` with the
/// default applied when unset. Kept tiny so every entry point shares one
/// definition of the bound.
fn resolve_page_size(req_size: Option<u32>) -> u32 {
    req_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

/// Fast approximate count via collection metadata. Returns `(count, true)`
/// so the UI can render the value with a leading "≈". Falls back to `(None,
/// true)` if the server refuses the estimate (e.g. on some Atlas tiers).
async fn estimate_count(coll: &mongodb::Collection<Document>) -> (Option<u64>, bool) {
    (coll.estimated_document_count().await.ok(), true)
}

/// Exact count against `filter` (O(scan)). Only used when the user opts in
/// via `CountMode::Exact`, typically for filtered queries where an estimate
/// would be misleading.
async fn exact_count(coll: &mongodb::Collection<Document>, filter: &Document) -> Option<u64> {
    coll.count_documents(filter.clone()).await.ok()
}

#[tauri::command]
pub async fn find_page(
    request: FindPageRequest,
    state: State<'_, AppState>,
) -> AppResult<DocumentPage> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let page_size = resolve_page_size(request.page_size);
    let user_filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let projection = parse_optional_doc(request.projection_json.as_deref())?;
    let user_sort = parse_optional_doc(request.sort_json.as_deref())?;
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);

    // Skip/limit paging. `skip = (page - 1) * page_size`. Deep pages are
    // O(skip) on the server, but this is the only correct strategy when the
    // user may supply an arbitrary sort; the UI caps practical depth via the
    // page controls.
    let skip = request.page.saturating_sub(1) * page_size as u64;

    let started = Instant::now();
    // Fetch one extra row to detect "more" without a count round-trip: if we
    // get back `page_size + 1` docs there is at least one more page, and we
    // trim the extra before returning. This keeps `has_more` accurate even
    // when the count is approximate or omitted.
    let fetch_limit = (page_size + 1) as i64;
    // `find` takes ownership of the filter; clone so the original stays
    // available for the exact-count path below.
    let mut find = coll.find(user_filter.clone()).limit(fetch_limit).skip(skip);
    if let Some(p) = projection {
        find = find.projection(p);
    }
    if let Some(s) = user_sort {
        find = find.sort(s);
    }
    // Match the export path's batch size so the driver pulls a full page per
    // network round-trip instead of the default 101-batch first getMore.
    find = find.batch_size(page_size);
    let cursor = find.await?;
    let mut docs: Vec<Document> = cursor.try_collect().await?;
    let has_more = docs.len() > page_size as usize;
    if has_more {
        docs.truncate(page_size as usize);
    }

    let elapsed = started.elapsed().as_millis() as u64;
    let (total, approx) = match request.count_mode {
        CountMode::None => (None, false),
        CountMode::Exact => match exact_count(&coll, &user_filter).await {
            Some(n) => (Some(n), false),
            None => estimate_count(&coll).await,
        },
        CountMode::Estimated => estimate_count(&coll).await,
    };

    // Record timeline entry only for the initial page (page 1) to avoid
    // flooding the timeline with pagination noise.
    if request.page <= 1 {
        let ctx = crate::mongo::operation_recorder::RecordContext::new(
            profile_id,
            request.connection_id.clone(),
            request.database.clone(),
            request.collection.clone(),
        );
        crate::mongo::operation_recorder::record_find(
            &state.timeline,
            &ctx,
            &request.filter_json,
            docs.len() as u64,
            elapsed,
            false,
        )
        .await;
    }

    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;
    Ok(DocumentPage {
        documents,
        limit: page_size,
        skip,
        has_more,
        execution_ms: Some(elapsed),
        total_count: total,
        total_count_approx: approx,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub pipeline_json: String,
    pub limit: Option<u32>,
}

#[tauri::command]
pub async fn aggregate_documents(
    request: AggregateRequest,
    state: State<'_, AppState>,
) -> AppResult<DocumentPage> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let mut pipeline: Vec<Document> = serde_json::from_str(&request.pipeline_json)?;
    if !pipeline.iter().any(|s| s.get("$limit").is_some()) {
        pipeline.push(bson::doc! { "$limit": limit as i64 });
    }
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let started = Instant::now();
    let cursor = coll.aggregate(pipeline).await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    let has_more = docs.len() as u32 == limit;
    let elapsed = started.elapsed().as_millis() as u64;
    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_aggregate(
        &state.timeline,
        &ctx,
        &request.pipeline_json,
        docs.len() as u64,
        elapsed,
        false,
    )
    .await;

    Ok(DocumentPage {
        documents,
        limit,
        skip: 0,
        has_more,
        execution_ms: Some(elapsed),
        total_count: None,
        total_count_approx: false,
    })
}

/// Paged aggregation request. Aggregation output has no guaranteed `_id`
/// order, so keyset paging does not apply — this command uses skip/limit
/// page jumping (`$skip`/`$limit` appended to the user pipeline). The count
/// is opt-in: `Estimated` is treated as `None` (an estimate of the collection
/// size is meaningless for a pipeline's output), and `Exact` runs the user
/// pipeline with a trailing `$count` stage in a second round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregatePageRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub pipeline_json: String,
    #[serde(default = "default_page")]
    pub page: u64,
    pub page_size: Option<u32>,
    #[serde(default)]
    pub count_mode: CountMode,
}

#[tauri::command]
pub async fn aggregate_page(
    request: AggregatePageRequest,
    state: State<'_, AppState>,
) -> AppResult<DocumentPage> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let page_size = resolve_page_size(request.page_size);
    let skip = request.page.saturating_sub(1) * page_size as u64;
    let mut pipeline: Vec<Document> = serde_json::from_str(&request.pipeline_json)?;
    // Append the paging stages at the end so they page the final output.
    // Fetch one extra to detect "more" without a count, matching `find_page`.
    if skip > 0 {
        pipeline.push(bson::doc! { "$skip": skip as i64 });
    }
    pipeline.push(bson::doc! { "$limit": (page_size + 1) as i64 });

    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let started = Instant::now();
    let cursor = coll.aggregate(pipeline).batch_size(page_size).await?;
    let mut docs: Vec<Document> = cursor.try_collect().await?;
    let has_more = docs.len() > page_size as usize;
    if has_more {
        docs.truncate(page_size as usize);
    }
    let elapsed = started.elapsed().as_millis() as u64;

    // Exact count = run the user pipeline (no paging stages) + `$count`.
    // Expensive (re-runs the pipeline), so only on explicit opt-in. Estimated
    // is meaningless for pipeline output, so it collapses to "no count".
    let total: Option<u64> = if matches!(request.count_mode, CountMode::Exact) {
        let mut count_pipeline: Vec<Document> = serde_json::from_str(&request.pipeline_json)?;
        count_pipeline.push(bson::doc! { "$count": "n" });
        match coll.aggregate(count_pipeline).await.ok() {
            Some(mut c) => {
                if c.advance().await.unwrap_or(false) {
                    c.deserialize_current().ok().and_then(|doc| {
                        doc.get_i64("n")
                            .ok()
                            .map(|v| v as u64)
                            .or_else(|| doc.get_i32("n").ok().map(|v| v as u64))
                    })
                } else {
                    None
                }
            }
            None => None,
        }
    } else {
        None
    };

    // Record timeline entry only for the initial page (page 1) to avoid
    // flooding the timeline with pagination noise.
    if request.page <= 1 {
        let ctx = crate::mongo::operation_recorder::RecordContext::new(
            profile_id,
            request.connection_id.clone(),
            request.database.clone(),
            request.collection.clone(),
        );
        crate::mongo::operation_recorder::record_aggregate(
            &state.timeline,
            &ctx,
            &request.pipeline_json,
            docs.len() as u64,
            elapsed,
            false,
        )
        .await;
    }

    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;
    Ok(DocumentPage {
        documents,
        limit: page_size,
        skip,
        has_more,
        execution_ms: Some(elapsed),
        total_count: total,
        // Aggregation counts are always exact when present.
        total_count_approx: false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: Option<String>,
}

#[tauri::command]
pub async fn count_documents(request: CountRequest, state: State<'_, AppState>) -> AppResult<u64> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(request.filter_json.as_deref())?.unwrap_or_default();
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    Ok(coll.count_documents(filter).await.unwrap_or(0))
}

#[tauri::command]
pub async fn list_indexes(
    connection_id: String,
    database: String,
    collection: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<IndexInfo>> {
    let entry = state.clients.get(&connection_id).await?;
    let coll = entry
        .client
        .database(&database)
        .collection::<Document>(&collection);
    let mut out = Vec::new();
    let mut cursor = coll.list_indexes().await?;
    while cursor.advance().await? {
        let model: mongodb::IndexModel = cursor.deserialize_current()?;
        let name = model
            .options
            .as_ref()
            .and_then(|o| o.name.clone())
            .unwrap_or_default();
        let key_doc = model.keys.clone();
        let key_json = doc_to_extjson(&key_doc)?;
        let unique = model
            .options
            .as_ref()
            .and_then(|o| o.unique)
            .unwrap_or(false);
        let sparse = model
            .options
            .as_ref()
            .and_then(|o| o.sparse)
            .unwrap_or(false);
        let ttl = model
            .options
            .as_ref()
            .and_then(|o| o.expire_after)
            .map(|d| d.as_secs() as i32);
        let partial = model
            .options
            .as_ref()
            .and_then(|o| o.partial_filter_expression.clone())
            .map(|d| doc_to_extjson(&d))
            .transpose()?;
        let hidden = model
            .options
            .as_ref()
            .and_then(|o| o.hidden)
            .unwrap_or(false);
        let collation = model
            .options
            .as_ref()
            .and_then(|o| o.collation.as_ref())
            .map(collation_from_driver);
        let wildcard_projection = model
            .options
            .as_ref()
            .and_then(|o| o.wildcard_projection.clone())
            .map(|d| doc_to_extjson(&d))
            .transpose()?;
        let is_text = key_doc
            .values()
            .any(|v| matches!(v, bson::Bson::String(s) if s == "text"));
        let is_geo = key_doc.values().any(|v| {
            matches!(v, bson::Bson::String(s) if s == "2dsphere" || s == "2d" || s == "geoHaystack")
        });
        let is_id = name == "_id_";
        out.push(IndexInfo {
            name,
            key: key_json,
            unique,
            sparse,
            hidden,
            ttl_seconds: ttl,
            partial_filter_expression: partial,
            collation,
            wildcard_projection,
            is_text,
            is_geo,
            is_id,
        });
    }
    Ok(out)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateIndexRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub name: String,
    pub key_json: String,
    pub unique: bool,
    pub sparse: bool,
    pub hidden: bool,
    pub ttl_seconds: Option<i32>,
    /// Optional partial-index filter, as Extended JSON. Parsed into a
    /// `Document` and forwarded to `IndexOptions::partial_filter_expression`.
    pub partial_filter_expression_json: Option<String>,
    /// Optional collation. `locale` is required when present.
    pub collation: Option<CollationDto>,
    /// Optional wildcard projection (`{"field": 1}` / `{"field": 0}`).
    pub wildcard_projection_json: Option<String>,
}

#[tauri::command]
pub async fn create_index(
    request: CreateIndexRequest,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let key: Document = serde_json::from_str(&request.key_json)?;
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let mut options = mongodb::options::IndexOptions::builder()
        .name(Some(request.name.clone()))
        .unique(request.unique)
        .sparse(request.sparse)
        .hidden(request.hidden)
        .build();
    if let Some(ttl) = request.ttl_seconds {
        options.expire_after = Some(std::time::Duration::from_secs(ttl as u64));
    }
    if let Some(json) = request.partial_filter_expression_json.as_deref() {
        let doc: Document = serde_json::from_str(json)?;
        options.partial_filter_expression = Some(doc);
    }
    if let Some(dto) = request.collation.as_ref() {
        if !dto.locale.trim().is_empty() {
            options.collation = Some(collation_to_driver(dto));
        }
    }
    if let Some(json) = request.wildcard_projection_json.as_deref() {
        let doc: Document = serde_json::from_str(json)?;
        options.wildcard_projection = Some(doc);
    }
    let model = mongodb::IndexModel::builder()
        .keys(key)
        .options(Some(options))
        .build();
    let started = Instant::now();
    coll.create_index(model).await?;
    let elapsed = started.elapsed().as_millis() as u64;
    let _ = crate::audit::interceptor::record_create_index(
        &state.audit_log,
        &request.database,
        &request.collection,
        &request.key_json,
        &serde_json::to_string(&serde_json::json!({
            "name": request.name,
            "unique": request.unique,
            "sparse": request.sparse,
            "hidden": request.hidden,
            "ttlSeconds": request.ttl_seconds,
            "partialFilterExpression": request.partial_filter_expression_json,
            "collation": request.collation,
            "wildcardProjection": request.wildcard_projection_json,
        }))
        .unwrap_or_default(),
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_index_create(
        &state.timeline,
        &ctx,
        &request.key_json,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(request.name)
}

#[tauri::command]
pub async fn drop_index(
    connection_id: String,
    database: String,
    collection: String,
    name: String,
    state: State<'_, AppState>,
) -> AppResult<()> {
    let entry = state.clients.get(&connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let coll = entry
        .client
        .database(&database)
        .collection::<Document>(&collection);
    let started = Instant::now();
    coll.drop_index(&name).await?;
    let elapsed = started.elapsed().as_millis() as u64;
    let _ = crate::audit::interceptor::record_drop_index(
        &state.audit_log,
        &database,
        &collection,
        &name,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        connection_id.clone(),
        database.clone(),
        collection.clone(),
    );
    crate::mongo::operation_recorder::record_index_drop(
        &state.timeline,
        &ctx,
        &name,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(())
}

/// Per-index usage statistics via the `$indexStats` aggregation stage.
/// Returns one `IndexStats` per index, ordered by name. Servers that do
/// not support `$indexStats` (e.g. MongoDB Serverless preview) will surface
/// an error to the caller; the frontend treats a failure as "no stats".
#[tauri::command]
pub async fn index_stats(
    connection_id: String,
    database: String,
    collection: String,
    state: State<'_, AppState>,
) -> AppResult<Vec<IndexStats>> {
    let entry = state.clients.get(&connection_id).await?;
    let coll = entry
        .client
        .database(&database)
        .collection::<Document>(&collection);
    let pipeline = vec![bson::doc! { "$indexStats": {} }];
    let mut cursor = coll.aggregate(pipeline).await?;
    let mut out = Vec::new();
    while cursor.advance().await? {
        let doc = cursor.deserialize_current()?;
        let name = doc.get_str("name").unwrap_or("").to_string();
        let ops = doc.get_i64("ops").unwrap_or(0);
        let since_ms = doc
            .get_datetime("since")
            .ok()
            .map(|dt| dt.timestamp_millis());
        let accesses = doc
            .get_document("accesses")
            .ok()
            .and_then(|a| a.get_i64("ops").ok());
        let size_bytes = doc.get_i64("size").ok();
        let building = doc.get_bool("building").ok();
        // Surface the rest of the row (e.g. `spec`, `metadata`) as raw JSON
        // so the UI can show extra detail without a schema bump per field.
        let metadata = doc_to_extjson(&doc).ok();
        out.push(IndexStats {
            name,
            ops,
            since_ms,
            accesses,
            size_bytes,
            building,
            metadata,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: String,
}

#[tauri::command]
pub async fn explain_find(
    request: ExplainRequest,
    state: State<'_, AppState>,
) -> AppResult<ExplainResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let doc = entry
        .client
        .database(&request.database)
        .run_command(bson::doc! {
            "explain": {
                "find": &request.collection,
                "filter": filter,
            },
            "verbosity": "executionStats",
        })
        .await?;
    let json = doc_to_extjson(&doc)?;
    let winning = doc
        .get_document("queryPlanner")
        .ok()
        .and_then(|qp| qp.get_document("winningPlan").ok().cloned())
        .map(|d| doc_to_extjson(&d))
        .transpose()?
        .unwrap_or(serde_json::Value::Null);
    let execution = doc
        .get_document("executionStats")
        .ok()
        .cloned()
        .map(|d| doc_to_extjson(&d))
        .transpose()?;
    Ok(ExplainResult {
        query_planner_winning_plan: winning,
        execution_stats: execution,
        raw: json,
    })
}

#[tauri::command]
pub async fn explain_aggregate(
    connection_id: String,
    database: String,
    collection: String,
    pipeline_json: String,
    state: State<'_, AppState>,
) -> AppResult<ExplainResult> {
    let entry = state.clients.get(&connection_id).await?;
    let pipeline: Vec<Document> = serde_json::from_str(&pipeline_json)?;
    let doc = entry
        .client
        .database(&database)
        .run_command(bson::doc! {
            "explain": {
                "aggregate": &collection,
                "pipeline": pipeline,
                "cursor": {},
            },
            "verbosity": "executionStats",
        })
        .await?;
    let json = doc_to_extjson(&doc)?;
    let winning = doc
        .get_document("queryPlanner")
        .ok()
        .and_then(|qp| qp.get_document("winningPlan").ok().cloned())
        .map(|d| doc_to_extjson(&d))
        .transpose()?
        .unwrap_or(json.clone());
    let execution = doc
        .get_document("executionStats")
        .ok()
        .cloned()
        .map(|d| doc_to_extjson(&d))
        .transpose()?;
    Ok(ExplainResult {
        query_planner_winning_plan: winning,
        execution_stats: execution,
        raw: json,
    })
}

#[tauri::command]
pub async fn sample_schema(
    connection_id: String,
    database: String,
    collection: String,
    state: State<'_, AppState>,
) -> AppResult<crate::mongo::schema::SchemaReport> {
    let entry = state.clients.get(&connection_id).await?;
    let coll = entry
        .client
        .database(&database)
        .collection::<Document>(&collection);
    let cursor = coll
        .aggregate(vec![
            bson::doc! { "$sample": { "size": DEFAULT_SAMPLE as i64 } },
        ])
        .await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    Ok(crate::mongo::schema::compute_schema_report(&docs))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleShapeRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub sample_size: Option<u32>,
}

#[tauri::command]
pub async fn sample_shape(
    request: SampleShapeRequest,
    state: State<'_, AppState>,
) -> AppResult<crate::mongo::shape::CollectionShape> {
    let entry = state.clients.get(&request.connection_id).await?;
    let db = entry.client.database(&request.database);
    let coll = db.collection::<Document>(&request.collection);
    let sample_size = request
        .sample_size
        .map(|s| s.clamp(1, 10000) as i64)
        .unwrap_or(DEFAULT_SAMPLE as i64);
    let cursor = coll
        .aggregate(vec![bson::doc! { "$sample": { "size": sample_size } }])
        .await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    let document_count = db
        .run_command(bson::doc! { "count": &request.collection })
        .await
        .ok()
        .and_then(|d| {
            d.get_i32("n")
                .ok()
                .map(|v| v as u64)
                .or_else(|| d.get_i64("n").ok().map(|v| v as u64))
        });
    Ok(crate::mongo::shape::compute_collection_shape(
        request.database,
        request.collection,
        CollectionKind::Collection,
        document_count,
        &docs,
        Vec::new(),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub document_json: String,
}

#[tauri::command]
pub async fn insert_document(
    request: InsertRequest,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let doc: Document = serde_json::from_str(&request.document_json)?;
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let start = Instant::now();
    let result = coll.insert_one(doc.clone()).await?;
    let elapsed = start.elapsed().as_millis() as u64;
    let id = result
        .inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .unwrap_or_else(|| {
            doc.get_object_id("_id")
                .map(|o| o.to_hex())
                .unwrap_or_default()
        });

    // Record audit event.
    let _ = crate::audit::interceptor::record_insert(
        &state.audit_log,
        &request.database,
        &request.collection,
        &request.document_json,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_insert(
        &state.timeline,
        &ctx,
        crate::mongo::timeline_store::OperationKind::InsertOne,
        &request.document_json,
        1,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: String,
    pub update_json: String,
    pub multi: bool,
    pub upsert: bool,
}

#[tauri::command]
pub async fn update_documents(
    request: UpdateRequest,
    state: State<'_, AppState>,
) -> AppResult<UpdateResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let update = parse_optional_doc(Some(&request.update_json))?
        .ok_or_else(|| AppError::InvalidBson("update document must be a JSON object".into()))?;
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let opts = mongodb::options::UpdateOptions::builder().upsert(request.upsert).build();
    let start = Instant::now();
    let res = if request.multi {
        coll.update_many(filter, update).with_options(opts).await?
    } else {
        coll.update_one(filter, update).with_options(opts).await?
    };
    let elapsed = start.elapsed().as_millis() as u64;

    // Record audit event.
    let _ = crate::audit::interceptor::record_update(
        &state.audit_log,
        &request.database,
        &request.collection,
        &request.filter_json,
        &request.update_json,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    let update_kind = if request.multi {
        crate::mongo::timeline_store::OperationKind::UpdateMany
    } else {
        crate::mongo::timeline_store::OperationKind::UpdateOne
    };
    crate::mongo::operation_recorder::record_update(
        &state.timeline,
        &ctx,
        update_kind,
        &request.filter_json,
        &request.update_json,
        res.matched_count,
        res.modified_count,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(UpdateResult {
        matched_count: res.matched_count,
        modified_count: res.modified_count,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: String,
    pub replacement_json: String,
    pub upsert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceResult {
    pub matched_count: u64,
    pub modified_count: u64,
    pub upserted_id: Option<String>,
}

#[tauri::command]
pub async fn replace_document(
    request: ReplaceRequest,
    state: State<'_, AppState>,
) -> AppResult<ReplaceResult> {
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let replacement: Document = serde_json::from_str(&request.replacement_json)?;
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let opts = mongodb::options::ReplaceOptions::builder().upsert(request.upsert).build();
    let start = Instant::now();
    let res = coll.replace_one(filter, replacement).with_options(opts).await?;
    let elapsed = start.elapsed().as_millis() as u64;

    let _ = crate::audit::interceptor::record_update(
        &state.audit_log,
        &request.database,
        &request.collection,
        &request.filter_json,
        &request.replacement_json,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_replace_one(
        &state.timeline,
        &ctx,
        &request.filter_json,
        &request.replacement_json,
        res.matched_count,
        res.modified_count,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(ReplaceResult {
        matched_count: res.matched_count,
        modified_count: res.modified_count,
        upserted_id: res.upserted_id.as_ref().and_then(|id| match id {
            bson::Bson::ObjectId(oid) => Some(oid.to_hex()),
            bson::Bson::String(s) => Some(s.clone()),
            _ => Some(id.to_string()),
        }),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertManyRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub documents_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertManyResult {
    pub inserted_count: u64,
    pub inserted_ids: Vec<String>,
}

#[tauri::command]
pub async fn insert_many_documents(
    request: InsertManyRequest,
    state: State<'_, AppState>,
) -> AppResult<InsertManyResult> {
    let docs: Vec<Document> = serde_json::from_str(&request.documents_json)?;
    if docs.is_empty() {
        return Err(AppError::Validation("documents array must not be empty".into()));
    }
    let entry = state.clients.get(&request.connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    let start = Instant::now();
    let result = coll.insert_many(docs).await?;
    let elapsed = start.elapsed().as_millis() as u64;
    let ids: Vec<String> = result
        .inserted_ids
        .values()
        .map(|id| match id {
            bson::Bson::ObjectId(oid) => oid.to_hex(),
            bson::Bson::String(s) => s.clone(),
            _ => id.to_string(),
        })
        .collect();

    let _ = crate::audit::interceptor::record_insert(
        &state.audit_log,
        &request.database,
        &request.collection,
        &request.documents_json,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        request.connection_id.clone(),
        request.database.clone(),
        request.collection.clone(),
    );
    crate::mongo::operation_recorder::record_insert_many(
        &state.timeline,
        &ctx,
        &request.documents_json,
        ids.len() as u64,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(InsertManyResult {
        inserted_count: ids.len() as u64,
        inserted_ids: ids,
    })
}

/// Result of an update operation, distinguishing documents that matched
/// the filter from those that were actually modified. `matched_count`
/// lets the frontend tell a true no-match (filter missed — likely a
/// round-trip bug) from a no-op match (doc matched, value unchanged).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    pub matched_count: u64,
    pub modified_count: u64,
}

#[tauri::command]
pub async fn delete_documents(
    connection_id: String,
    database: String,
    collection: String,
    filter_json: String,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    let entry = state.clients.get(&connection_id).await?;
    let profile_id = entry.profile_id.clone();
    let filter = parse_optional_doc(Some(&filter_json))?.unwrap_or_default();
    let coll = entry
        .client
        .database(&database)
        .collection::<Document>(&collection);
    let start = Instant::now();
    let res = coll.delete_many(filter).await?;
    let elapsed = start.elapsed().as_millis() as u64;

    // Record audit event.
    let _ = crate::audit::interceptor::record_delete(
        &state.audit_log,
        &database,
        &collection,
        &filter_json,
    );

    // Record timeline entry.
    let ctx = crate::mongo::operation_recorder::RecordContext::new(
        profile_id,
        connection_id.clone(),
        database.clone(),
        collection.clone(),
    );
    crate::mongo::operation_recorder::record_delete(
        &state.timeline,
        &ctx,
        crate::mongo::timeline_store::OperationKind::DeleteMany,
        &filter_json,
        res.deleted_count,
        elapsed,
        false,
        None,
    )
    .await;

    Ok(res.deleted_count)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: Option<String>,
}

#[tauri::command]
pub async fn preview_delete(request: PreviewRequest, state: State<'_, AppState>) -> AppResult<u64> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(request.filter_json.as_deref())?.unwrap_or_default();
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    Ok(coll.count_documents(filter).await.unwrap_or(0))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewUpdateRequest {
    pub connection_id: String,
    pub database: String,
    pub collection: String,
    pub filter_json: Option<String>,
    pub update_json: String,
}

#[tauri::command]
pub async fn preview_update(
    request: PreviewUpdateRequest,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    // Validate update JSON is a valid BSON document first.
    let _update = parse_optional_doc(Some(&request.update_json))?
        .ok_or_else(|| AppError::InvalidBson("update document must be a JSON object".into()))?;
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(request.filter_json.as_deref())?.unwrap_or_default();
    let coll = entry
        .client
        .database(&request.database)
        .collection::<Document>(&request.collection);
    Ok(coll.count_documents(filter).await.unwrap_or(0))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VqbTranslateRequest {
    pub node: crate::mongo::vqb::VqbNode,
}

#[tauri::command]
pub async fn translate_vqb(request: VqbTranslateRequest) -> AppResult<serde_json::Value> {
    crate::mongo::vqb::to_filter(&request.node)
}

#[allow(dead_code)]
fn _kind_marker(_k: CollectionKind) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collation_round_trips_through_driver() {
        let dto = CollationDto {
            locale: "de".to_string(),
            strength: Some(2),
            case_level: Some(true),
            case_first: Some("lower".to_string()),
            numeric_ordering: Some(true),
            alternate: Some("shifted".to_string()),
            max_variable: Some("space".to_string()),
            normalization: Some(true),
            backwards: Some(false),
        };
        let driver = collation_to_driver(&dto);
        let back = collation_from_driver(&driver);
        assert_eq!(back, dto);
    }

    #[test]
    fn collation_strength_maps_to_named_levels() {
        for (raw, expected) in [(1i32, 1u32), (2, 2), (3, 3), (4, 4), (5, 5)] {
            let dto = CollationDto {
                locale: "en".to_string(),
                strength: Some(raw),
                ..Default::default()
            };
            let driver = collation_to_driver(&dto);
            let back = collation_from_driver(&driver);
            assert_eq!(back.strength, Some(expected as i32), "strength {raw}");
        }
    }

    #[test]
    fn collation_unknown_strings_degrade_to_none() {
        // A malformed payload must not panic or error; unknown enum
        // strings collapse to None so the create-index call still works
        // with the locale + the fields that did parse.
        let dto = CollationDto {
            locale: "en".to_string(),
            case_first: Some("bogus".to_string()),
            alternate: Some("nope".to_string()),
            max_variable: Some("garbage".to_string()),
            strength: Some(99), // out of range
            ..Default::default()
        };
        let driver = collation_to_driver(&dto);
        let back = collation_from_driver(&driver);
        assert_eq!(back.locale, "en");
        assert_eq!(back.strength, None);
        assert_eq!(back.case_first, None);
        assert_eq!(back.alternate, None);
        assert_eq!(back.max_variable, None);
    }

    #[test]
    fn collation_locale_only_preserves_locale() {
        let dto = CollationDto {
            locale: "ja".to_string(),
            ..Default::default()
        };
        let back = collation_from_driver(&collation_to_driver(&dto));
        assert_eq!(back, dto);
    }

    // ---- DTO validation tests for new write commands ----

    #[test]
    fn update_request_serializes_upsert() {
        let req = UpdateRequest {
            connection_id: "c1".into(),
            database: "db".into(),
            collection: "coll".into(),
            filter_json: "{}".into(),
            update_json: "{ \"$set\": { \"x\": 1 } }".into(),
            multi: true,
            upsert: true,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("\"upsert\":true"), "upsert must serialize as camelCase");
        assert!(json.contains("\"multi\":true"));
    }

    #[test]
    fn replace_request_serializes_and_round_trips() {
        let req = ReplaceRequest {
            connection_id: "c1".into(),
            database: "db".into(),
            collection: "coll".into(),
            filter_json: "{ \"_id\": \"abc\" }".into(),
            replacement_json: "{ \"name\": \"new\" }".into(),
            upsert: false,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let back: ReplaceRequest = serde_json::from_str(&json).expect("deserialize");
        assert!(!back.upsert);
        assert_eq!(back.filter_json, req.filter_json);
    }

    #[test]
    fn replace_result_handles_object_id() {
        let res = ReplaceResult {
            matched_count: 1,
            modified_count: 1,
            upserted_id: Some("abc123".into()),
        };
        let json = serde_json::to_string(&res).expect("serialize");
        assert!(json.contains("\"upsertedId\":\"abc123\""));
    }

    #[test]
    fn insert_many_request_parses_document_array() {
        let req = InsertManyRequest {
            connection_id: "c1".into(),
            database: "db".into(),
            collection: "coll".into(),
            documents_json: "[{\"a\":1},{\"b\":2}]".into(),
        };
        let docs: Vec<Document> = serde_json::from_str(&req.documents_json).expect("parse docs");
        assert_eq!(docs.len(), 2);
    }

    #[test]
    fn insert_many_result_serializes_ids() {
        let res = InsertManyResult {
            inserted_count: 2,
            inserted_ids: vec!["id1".into(), "id2".into()],
        };
        let json = serde_json::to_string(&res).expect("serialize");
        let back: InsertManyResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.inserted_count, 2);
        assert_eq!(back.inserted_ids.len(), 2);
    }

    #[test]
    fn preview_update_request_validates_update_json() {
        // Verify that parse_optional_doc rejects non-object JSON.
        let bad = "\"not an object\"";
        let doc = parse_optional_doc(Some(bad));
        assert!(doc.is_err() || doc.unwrap().is_none());
    }
}
