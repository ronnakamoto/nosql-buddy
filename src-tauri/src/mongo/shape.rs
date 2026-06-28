//! Recursive document-shape inference. `compute_collection_shape` is a pure
//! function over a sample of BSON documents: it produces a tree of field paths
//! (including nested objects and arrays) with per-path type probabilities,
//! presence, null ratios, and scalar distributions. It intentionally does not
//! detect relationships between collections — that lives in `relationship.rs`.

use std::collections::BTreeMap;

use bson::{Bson, Document};
use serde::{Deserialize, Serialize};

use crate::mongo::schema::{
    date_histogram, histogram, SchemaDateStats, SchemaNumericStats, SchemaValueCount,
};
use crate::mongo::types::{CollectionKind, IndexInfo};

/// Maximum depth of the shape tree. Beyond this we emit a collapsed placeholder.
const MAX_SHAPE_DEPTH: usize = 6;
/// Maximum number of array elements inspected per document per array path.
const ARRAY_ELEMENT_SAMPLE_PER_DOC: usize = 100;
/// A field is low-cardinality for top-values if the distinct non-null values are
/// at most this threshold (mirrors `schema.rs`).
const TOP_VALUES_CARDINALITY_THRESHOLD: usize = 30;
const TOP_VALUES_LIMIT: usize = 10;
const VALUE_LABEL_MAX: usize = 48;

/// One node in a recursive document-shape tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeNode {
    /// Full dot-notation path. Array elements are represented as `[]`.
    /// Example: `address.city`, `items[].productId`.
    pub path: String,
    /// The leaf key (the last segment of `path`).
    pub name: String,
    /// BSON type -> probability (0..1). Probabilities are computed over the
    /// total document count, so missing paths appear as a lowered presence.
    pub types: BTreeMap<String, f64>,
    /// Fraction of total sampled documents where this path is present.
    pub presence: f64,
    /// Fraction of total sampled documents where this path is explicitly null.
    pub null_ratio: f64,
    /// Distinct non-null values, if computed (only for scalar candidates).
    pub cardinality: Option<u64>,
    /// Children when this path is an object in some documents.
    pub children: Vec<ShapeNode>,
    /// Element shape when this path is an array in some documents.
    pub array_item: Option<Box<ShapeNode>>,
    /// Low-cardinality top values (strings / booleans).
    pub top_values: Option<Vec<SchemaValueCount>>,
    pub numeric_stats: Option<SchemaNumericStats>,
    pub date_stats: Option<SchemaDateStats>,
}

/// Full shape for a single collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionShape {
    pub database: String,
    pub collection: String,
    pub kind: CollectionKind,
    pub document_count: Option<u64>,
    pub sampled_documents: u64,
    pub root: ShapeNode,
    pub max_depth: u32,
    pub warnings: Vec<String>,
    pub indexes: Vec<IndexInfo>,
}

/// Internal accumulator for a single path. Keeps counts, not probabilities, so
/// probabilities are computed once after the walk.
#[derive(Debug, Default)]
struct PathAccumulator {
    /// Counts of immediate BSON type at this path.
    type_counts: BTreeMap<String, u64>,
    present_count: u64,
    null_count: u64,
    /// Low-cardinality scalar value counts (strings / booleans).
    value_counts: BTreeMap<String, u64>,
    /// Numeric samples for histograms.
    numeric_values: Vec<f64>,
    /// Date samples for histograms.
    date_values: Vec<i64>,
    /// Child paths observed when this path is an object.
    children: BTreeMap<String, PathAccumulator>,
    /// Element shape accumulator when this path is an array.
    array_element: Option<Box<PathAccumulator>>,
}

