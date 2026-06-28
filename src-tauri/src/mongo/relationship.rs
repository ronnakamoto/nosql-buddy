//! Relationship detection between collections. All detection here is pure: it
//! takes a set of `CollectionShape`s plus precomputed signals (naming matches,
//! $lookup usage, application schema refs, and ObjectId cross-match results)
//! and produces a ranked list of `RelationshipEdge`s with confidence + evidence.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::mongo::shape::{CollectionShape, ShapeNode};

/// Tunable thresholds for the detector.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipConfig {
    pub confidence_threshold: f64,
    pub enable_object_id_match: bool,
    pub enable_naming: bool,
    pub enable_lookup: bool,
    pub enable_index: bool,
    pub enable_app_schema: bool,
    pub object_id_match_threshold: f64,
}

impl Default for RelationshipConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.4,
            enable_object_id_match: true,
            enable_naming: true,
            enable_lookup: true,
            enable_index: true,
            enable_app_schema: true,
            object_id_match_threshold: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RelationshipKind {
    #[serde(rename = "one-to-one")]
    OneToOne,
    #[serde(rename = "one-to-many")]
    OneToMany,
    #[serde(rename = "many-to-one")]
    ManyToOne,
    #[serde(rename = "many-to-many")]
    ManyToMany,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SignalKind {
    #[serde(rename = "objectIdMatch")]
    ObjectIdMatch,
    #[serde(rename = "namingConvention")]
    NamingConvention,
    #[serde(rename = "lookup")]
    Lookup,
    #[serde(rename = "index")]
    Index,
    #[serde(rename = "appSchema")]
    AppSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipSignal {
    pub kind: SignalKind,
    pub detail: String,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipEdge {
    pub id: String,
    pub from_collection: String,
    pub to_collection: String,
    pub from_field: String,
    pub to_field: String,
    pub kind: RelationshipKind,
    pub confidence: f64,
    pub signals: Vec<RelationshipSignal>,
    /// Optional intermediate collection for many-to-many.
    pub via_collection: Option<String>,
    /// User override; if `true`, confidence is treated as 1.0.
    pub confirmed: bool,
    /// User override; if `true`, excluded from the canvas.
    pub hidden: bool,
}

/// A $lookup or view-derived relationship signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookupSignal {
    pub from_collection: String,
    pub to_collection: String,
    pub local_field: String,
    pub foreign_field: String,
    pub count: u32,
}

/// An application schema (Mongoose / Prisma) reference signal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSchemaSignal {
    pub from_collection: String,
    pub to_collection: String,
    pub from_field: String,
    pub to_field: String,
    pub is_array: bool,
    pub source: String,
}

/// Result of sampling ObjectId values and testing them against candidate `_id`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectIdMatchSignal {
    pub from_collection: String,
    pub from_field: String,
    pub to_collection: String,
    pub to_field: String,
    pub match_ratio: f64,
    pub sampled: u64,
    pub matched: u64,
}

