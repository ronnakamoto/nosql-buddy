//! Field-mapping transform for the import/export pipeline.
//!
//! A `FieldMappingTransform` is a pure [`Transform`] adapter that rewrites each
//! document according to a user-edited mapping table: rename, skip, flatten
//! (dotted source paths), and optional type coercion. It plugs into the same
//! streaming pipeline used by import and export, so no individual format has to
//! know about it.
//!
//! "Flatten" is expressed as multiple entries whose `source` is a dotted path
//! into a nested document (e.g. `address.city` -> `city`). The UI derives those
//! entries from a schema sample or import preview by expanding nested objects.
//!
//! The transform is order-independent: it builds a fresh output document from
//! the declared entries, so a `target` can never collide with a `source` it
//! also reads from (no in-place mutation hazards).

use bson::{Bson, DateTime, Document};
use serde::{Deserialize, Serialize};

use super::core::Transform;
use crate::error::{AppError, AppResult};

/// One row in the field-mapping table.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMappingEntry {
    /// Dotted path into the source document (e.g. `address.city`).
    pub source: String,
    /// Output field name. May equal `source` (identity rename).
    pub target: String,
    /// If true, this field is dropped from the output entirely.
    pub skip: bool,
    /// Optional BSON type coercion applied after extraction.
    pub type_override: Option<TypeOverride>,
}

/// The set of BSON types the user can force a field into. Keeps the surface
/// small and the round-trip contract explicit.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TypeOverride {
    String,
    Int32,
    Int64,
    Double,
    Boolean,
    /// milliseconds since epoch
    Date,
    ObjectId,
}

/// A field-mapping transform built from a confirmed mapping table.
///
/// Construct via [`FieldMappingTransform::new`] so the entries are validated
/// once up front (non-empty source/target, no duplicate non-skipped targets).
#[derive(Debug)]
pub struct FieldMappingTransform {
    entries: Vec<FieldMappingEntry>,
}

impl FieldMappingTransform {
    pub fn new(entries: Vec<FieldMappingEntry>) -> AppResult<Self> {
        validate(&entries)?;
        Ok(Self { entries })
    }

    /// The ordered list of output column names for a CSV sink: the non-skipped
    /// `target` values, in declared order. Empty when no mapping is active.
    pub fn csv_columns(entries: &[FieldMappingEntry]) -> Vec<String> {
        entries
            .iter()
            .filter(|e| !e.skip)
            .map(|e| e.target.clone())
            .collect()
    }
}

impl Transform for FieldMappingTransform {
    fn apply(&self, mut doc: Document) -> AppResult<Option<Document>> {
        // Build a fresh output document so renamed targets never shadow sources
        // that later entries still need to read.
        let mut out = Document::new();
        for entry in &self.entries {
            if entry.skip {
                continue;
            }
            let Some(value) = get_bson_path(&doc, &entry.source) else {
                continue;
            };
            let coerced = match entry.type_override {
                Some(t) => coerce(value.clone(), t)?,
                None => value.clone(),
            };
            insert_dotted(&mut out, &entry.target, coerced);
        }
        // Free the input before returning so the sink never sees stale fields.
        doc.clear();
        Ok(Some(out))
    }
}

/// Walk a dotted path (`a.b.c`) into a document, returning the leaf value.
fn get_bson_path<'a>(doc: &'a Document, path: &str) -> Option<&'a Bson> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut current = doc.get(first)?;
    for part in parts {
        match current {
            Bson::Document(d) => current = d.get(part)?,
            _ => return None,
        }
    }
    Some(current)
}

/// Insert a value at a dotted path, creating intermediate documents as needed.
/// A path segment that collides with a non-document value is an error.
fn insert_dotted(out: &mut Document, path: &str, value: Bson) {
    let mut parts = path.split('.');
    let first = match parts.next() {
        Some(f) => f,
        None => return,
    };
    let rest: Vec<&str> = parts.collect();
    if rest.is_empty() {
        out.insert(first, value);
        return;
    }
    let entry = out
        .entry(first.to_string())
        .or_insert_with(|| Bson::Document(Document::new()));
    if let Bson::Document(inner) = entry {
        insert_dotted(inner, &rest.join("."), value);
    } else {
        // Replace a scalar leaf with a document when the user mapped a nested
        // target under it; this matches how most import wizards behave.
        let mut inner = Document::new();
        insert_dotted(&mut inner, &rest.join("."), value);
        *entry = Bson::Document(inner);
    }
}