/// Compute a recursive shape tree for a sample of documents. `database`,
/// `collection`, and `kind` are metadata copied through to the result.
///
/// If the input is empty, an empty root with no children is returned.
#[allow(clippy::too_many_arguments)]
pub fn compute_collection_shape(
    database: String,
    collection: String,
    kind: CollectionKind,
    document_count: Option<u64>,
    docs: &[Document],
    indexes: Vec<IndexInfo>,
) -> CollectionShape {
    let total = docs.len() as u64;
    let mut root = PathAccumulator::default();
    for doc in docs {
        walk_document(doc, &mut root, total);
    }

    let mut warnings = Vec::new();
    let root_node = build_shape_node(
        "".to_string(),
        "".to_string(),
        &root,
        total,
        total,
        0,
        &mut warnings,
    );

    let max_depth = max_depth(&root_node);

    CollectionShape {
        database,
        collection,
        kind,
        document_count,
        sampled_documents: total,
        root: root_node,
        max_depth,
        warnings,
        indexes,
    }
}

/// Walk a document at the root level, feeding accumulators.
fn walk_document(doc: &Document, root: &mut PathAccumulator, total: u64) {
    // The root is the document itself; we only care about its children.
    root.present_count += 1;
    *root.type_counts.entry("object".to_string()).or_insert(0) += 1;

    for (key, value) in doc.iter() {
        walk_value(value, key, key, root, total, 1);
    }
}

/// Walk a value at a named field, creating the child accumulator as needed.
///
/// `path` is the full path for this value; `name` is the leaf key.
/// `depth` is the current depth (root document is 0, top-level fields are 1).
fn walk_value(
    value: &Bson,
    path: &str,
    name: &str,
    parent: &mut PathAccumulator,
    total: u64,
    depth: usize,
) {
    let acc = parent.children.entry(name.to_string()).or_default();
    accumulate_value(value, path, name, acc, total, depth);
}

/// Accumulate a value directly into an existing accumulator.
///
/// This is used by `walk_value` for regular fields, and recursively for array
/// elements, where the element accumulator itself is the shape of the element
/// (not a named child).
fn accumulate_value(
    value: &Bson,
    path: &str,
    name: &str,
    acc: &mut PathAccumulator,
    total: u64,
    depth: usize,
) {
    acc.present_count += 1;
    let type_name = bson_type_name(value);
    *acc.type_counts.entry(type_name).or_insert(0) += 1;

    if matches!(value, Bson::Null) {
        acc.null_count += 1;
    }

    // Scalar accumulators.
    if let Some(label) = top_value_label(value) {
        *acc.value_counts.entry(label).or_insert(0) += 1;
    }
    if let Some(n) = as_f64(value) {
        acc.numeric_values.push(n);
    }
    if let Some(ms) = as_epoch_millis(value) {
        acc.date_values.push(ms);
    }

    match value {
        Bson::Document(sub) => {
            for (key, sub_value) in sub.iter() {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if depth < MAX_SHAPE_DEPTH {
                    walk_value(sub_value, &child_path, key, acc, total, depth + 1);
                }
            }
        }
        Bson::Array(arr) => {
            let element_acc = acc.array_element.get_or_insert_default();
            for elem in arr.iter().take(ARRAY_ELEMENT_SAMPLE_PER_DOC) {
                let elem_path = format!("{path}[]");
                let elem_name = format!("{name}[]");
                accumulate_value(elem, &elem_path, &elem_name, element_acc, total, depth + 1);
            }
        }
        _ => {}
    }
}

