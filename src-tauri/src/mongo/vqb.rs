//! Visual Query Builder — node tree to MongoDB filter conversion.
//!
//! A VQB query is a tree of `VqbNode`s: nested groups (`$and` / `$or` / `$nor`)
//! and leaf conditions (field / operator / value). The backend converts this tree
//! to a plain MongoDB filter document and accepts date tags (`#today`) in string
//! values just like the rest of the filter pipeline.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::expand_date_tags;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VqbCombinator {
    And,
    Or,
    Nor,
}

impl VqbCombinator {
    fn as_mongo_op(&self) -> &'static str {
        match self {
            VqbCombinator::And => "$and",
            VqbCombinator::Or => "$or",
            VqbCombinator::Nor => "$nor",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum VqbNode {
    Group {
        combinator: VqbCombinator,
        children: Vec<VqbNode>,
    },
    Condition {
        field: String,
        operator: String,
        /// `None` means the value is intentionally absent (e.g. `is null`).
        value: Option<serde_json::Value>,
        /// Disabled conditions are ignored when building the filter.
        enabled: bool,
    },
}

impl VqbNode {
    /// Create a default root group with a single empty condition.
    pub fn default_root() -> Self {
        VqbNode::Group {
            combinator: VqbCombinator::And,
            children: vec![VqbNode::Condition {
                field: String::new(),
                operator: "eq".into(),
                value: None,
                enabled: true,
            }],
        }
    }
}

/// Convert a VQB node tree into a MongoDB filter document.
///
/// The result is returned as a JSON value so the frontend can display it as a
/// filter string; the regular `find_documents` command will parse it back to BSON.
pub fn to_filter(node: &VqbNode) -> AppResult<serde_json::Value> {
    let filter = build_filter(node)?;
    Ok(filter)
}

fn build_filter(node: &VqbNode) -> AppResult<serde_json::Value> {
    match node {
        VqbNode::Group { combinator, children } => {
            let parts: Vec<_> = children
                .iter()
                .filter_map(|child| match build_filter(child) {
                    Ok(serde_json::Value::Object(map)) if map.is_empty() => None,
                    Ok(v) => Some(v),
                    Err(e) => {
                        // Propagate errors via a sentinel we can detect later.
                        // In practice every child is independent, so collecting
                        // here is fine; we'll return the first error.
                        Some(serde_json::Value::String(format!("__error:{e}")))
                    }
                })
                .collect();
            if parts.is_empty() {
                return Ok(serde_json::json!({}));
            }
            // Check for any propagated error.
            if let Some(err) = parts.iter().find_map(|v| v.as_str().filter(|s| s.starts_with("__error:"))) {
                return Err(AppError::Internal(err.strip_prefix("__error:").unwrap_or(err).into()));
            }
            Ok(serde_json::json!({
                combinator.as_mongo_op(): parts
            }))
        }
        VqbNode::Condition {
            field,
            operator,
            value,
            enabled,
        } => {
            if !enabled || field.trim().is_empty() {
                return Ok(serde_json::json!({}));
            }
            let mongo_op = mongo_operator(operator)?;
            let value = parse_condition_value(value, operator)?;
            if mongo_op == "$text" {
                return Ok(serde_json::json!({ "$text": { "$search": value } }));
            }
            if mongo_op == "$exists" {
                return Ok(serde_json::json!({ field: { "$exists": value } }));
            }
            if mongo_op == "$type" {
                return Ok(serde_json::json!({ field: { "$type": value } }));
            }
            if mongo_op == "$size" {
                return Ok(serde_json::json!({ field: { "$size": value } }));
            }
            if operator == "is_null" {
                return Ok(serde_json::json!({ field: serde_json::Value::Null }));
            }
            if operator == "is_not_null" {
                return Ok(serde_json::json!({ field: { "$ne": serde_json::Value::Null } }));
            }
            Ok(serde_json::json!({ field: { mongo_op: value } }))
        }
    }
}

fn mongo_operator(operator: &str) -> AppResult<&'static str> {
    match operator {
        "eq" => Ok("$eq"),
        "ne" => Ok("$ne"),
        "gt" => Ok("$gt"),
        "gte" => Ok("$gte"),
        "lt" => Ok("$lt"),
        "lte" => Ok("$lte"),
        "in" => Ok("$in"),
        "nin" => Ok("$nin"),
        "exists" => Ok("$exists"),
        "regex" => Ok("$regex"),
        "text" => Ok("$text"),
        "type" => Ok("$type"),
        "size" => Ok("$size"),
        "is_null" => Ok("$is_null"),
        "is_not_null" => Ok("$is_not_null"),
        _ => Err(AppError::SqlParse(format!("unknown VQB operator: {operator}"))),
    }
}

/// Parse the value the user typed into a strongly typed JSON value.
///
/// - `exists` / `type` / `size` are treated specially.
/// - `in` / `nin` expect either a JSON array string or a plain comma-separated list.
/// - String values that look like booleans, null, or numbers are coerced.
/// - String values starting with `#` are expanded as date tags.
fn parse_condition_value(
    value: &Option<serde_json::Value>,
    operator: &str,
) -> AppResult<serde_json::Value> {
    let raw = match value {
        Some(v) => v.clone(),
        None => return Ok(serde_json::Value::Null),
    };

    match operator {
        "exists" => Ok(serde_json::json!(raw.as_bool().unwrap_or(true))),
        "type" => Ok(coerce_to_string_or_number(raw)),
        "size" => Ok(coerce_to_number(raw)),
        "in" | "nin" => parse_array_value(raw),
        _ => parse_scalar_value(raw),
    }
}

fn parse_scalar_value(raw: serde_json::Value) -> AppResult<serde_json::Value> {
    let mut json = match raw {
        serde_json::Value::String(s) => parse_scalar_string(&s),
        other => other,
    };
    // Run date-tag expansion on any string that survives.
    expand_date_tags(&mut json);
    Ok(json)
}

fn parse_scalar_string(s: &str) -> serde_json::Value {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return serde_json::Value::String(s.into());
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return serde_json::Value::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return serde_json::Value::Bool(false);
    }
    if trimmed.eq_ignore_ascii_case("null") {
        return serde_json::Value::Null;
    }
    if let Ok(n) = trimmed.parse::<f64>() {
        // Preserve integers as integers if possible for cleaner JSON.
        if n.is_nan() || n.is_infinite() {
            return serde_json::Value::String(s.into());
        }
        if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
            return serde_json::json!(n as i64);
        }
        return serde_json::json!(n);
    }
    serde_json::Value::String(s.into())
}