/// Coerce a BSON value into the declared target type. Lossy conversions are
/// explicit: unknown coercions return a structured error so the pipeline counts
/// them as row errors instead of silently dropping data.
fn coerce(value: Bson, target: TypeOverride) -> AppResult<Bson> {
    // Unwrap null early: a null stays a null regardless of the declared type,
    // so missing fields don't turn into bogus zeros.
    if matches!(value, Bson::Null) {
        return Ok(Bson::Null);
    }
    Ok(match target {
        TypeOverride::String => Bson::String(bson_to_string(&value)),
        TypeOverride::Int32 => Bson::Int32(bson_to_i32(&value)?),
        TypeOverride::Int64 => Bson::Int64(bson_to_i64(&value)?),
        TypeOverride::Double => Bson::Double(bson_to_f64(&value)?),
        TypeOverride::Boolean => Bson::Boolean(bson_to_bool(&value)?),
        TypeOverride::Date => Bson::DateTime(bson_to_date(&value)?),
        TypeOverride::ObjectId => Bson::ObjectId(bson_to_objectid(&value)?),
    })
}

fn bson_to_string(value: &Bson) -> String {
    match value {
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(n) => n.to_string(),
        Bson::Boolean(b) => b.to_string(),
        Bson::ObjectId(oid) => oid.to_hex(),
        Bson::DateTime(dt) => dt
            .try_to_rfc3339_string()
            .unwrap_or_else(|_| dt.timestamp_millis().to_string()),
        Bson::Decimal128(d) => d.to_string(),
        // Nested types serialize as relaxed Extended JSON, matching the CSV cell
        // encoding rules so a string override round-trips predictably.
        other => {
            let json = other.clone().into_relaxed_extjson();
            serde_json::to_string(&json).unwrap_or_default()
        }
    }
}