/// Build a `ShapeNode` from a `PathAccumulator`. Probabilities are computed over
/// `denominator`, not `present_count`, so the UI can distinguish "absent" from
/// "null". For the root and object fields the denominator is the document count.
/// For array elements the denominator is the sampled element count.
fn build_shape_node(
    path: String,
    name: String,
    acc: &PathAccumulator,
    denominator: u64,
    total_docs: u64,
    depth: usize,
    warnings: &mut Vec<String>,
) -> ShapeNode {
    let presence = if denominator > 0 {
        acc.present_count as f64 / denominator as f64
    } else {
        0.0
    };
    let null_ratio = if denominator > 0 {
        acc.null_count as f64 / denominator as f64
    } else {
        0.0
    };

    let types: BTreeMap<String, f64> = acc
        .type_counts
        .iter()
        .map(|(t, c)| (t.clone(), *c as f64 / denominator as f64))
        .collect();

    // Top values only when the scalar value set is small enough.
    let top_values = if acc.value_counts.len() <= TOP_VALUES_CARDINALITY_THRESHOLD {
        let mut entries: Vec<(String, u64)> = acc
            .value_counts
            .iter()
            .map(|(v, c)| (v.clone(), *c))
            .collect();
        entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let limited: Vec<SchemaValueCount> = entries
            .into_iter()
            .take(TOP_VALUES_LIMIT)
            .map(|(value, count)| SchemaValueCount { value, count })
            .collect();
        if limited.is_empty() {
            None
        } else {
            Some(limited)
        }
    } else {
        None
    };

    let cardinality = if acc.value_counts.len() > 1 {
        Some(acc.value_counts.len() as u64)
    } else {
        None
    };

    let numeric_stats = if acc.numeric_values.is_empty() {
        None
    } else {
        let (min, max, sum) = acc.numeric_values.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY, 0.0f64),
            |(mn, mx, s), &v| (mn.min(v), mx.max(v), s + v),
        );
        let mean = sum / acc.numeric_values.len() as f64;
        Some(SchemaNumericStats {
            min,
            max,
            mean,
            buckets: histogram(&acc.numeric_values, min, max),
        })
    };

    let date_stats = if acc.date_values.is_empty() {
        None
    } else {
        let (min_ms, max_ms) = acc
            .date_values
            .iter()
            .fold((i64::MAX, i64::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));
        Some(SchemaDateStats {
            min_ms,
            max_ms,
            buckets: date_histogram(&acc.date_values, min_ms, max_ms),
        })
    };

    // Recurse into children (object fields). If we are at the depth cap, keep
    // the children that were already collected but do not expand further; a
    // placeholder is emitted instead.
    let mut children: Vec<ShapeNode> = acc
        .children
        .iter()
        .map(|(name, child)| {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}.{name}")
            };
            if depth + 1 < MAX_SHAPE_DEPTH {
                build_shape_node(
                    child_path,
                    name.clone(),
                    child,
                    total_docs,
                    total_docs,
                    depth + 1,
                    warnings,
                )
            } else {
                placeholder_node(child_path, name.clone())
            }
        })
        .collect();
    children.sort_by(|a, b| a.name.cmp(&b.name));

    // If we hit the depth cap, add a placeholder child so the UI shows that
    // deeper structure exists but was omitted.
    if depth == MAX_SHAPE_DEPTH && (types.contains_key("object") || types.contains_key("array")) {
        children.push(placeholder_node(format!("{path}.…"), "…".to_string()));
    }

    // Build array element shape from the union of all observed elements.
    // The element denominator is the number of sampled array elements.
    let element_denominator = acc
        .array_element
        .as_ref()
        .map(|e| e.present_count)
        .unwrap_or(0);
    let array_item = acc.array_element.as_ref().map(|elem| {
        let elem_path = format!("{path}[]");
        let elem_name = format!("{name}[]");
        Box::new(build_shape_node(
            elem_path,
            elem_name,
            elem,
            element_denominator,
            total_docs,
            depth + 1,
            warnings,
        ))
    });

    // Warn about polymorphism: if a field has both object and array types with
    // non-trivial probability, note it.
    let obj_prob = types.get("object").copied().unwrap_or(0.0);
    let arr_prob = types.get("array").copied().unwrap_or(0.0);
    if obj_prob > 0.1 && arr_prob > 0.1 {
        warnings.push(format!(
            "{path} is polymorphic: object {obj_prob:.0}% and array {arr_prob:.0}%"
        ));
    }

    ShapeNode {
        path,
        name,
        types,
        presence,
        null_ratio,
        cardinality,
        children,
        array_item,
        top_values,
        numeric_stats,
        date_stats,
    }
}