fn parse_array_value(raw: serde_json::Value) -> AppResult<serde_json::Value> {
    let mut items: Vec<serde_json::Value> = Vec::new();
    match raw {
        serde_json::Value::Array(arr) => {
            for v in arr {
                items.push(parse_scalar_value(v)?);
            }
        }
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            // Try JSON array first.
            if trimmed.starts_with('[') {
                if let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(trimmed) {
                    for v in arr {
                        items.push(parse_scalar_value(v)?);
                    }
                } else {
                    return Err(AppError::SqlParse(format!(
                        "invalid JSON array for $in/$nin: {s}"
                    )));
                }
            } else {
                // Comma-separated list.
                for part in trimmed.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
                    items.push(parse_scalar_string(part));
                }
            }
        }
        other => {
            return Err(AppError::SqlParse(format!(
                "$in/$nin value must be an array or comma-separated list, got {other}"
            )));
        }
    }
    // Expand date tags inside each item.
    for item in &mut items {
        expand_date_tags(item);
    }
    Ok(serde_json::Value::Array(items))
}

fn coerce_to_string_or_number(raw: serde_json::Value) -> serde_json::Value {
    match raw {
        serde_json::Value::String(s) => {
            if let Ok(n) = s.parse::<i64>() {
                serde_json::json!(n)
            } else {
                serde_json::Value::String(s)
            }
        }
        other => other,
    }
}

fn coerce_to_number(raw: serde_json::Value) -> serde_json::Value {
    match raw {
        serde_json::Value::String(s) => {
            if let Ok(n) = s.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(n) = s.parse::<f64>() {
                serde_json::json!(n)
            } else {
                serde_json::Value::String(s)
            }
        }
        serde_json::Value::Number(n) => serde_json::Value::Number(n),
        _ => serde_json::Value::Number(serde_json::Number::from(0)),
    }
}