fn bson_to_i32(value: &Bson) -> AppResult<i32> {
    Ok(match value {
        Bson::Int32(n) => *n,
        Bson::Int64(n) => *n as i32,
        Bson::Double(n) => *n as i32,
        Bson::Boolean(b) => *b as i32,
        Bson::String(s) => s
            .trim()
            .parse::<i32>()
            .map_err(|e| AppError::Validation(format!("cannot coerce \"{s}\" to int32: {e}")))?,
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to int32",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_to_i64(value: &Bson) -> AppResult<i64> {
    Ok(match value {
        Bson::Int32(n) => *n as i64,
        Bson::Int64(n) => *n,
        Bson::Double(n) => *n as i64,
        Bson::Boolean(b) => *b as i64,
        Bson::String(s) => s
            .trim()
            .parse::<i64>()
            .map_err(|e| AppError::Validation(format!("cannot coerce \"{s}\" to int64: {e}")))?,
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to int64",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_to_f64(value: &Bson) -> AppResult<f64> {
    Ok(match value {
        Bson::Int32(n) => *n as f64,
        Bson::Int64(n) => *n as f64,
        Bson::Double(n) => *n,
        Bson::Boolean(b) => *b as i64 as f64,
        Bson::String(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|e| AppError::Validation(format!("cannot coerce \"{s}\" to double: {e}")))?,
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to double",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_to_bool(value: &Bson) -> AppResult<bool> {
    Ok(match value {
        Bson::Boolean(b) => *b,
        Bson::Int32(n) => *n != 0,
        Bson::Int64(n) => *n != 0,
        Bson::Double(n) => *n != 0.0,
        Bson::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "t" => true,
            "false" | "0" | "no" | "n" | "f" | "" => false,
            _ => {
                return Err(AppError::Validation(format!(
                    "cannot coerce \"{s}\" to bool"
                )))
            }
        },
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to bool",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_to_date(value: &Bson) -> AppResult<DateTime> {
    Ok(match value {
        Bson::DateTime(dt) => *dt,
        Bson::String(s) => {
            let trimmed = s.trim();
            // Try RFC3339 first, then bare epoch millis, then bare epoch seconds.
            if let Ok(dt) = DateTime::parse_rfc3339_str(trimmed) {
                dt
            } else if let Ok(millis) = trimmed.parse::<i64>() {
                DateTime::from_millis(millis)
            } else if let Ok(secs) = trimmed.parse::<i64>() {
                DateTime::from_millis(secs.saturating_mul(1000))
            } else {
                return Err(AppError::Validation(format!(
                    "cannot coerce \"{s}\" to date (expected RFC3339 or epoch millis)"
                )));
            }
        }
        Bson::Int32(n) => DateTime::from_millis(*n as i64),
        Bson::Int64(n) => DateTime::from_millis(*n),
        Bson::Double(n) => DateTime::from_millis(*n as i64),
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to date",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_to_objectid(value: &Bson) -> AppResult<bson::oid::ObjectId> {
    Ok(match value {
        Bson::ObjectId(oid) => *oid,
        Bson::String(s) => bson::oid::ObjectId::parse_str(s.trim()).map_err(|e| {
            AppError::Validation(format!("cannot coerce \"{s}\" to objectId: {e}"))
        })?,
        other => {
            return Err(AppError::Validation(format!(
                "cannot coerce {} to objectId",
                bson_type_name(other)
            )))
        }
    })
}

fn bson_type_name(value: &Bson) -> &'static str {
    match value {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Document(_) => "object",
        Bson::Array(_) => "array",
        Bson::ObjectId(_) => "objectId",
        Bson::Boolean(_) => "bool",
        Bson::DateTime(_) => "date",
        Bson::Null => "null",
        Bson::Int32(_) => "int",
        Bson::Int64(_) => "long",
        Bson::Decimal128(_) => "decimal",
        Bson::Binary(_) => "binary",
        _ => "other",
    }
}

/// Validate a mapping table once up front so the streaming loop never has to.
fn validate(entries: &[FieldMappingEntry]) -> AppResult<()> {
    let mut seen = std::collections::HashSet::new();
    for (i, e) in entries.iter().enumerate() {
        if e.source.trim().is_empty() {
            return Err(AppError::Validation(format!(
                "field mapping entry {i} has an empty source path"
            )));
        }
        if !e.skip && e.target.trim().is_empty() {
            return Err(AppError::Validation(format!(
                "field mapping entry {i} has an empty target name"
            )));
        }
        if !e.skip && !seen.insert(e.target.trim().to_string()) {
            return Err(AppError::Validation(format!(
                "field mapping has a duplicate target name: \"{}\"",
                e.target.trim()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{doc, oid::ObjectId, Bson, DateTime};
    use std::str::FromStr;

    fn entry(source: &str, target: &str) -> FieldMappingEntry {
        FieldMappingEntry {
            source: source.into(),
            target: target.into(),
            skip: false,
            type_override: None,
        }
    }

    fn skipped(source: &str) -> FieldMappingEntry {
        FieldMappingEntry {
            source: source.into(),
            target: source.into(),
            skip: true,
            type_override: None,
        }
    }

    fn coerce_entry(
        source: &str,
        target: &str,
        t: TypeOverride,
    ) -> FieldMappingEntry {
        FieldMappingEntry {
            source: source.into(),
            target: target.into(),
            skip: false,
            type_override: Some(t),
        }
    }

    #[test]
    fn rename_rewrites_top_level_field() {
        // The mapping table is the complete output schema: undeclared fields
        // (`n`) are dropped, declared ones are renamed.
        let t = FieldMappingTransform::new(vec![entry("name", "fullName")]).unwrap();
        let out = t.apply(doc! { "name": "Ada", "n": 1 }).unwrap().unwrap();
        assert_eq!(out.get_str("fullName").unwrap(), "Ada");
        assert!(out.get("name").is_none());
        assert!(out.get("n").is_none());
    }

    #[test]
    fn skip_drops_a_field_but_keeps_declared_siblings() {
        let t =
            FieldMappingTransform::new(vec![entry("name", "name"), skipped("secret")]).unwrap();
        let out = t
            .apply(doc! { "name": "Ada", "secret": "pw" })
            .unwrap()
            .unwrap();
        assert!(out.get("secret").is_none());
        assert_eq!(out.get_str("name").unwrap(), "Ada");
    }

    #[test]
    fn flatten_dot_path_lifts_nested_field() {
        let t = FieldMappingTransform::new(vec![
            entry("address.city", "city"),
            entry("address.zip", "zip"),
        ])
        .unwrap();
        let out = t
            .apply(doc! { "address": { "city": "NYC", "zip": "10001" } })
            .unwrap()
            .unwrap();
        assert_eq!(out.get_str("city").unwrap(), "NYC");
        assert_eq!(out.get_str("zip").unwrap(), "10001");
        assert!(out.get("address").is_none());
    }

    #[test]
    fn missing_source_is_silently_omitted() {
        // A declared source that's absent from the input contributes nothing;
        // the target simply isn't present in the output.
        let t = FieldMappingTransform::new(vec![entry("missing", "x")]).unwrap();
        let out = t.apply(doc! { "name": "Ada" }).unwrap().unwrap();
        assert!(out.get("x").is_none());
        assert!(out.get("name").is_none());
    }

    #[test]
    fn null_passes_through_type_override() {
        let t = FieldMappingTransform::new(vec![coerce_entry("n", "n", TypeOverride::Int32)])
            .unwrap();
        let out = t.apply(doc! { "n": Bson::Null }).unwrap().unwrap();
        assert!(matches!(out.get("n"), Some(Bson::Null)));
    }

    #[test]
    fn coerce_int_from_string_and_double() {
        let t = FieldMappingTransform::new(vec![coerce_entry("n", "n", TypeOverride::Int32)])
            .unwrap();
        let out = t.apply(doc! { "n": "42" }).unwrap().unwrap();
        assert_eq!(out.get_i32("n").unwrap(), 42);
        let out = t.apply(doc! { "n": 42.9f64 }).unwrap().unwrap();
        assert_eq!(out.get_i32("n").unwrap(), 42);
    }

    #[test]
    fn coerce_bool_from_strings_and_numbers() {
        let t = FieldMappingTransform::new(vec![coerce_entry("flag", "flag", TypeOverride::Boolean)])
            .unwrap();
        for (input, expected) in [
            ("true", true),
            ("yes", true),
            ("1", true),
            ("false", false),
            ("no", false),
            ("0", false),
        ] {
            let out = t.apply(doc! { "flag": input }).unwrap().unwrap();
            assert_eq!(out.get_bool("flag").unwrap(), expected, "input={input}");
        }
        let out = t.apply(doc! { "flag": 1i32 }).unwrap().unwrap();
        assert!(out.get_bool("flag").unwrap());
    }

    #[test]
    fn coerce_date_from_rfc3339_and_epoch_millis() {
        let t = FieldMappingTransform::new(vec![coerce_entry("ts", "ts", TypeOverride::Date)])
            .unwrap();
        let out = t
            .apply(doc! { "ts": "2024-01-01T00:00:00Z" })
            .unwrap()
            .unwrap();
        let dt = out.get_datetime("ts").unwrap();
        assert_eq!(dt.timestamp_millis(), 1_704_067_200_000);

        let out = t.apply(doc! { "ts": 1_704_067_200_000i64 }).unwrap().unwrap();
        assert_eq!(
            out.get_datetime("ts").unwrap().timestamp_millis(),
            1_704_067_200_000
        );
    }

    #[test]
    fn coerce_objectid_from_string() {
        let t =
            FieldMappingTransform::new(vec![coerce_entry("id", "id", TypeOverride::ObjectId)])
                .unwrap();
        let hex = "64f1a2b3c4d5e6f789012345";
        let out = t.apply(doc! { "id": hex }).unwrap().unwrap();
        assert_eq!(out.get_object_id("id").unwrap().to_hex(), hex);
    }

    #[test]
    fn coerce_string_renders_scalars_and_nested() {
        let t =
            FieldMappingTransform::new(vec![coerce_entry("x", "x", TypeOverride::String)])
                .unwrap();
        assert_eq!(
            t.apply(doc! { "x": 42i32 }).unwrap().unwrap().get_str("x").unwrap(),
            "42"
        );
        let nested = t
            .apply(doc! { "x": { "a": 1 } })
            .unwrap()
            .unwrap()
            .get_str("x")
            .unwrap()
            .to_string();
        assert!(nested.contains("\"a\"") && nested.contains("1"));
    }

    #[test]
    fn invalid_coercion_is_a_row_error() {
        let t =
            FieldMappingTransform::new(vec![coerce_entry("n", "n", TypeOverride::Int32)])
                .unwrap();
        let err = t.apply(doc! { "n": "not a number" }).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn duplicate_target_is_rejected_up_front() {
        let err = FieldMappingTransform::new(vec![entry("a", "x"), entry("b", "x")]).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn skipped_entries_do_not_need_unique_targets() {
        // Two skipped rows may share a target name because neither emits. A
        // declared sibling is still passed through.
        let t = FieldMappingTransform::new(vec![
            entry("b", "b"),
            skipped("a"),
            skipped("a"),
        ])
        .unwrap();
        let out = t.apply(doc! { "a": 1, "b": 2 }).unwrap().unwrap();
        assert!(out.get("a").is_none());
        assert_eq!(out.get_i32("b").unwrap(), 2);
    }

    #[test]
    fn csv_columns_skips_dropped_and_preserves_order() {
        let entries = vec![
            entry("_id", "_id"),
            skipped("secret"),
            entry("address.city", "city"),
        ];
        assert_eq!(
            FieldMappingTransform::csv_columns(&entries),
            vec!["_id".to_string(), "city".to_string()]
        );
    }

    #[test]
    fn empty_entries_pass_through_everything_in_input() {
        // No mapping rows => empty output document, by design. The pipeline
        // treats an empty mapping as "drop everything", which the UI never
        // constructs (it always supplies at least the discovered fields).
        let t = FieldMappingTransform::new(vec![]).unwrap();
        let out = t.apply(doc! { "a": 1 }).unwrap().unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn insert_dotted_builds_intermediate_documents() {
        let t = FieldMappingTransform::new(vec![entry("city", "address.city")]).unwrap();
        let out = t.apply(doc! { "city": "NYC" }).unwrap().unwrap();
        assert_eq!(
            out.get_document("address").unwrap().get_str("city").unwrap(),
            "NYC"
        );
    }

    #[test]
    fn rename_does_not_let_target_shadow_a_later_source() {
        // `a -> b` then `b -> c` must read the *original* b, not the renamed one.
        let t = FieldMappingTransform::new(vec![entry("a", "b"), entry("b", "c")]).unwrap();
        let out = t.apply(doc! { "a": 1, "b": 2 }).unwrap().unwrap();
        assert_eq!(out.get_i32("b").unwrap(), 1);
        assert_eq!(out.get_i32("c").unwrap(), 2);
    }

    #[test]
    fn round_trips_through_string_and_back() {
        // Exporting an int as a string and re-importing with an int32 override
        // must restore the original value (the documented round-trip contract).
        let export = FieldMappingTransform::new(vec![coerce_entry("n", "n", TypeOverride::String)])
            .unwrap();
        let exported = export.apply(doc! { "n": 42i32 }).unwrap().unwrap();
        assert_eq!(exported.get_str("n").unwrap(), "42");

        let import = FieldMappingTransform::new(vec![coerce_entry("n", "n", TypeOverride::Int32)])
            .unwrap();
        let reimported = import.apply(exported).unwrap().unwrap();
        assert_eq!(reimported.get_i32("n").unwrap(), 42);
    }

    #[test]
    fn coerce_date_from_bson_datetime_is_identity() {
        let t = FieldMappingTransform::new(vec![coerce_entry("ts", "ts", TypeOverride::Date)])
            .unwrap();
        let original = DateTime::from_millis(1_700_000_000_123);
        let out = t.apply(doc! { "ts": original }).unwrap().unwrap();
        assert_eq!(out.get_datetime("ts").unwrap().timestamp_millis(), 1_700_000_000_123);
    }

    #[test]
    fn coerce_objectid_from_bson_objectid_is_identity() {
        let t =
            FieldMappingTransform::new(vec![coerce_entry("id", "id", TypeOverride::ObjectId)])
                .unwrap();
        let oid = ObjectId::from_str("64f1a2b3c4d5e6f789012345").unwrap();
        let out = t.apply(doc! { "id": oid }).unwrap().unwrap();
        assert_eq!(out.get_object_id("id").unwrap().to_hex(), oid.to_hex());
    }
}
