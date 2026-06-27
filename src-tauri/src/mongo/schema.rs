//! Schema analysis. `compute_schema_report` is a pure function over a sample of
//! BSON documents: it infers per-field type counts, missing/null ratios, and
//! value distributions (top values for low-cardinality fields, numeric
//! histograms, date distributions). Kept separate from the Tauri command layer
//! so it can be unit-tested without a Mongo connection.

use std::collections::BTreeMap;

use bson::{Bson, Document};
use serde::{Deserialize, Serialize};

/// Number of histogram buckets for numeric and date distributions.
const NUM_BUCKETS: usize = 10;
/// A field is considered low-cardinality (worth emitting top-values for) if the
/// number of distinct non-null values is at most this threshold.
const TOP_VALUES_CARDINALITY_THRESHOLD: usize = 30;
/// Maximum number of top-value entries emitted per field.
const TOP_VALUES_LIMIT: usize = 10;
/// Truncate string value labels longer than this to keep the chart readable.
const VALUE_LABEL_MAX: usize = 48;

/// Full schema report returned by `sample_schema`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaReport {
    pub sampled_documents: u64,
    pub fields: Vec<SchemaField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaField {
    pub name: String,
    pub types: BTreeMap<String, u64>,
    pub null_ratio: f64,
    /// Documents where the field key is absent entirely (distinct from null).
    pub missing_count: u64,
    /// Top values for low-cardinality fields; `None` when cardinality is too high.
    pub top_values: Option<Vec<SchemaValueCount>>,
    /// Numeric distribution; `None` when the field has no numeric values.
    pub numeric_stats: Option<SchemaNumericStats>,
    /// Date distribution; `None` when the field has no date values.
    pub date_stats: Option<SchemaDateStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaValueCount {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaNumericStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub buckets: Vec<SchemaBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaBucket {
    pub lo: f64,
    pub hi: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDateStats {
    pub min_ms: i64,
    pub max_ms: i64,
    pub buckets: Vec<SchemaDateBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDateBucket {
    pub lo_ms: i64,
    pub hi_ms: i64,
    pub count: u64,
}

/// Compute a schema report from an in-memory sample of documents. Pure: no I/O,
/// no panics on empty input (returns an empty report).
pub fn compute_schema_report(docs: &[Document]) -> SchemaReport {
    let total = docs.len() as u64;

    // Per-field accumulators.
    let mut type_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    let mut present_counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut null_counts: BTreeMap<String, u64> = BTreeMap::new();
    // value -> count, keyed by field name. Only for top-value-eligible types.
    let mut value_counts: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    // numeric values per field.
    let mut numeric_values: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    // date epoch-millis per field.
    let mut date_values: BTreeMap<String, Vec<i64>> = BTreeMap::new();

    for doc in docs {
        for (key, value) in doc {
            *present_counts.entry(key.clone()).or_insert(0) += 1;
            let type_entry = type_counts.entry(key.clone()).or_default();
            *type_entry.entry(bson_type_name(value)).or_insert(0) += 1;

            if matches!(value, Bson::Null) {
                *null_counts.entry(key.clone()).or_insert(0) += 1;
            }

            // Top-value candidates: strings and bools (low-cardinality scalars).
            if let Some(label) = top_value_label(value) {
                *value_counts
                    .entry(key.clone())
                    .or_default()
                    .entry(label)
                    .or_insert(0) += 1;
            }

            if let Some(n) = as_f64(value) {
                numeric_values.entry(key.clone()).or_default().push(n);
            }

            if let Some(ms) = as_epoch_millis(value) {
                date_values.entry(key.clone()).or_default().push(ms);
            }
        }
    }

    let mut fields: Vec<SchemaField> = type_counts
        .into_iter()
        .map(|(name, types)| {
            let present = *present_counts.get(&name).unwrap_or(&0);
            let missing = total.saturating_sub(present);
            let null_count = *null_counts.get(&name).unwrap_or(&0);
            let null_ratio = if total > 0 {
                null_count as f64 / total as f64
            } else {
                0.0
            };

            let top_values = value_counts
                .get(&name)
                .filter(|m| m.len() <= TOP_VALUES_CARDINALITY_THRESHOLD)
                .map(|m| {
                    let mut entries: Vec<(String, u64)> =
                        m.iter().map(|(v, c)| (v.clone(), *c)).collect();
                    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    entries
                        .into_iter()
                        .take(TOP_VALUES_LIMIT)
                        .map(|(value, count)| SchemaValueCount { value, count })
                        .collect::<Vec<_>>()
                });

            let numeric_stats = numeric_values.get(&name).map(|vals| {
                let (min, max, sum) = vals.iter().fold(
                    (f64::INFINITY, f64::NEG_INFINITY, 0.0f64),
                    |(mn, mx, s), &v| (mn.min(v), mx.max(v), s + v),
                );
                let mean = if vals.is_empty() {
                    0.0
                } else {
                    sum / vals.len() as f64
                };
                let buckets = histogram(vals, min, max);
                SchemaNumericStats {
                    min,
                    max,
                    mean,
                    buckets,
                }
            });

            let date_stats = date_values.get(&name).map(|vals| {
                let (min_ms, max_ms) = vals
                    .iter()
                    .fold((i64::MAX, i64::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)));
                let buckets = date_histogram(vals, min_ms, max_ms);
                SchemaDateStats {
                    min_ms,
                    max_ms,
                    buckets,
                }
            });

            SchemaField {
                name,
                types,
                null_ratio,
                missing_count: missing,
                top_values,
                numeric_stats,
                date_stats,
            }
        })
        .collect();

    fields.sort_by(|a, b| a.name.cmp(&b.name));

    SchemaReport {
        sampled_documents: total,
        fields,
    }
}

/// Build `NUM_BUCKETS` equal-width buckets between `min` and `max`. If all
/// values are equal, a single bucket holding everything is returned.
fn histogram(vals: &[f64], min: f64, max: f64) -> Vec<SchemaBucket> {
    if vals.is_empty() {
        return Vec::new();
    }
    if (max - min).abs() < f64::EPSILON {
        return vec![SchemaBucket {
            lo: min,
            hi: max,
            count: vals.len() as u64,
        }];
    }
    let width = (max - min) / NUM_BUCKETS as f64;
    let mut counts = [0u64; NUM_BUCKETS];
    for &v in vals {
        // Map value to bucket index; clamp max into the last bucket.
        let mut idx = ((v - min) / width).floor() as usize;
        if idx >= NUM_BUCKETS {
            idx = NUM_BUCKETS - 1;
        }
        counts[idx] += 1;
    }
    (0..NUM_BUCKETS)
        .map(|i| SchemaBucket {
            lo: min + width * i as f64,
            hi: min + width * (i + 1) as f64,
            count: counts[i],
        })
        .collect()
}

/// Build `NUM_BUCKETS` equal-width time buckets (epoch millis) between min and max.
fn date_histogram(vals: &[i64], min_ms: i64, max_ms: i64) -> Vec<SchemaDateBucket> {
    if vals.is_empty() {
        return Vec::new();
    }
    if min_ms == max_ms {
        return vec![SchemaDateBucket {
            lo_ms: min_ms,
            hi_ms: max_ms,
            count: vals.len() as u64,
        }];
    }
    let span = (max_ms - min_ms) as f64 / NUM_BUCKETS as f64;
    let mut counts = [0u64; NUM_BUCKETS];
    for &v in vals {
        let mut idx = ((v - min_ms) as f64 / span).floor() as usize;
        if idx >= NUM_BUCKETS {
            idx = NUM_BUCKETS - 1;
        }
        counts[idx] += 1;
    }
    (0..NUM_BUCKETS)
        .map(|i| SchemaDateBucket {
            lo_ms: min_ms + (span * i as f64) as i64,
            hi_ms: min_ms + (span * (i + 1) as f64) as i64,
            count: counts[i],
        })
        .collect()
}

/// Display label for a top-value-eligible BSON value, or `None` if the value
/// type is not eligible for top-values. Strings are truncated to keep charts
/// readable.
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
        Bson::Boolean(b) => Some(if *b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        _ => None,
    }
}

/// Convert a numeric BSON value to `f64`. Returns `None` for non-numerics.
fn as_f64(value: &Bson) -> Option<f64> {
    match value {
        Bson::Double(d) => Some(*d),
        Bson::Int32(i) => Some(*i as f64),
        Bson::Int64(i) => Some(*i as f64),
        Bson::Decimal128(d) => {
            // Decimal128 -> string -> f64 parse. Best-effort; skip if unparseable.
            d.to_string().parse::<f64>().ok()
        }
        _ => None,
    }
}

/// Convert a date BSON value to epoch millis. Returns `None` for non-dates.
fn as_epoch_millis(value: &Bson) -> Option<i64> {
    match value {
        Bson::DateTime(dt) => Some(dt.timestamp_millis()),
        _ => None,
    }
}

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
        _ => "other".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn empty_sample_returns_empty_report() {
        let report = compute_schema_report(&[]);
        assert_eq!(report.sampled_documents, 0);
        assert!(report.fields.is_empty());
    }

    #[test]
    fn type_counts_and_null_ratio() {
        let docs = vec![
            doc! { "name": "alice", "age": 30, "active": true },
            doc! { "name": "bob", "age": null, "active": false },
            doc! { "name": "carol", "active": true },
        ];
        let report = compute_schema_report(&docs);
        assert_eq!(report.sampled_documents, 3);

        let age = report.fields.iter().find(|f| f.name == "age").unwrap();
        assert_eq!(age.types.get("int"), Some(&1));
        assert_eq!(age.types.get("null"), Some(&1));
        // 1 null out of 3 docs
        assert!((age.null_ratio - 1.0 / 3.0).abs() < 1e-9);
        // present in 2 of 3 docs (carol omits age entirely) -> missing 1
        assert_eq!(age.missing_count, 1);
    }

    #[test]
    fn missing_count_distinct_from_null() {
        // Field "status" is null in one doc, absent in another, present in a third.
        let docs = vec![
            doc! { "status": "ok" },
            doc! { "status": Bson::Null },
            doc! { "other": 1 },
        ];
        let report = compute_schema_report(&docs);
        let status = report.fields.iter().find(|f| f.name == "status").unwrap();
        // present in 2 of 3 docs -> missing 1
        assert_eq!(status.missing_count, 1);
        // 1 null out of 3 total
        assert!((status.null_ratio - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn top_values_for_low_cardinality_string() {
        let docs = vec![
            doc! { "color": "red" },
            doc! { "color": "red" },
            doc! { "color": "blue" },
            doc! { "color": "blue" },
            doc! { "color": "green" },
        ];
        let report = compute_schema_report(&docs);
        let color = report.fields.iter().find(|f| f.name == "color").unwrap();
        let top = color
            .top_values
            .as_ref()
            .expect("top_values should be present");
        assert_eq!(top.len(), 3);
        // Sorted by count desc then value asc: blue=2, red=2, green=1
        assert_eq!(top[0].value, "blue");
        assert_eq!(top[0].count, 2);
        assert_eq!(top[1].value, "red");
        assert_eq!(top[1].count, 2);
        assert_eq!(top[2].value, "green");
        assert_eq!(top[2].count, 1);
    }

    #[test]
    fn top_values_none_for_high_cardinality() {
        // 31 distinct values -> above threshold -> top_values is None.
        let docs: Vec<Document> = (0..31).map(|i| doc! { "id": format!("v{i}") }).collect();
        let report = compute_schema_report(&docs);
        let id_field = report.fields.iter().find(|f| f.name == "id").unwrap();
        assert!(
            id_field.top_values.is_none(),
            "top_values should be None for high cardinality"
        );
    }

    #[test]
    fn top_values_includes_bools() {
        let docs = vec![
            doc! { "flag": true },
            doc! { "flag": true },
            doc! { "flag": false },
        ];
        let report = compute_schema_report(&docs);
        let flag = report.fields.iter().find(|f| f.name == "flag").unwrap();
        let top = flag.top_values.as_ref().unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].value, "true");
        assert_eq!(top[0].count, 2);
    }

    #[test]
    fn numeric_histogram_buckets_sum_to_total() {
        let docs: Vec<Document> = (0..20i32).map(|i| doc! { "score": i }).collect();
        let report = compute_schema_report(&docs);
        let score = report.fields.iter().find(|f| f.name == "score").unwrap();
        let stats = score.numeric_stats.as_ref().expect("numeric_stats present");
        assert_eq!(stats.min, 0.0);
        assert_eq!(stats.max, 19.0);
        assert!((stats.mean - 9.5).abs() < 1e-9);
        assert_eq!(stats.buckets.len(), NUM_BUCKETS);
        let total: u64 = stats.buckets.iter().map(|b| b.count).sum();
        assert_eq!(total, 20);
        // Buckets span [min, max] monotonically.
        assert!(stats.buckets[0].lo <= stats.buckets[0].hi);
    }

    #[test]
    fn numeric_histogram_single_value_collapses_to_one_bucket() {
        let docs = vec![doc! { "x": 5 }, doc! { "x": 5 }, doc! { "x": 5 }];
        let report = compute_schema_report(&docs);
        let x = report.fields.iter().find(|f| f.name == "x").unwrap();
        let stats = x.numeric_stats.as_ref().unwrap();
        assert_eq!(stats.buckets.len(), 1);
        assert_eq!(stats.buckets[0].count, 3);
        assert_eq!(stats.min, 5.0);
        assert_eq!(stats.max, 5.0);
    }

    #[test]
    fn date_histogram_buckets_sum_to_total() {
        let base = 1_700_000_000_000i64;
        let docs: Vec<Document> = (0..10i64)
            .map(|i| doc! { "ts": bson::DateTime::from_millis(base + i * 1000) })
            .collect();
        let report = compute_schema_report(&docs);
        let ts = report.fields.iter().find(|f| f.name == "ts").unwrap();
        let stats = ts.date_stats.as_ref().expect("date_stats present");
        assert_eq!(stats.min_ms, base);
        assert_eq!(stats.max_ms, base + 9000);
        assert_eq!(stats.buckets.len(), NUM_BUCKETS);
        let total: u64 = stats.buckets.iter().map(|b| b.count).sum();
        assert_eq!(total, 10);
    }

    #[test]
    fn mixed_numeric_types_aggregate_into_one_stats() {
        let docs = vec![doc! { "n": 1i32 }, doc! { "n": 2i64 }, doc! { "n": 3.5f64 }];
        let report = compute_schema_report(&docs);
        let n = report.fields.iter().find(|f| f.name == "n").unwrap();
        // Three distinct BSON types recorded.
        assert_eq!(n.types.get("int"), Some(&1));
        assert_eq!(n.types.get("long"), Some(&1));
        assert_eq!(n.types.get("double"), Some(&1));
        let stats = n.numeric_stats.as_ref().unwrap();
        assert_eq!(stats.min, 1.0);
        assert!((stats.max - 3.5).abs() < 1e-9);
        assert!((stats.mean - (1.0 + 2.0 + 3.5) / 3.0).abs() < 1e-9);
    }

    #[test]
    fn fields_sorted_by_name() {
        let docs = vec![doc! { "zeta": 1, "alpha": 2, "mid": 3 }];
        let report = compute_schema_report(&docs);
        let names: Vec<&str> = report.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn long_string_value_truncated_in_top_values() {
        let long = "a".repeat(100);
        let docs = vec![doc! { "s": long.as_str() }];
        let report = compute_schema_report(&docs);
        let s = report.fields.iter().find(|f| f.name == "s").unwrap();
        let top = s.top_values.as_ref().unwrap();
        assert!(top[0].value.ends_with('…'));
        assert!(top[0].value.chars().count() <= VALUE_LABEL_MAX + 1);
    }
}