/// Try to parse a MongoDB filter document back into a VQB node tree.
///
/// This is best-effort: only the patterns produced by `to_filter` are guaranteed
/// to round-trip. Unknown shapes are returned as a single raw condition with the
/// `eq` operator holding the original value.
pub fn from_filter(value: &serde_json::Value) -> VqbNode {
    from_filter_value(value).unwrap_or_else(VqbNode::default_root)
}

fn from_filter_value(value: &serde_json::Value) -> Option<VqbNode> {
    let obj = value.as_object()?;
    if obj.is_empty() {
        return None;
    }

    // Check for top-level group operators.
    for (op, arr) in [("$and", VqbCombinator::And), ("$or", VqbCombinator::Or), ("$nor", VqbCombinator::Nor)] {
        if let Some(children) = obj.get(op).and_then(|v| v.as_array()) {
            let nodes: Vec<_> = children.iter().filter_map(from_filter_value).collect();
            if nodes.is_empty() {
                return None;
            }
            return Some(VqbNode::Group {
                combinator: arr,
                children: nodes,
            });
        }
    }

    // Otherwise treat as a flat condition per key.
    let mut children: Vec<VqbNode> = Vec::new();
    for (field, val) in obj.iter() {
        if field == "$text" {
            if let Some(search) = val.get("$search") {
                children.push(VqbNode::Condition {
                    field: "$text".into(),
                    operator: "text".into(),
                    value: Some(search.clone()),
                    enabled: true,
                });
            }
            continue;
        }
        if let Some(cond) = condition_from_field_value(field, val) {
            children.push(cond);
        }
    }
    if children.is_empty() {
        return None;
    }
    if children.len() == 1 {
        return children.into_iter().next();
    }
    Some(VqbNode::Group {
        combinator: VqbCombinator::And,
        children,
    })
}

fn condition_from_field_value(field: &str, value: &serde_json::Value) -> Option<VqbNode> {
    if let serde_json::Value::Object(map) = value {
        if map.is_empty() {
            return None;
        }
        // If the object has exactly one operator key, parse it.
        let mut entries = map.iter();
        if let Some((op, v)) = entries.next() {
            if entries.next().is_none() {
                let operator = mongo_to_vqb_op(op)?;
                return Some(VqbNode::Condition {
                    field: field.into(),
                    operator,
                    value: Some(v.clone()),
                    enabled: true,
                });
            }
        }
        // Multi-operator object: turn into a group of conditions on the same field.
        let children: Vec<_> = map
            .iter()
            .filter_map(|(op, v)| {
                let operator = mongo_to_vqb_op(op)?;
                Some(VqbNode::Condition {
                    field: field.into(),
                    operator,
                    value: Some(v.clone()),
                    enabled: true,
                })
            })
            .collect();
        if children.is_empty() {
            return None;
        }
        return Some(VqbNode::Group {
            combinator: VqbCombinator::And,
            children,
        });
    }

    // Direct value: treat as equality.
    let operator = if value.is_null() { "is_null" } else { "eq" };
    Some(VqbNode::Condition {
        field: field.into(),
        operator: operator.into(),
        value: Some(value.clone()),
        enabled: true,
    })
}

fn mongo_to_vqb_op(op: &str) -> Option<String> {
    match op {
        "$eq" => Some("eq".into()),
        "$ne" => Some("ne".into()),
        "$gt" => Some("gt".into()),
        "$gte" => Some("gte".into()),
        "$lt" => Some("lt".into()),
        "$lte" => Some("lte".into()),
        "$in" => Some("in".into()),
        "$nin" => Some("nin".into()),
        "$exists" => Some("exists".into()),
        "$regex" => Some("regex".into()),
        "$type" => Some("type".into()),
        "$size" => Some("size".into()),
        _ => None,
    }
}

