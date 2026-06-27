//! BSON <-> JSON helpers.
//!
//! Documents are returned to the frontend as MongoDB Extended JSON
//! (canonical form) so that ObjectId, Date, Decimal128, Binary, and other
//! BSON types round-trip without losing fidelity. Filter/projection/sort
//! strings are parsed as Extended JSON; we fall back to plain JSON for
//! hand-written `{}` filters.

use bson::{Bson, Document};
use chrono::{DateTime, Duration, Utc};

use crate::error::{AppError, AppResult};

/// Parse a JSON value into a BSON `Document`. Accepts either Extended JSON
/// (the canonical or relaxed forms) or plain JSON. Any string values that
/// look like date tags (`#today`, `#lastweek`, etc.) are expanded into
/// MongoDB Extended JSON date objects before conversion to BSON.
pub fn parse_filter(input: &str) -> AppResult<Document> {
    let mut json: serde_json::Value = serde_json::from_str(input)?;
    expand_date_tags(&mut json);
    let bson: Bson = bson::to_bson(&json)?;
    match bson {
        Bson::Document(doc) => Ok(doc),
        other => Err(AppError::InvalidBson(format!(
            "filter must be a JSON object, got {}",
            other_element_type(&other)
        ))),
    }
}

/// Recognised date tags and their UTC start-of-day resolution.
///
/// Supported tags:
/// - `#today`      -> start of today UTC
/// - `#yesterday`  -> start of yesterday UTC
/// - `#lastweek`   -> start of 7 days ago UTC
/// - `#lastmonth`  -> start of 30 days ago UTC
pub fn resolve_date_tag(tag: &str) -> Option<DateTime<Utc>> {
    let now = Utc::now();
    let today = now.date_naive();
    let target = match tag.to_lowercase().as_str() {
        "today" => today,
        "yesterday" => today.checked_sub_signed(Duration::days(1))?,
        "lastweek" | "last_week" => today.checked_sub_signed(Duration::days(7))?,
        "lastmonth" | "last_month" => today.checked_sub_signed(Duration::days(30))?,
        _ => return None,
    };
    Some(DateTime::from_naive_utc_and_offset(
        target.and_hms_opt(0, 0, 0)?,
        Utc,
    ))
}

/// Encode a UTC date as MongoDB Extended JSON: `{"$date": {"$numberLong": "..."}}`.
pub fn date_to_extjson(dt: DateTime<Utc>) -> serde_json::Value {
    serde_json::json!({
        "$date": { "$numberLong": dt.timestamp_millis().to_string() }
    })
}

/// Recursively expand date tags inside a JSON value in place.
pub(crate) fn expand_date_tags(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            if let Some(tag) = s.strip_prefix('#') {
                if let Some(dt) = resolve_date_tag(tag) {
                    *value = date_to_extjson(dt);
                }
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values_mut() {
                expand_date_tags(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                expand_date_tags(v);
            }
        }
        _ => {}
    }
}

/// Parse a JSON value into a BSON `Document`, or `Ok(None)` for empty input.
pub fn parse_optional_doc(input: Option<&str>) -> AppResult<Option<Document>> {
    match input {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => Ok(Some(parse_filter(s)?)),
    }
}

/// Encode a BSON `Document` as Extended JSON (`serde_json::Value` form).
pub fn doc_to_extjson(doc: &Document) -> AppResult<serde_json::Value> {
    let bson = Bson::Document(doc.clone());
    let ext = bson.into_relaxed_extjson();
    Ok(ext)
}

/// Encode a BSON value as Extended JSON.
pub fn bson_to_extjson(value: &Bson) -> AppResult<serde_json::Value> {
    Ok(value.clone().into_relaxed_extjson())
}

/// Encode a BSON `Document` as a plain JSON `Value`, with BSON-only types
/// (ObjectId, Date, Binary, etc.) stringified for display. Used in the UI
/// when extended JSON would be too noisy (the result grid uses this and
/// offers a "raw extended JSON" toggle in the JSON view).
pub fn doc_to_display_json(doc: &Document) -> AppResult<serde_json::Value> {
    let bson = Bson::Document(doc.clone());
    let mut json = bson.clone().into_relaxed_extjson();
    simplify_for_display(&mut json);
    Ok(json)
}

fn simplify_for_display(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (_k, v) in map.iter_mut() {
                simplify_for_display(v);
            }
            if let Some(serde_json::Value::String(oid)) = map.get("$oid") {
                let oid = oid.clone();
                map.remove("$oid");
                map.insert("_idDisplay".to_string(), serde_json::Value::String(oid));
            }
            if let Some(serde_json::Value::Object(inner)) = map.get("$date") {
                if let Some(serde_json::Value::String(iso)) =
                    inner.get("$numberLong").or_else(|| inner.get("$numberInt"))
                {
                    let iso = iso.clone();
                    map.remove("$date");
                    map.insert(
                        "_dateDisplay".to_string(),
                        serde_json::Value::String(iso.clone()),
                    );
                } else if let Some(serde_json::Value::String(s)) = inner.get("$string") {
                    let s = s.clone();
                    map.remove("$date");
                    map.insert("_dateDisplay".to_string(), serde_json::Value::String(s));
                }
            }
            if let Some(serde_json::Value::Object(inner)) = map.get("$numberDecimal") {
                if let Some(serde_json::Value::String(s)) = inner.get("$numberString") {
                    let s = s.clone();
                    map.remove("$numberDecimal");
                    map.insert("_decimalDisplay".to_string(), serde_json::Value::String(s));
                }
            }
            if let Some(serde_json::Value::String(b64)) = map.get("$binary") {
                let b64 = b64.clone();
                map.remove("$binary");
                map.insert("_binaryDisplay".to_string(), serde_json::Value::String(b64));
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr.iter_mut() {
                simplify_for_display(v);
            }
        }
        _ => {}
    }
}

