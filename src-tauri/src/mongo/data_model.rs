//! Data-model graph: scan one database, infer recursive shapes, list indexes,
//! detect relationships, and return a serializable graph. The graph is the
//! source of truth for the diagram canvas, the shape tree, and the
//! relationships table.

use std::collections::HashMap;

use bson::{doc, Bson, Document};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::mongo::client_registry::classify_collection_name;
use crate::mongo::relationship::{
    detect_relationships, AppSchemaSignal, LookupSignal, ObjectIdMatchSignal, RelationshipConfig,
    RelationshipEdge,
};
use crate::mongo::shape::{compute_collection_shape, CollectionShape};
use crate::mongo::types::{CollectionKind, IndexInfo};

/// Full inferred model for one database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataModelGraph {
    pub database: String,
    pub nodes: Vec<CollectionShape>,
    pub edges: Vec<RelationshipEdge>,
    pub generated_at: String,
    pub sample_size: u32,
    pub confidence_threshold: f64,
    pub warnings: Vec<String>,
}

/// Scan configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanConfig {
    pub database: String,
    pub collections: Vec<String>,
    pub sample_size: u32,
    pub signals: SignalConfig,
    pub confidence_threshold: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalConfig {
    pub object_id_match: bool,
    pub naming: bool,
    pub lookup: bool,
    pub index: bool,
    pub app_schema: bool,
}

/// Scan a database and build a `DataModelGraph`. `lookup_signals` and
/// `app_schema_signals` are provided by the caller (query history / views /
/// optional schema overlay). Returns the graph and a per-collection warning list.
#[allow(clippy::too_many_arguments)]
pub async fn scan_database_model(
    client: &mongodb::Client,
    config: &ScanConfig,
    lookup_signals: &[LookupSignal],
    app_schema_signals: &[AppSchemaSignal],
) -> AppResult<DataModelGraph> {
    let db = client.database(&config.database);
    let total = config.collections.len();
    let mut nodes: Vec<CollectionShape> = Vec::with_capacity(total);
    let mut warnings: Vec<String> = Vec::new();

    for (i, _name) in config.collections.iter().enumerate() {
        let _ = i; // progress is emitted via Tauri events in a future iteration
        let name = _name;
        let kind: CollectionKind = classify_collection_name(client, &config.database, name).await;
        let coll = db.collection::<Document>(name);
        let sample = config.sample_size.clamp(1, 10_000) as i64;
        let docs: Vec<Document> = match coll
            .aggregate(vec![doc! { "$sample": { "size": sample } }])
            .await
        {
            Ok(cursor) => cursor.try_collect().await.unwrap_or_default(),
            Err(e) => {
                warnings.push(format!("{}: sample failed: {}", name, e));
                continue;
            }
        };
        let document_count = db
            .run_command(doc! { "count": name })
            .await
            .ok()
            .and_then(|d| {
                d.get_i32("n")
                    .ok()
                    .map(|v| v as u64)
                    .or_else(|| d.get_i64("n").ok().map(|v| v as u64))
            });
        let indexes = match list_indexes_for_collection(client, &config.database, name).await {
            Ok(idx) => idx,
            Err(e) => {
                warnings.push(format!("{}: list indexes failed: {}", name, e));
                Vec::new()
            }
        };
        nodes.push(compute_collection_shape(
            config.database.clone(),
            name.clone(),
            kind,
            document_count,
            &docs,
            indexes,
        ));
    }

    let mut object_id_signals: Vec<ObjectIdMatchSignal> = Vec::new();
    if config.signals.object_id_match {
        object_id_signals = collect_object_id_matches(client, &config.database, &nodes).await?;
    }

    let rel_config = RelationshipConfig {
        confidence_threshold: config.confidence_threshold,
        enable_object_id_match: config.signals.object_id_match,
        enable_naming: config.signals.naming,
        enable_lookup: config.signals.lookup,
        enable_index: config.signals.index,
        enable_app_schema: config.signals.app_schema,
        object_id_match_threshold: 0.5,
    };

    let edges = detect_relationships(&nodes, &object_id_signals, lookup_signals, app_schema_signals, &rel_config);

    Ok(DataModelGraph {
        database: config.database.clone(),
        nodes,
        edges,
        generated_at: chrono::Utc::now().to_rfc3339(),
        sample_size: config.sample_size,
        confidence_threshold: config.confidence_threshold,
        warnings,
    })
}

/// List indexes for a single collection.
async fn list_indexes_for_collection(
    client: &mongodb::Client,
    database: &str,
    collection: &str,
) -> AppResult<Vec<IndexInfo>> {
    let coll = client.database(database).collection::<Document>(collection);
    let mut out = Vec::new();
    let mut cursor = coll.list_indexes().await?;
    while cursor.advance().await? {
        let model: mongodb::IndexModel = cursor.deserialize_current()?;
        let name = model
            .options
            .as_ref()
            .and_then(|o| o.name.clone())
            .unwrap_or_default();
        let key = serde_json::to_value(&model.keys)?;
        let unique = model.options.as_ref().and_then(|o| o.unique).unwrap_or(false);
        let sparse = model.options.as_ref().and_then(|o| o.sparse).unwrap_or(false);
        let hidden = model.options.as_ref().and_then(|o| o.hidden).unwrap_or(false);
        let ttl = model
            .options
            .as_ref()
            .and_then(|o| o.expire_after)
            .map(|d| d.as_secs() as i32);
        let partial = model
            .options
            .as_ref()
            .and_then(|o| o.partial_filter_expression.clone())
            .map(|d| serde_json::to_value(&d).unwrap_or(serde_json::Value::Null));
        let collation = None; // simplified for shape
        let wildcard = model
            .options
            .as_ref()
            .and_then(|o| o.wildcard_projection.clone())
            .map(|d| serde_json::to_value(&d).unwrap_or(serde_json::Value::Null));
        let is_id = name == "_id_";
        out.push(IndexInfo {
            name,
            key,
            unique,
            sparse,
            hidden,
            ttl_seconds: ttl,
            partial_filter_expression: partial,
            collation,
            wildcard_projection: wildcard,
            is_text: false,
            is_geo: false,
            is_id,
        });
    }
    Ok(out)
}