/// Pure relationship detector. Combines all available signals.
#[allow(clippy::too_many_arguments)]
pub fn detect_relationships(
    shapes: &[CollectionShape],
    object_id_signals: &[ObjectIdMatchSignal],
    lookup_signals: &[LookupSignal],
    app_schema_signals: &[AppSchemaSignal],
    config: &RelationshipConfig,
) -> Vec<RelationshipEdge> {
    let collection_names: HashSet<String> =
        shapes.iter().map(|s| s.collection.clone()).collect();
    let shape_by_name: HashMap<String, &CollectionShape> =
        shapes.iter().map(|s| (s.collection.clone(), s)).collect();

    // Candidate ref fields: every field whose dominant type is objectId (or
    // array of objectId), plus fields that have a naming match.
    let mut candidate_map: BTreeMap<(String, String), Candidate> = BTreeMap::new();

    // 1. Collect objectId fields from shapes.
    for shape in shapes {
        collect_object_id_fields(&shape.root, &shape.collection, &mut candidate_map);
    }

    // 2. Naming convention signal.
    if config.enable_naming {
        for (key, cand) in candidate_map.iter_mut() {
            if let Some(target) = naming_match(&key.1, &collection_names) {
                if target != key.0 {
                    cand.naming_target = Some(target);
                    cand.naming_suffix = naming_suffix(&key.1);
                }
            }
        }
    }

    // 3. Lookup signal.
    if config.enable_lookup {
        for sig in lookup_signals {
            let key = (sig.from_collection.clone(), sig.local_field.clone());
            if let Some(cand) = candidate_map.get_mut(&key) {
                cand.lookup_targets
                    .insert((sig.to_collection.clone(), sig.foreign_field.clone()));
            } else {
                let mut cand = Candidate::new(sig.from_collection.clone(), sig.local_field.clone());
                cand.lookup_targets
                    .insert((sig.to_collection.clone(), sig.foreign_field.clone()));
                candidate_map.insert(key, cand);
            }
        }
    }

    // 4. App schema signal.
    if config.enable_app_schema {
        for sig in app_schema_signals {
            let key = (sig.from_collection.clone(), sig.from_field.clone());
            let cand = candidate_map
                .entry(key.clone())
                .or_insert_with(|| Candidate::new(sig.from_collection.clone(), sig.from_field.clone()));
            cand.app_schema_target = Some(AppSchemaTarget {
                to_collection: sig.to_collection.clone(),
                to_field: sig.to_field.clone(),
                is_array: sig.is_array,
                source: sig.source.clone(),
            });
        }
    }

    // 5. Index signal.
    if config.enable_index {
        for shape in shapes {
            for idx in &shape.indexes {
                if let Some(obj) = idx.key.as_object() {
                    let keys: Vec<String> = obj.keys().cloned().collect();
                    if keys.len() == 1 {
                        let field = keys.into_iter().next().unwrap();
                        let key = (shape.collection.clone(), field);
                        if let Some(cand) = candidate_map.get_mut(&key) {
                            cand.indexed = true;
                            cand.index_unique = idx.unique;
                        }
                    }
                }
            }
        }
    }

    // 6. ObjectId match signal.
    if config.enable_object_id_match {
        for sig in object_id_signals {
            if sig.match_ratio >= config.object_id_match_threshold {
                let key = (sig.from_collection.clone(), sig.from_field.clone());
                let cand = candidate_map
                    .entry(key.clone())
                    .or_insert_with(|| Candidate::new(sig.from_collection.clone(), sig.from_field.clone()));
                cand.object_id_matches.push(ObjectIdMatchDetail {
                    to_collection: sig.to_collection.clone(),
                    to_field: sig.to_field.clone(),
                    match_ratio: sig.match_ratio,
                    sampled: sig.sampled,
                    matched: sig.matched,
                });
            }
        }
    }

    // Build edges.
    let mut edges: Vec<RelationshipEdge> = Vec::new();
    let mut seen: HashSet<(String, String, String, String)> = HashSet::new();

    for (from_collection, from_field) in candidate_map.keys() {
        let cand = candidate_map.get(&(from_collection.clone(), from_field.clone())).unwrap();
        if let Some(edge) = build_edge(cand, &shape_by_name, config, &mut seen) {
            edges.push(edge);
        }
    }

    // Detect many-to-many via join collections.
    let join_edges = detect_many_to_many(&edges, shapes);
    edges.extend(join_edges);

    edges.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    edges
}

/// Internal candidate accumulator for a single (collection, field) pair.
#[derive(Debug, Default)]
struct Candidate {
    from_collection: String,
    from_field: String,
    /// True if the field is an array of objectIds (or objectId in app schema).
    is_array: bool,
    /// Dominant type is objectId.
    is_object_id: bool,
    /// Naming convention target.
    naming_target: Option<String>,
    naming_suffix: Option<String>,
    /// $lookup targets.
    lookup_targets: HashSet<(String, String)>,
    /// ObjectId cross-match targets.
    object_id_matches: Vec<ObjectIdMatchDetail>,
    /// App schema reference.
    app_schema_target: Option<AppSchemaTarget>,
    /// Field is indexed.
    indexed: bool,
    /// Index is unique.
    index_unique: bool,
}