/// A collapsed placeholder node emitted when the depth cap is reached.
fn placeholder_node(path: String, name: String) -> ShapeNode {
    let mut types = BTreeMap::new();
    types.insert("unknown".to_string(), 1.0);
    ShapeNode {
        path,
        name,
        types,
        presence: 1.0,
        null_ratio: 0.0,
        cardinality: None,
        children: Vec::new(),
        array_item: None,
        top_values: None,
        numeric_stats: None,
        date_stats: None,
    }
}

/// Maximum depth of the generated tree (root is depth 0). Placeholder
/// children emitted at the depth cap are not counted.
fn max_depth(node: &ShapeNode) -> u32 {
    let child_max = node
        .children
        .iter()
        .filter(|c| c.name != "…")
        .map(|c| 1 + max_depth(c))
        .max()
        .unwrap_or(0);
    let array_max = node
        .array_item
        .as_ref()
        .map(|b| 1 + max_depth(b))
        .unwrap_or(0);
    child_max.max(array_max)
}

/// Display label for a top-value-eligible scalar. Strings are truncated.
fn top_value_label(value: &Bson) -> Option<String> {
    match value {
        Bson::String(s) => {
            let truncated = if s.chars().count() > VALUE_LABEL_MAX {
                let head: String = s.chars().take(VALUE_LABEL_MAX).collect();
                format!("{head}…")
            } else {
                s.clone()
            };
            Some(truncated)
        }
        Bson::Boolean(b) => Some(if *b { "true".to_string() } else { "false".to_string() }),
        _ => None,
    }
}

/// Convert a numeric BSON value to `f64`.
fn as_f64(value: &Bson) -> Option<f64> {
    match value {
        Bson::Double(d) => Some(*d),
        Bson::Int32(i) => Some(*i as f64),
        Bson::Int64(i) => Some(*i as f64),
        Bson::Decimal128(d) => d.to_string().parse::<f64>().ok(),
        _ => None,
    }
}

/// Convert a date BSON value to epoch millis.
fn as_epoch_millis(value: &Bson) -> Option<i64> {
    match value {
        Bson::DateTime(dt) => Some(dt.timestamp_millis()),
        _ => None,
    }
}