/// Collect bounded ObjectId cross-match signals. For each objectId field, we
/// sample values and test them against candidate `_id` collections. The number
/// of queries is capped by only running the match for fields that have a naming
/// hint and by limiting the total candidate checks.
async fn collect_object_id_matches(
    client: &mongodb::Client,
    database: &str,
    nodes: &[CollectionShape],
) -> AppResult<Vec<ObjectIdMatchSignal>> {
    let mut out = Vec::new();
    let mut queries = 0usize;
    const MAX_QUERIES: usize = 100;
    const SAMPLE_PER_FIELD: usize = 50;

    let collection_names: Vec<String> = nodes.iter().map(|n| n.collection.clone()).collect();
    let node_by_name: HashMap<String, &CollectionShape> =
        nodes.iter().map(|n| (n.collection.clone(), n)).collect();

    for shape in nodes {
        let fields = collect_object_id_fields(&shape.root);
        for (field_path, is_array) in fields {
            if queries >= MAX_QUERIES {
                break;
            }
            let candidates = naming_candidates(&field_path, &collection_names);
            let values = sample_object_ids(client, database, &shape.collection, &field_path, is_array, SAMPLE_PER_FIELD).await;
            if values.is_empty() {
                continue;
            }
            for target in candidates {
                if queries >= MAX_QUERIES {
                    break;
                }
                let target_shape = match node_by_name.get(&target) {
                    Some(n) => n,
                    None => continue,
                };
                // Only run if the target's _id is objectId.
                if !id_is_object_id(target_shape) {
                    continue;
                }
                let matched = count_id_matches(client, database, &target, &values).await?;
                let ratio = matched as f64 / values.len() as f64;
                out.push(ObjectIdMatchSignal {
                    from_collection: shape.collection.clone(),
                    from_field: field_path.clone(),
                    to_collection: target,
                    to_field: "_id".to_string(),
                    match_ratio: ratio,
                    sampled: values.len() as u64,
                    matched,
                });
                queries += 1;
            }
        }
    }
    Ok(out)
}

/// Recursively collect all field paths whose dominant type is objectId or array of objectId.
fn collect_object_id_fields(node: &crate::mongo::shape::ShapeNode) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for child in &node.children {
        collect_object_id_fields_inner(child, &mut out);
    }
    out
}

fn collect_object_id_fields_inner(
    node: &crate::mongo::shape::ShapeNode,
    out: &mut Vec<(String, bool)>,
) {
    let dominant = node.types.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap());
    let is_object_id = dominant.map(|(t, _)| t == "objectId").unwrap_or(false);
    let is_array = node.types.contains_key("array");
    let is_array_of_object_ids = is_array
        && node
            .array_item
            .as_ref()
            .map(|item| {
                item.types
                    .iter()
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                    .map(|(t, _)| t == "objectId")
                    .unwrap_or(false)
            })
            .unwrap_or(false);

    if is_object_id || is_array_of_object_ids {
        out.push((node.path.clone(), is_array_of_object_ids));
    }

    for child in &node.children {
        collect_object_id_fields_inner(child, out);
    }
    if let Some(item) = &node.array_item {
        collect_object_id_fields_inner(item, out);
    }
}

fn naming_candidates(field_path: &str, collections: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    let set: HashSet<String> = collections.iter().cloned().collect();
    crate::mongo::relationship::naming_match(field_path, &set)
        .into_iter()
        .collect()
}

fn id_is_object_id(shape: &CollectionShape) -> bool {
    shape
        .root
        .children
        .iter()
        .find(|c| c.name == "_id")
        .map(|c| {
            c.types
                .iter()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .map(|(t, _)| t == "objectId")
                .unwrap_or(false)
        })
        .unwrap_or(false)
}

async fn sample_object_ids(
    client: &mongodb::Client,
    database: &str,
    collection: &str,
    field_path: &str,
    _is_array: bool,
    limit: usize,
) -> Vec<bson::oid::ObjectId> {
    let coll = client.database(database).collection::<Document>(collection);
    // Use $sample to get documents, then extract the field. For nested/array
    // paths this is a best-effort extraction; top-level paths are fully
    // supported.
    let pipeline = vec![
        doc! { "$sample": { "size": limit as i64 } },
        doc! { "$project": { "v": format!("${}", field_path) } },
    ];
    let Ok(cursor) = coll.aggregate(pipeline).await else {
        return Vec::new();
    };
    let Ok(docs) = cursor.try_collect::<Vec<Document>>().await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for doc in docs {
        if let Some(v) = doc.get("v") {
            match v {
                Bson::ObjectId(oid) => out.push(*oid),
                Bson::Array(arr) => {
                    for elem in arr.iter().take(4) {
                        if let Bson::ObjectId(oid) = elem {
                            out.push(*oid);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

async fn count_id_matches(
    client: &mongodb::Client,
    database: &str,
    collection: &str,
    ids: &[bson::oid::ObjectId],
) -> AppResult<u64> {
    let coll = client.database(database).collection::<Document>(collection);
    let bson_ids: Vec<Bson> = ids.iter().map(|id| Bson::ObjectId(*id)).collect();
    let count = coll
        .count_documents(doc! { "_id": { "$in": bson_ids } })
        .await?;
    Ok(count)
}