/// Convenience: convert a JSON filter string into a VQB node tree.
///
/// Returns `Ok(None)` for an empty filter. Returns an error if the string is
/// not valid JSON.
pub fn parse_filter_to_vqb(input: &str) -> AppResult<Option<VqbNode>> {
    if input.trim().is_empty() || input.trim() == "{}" {
        return Ok(None);
    }
    let json: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| AppError::SqlParse(format!("invalid filter JSON: {e}")))?;
    Ok(from_filter_value(&json))
}

/// Convenience: take a VQB node tree and return a compact JSON filter string.
pub fn vqb_to_filter_string(node: &VqbNode) -> AppResult<String> {
    let filter = to_filter(node)?;
    Ok(filter.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_condition_to_filter() {
        let node = VqbNode::Condition {
            field: "price".into(),
            operator: "gt".into(),
            value: Some(serde_json::json!("10")),
            enabled: true,
        };
        let filter = to_filter(&node).unwrap();
        assert_eq!(filter["price"]["$gt"], 10);
    }

    #[test]
    fn disabled_condition_ignored() {
        let node = VqbNode::Condition {
            field: "price".into(),
            operator: "gt".into(),
            value: Some(serde_json::json!("10")),
            enabled: false,
        };
        let filter = to_filter(&node).unwrap();
        assert!(filter.as_object().unwrap().is_empty());
    }

    #[test]
    fn group_and() {
        let node = VqbNode::Group {
            combinator: VqbCombinator::And,
            children: vec![
                VqbNode::Condition {
                    field: "a".into(),
                    operator: "eq".into(),
                    value: Some(serde_json::json!("1")),
                    enabled: true,
                },
                VqbNode::Condition {
                    field: "b".into(),
                    operator: "eq".into(),
                    value: Some(serde_json::json!("2")),
                    enabled: true,
                },
            ],
        };
        let filter = to_filter(&node).unwrap();
        let arr = filter["$and"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn in_operator_parses_comma_list() {
        let node = VqbNode::Condition {
            field: "status".into(),
            operator: "in".into(),
            value: Some(serde_json::json!("a, b, c")),
            enabled: true,
        };
        let filter = to_filter(&node).unwrap();
        let arr = filter["status"]["$in"].as_array().unwrap();
        let expected: Vec<serde_json::Value> = vec!["a".into(), "b".into(), "c".into()];
        assert_eq!(*arr, expected);
    }

    #[test]
    fn text_operator_emits_dollar_text() {
        let node = VqbNode::Condition {
            field: "$text".into(),
            operator: "text".into(),
            value: Some(serde_json::json!("foo bar")),
            enabled: true,
        };
        let filter = to_filter(&node).unwrap();
        assert_eq!(filter["$text"]["$search"], "foo bar");
    }

    #[test]
    fn round_trip_simple_filter() {
        let json = serde_json::json!({ "price": { "$gt": 10 }, "name": "alice" });
        let node = from_filter_value(&json).unwrap();
        let filter = to_filter(&node).unwrap();
        let and = filter["$and"].as_array().unwrap();
        assert_eq!(and[0]["price"]["$gt"], 10);
        assert_eq!(and[1]["name"]["$eq"], "alice");
    }

    #[test]
    fn round_trip_and_group() {
        let json = serde_json::json!({ "$and": [{"a": 1}, {"b": 2}] });
        let node = from_filter_value(&json).unwrap();
        let filter = to_filter(&node).unwrap();
        assert_eq!(filter["$and"][0]["a"]["$eq"], 1);
        assert_eq!(filter["$and"][1]["b"]["$eq"], 2);
    }

    #[test]
    fn round_trip_or_group() {
        let json = serde_json::json!({ "$or": [{"a": 1}, {"b": 2}] });
        let node = from_filter_value(&json).unwrap();
        let filter = to_filter(&node).unwrap();
        assert_eq!(filter["$or"][0]["a"]["$eq"], 1);
    }

    #[test]
    fn date_tag_expanded_in_scalar_value() {
        let node = VqbNode::Condition {
            field: "createdAt".into(),
            operator: "gte".into(),
            value: Some(serde_json::json!("#today")),
            enabled: true,
        };
        let filter = to_filter(&node).unwrap();
        assert!(filter["createdAt"]["$gte"].get("$date").is_some());
    }
}