/// Stable BSON type name string.
fn bson_type_name(value: &Bson) -> String {
    match value {
        Bson::Double(_) => "double".to_string(),
        Bson::String(_) => "string".to_string(),
        Bson::Array(_) => "array".to_string(),
        Bson::Document(_) => "object".to_string(),
        Bson::Boolean(_) => "bool".to_string(),
        Bson::Null => "null".to_string(),
        Bson::Int32(_) => "int".to_string(),
        Bson::Int64(_) => "long".to_string(),
        Bson::ObjectId(_) => "objectId".to_string(),
        Bson::DateTime(_) => "date".to_string(),
        Bson::Decimal128(_) => "decimal".to_string(),
        Bson::Binary(_) => "binary".to_string(),
        Bson::Timestamp(_) => "timestamp".to_string(),
        Bson::RegularExpression(_) => "regex".to_string(),
        Bson::JavaScriptCode(_) => "javascript".to_string(),
        Bson::JavaScriptCodeWithScope(_) => "javascriptWithScope".to_string(),
        Bson::MinKey => "minKey".to_string(),
        Bson::MaxKey => "maxKey".to_string(),
        Bson::Undefined => "undefined".to_string(),
        Bson::DbPointer(_) => "dbPointer".to_string(),
        Bson::Symbol(_) => "symbol".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    fn make_shape(docs: Vec<Document>) -> CollectionShape {
        compute_collection_shape(
            "test".to_string(),
            "sample".to_string(),
            CollectionKind::Collection,
            None,
            &docs,
            Vec::new(),
        )
    }

    #[test]
    fn empty_sample_returns_empty_shape() {
        let shape = make_shape(vec![]);
        assert_eq!(shape.sampled_documents, 0);
        assert!(shape.root.children.is_empty());
        assert!(shape.warnings.is_empty());
    }

    #[test]
    fn top_level_shape() {
        let docs = vec![
            doc! { "name": "alice", "age": 30 },
            doc! { "name": "bob", "age": null },
        ];
        let shape = make_shape(docs);
        assert_eq!(shape.sampled_documents, 2);
        let name = shape.root.children.iter().find(|c| c.name == "name").unwrap();
        let age = shape.root.children.iter().find(|c| c.name == "age").unwrap();
        assert_eq!(name.types.get("string"), Some(&1.0));
        assert_eq!(age.types.get("int"), Some(&0.5));
        assert_eq!(age.types.get("null"), Some(&0.5));
        assert_eq!(age.presence, 1.0);
        assert_eq!(age.null_ratio, 0.5);
    }

    #[test]
    fn nested_shape() {
        let docs = vec![
            doc! { "address": { "city": "NYC", "zip": 10001 } },
            doc! { "address": { "city": "LA", "zip": 90001 } },
        ];
        let shape = make_shape(docs);
        let address = shape.root.children.iter().find(|c| c.name == "address").unwrap();
        assert_eq!(address.types.get("object"), Some(&1.0));
        assert_eq!(address.children.len(), 2);
        let city = address.children.iter().find(|c| c.name == "city").unwrap();
        assert_eq!(city.types.get("string"), Some(&1.0));
        assert_eq!(city.path, "address.city");
    }

    #[test]
    fn array_shape() {
        let docs = vec![
            doc! { "tags": ["a", "b", "c"] },
            doc! { "tags": ["a", "d"] },
        ];
        let shape = make_shape(docs);
        let tags = shape.root.children.iter().find(|c| c.name == "tags").unwrap();
        assert_eq!(tags.types.get("array"), Some(&1.0));
        let item = tags.array_item.as_ref().expect("array item shape");
        assert_eq!(item.types.get("string"), Some(&1.0));
        let top = item.top_values.as_ref().expect("top values");
        assert!(top.iter().any(|v| v.value == "a" && v.count == 2));
    }

    #[test]
    fn missing_field_lowers_presence() {
        let docs = vec![
            doc! { "a": 1 },
            doc! { "b": 2 },
        ];
        let shape = make_shape(docs);
        let a = shape.root.children.iter().find(|c| c.name == "a").unwrap();
        let b = shape.root.children.iter().find(|c| c.name == "b").unwrap();
        assert_eq!(a.presence, 0.5);
        assert_eq!(b.presence, 0.5);
    }

    #[test]
    fn object_id_type_detected() {
        let docs = vec![
            doc! { "userId": bson::oid::ObjectId::new() },
        ];
        let shape = make_shape(docs);
        let user_id = shape.root.children.iter().find(|c| c.name == "userId").unwrap();
        assert_eq!(user_id.types.get("objectId"), Some(&1.0));
    }

    #[test]
    fn depth_cap_emits_placeholder() {
        let docs = vec![
            doc! { "a": { "b": { "c": { "d": { "e": { "f": { "g": 1 } } } } } } },
        ];
        let shape = make_shape(docs);
        assert_eq!(shape.max_depth, MAX_SHAPE_DEPTH as u32);
        // At least one path should have a collapsed "unknown" placeholder.
        fn has_placeholder(node: &ShapeNode) -> bool {
            node.children
                .iter()
                .any(|c| c.types.contains_key("unknown") || has_placeholder(c))
        }
        assert!(has_placeholder(&shape.root));
    }

    #[test]
    fn polymorphic_field_warning() {
        let docs = vec![
            doc! { "meta": { "x": 1 } },
            doc! { "meta": ["a", "b"] },
        ];
        let shape = make_shape(docs);
        assert!(
            shape.warnings.iter().any(|w| w.contains("polymorphic")),
            "expected polymorphism warning, got {:?}",
            shape.warnings
        );
    }
}
