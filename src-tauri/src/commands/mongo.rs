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
    CollectionKind, CollectionStats, CollectionSummary, DatabaseSummary, DocumentPage,
    ExplainResult, IndexInfo,
};
use crate::state::AppState;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;
const DEFAULT_SAMPLE: u32 = 200;

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
    let total = coll.count_documents(bson::doc! {}).await.ok();
    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;
    Ok(DocumentPage {
        documents,
        limit,
        skip,
        has_more,
        execution_ms: Some(elapsed),
        total_count: total,
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
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let mut pipeline: Vec<Document> = serde_json::from_str(&request.pipeline_json)?;
    if !pipeline.iter().any(|s| s.get("$limit").is_some()) {
        pipeline.push(bson::doc! { "$limit": limit as i64 });
    }
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
    let started = Instant::now();
    let cursor = coll.aggregate(pipeline).await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    let has_more = docs.len() as u32 == limit;
    let elapsed = started.elapsed().as_millis() as u64;
    let documents = docs
        .iter()
        .map(doc_to_display_json)
        .collect::<AppResult<Vec<_>>>()?;
    Ok(DocumentPage {
        documents,
        limit,
        skip: 0,
        has_more,
        execution_ms: Some(elapsed),
        total_count: None,
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
pub async fn count_documents(
    request: CountRequest,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(request.filter_json.as_deref())?.unwrap_or_default();
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
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
    let coll = entry.client.database(&database).collection::<Document>(&collection);
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
            ttl_seconds: ttl,
            partial_filter_expression: partial,
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
    pub ttl_seconds: Option<i32>,
}

#[tauri::command]
pub async fn create_index(
    request: CreateIndexRequest,
    state: State<'_, AppState>,
) -> AppResult<String> {
    let entry = state.clients.get(&request.connection_id).await?;
    let key: Document = serde_json::from_str(&request.key_json)?;
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
    let mut options = mongodb::options::IndexOptions::builder()
        .name(Some(request.name.clone()))
        .unique(request.unique)
        .sparse(request.sparse)
        .build();
    if let Some(ttl) = request.ttl_seconds {
        options.expire_after = Some(std::time::Duration::from_secs(ttl as u64));
    }
    let model = mongodb::IndexModel::builder()
        .keys(key)
        .options(Some(options))
        .build();
    coll.create_index(model).await?;
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
    let coll = entry.client.database(&database).collection::<Document>(&collection);
    coll.drop_index(name).await?;
    Ok(())
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
    let coll = entry.client.database(&database).collection::<Document>(&collection);
    let cursor = coll.aggregate(vec![bson::doc! { "$sample": { "size": DEFAULT_SAMPLE as i64 } }]).await?;
    let docs: Vec<Document> = cursor.try_collect().await?;
    Ok(crate::mongo::schema::compute_schema_report(&docs))
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
    let doc: Document = serde_json::from_str(&request.document_json)?;
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
    let result = coll.insert_one(doc.clone()).await?;
    let id = result
        .inserted_id
        .as_object_id()
        .map(|o| o.to_hex())
        .unwrap_or_else(|| doc.get_object_id("_id").map(|o| o.to_hex()).unwrap_or_default());
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
}

#[tauri::command]
pub async fn update_documents(
    request: UpdateRequest,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(Some(&request.filter_json))?.unwrap_or_default();
    let update = parse_optional_doc(Some(&request.update_json))?.ok_or_else(|| {
        AppError::InvalidBson("update document must be a JSON object".into())
    })?;
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
    let res = if request.multi {
        coll.update_many(filter, update).await?
    } else {
        coll.update_one(filter, update).await?
    };
    Ok(res.modified_count)
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
    let filter = parse_optional_doc(Some(&filter_json))?.unwrap_or_default();
    let coll = entry.client.database(&database).collection::<Document>(&collection);
    let res = coll.delete_many(filter).await?;
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
pub async fn preview_delete(
    request: PreviewRequest,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    let entry = state.clients.get(&request.connection_id).await?;
    let filter = parse_optional_doc(request.filter_json.as_deref())?.unwrap_or_default();
    let coll = entry.client.database(&request.database).collection::<Document>(&request.collection);
    Ok(coll.count_documents(filter).await.unwrap_or(0))
}

#[tauri::command]
pub async fn preview_update(
    request: PreviewRequest,
    state: State<'_, AppState>,
) -> AppResult<u64> {
    preview_delete(request, state).await
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