fn other_element_type(b: &Bson) -> &'static str {
    match b {
        Bson::Double(_) => "double",
        Bson::String(_) => "string",
        Bson::Array(_) => "array",
        Bson::Document(_) => "document",
        Bson::Boolean(_) => "bool",
        Bson::Null => "null",
        Bson::Int32(_) => "int32",
        Bson::Int64(_) => "int64",
        _ => "bson",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn round_trip_object_id_via_extjson() {
        let oid = bson::oid::ObjectId::new();
        let doc = doc! { "_id": oid, "n": 1i32 };
        let json = doc_to_extjson(&doc).expect("encode");
        assert!(json.get("_id").is_some());
    }

    #[test]
    fn parse_filter_rejects_non_object() {
        let result = parse_filter("[1,2,3]");
        assert!(result.is_err());
    }

    #[test]
    fn parse_optional_doc_handles_none() {
        let result = parse_optional_doc(None).expect("ok");
        assert!(result.is_none());
    }

    #[test]
    fn expands_today_tag_to_date() {
        let doc = parse_filter(r##"{"createdAt": "#today"}"##).expect("parse");
        let bson_date = doc.get("createdAt").expect("field");
        assert!(matches!(bson_date, Bson::DateTime(_)));
    }

    #[test]
    fn expands_lastweek_tag_to_date() {
        let doc = parse_filter(r##"{"createdAt": {"$gte": "#lastweek"}}"##).expect("parse");
        let inner = doc
            .get_document("createdAt")
            .expect("object")
            .get("$gte")
            .expect("$gte");
        assert!(matches!(inner, Bson::DateTime(_)));
    }

    #[test]
    fn preserves_unknown_hash_tags() {
        // Unknown tags are left as strings so the user sees a clear
        // runtime error rather than silently losing the value.
        let doc = parse_filter(r##"{"name": "#nope"}"##).expect("parse");
        assert_eq!(doc.get_str("name").expect("string"), "#nope");
    }

    #[test]
    fn expands_tags_inside_arrays() {
        let doc =
            parse_filter(r##"{"createdAt": {"$in": ["#today", "#yesterday"]}}"##).expect("parse");
        let arr = doc
            .get_document("createdAt")
            .expect("object")
            .get_array("$in")
            .expect("array");
        assert!(arr.iter().all(|v| matches!(v, Bson::DateTime(_))));
    }

    /// Regression guard for the silent-update bug: `find_documents`
    /// returns `_id` in *display* form (`{ _idDisplay: "hex" }`), and
    /// sending that back as a filter must NOT parse as an ObjectId —
    /// otherwise a display-form `_id` would silently match nothing.
    /// The frontend reconstructs `{ $oid: "hex" }` before sending; this
    /// test pins both halves of that contract at the backend boundary.
    #[test]
    fn display_form_id_does_not_parse_as_objectid_but_reconstructed_does() {
        let oid = bson::oid::ObjectId::new();
        let hex = oid.to_hex();
        let doc = doc! { "_id": oid, "n": 1i32 };

        // What `find_documents` actually returns to the frontend.
        let display = doc_to_display_json(&doc).expect("display");
        let id_display = display.get("_id").expect("_id present");
        assert_eq!(
            id_display.get("_idDisplay").and_then(|v| v.as_str()),
            Some(hex.as_str()),
            "display form should expose _idDisplay"
        );
        assert!(
            id_display.get("$oid").is_none(),
            "display form must NOT retain $oid (that's the whole point of simplify_for_display)"
        );

        // Sending the display form back as a filter parses to a
        // subdocument, NOT an ObjectId — this is the bug, and it must
        // stay detectable so the frontend's reconstruction stays
        // necessary.
        let display_filter = format!(r#"{{"_id":{{"_idDisplay":"{hex}"}}}}"#);
        let parsed_display = parse_filter(&display_filter).expect("parse display filter");
        let parsed_display_id = parsed_display.get("_id").expect("_id");
        assert!(
            matches!(parsed_display_id, Bson::Document(_)),
            "display-form _id must parse as a subdocument, not an ObjectId — \
             otherwise the round-trip bug would be invisible"
        );

        // The reconstructed extended-JSON form the frontend now sends
        // must parse back to the original ObjectId.
        let reconstructed = format!(r#"{{"_id":{{"$oid":"{hex}"}}}}"#);
        let parsed = parse_filter(&reconstructed).expect("parse reconstructed filter");
        let parsed_id = parsed.get("_id").expect("_id");
        match parsed_id {
            Bson::ObjectId(parsed_oid) => {
                assert_eq!(parsed_oid, &oid, "reconstructed $oid must equal original");
            }
            other => panic!("expected ObjectId, got {other:?}"),
        }
    }
}