impl Candidate {
    fn new(from_collection: String, from_field: String) -> Self {
        Self {
            from_collection,
            from_field,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
struct ObjectIdMatchDetail {
    to_collection: String,
    to_field: String,
    match_ratio: f64,
    sampled: u64,
    matched: u64,
}

#[derive(Debug, Clone)]
struct AppSchemaTarget {
    to_collection: String,
    to_field: String,
    is_array: bool,
    source: String,
}

/// Recursively find fields whose dominant type is objectId or array of objectId.
fn collect_object_id_fields(
    node: &ShapeNode,
    collection: &str,
    out: &mut BTreeMap<(String, String), Candidate>,
) {
    for child in &node.children {
        examine_field(child, collection, out);
    }
}

fn examine_field(
    node: &ShapeNode,
    collection: &str,
    out: &mut BTreeMap<(String, String), Candidate>,
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
        let key = (collection.to_string(), node.path.clone());
        let cand = out.entry(key).or_insert_with(|| {
            Candidate::new(collection.to_string(), node.path.clone())
        });
        cand.is_object_id = true;
        cand.is_array = is_array_of_object_ids;
    }

    // Recurse into object children.
    for child in &node.children {
        examine_field(child, collection, out);
    }

    // Recurse into array element if it's an object.
    if let Some(item) = &node.array_item {
        examine_field(item, collection, out);
    }
}

/// Try to map a field name to a collection name via naming conventions.
pub(crate) fn naming_match(field_name: &str, collections: &HashSet<String>) -> Option<String> {
    let stripped = strip_ref_suffix(field_name);
    if stripped.is_empty() {
        return None;
    }
    let stripped_ref: &str = &stripped;
    // Try exact stripped.
    if collections.contains(stripped_ref) {
        return Some(stripped);
    }
    // Try singular -> plural.
    let plural = pluralize(stripped_ref);
    if collections.contains(&plural) {
        return Some(plural);
    }
    // Try plural -> singular.
    let singular = singularize(stripped_ref);
    if singular != stripped && collections.contains(&singular) {
        return Some(singular);
    }
    // Try with common suffixes.
    for alt in [&plural, &singular] {
        let capitalized = capitalize(alt);
        if collections.contains(&capitalized) {
            return Some(capitalized);
        }
    }
    None
}

fn strip_ref_suffix(name: &str) -> String {
    let lowered = name.to_lowercase();
    for suffix in ["_ids", "ids", "_id", "id", "_refs", "refs", "_ref", "ref"] {
        if lowered.ends_with(suffix) {
            return name[..name.len() - suffix.len()].to_string();
        }
    }
    name.to_string()
}

fn naming_suffix(name: &str) -> Option<String> {
    let lowered = name.to_lowercase();
    for suffix in ["_ids", "ids", "_id", "id", "_refs", "refs", "_ref", "ref"] {
        if lowered.ends_with(suffix) {
            return Some(suffix.to_string());
        }
    }
    None
}

fn pluralize(s: &str) -> String {
    if s.ends_with('s') {
        return s.to_string();
    }
    if s.ends_with("y") && !s.ends_with("ay") && !s.ends_with("ey") && !s.ends_with("oy") && !s.ends_with("uy") {
        return s[..s.len() - 1].to_string() + "ies";
    }
    if s.ends_with("ss") || s.ends_with("sh") || s.ends_with("ch") || s.ends_with("x") || s.ends_with("z") {
        return s.to_string() + "es";
    }
    if s.ends_with("f") && s.len() > 1 {
        return s[..s.len() - 1].to_string() + "ves";
    }
    if s.ends_with("fe") && s.len() > 2 {
        return s[..s.len() - 2].to_string() + "ves";
    }
    format!("{s}s")
}

fn singularize(s: &str) -> String {
    if s.ends_with("ies") && s.len() > 3 {
        return s[..s.len() - 3].to_string() + "y";
    }
    if s.ends_with("ves") && s.len() > 3 {
        return s[..s.len() - 3].to_string() + "f";
    }
    if s.ends_with("es") && s.len() > 2 {
        let stem = &s[..s.len() - 2];
        if stem.ends_with("ss") || stem.ends_with("sh") || stem.ends_with("ch") || stem.ends_with("x") || stem.ends_with("z") {
            return stem.to_string();
        }
    }
    if s.ends_with('s') && !s.ends_with("ss") && s.len() > 1 {
        return s[..s.len() - 1].to_string();
    }
    s.to_string()
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Build a single edge from a candidate, picking the best target.
fn build_edge(
    cand: &Candidate,
    shape_by_name: &HashMap<String, &CollectionShape>,
    config: &RelationshipConfig,
    seen: &mut HashSet<(String, String, String, String)>,
) -> Option<RelationshipEdge> {
    let mut signals: Vec<RelationshipSignal> = Vec::new();
    let mut best_target: Option<String> = None;
    let mut best_to_field: String = "_id".to_string();
    let mut is_array = cand.is_array;

    // App schema is the strongest signal.
    if let Some(app) = &cand.app_schema_target {
        signals.push(RelationshipSignal {
            kind: SignalKind::AppSchema,
            detail: format!("{} ref: {}", if app.is_array { "Array" } else { "Single" }, app.source),
            weight: 0.7,
        });
        best_target = Some(app.to_collection.clone());
        best_to_field = app.to_field.clone();
        is_array = app.is_array || is_array;
    }

    // ObjectId match is the next strongest.
    if let Some(best_match) = cand.object_id_matches.iter().max_by(|a, b| {
        a.match_ratio
            .partial_cmp(&b.match_ratio)
            .unwrap()
    }) {
        signals.push(RelationshipSignal {
            kind: SignalKind::ObjectIdMatch,
            detail: format!(
                "{:.0}% of sampled {} values exist in {}.{} ({} / {})",
                best_match.match_ratio * 100.0,
                cand.from_field,
                best_match.to_collection,
                best_match.to_field,
                best_match.matched,
                best_match.sampled
            ),
            weight: 0.6,
        });
        if best_target.is_none() || best_match.match_ratio > 0.9 {
            best_target = Some(best_match.to_collection.clone());
            best_to_field = best_match.to_field.clone();
        }
    }

    // Lookup signal.
    for (to_coll, to_field) in &cand.lookup_targets {
        signals.push(RelationshipSignal {
            kind: SignalKind::Lookup,
            detail: format!("$lookup from {}.{} to {}.{}", cand.from_collection, cand.from_field, to_coll, to_field),
            weight: 0.4,
        });
        if best_target.is_none() {
            best_target = Some(to_coll.clone());
            best_to_field = to_field.clone();
        }
    }

    // Naming convention.
    if let Some(target) = &cand.naming_target {
        signals.push(RelationshipSignal {
            kind: SignalKind::NamingConvention,
            detail: format!("{} looks like a reference to '{}'", cand.from_field, target),
            weight: 0.25,
        });
        if best_target.is_none() {
            best_target = Some(target.clone());
        }
    }

    // Index signal.
    if cand.indexed {
        signals.push(RelationshipSignal {
            kind: SignalKind::Index,
            detail: format!(
                "{} has a{} index",
                cand.from_field,
                if cand.index_unique { " unique" } else { "n" }
            ),
            weight: 0.1,
        });
    }

    if signals.is_empty() {
        return None;
    }

    let target = best_target?;
    let key = (
        cand.from_collection.clone(),
        cand.from_field.clone(),
        target.clone(),
        best_to_field.clone(),
    );
    if !seen.insert(key) {
        return None;
    }

    let mut confidence = clamp(
        signals.iter().map(|s| s.weight).sum::<f64>()
            + if cand.index_unique { 0.05 } else { 0.0 }
            + if cand.indexed && is_array { -0.05 } else { 0.0 },
        0.0,
        1.0,
    );

    // Strong behavioral evidence: boost to high confidence so it draws solid.
    let has_strong_object_id_match = cand.object_id_matches.iter().any(|m| m.match_ratio >= 0.8);
    let has_app_schema = cand.app_schema_target.is_some();
    if has_strong_object_id_match || has_app_schema {
        confidence = confidence.max(0.9);
    }

    if confidence < config.confidence_threshold {
        return None;
    }

    let kind = infer_cardinality(cand, is_array, shape_by_name.get(&target));

    Some(RelationshipEdge {
        id: Uuid::new_v4().to_string(),
        from_collection: cand.from_collection.clone(),
        to_collection: target,
        from_field: cand.from_field.clone(),
        to_field: best_to_field,
        kind,
        confidence,
        signals,
        via_collection: None,
        confirmed: false,
        hidden: false,
    })
}

fn infer_cardinality(
    cand: &Candidate,
    is_array: bool,
    _target_shape: Option<&&CollectionShape>,
) -> RelationshipKind {
    if is_array {
        // If the target also has an array back to us, it's many-to-many (not
        // detected here; handled later). Otherwise array-of-refs means
        // one-to-many from target to this field.
        RelationshipKind::OneToMany
    } else if cand.index_unique && cand.is_object_id {
        RelationshipKind::OneToOne
    } else {
        // Single ref field -> many-to-one (this collection -> target).
        RelationshipKind::ManyToOne
    }
}

/// Detect many-to-many relationships via intermediate "join" collections.
fn detect_many_to_many(
    edges: &[RelationshipEdge],
    shapes: &[CollectionShape],
) -> Vec<RelationshipEdge> {
    let mut out = Vec::new();
    let mut by_collection: HashMap<String, Vec<&RelationshipEdge>> = HashMap::new();
    for e in edges {
        by_collection
            .entry(e.from_collection.clone())
            .or_default()
            .push(e);
    }

    for shape in shapes {
        if shape.collection == "_" {
            continue;
        }
        let refs = by_collection.get(&shape.collection).cloned().unwrap_or_default();
        if refs.len() >= 2 {
            // A collection with two or more outgoing refs to distinct targets
            // is a candidate join table.
            let targets: Vec<&RelationshipEdge> = refs
                .iter()
                .filter(|r| r.kind == RelationshipKind::ManyToOne)
                .copied()
                .collect();
            if targets.len() >= 2 {
                for pair in pairs(&targets) {
                    let id = Uuid::new_v4().to_string();
                    out.push(RelationshipEdge {
                        id,
                        from_collection: pair[0].to_collection.clone(),
                        to_collection: pair[1].to_collection.clone(),
                        from_field: pair[0].from_field.clone(),
                        to_field: pair[1].from_field.clone(),
                        kind: RelationshipKind::ManyToMany,
                        confidence: 0.6,
                        signals: vec![
                            RelationshipSignal {
                                kind: SignalKind::NamingConvention,
                                detail: format!(
                                    "Join collection {} links {} and {}",
                                    shape.collection,
                                    pair[0].to_collection,
                                    pair[1].to_collection
                                ),
                                weight: 0.35,
                            },
                            RelationshipSignal {
                                kind: SignalKind::Index,
                                detail: "Multiple outgoing references in one collection".to_string(),
                                weight: 0.25,
                            },
                        ],
                        via_collection: Some(shape.collection.clone()),
                        confirmed: false,
                        hidden: false,
                    });
                }
            }
        }
    }
    out
}

fn pairs<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    let mut out = Vec::new();
    for i in 0..items.len() {
        for j in (i + 1)..items.len() {
            out.push(vec![items[i].clone(), items[j].clone()]);
        }
    }
    out
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    use crate::mongo::types::CollectionKind;

    fn make_shape(collection: &str, docs: Vec<bson::Document>) -> CollectionShape {
        crate::mongo::shape::compute_collection_shape(
            "test".to_string(),
            collection.to_string(),
            CollectionKind::Collection,
            Some(docs.len() as u64),
            &docs,
            Vec::new(),
        )
    }

    #[test]
    fn naming_match_user_id_to_users() {
        let docs = vec![doc! { "userId": bson::oid::ObjectId::new() }];
        let shapes = vec![
            make_shape("users", vec![doc! { "_id": bson::oid::ObjectId::new() }]),
            make_shape("orders", docs),
        ];
        let config = RelationshipConfig {
            confidence_threshold: 0.2,
            ..RelationshipConfig::default()
        };
        let edges = detect_relationships(&shapes, &[], &[], &[], &config);
        assert!(
            edges.iter().any(|e| e.from_collection == "orders"
                && e.from_field == "userId"
                && e.to_collection == "users"
                && e.to_field == "_id"),
            "expected naming edge: {edges:?}"
        );
    }

    #[test]
    fn naming_match_underscore_category_id_to_categories() {
        let docs = vec![doc! { "category_id": bson::oid::ObjectId::new() }];
        let shapes = vec![
            make_shape("categories", vec![doc! { "_id": bson::oid::ObjectId::new() }]),
            make_shape("products", docs),
        ];
        let config = RelationshipConfig {
            confidence_threshold: 0.2,
            ..RelationshipConfig::default()
        };
        let edges = detect_relationships(&shapes, &[], &[], &[], &config);
        assert!(
            edges.iter().any(|e| e.from_collection == "products"
                && e.from_field == "category_id"
                && e.to_collection == "categories"),
            "expected category_id -> categories edge: {edges:?}"
        );
    }

    #[test]
    fn lookup_signal_overrides_naming() {
        let shapes = vec![
            make_shape("users", vec![doc! { "_id": bson::oid::ObjectId::new() }]),
            make_shape("orders", vec![doc! { "userId": bson::oid::ObjectId::new() }]),
        ];
        let lookups = vec![LookupSignal {
            from_collection: "orders".to_string(),
            to_collection: "profiles".to_string(),
            local_field: "userId".to_string(),
            foreign_field: "_id".to_string(),
            count: 1,
        }];
        let edges = detect_relationships(&shapes, &[], &lookups, &[], &RelationshipConfig::default());
        let edge = edges
            .iter()
            .find(|e| e.from_collection == "orders" && e.from_field == "userId")
            .expect("edge should exist");
        assert_eq!(edge.to_collection, "profiles");
        assert!(edge.signals.iter().any(|s| s.kind == SignalKind::Lookup));
        assert!(edge.signals.iter().any(|s| s.kind == SignalKind::NamingConvention));
    }

    #[test]
    fn app_schema_is_strongest_signal() {
        let shapes = vec![
            make_shape("users", vec![doc! { "_id": bson::oid::ObjectId::new() }]),
            make_shape("posts", vec![doc! { "authorId": bson::oid::ObjectId::new() }]),
        ];
        let app = vec![AppSchemaSignal {
            from_collection: "posts".to_string(),
            to_collection: "users".to_string(),
            from_field: "authorId".to_string(),
            to_field: "_id".to_string(),
            is_array: false,
            source: "Mongoose ref: 'User'".to_string(),
        }];
        let edges = detect_relationships(&shapes, &[], &[], &app, &RelationshipConfig::default());
        let edge = edges
            .iter()
            .find(|e| e.from_collection == "posts" && e.from_field == "authorId")
            .expect("edge should exist");
        assert_eq!(edge.to_collection, "users");
        assert!(edge.signals.iter().any(|s| s.kind == SignalKind::AppSchema));
        assert_eq!(edge.kind, RelationshipKind::ManyToOne);
    }

    #[test]
    fn object_id_match_high_confidence() {
        let shapes = vec![
            make_shape("users", vec![doc! { "_id": bson::oid::ObjectId::new() }]),
            make_shape("posts", vec![doc! { "authorId": bson::oid::ObjectId::new() }]),
        ];
        let oid = vec![ObjectIdMatchSignal {
            from_collection: "posts".to_string(),
            from_field: "authorId".to_string(),
            to_collection: "users".to_string(),
            to_field: "_id".to_string(),
            match_ratio: 0.85,
            sampled: 20,
            matched: 17,
        }];
        let edges = detect_relationships(&shapes, &oid, &[], &[], &RelationshipConfig::default());
        let edge = edges
            .iter()
            .find(|e| e.from_collection == "posts")
            .expect("edge should exist");
        assert_eq!(edge.to_collection, "users");
        assert!(edge.confidence >= 0.9);
        assert!(edge.signals.iter().any(|s| s.kind == SignalKind::ObjectIdMatch));
    }

    #[test]
    fn unique_index_makes_one_to_one() {
        let shape = make_shape("partners", vec![doc! { "_id": bson::oid::ObjectId::new() }]);
        let mut child = make_shape("users", vec![doc! { "partnerId": bson::oid::ObjectId::new() }]);
        child.indexes = vec![crate::mongo::types::IndexInfo {
            name: "partnerId_1".to_string(),
            key: [("partnerId".to_string(), bson::Bson::Int32(1))]
                .into_iter()
                .collect(),
            unique: true,
            sparse: false,
            hidden: false,
            ttl_seconds: None,
            partial_filter_expression: None,
            collation: None,
            wildcard_projection: None,
            is_text: false,
            is_geo: false,
            is_id: false,
        }];
        let config = RelationshipConfig {
            confidence_threshold: 0.2,
            ..RelationshipConfig::default()
        };
        let edges = detect_relationships(&[shape, child], &[], &[], &[], &config);
        let edge = edges
            .iter()
            .find(|e| e.from_field == "partnerId")
            .expect("edge should exist");
        assert_eq!(edge.to_collection, "partners");
        assert_eq!(edge.kind, RelationshipKind::OneToOne);
    }

    #[test]
    fn many_to_many_via_join_collection() {
        let users = make_shape("users", vec![doc! { "_id": bson::oid::ObjectId::new() }]);
        let courses = make_shape("courses", vec![doc! { "_id": bson::oid::ObjectId::new() }]);
        let mut enrollments = make_shape(
            "enrollments",
            vec![doc! {
                "userId": bson::oid::ObjectId::new(),
                "courseId": bson::oid::ObjectId::new(),
            }],
        );
        enrollments.indexes = vec![
            crate::mongo::types::IndexInfo {
                name: "userId_1".to_string(),
                key: [("userId".to_string(), bson::Bson::Int32(1))]
                    .into_iter()
                    .collect(),
                unique: false,
                sparse: false,
                hidden: false,
                ttl_seconds: None,
                partial_filter_expression: None,
                collation: None,
                wildcard_projection: None,
                is_text: false,
                is_geo: false,
                is_id: false,
            },
            crate::mongo::types::IndexInfo {
                name: "courseId_1".to_string(),
                key: [("courseId".to_string(), bson::Bson::Int32(1))]
                    .into_iter()
                    .collect(),
                unique: false,
                sparse: false,
                hidden: false,
                ttl_seconds: None,
                partial_filter_expression: None,
                collation: None,
                wildcard_projection: None,
                is_text: false,
                is_geo: false,
                is_id: false,
            },
        ];
        let shapes = vec![users, courses, enrollments];
        let config = RelationshipConfig {
            confidence_threshold: 0.2,
            ..RelationshipConfig::default()
        };
        let edges = detect_relationships(&shapes, &[], &[], &[], &config);
        assert!(
            edges.iter().any(|e| e.kind == RelationshipKind::ManyToMany
                && e.via_collection.as_deref() == Some("enrollments")
                && (
                    (e.from_collection == "users" && e.to_collection == "courses")
                    || (e.from_collection == "courses" && e.to_collection == "users")
                )),
            "expected many-to-many edge: {edges:?}"
        );
    }
}
