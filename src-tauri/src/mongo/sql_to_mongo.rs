//! SQL -> MongoDB translation, built on [`sqlparser`] (the Apache
//! DataFusion SQL parser) for ANSI-SQL-accurate parsing. Supports
//! `SELECT` (aliases, arithmetic, `LIKE`/`ILIKE`, `BETWEEN`, `IN`,
//! functions), `FROM`, `WHERE`, `JOIN` (translated to `$lookup`),
//! `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`/`OFFSET`, `DISTINCT`,
//! `UPDATE`, `INSERT`, and `DELETE`. The output is always an
//! aggregation pipeline (or find-shortcut / write document) the user
//! can edit.
//!
//! A few ergonomic, non-standard extensions are layered on top of
//! standard SQL for MongoDB-flavored usage:
//! - `#today`, `#lastweek`, etc. — relative date tags (see
//!   [`crate::mongo::bson_json::resolve_date_tag`]). These tokenize
//!   as plain identifiers (`#today` is accepted as one identifier by
//!   the tokenizer) and are recognized by their leading `#`.
//! - Double-quoted string literals (`"active"`) — ANSI SQL treats
//!   double quotes as *quoted identifiers*, not string literals, but
//!   this dialect targets ad-hoc MongoDB querying where users
//!   commonly reach for double quotes out of JSON/JS habit. A quoted
//!   identifier is therefore treated as a string literal rather than
//!   a (vanishingly unlikely) real quoted column name.
//! - `REMOVE` as an alias for `DELETE`, and a trailing bare `UPSERT`
//!   keyword on `UPDATE ... WHERE ...` to set the upsert flag.
//! - `INSERT INTO <collection> [VALUES] { ...raw JSON... }` — insert
//!   one or more literal MongoDB documents instead of a SQL tuple,
//!   for pasting documents directly.
//!
//! Anything outside standard SQL plus these extensions is rejected
//! with a clear `AppError::SqlParse`.

use serde::Serialize;
use std::collections::BTreeMap;

use sqlparser::ast::{
    AssignmentTarget, BinaryOperator, Delete as SqlDelete, Distinct, Expr as SqlExpr, FromTable,
    Function, FunctionArg, FunctionArgExpr, FunctionArguments, GroupByExpr, Ident,
    Insert as SqlInsert, Join, JoinConstraint, JoinOperator, LimitClause, ObjectName, OrderByKind,
    Query, Select, SelectItem as SqlSelectItem, SetExpr, Statement, TableFactor, TableObject,
    UnaryOperator, Update as SqlUpdate, Value as SqlValue,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser as SqlParser;
use sqlparser::tokenizer::{Token, Tokenizer};

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::{date_to_extjson, resolve_date_tag};
use crate::mongo::query_code::{self, Language};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SqlOperation {
    Find,
    Aggregate,
    Update {
        filter: serde_json::Value,
        update: serde_json::Value,
        multi: bool,
        upsert: bool,
    },
    Insert {
        documents: Vec<serde_json::Value>,
    },
    Delete {
        filter: serde_json::Value,
        multi: bool,
    },
    Replace {
        filter: serde_json::Value,
        replacement: serde_json::Value,
        upsert: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SqlTranslation {
    pub database: String,
    pub collection: String,
    pub operation: SqlOperation,
    pub pipeline: serde_json::Value,
    pub find: Option<serde_json::Value>,
    pub warnings: Vec<String>,
    /// Generated driver code per language, keyed by kebab-case language
    /// name (e.g. "node-js", "python"). Filled in by [`translate`].
    pub code: BTreeMap<String, String>,
}

pub fn translate(database: &str, sql: &str) -> AppResult<SqlTranslation> {
    let mut warnings = Vec::new();
    let trimmed = sql.trim();

    // Non-standard shorthand: paste a raw MongoDB document instead of
    // a SQL tuple. Checked first since it isn't valid SQL and would
    // otherwise fail in the real parser.
    if let Some((collection, documents)) = detect_json_insert(trimmed)? {
        let code = generate_insert_code_variants(database, &collection, &documents);
        return Ok(SqlTranslation {
            database: database.to_string(),
            collection,
            operation: SqlOperation::Insert {
                documents: documents.clone(),
            },
            pipeline: serde_json::Value::Array(vec![]),
            find: None,
            warnings,
            code,
        });
    }

    // `REMOVE` is a MongoDB-flavored alias for `DELETE`, and a leading
    // `UPSERT` is an alias for `INSERT`. Rewrite before handing off to
    // the real SQL parser, which doesn't know either word.
    let normalized = rewrite_leading_alias(trimmed);

    // A trailing bare `UPSERT` keyword on `UPDATE ... WHERE ...` sets
    // the upsert flag; it isn't standard SQL, so strip it (using the
    // tokenizer, so it's never confused with a string literal) before
    // parsing and remember whether it was present.
    let (normalized, upsert) = if is_leading_keyword(&normalized, "UPDATE") {
        strip_trailing_word(&normalized, "UPSERT")
    } else {
        (normalized, false)
    };

    let dialect = GenericDialect {};
    let mut statements = SqlParser::parse_sql(&dialect, &normalized)
        .map_err(|e| AppError::SqlParse(e.to_string()))?;
    if statements.len() != 1 {
        return Err(AppError::SqlParse(
            "expected exactly one SQL statement".into(),
        ));
    }
    let statement = statements.remove(0);

    match statement {
        Statement::Query(query) => translate_select(database, &query, &mut warnings),
        Statement::Update(update) => translate_update(database, &update, upsert, &mut warnings),
        Statement::Insert(insert) => translate_insert(database, &insert, &mut warnings),
        Statement::Delete(delete) => translate_delete(database, &delete, &mut warnings),
        other => Err(AppError::SqlParse(format!(
            "expected SELECT, UPDATE, INSERT, or DELETE statement, got: {other}"
        ))),
    }
}

// ---------- Non-standard leading/trailing keyword handling ----------

/// `true` when `sql` (after leading whitespace) starts with the exact
/// keyword `kw`, case-insensitively, at a word boundary.
fn is_leading_keyword(sql: &str, kw: &str) -> bool {
    let trimmed = sql.trim_start();
    if trimmed.len() < kw.len() {
        return false;
    }
    if !trimmed.as_bytes()[..kw.len()].eq_ignore_ascii_case(kw.as_bytes()) {
        return false;
    }
    !matches!(trimmed[kw.len()..].chars().next(), Some(c) if c.is_ascii_alphanumeric() || c == '_')
}

/// Rewrite a leading `REMOVE` to `DELETE` and a leading `UPSERT` to
/// `INSERT` so the standard SQL parser can handle the rest.
fn rewrite_leading_alias(sql: &str) -> String {
    let ws_len = sql.len() - sql.trim_start().len();
    let (prefix, rest) = sql.split_at(ws_len);
    if is_leading_keyword(rest, "REMOVE") {
        format!("{prefix}DELETE{}", &rest["REMOVE".len()..])
    } else if is_leading_keyword(rest, "UPSERT") {
        format!("{prefix}INSERT{}", &rest["UPSERT".len()..])
    } else {
        sql.to_string()
    }
}

/// If the last meaningful token in `sql` is the bare word `word`
/// (case-insensitive, unquoted), strip it (and any trailing
/// whitespace/semicolon) and return `(rest, true)`. Uses the real SQL
/// tokenizer so this is never confused with a string literal ending
/// in the same text. Falls back to `(sql, false)` on any tokenizer
/// error, letting the real parser produce the actual error message.
fn strip_trailing_word(sql: &str, word: &str) -> (String, bool) {
    let dialect = GenericDialect {};
    let mut tokenizer = Tokenizer::new(&dialect, sql);
    let tokens = match tokenizer.tokenize() {
        Ok(t) => t,
        Err(_) => return (sql.to_string(), false),
    };
    let mut end = tokens.len();
    while end > 0 && matches!(tokens[end - 1], Token::Whitespace(_) | Token::SemiColon) {
        end -= 1;
    }
    if end > 0 {
        if let Token::Word(w) = &tokens[end - 1] {
            if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(word) {
                let rebuilt: String = tokens[..end - 1].iter().map(|t| t.to_string()).collect();
                return (rebuilt, true);
            }
        }
    }
    (sql.to_string(), false)
}

// ---------- Non-standard raw-JSON INSERT shorthand ----------

/// Minimal scanner used only to detect and parse the non-standard
/// `INSERT INTO <collection> [VALUES] { ...raw JSON document(s)... }`
/// shorthand, which lets users paste MongoDB documents directly
/// instead of writing a SQL tuple. Standard `INSERT ... VALUES (...)`
/// is handled by the real SQL parser instead (see [`translate_insert`]).
struct JsonScan<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> JsonScan<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        let rest = &self.src[self.pos..];
        if rest.len() < kw.len() {
            return false;
        }
        if !rest.as_bytes()[..kw.len()].eq_ignore_ascii_case(kw.as_bytes()) {
            return false;
        }
        !matches!(rest[kw.len()..].chars().next(), Some(c) if c.is_ascii_alphanumeric() || c == '_')
    }

    fn consume_keyword(&mut self, kw: &str) -> bool {
        if self.peek_keyword(kw) {
            self.pos += kw.len();
            true
        } else {
            false
        }
    }

    fn parse_identifier(&mut self) -> Option<String> {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.pos += c.len_utf8();
            } else {
                break;
            }
        }
        if self.pos == start {
            None
        } else {
            Some(self.src[start..self.pos].to_string())
        }
    }

    /// Parse a raw JSON value (object or array) by consuming balanced
    /// braces/brackets, then delegating to `serde_json::from_str`.
    fn parse_json_value(&mut self) -> AppResult<serde_json::Value> {
        self.skip_ws();
        let start = self.pos;
        let open = match self.peek_char() {
            Some('{') => '{',
            Some('[') => '[',
            _ => return Err(AppError::SqlParse("expected `{` or `[` to start a JSON value".into())),
        };
        let close = if open == '{' { '}' } else { ']' };
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escape = false;
        for c in self.src[self.pos..].chars() {
            if in_string {
                if escape {
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else {
                match c {
                    '"' => in_string = true,
                    _ if c == open => depth += 1,
                    _ if c == close => {
                        depth -= 1;
                        if depth == 0 {
                            self.pos += c.len_utf8();
                            break;
                        }
                    }
                    _ => {}
                }
            }
            self.pos += c.len_utf8();
        }
        if depth != 0 {
            return Err(AppError::SqlParse("unterminated JSON value".into()));
        }
        let json_str = &self.src[start..self.pos];
        serde_json::from_str(json_str)
            .map_err(|e| AppError::SqlParse(format!("invalid JSON value: {e}")))
    }
}

/// If `sql` matches the `INSERT|UPSERT INTO <collection> [VALUES]
/// { ... }` shorthand, parse and return the collection + documents.
/// Returns `Ok(None)` when the input doesn't match this shape at all
/// (so the caller falls back to standard SQL parsing), and `Err` when
/// it looks like the shorthand but the JSON itself is malformed.
fn detect_json_insert(sql: &str) -> AppResult<Option<(String, Vec<serde_json::Value>)>> {
    let mut s = JsonScan::new(sql);
    s.skip_ws();
    if !(s.consume_keyword("INSERT") || s.consume_keyword("UPSERT")) {
        return Ok(None);
    }
    s.skip_ws();
    if !s.consume_keyword("INTO") {
        return Ok(None);
    }
    s.skip_ws();
    let collection = match s.parse_identifier() {
        Some(c) => c,
        None => return Ok(None),
    };
    s.skip_ws();
    if s.consume_keyword("VALUES") {
        s.skip_ws();
    }
    if s.peek_char() != Some('{') {
        return Ok(None);
    }

    let mut documents = Vec::new();
    loop {
        documents.push(s.parse_json_value()?);
        s.skip_ws();
        if s.peek_char() == Some(',') {
            s.pos += 1;
            s.skip_ws();
            continue;
        }
        break;
    }
    s.skip_ws();
    if s.pos < s.src.len() {
        return Err(AppError::SqlParse(format!(
            "unexpected trailing input at position {}",
            s.pos
        )));
    }
    Ok(Some((collection, documents)))
}

// ---------- SELECT ----------

enum ProjItem {
    Star,
    Item { expr: SqlExpr, alias: Option<String> },
}

struct OrderItem {
    expr: SqlExpr,
    desc: bool,
}

struct JoinClause {
    collection: String,
    local_field: String,
    foreign_field: String,
    kind: JoinKind,
}

#[derive(Debug, Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
}

fn unwrap_query_body(body: &SetExpr) -> AppResult<&Select> {
    match body {
        SetExpr::Select(s) => Ok(s.as_ref()),
        SetExpr::Query(inner) => unwrap_query_body(inner.body.as_ref()),
        _ => Err(AppError::SqlParse(
            "only simple SELECT statements are supported (no UNION/EXCEPT/INTERSECT)".into(),
        )),
    }
}

fn table_name_from_object(name: &ObjectName) -> AppResult<String> {
    Ok(name.to_string())
}

fn table_factor_name(tf: &TableFactor) -> AppResult<String> {
    match tf {
        TableFactor::Table { name, .. } => table_name_from_object(name),
        _ => Err(AppError::SqlParse(
            "FROM/JOIN must reference a simple table name (subqueries and table functions are not supported)".into(),
        )),
    }
}

fn convert_projection(items: &[SqlSelectItem]) -> AppResult<Vec<ProjItem>> {
    items
        .iter()
        .map(|item| match item {
            SqlSelectItem::Wildcard(_) => Ok(ProjItem::Star),
            SqlSelectItem::UnnamedExpr(e) => Ok(ProjItem::Item {
                expr: e.clone(),
                alias: None,
            }),
            SqlSelectItem::ExprWithAlias { expr, alias } => Ok(ProjItem::Item {
                expr: expr.clone(),
                alias: Some(alias.value.clone()),
            }),
            SqlSelectItem::QualifiedWildcard(..) => Err(AppError::SqlParse(
                "qualified wildcards (table.*) are not supported".into(),
            )),
            SqlSelectItem::ExprWithAliases { .. } => Err(AppError::SqlParse(
                "multi-alias projections are not supported".into(),
            )),
        })
        .collect()
}

fn projection_name(expr: &SqlExpr, alias: &Option<String>) -> String {
    if let Some(a) = alias {
        return a.clone();
    }
    field_name(expr).unwrap_or_else(|| "expr".to_string())
}

fn join_side_field(expr: &SqlExpr) -> AppResult<String> {
    match expr {
        SqlExpr::Identifier(ident) => Ok(ident.value.clone()),
        // Strip the leading table/alias qualifier; keep the final
        // segment as the actual document field name.
        SqlExpr::CompoundIdentifier(idents) => idents
            .last()
            .map(|i| i.value.clone())
            .ok_or_else(|| AppError::SqlParse("empty identifier in JOIN ON clause".into())),
        _ => Err(AppError::SqlParse(
            "JOIN ON clause must compare simple field references".into(),
        )),
    }
}

fn join_condition_fields(expr: &SqlExpr) -> AppResult<(String, String)> {
    match expr {
        SqlExpr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => Ok((join_side_field(left)?, join_side_field(right)?)),
        SqlExpr::Nested(inner) => join_condition_fields(inner),
        _ => Err(AppError::SqlParse(
            "JOIN ON clause must be a simple equality: <field> = <field>".into(),
        )),
    }
}

fn build_join(join: &Join) -> AppResult<JoinClause> {
    let collection = table_factor_name(&join.relation)?;
    let (kind, constraint) = match &join.join_operator {
        JoinOperator::Join(c) | JoinOperator::Inner(c) => (JoinKind::Inner, c),
        JoinOperator::Left(c) | JoinOperator::LeftOuter(c) => (JoinKind::Left, c),
        other => {
            return Err(AppError::SqlParse(format!(
                "unsupported join type `{other:?}`; only INNER/LEFT JOIN ... ON <a> = <b> is supported"
            )))
        }
    };
    let on_expr = match constraint {
        JoinConstraint::On(e) => e,
        _ => {
            return Err(AppError::SqlParse(
                "JOIN requires an ON <field> = <field> condition".into(),
            ))
        }
    };
    let (local_field, foreign_field) = join_condition_fields(on_expr)?;
    Ok(JoinClause {
        collection,
        local_field,
        foreign_field,
        kind,
    })
}

/// Translate a join into `$lookup` (+ a follow-up `$match` for `INNER`
/// joins, which drops rows with no match so `INNER` behaves
/// differently from `LEFT` — plain `$lookup` alone always keeps every
/// left-hand document, which is exactly `LEFT JOIN` semantics).
fn join_to_lookup(join: &JoinClause) -> Vec<serde_json::Value> {
    let as_name = format!("{}_joined", join.collection);
    let lookup = serde_json::json!({
        "$lookup": {
            "from": join.collection,
            "localField": join.local_field,
            "foreignField": join.foreign_field,
            "as": as_name,
        }
    });
    match join.kind {
        JoinKind::Left => vec![lookup],
        JoinKind::Inner => {
            let mut cond = serde_json::Map::new();
            cond.insert(as_name, serde_json::json!({ "$ne": [] }));
            vec![lookup, serde_json::json!({ "$match": cond })]
        }
    }
}

fn translate_select(
    database: &str,
    query: &Query,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    if query.with.is_some() {
        return Err(AppError::SqlParse(
            "WITH (common table expressions) is not supported".into(),
        ));
    }
    let select = unwrap_query_body(query.body.as_ref())?;

    if select.from.len() > 1 {
        return Err(AppError::SqlParse(
            "comma-separated FROM tables are not supported; use JOIN instead".into(),
        ));
    }
    let twj = select.from.first().ok_or_else(|| {
        AppError::SqlParse("FROM clause with at least one table is required".into())
    })?;
    let collection = table_factor_name(&twj.relation)?;
    let joins = twj
        .joins
        .iter()
        .map(build_join)
        .collect::<AppResult<Vec<_>>>()?;

    let projection = convert_projection(&select.projection)?;

    let mut stages: Vec<serde_json::Value> = Vec::new();
    let mut find_filter: Option<serde_json::Value> = None;
    let mut find_projection: Option<serde_json::Value> = None;
    let mut find_sort: Option<serde_json::Value> = None;
    let mut find_limit: Option<i64> = None;
    let mut find_skip: Option<u64> = None;

    if let Some(filter_expr) = &select.selection {
        // WHERE is a pre-group filter document, so emit MongoDB filter
        // shape (`{field: {$op: value}}` or `{field: value}` for `$eq`)
        // and only fall back to `{$expr: ...}` when both sides are field
        // references or the comparison is non-scalar. This keeps indexes
        // usable for the common `field <op> literal` case.
        let value = expr_to_filter(filter_expr, warnings)?;
        find_filter = Some(value.clone());
        stages.push(serde_json::json!({ "$match": value }));
    }

    for join in &joins {
        stages.extend(join_to_lookup(join));
    }

    if matches!(&select.distinct, Some(Distinct::On(_))) {
        return Err(AppError::SqlParse("DISTINCT ON (...) is not supported".into()));
    }
    let distinct = matches!(select.distinct, Some(Distinct::Distinct));

    let group_by_exprs: Vec<SqlExpr> = match &select.group_by {
        GroupByExpr::Expressions(exprs, modifiers) => {
            if !modifiers.is_empty() {
                warnings.push("GROUP BY modifiers (ROLLUP/CUBE/etc.) are ignored".into());
            }
            exprs.clone()
        }
        GroupByExpr::All(_) => return Err(AppError::SqlParse("GROUP BY ALL is not supported".into())),
    };

    if distinct && group_by_exprs.is_empty() {
        // DISTINCT: collapse duplicates by grouping on the projected
        // fields, then replace the root so the output shape matches
        // what the user asked for (instead of `_id: {...}`).
        let key = build_distinct_group_key(&projection)?;
        stages.push(serde_json::json!({ "$group": { "_id": key } }));
        stages.push(serde_json::json!({ "$replaceRoot": { "newRoot": "$_id" } }));
        // No `find` shortcut: distinct always runs through aggregate.
    } else if !group_by_exprs.is_empty() {
        let group = build_group_stage(&group_by_exprs, &projection, warnings)?;
        stages.push(serde_json::json!({ "$group": group }));
    } else {
        let project = build_project_stage(&projection, warnings)?;
        if !project.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            stages.push(serde_json::json!({ "$project": project.clone() }));
            find_projection = Some(project);
        }
    }

    if let Some(having) = &select.having {
        // HAVING runs after $group, so every reference is an aggregation
        // expression and must be wrapped in $expr to be valid inside a
        // $match stage. Bare `{$gt: [...]}` would hit the same
        // "unknown top level operator" error as the WHERE bug.
        let value = expr_to_agg_expr(having, warnings)?;
        stages.push(serde_json::json!({ "$match": { "$expr": value } }));
    }

    let order_by: Vec<OrderItem> = match &query.order_by {
        Some(ob) => match &ob.kind {
            OrderByKind::Expressions(exprs) => exprs
                .iter()
                .map(|oe| OrderItem {
                    expr: oe.expr.clone(),
                    desc: oe.options.asc == Some(false),
                })
                .collect(),
            OrderByKind::All(_) => {
                return Err(AppError::SqlParse("ORDER BY ALL is not supported".into()))
            }
        },
        None => Vec::new(),
    };
    if !order_by.is_empty() {
        let sort = build_sort_stage(&order_by);
        stages.push(serde_json::json!({ "$sort": sort.clone() }));
        find_sort = Some(sort);
    }

    if let Some(limit_clause) = &query.limit_clause {
        match limit_clause {
            LimitClause::LimitOffset {
                limit,
                offset,
                limit_by,
            } => {
                if !limit_by.is_empty() {
                    warnings.push("LIMIT BY is not supported and was ignored".into());
                }
                if let Some(limit_expr) = limit {
                    let n = expr_to_i64(limit_expr, warnings)?;
                    stages.push(serde_json::json!({ "$limit": n }));
                    find_limit = Some(n);
                }
                if let Some(off) = offset {
                    let n = expr_to_u64(&off.value, warnings)?;
                    stages.push(serde_json::json!({ "$skip": n }));
                    find_skip = Some(n);
                }
            }
            LimitClause::OffsetCommaLimit { offset, limit } => {
                let off_n = expr_to_u64(offset, warnings)?;
                let lim_n = expr_to_i64(limit, warnings)?;
                stages.push(serde_json::json!({ "$skip": off_n }));
                stages.push(serde_json::json!({ "$limit": lim_n }));
                find_skip = Some(off_n);
                find_limit = Some(lim_n);
            }
        }
    }

    let looks_like_find = stages.iter().all(|s| {
        s.get("$match").is_some()
            || s.get("$sort").is_some()
            || s.get("$project").is_some()
            || s.get("$limit").is_some()
            || s.get("$skip").is_some()
    });
    let find = if looks_like_find && find_filter.is_some() {
        let mut obj = serde_json::Map::new();
        if let Some(f) = find_filter {
            obj.insert("filter".into(), f);
        }
        if let Some(p) = find_projection {
            obj.insert("projection".into(), p);
        }
        if let Some(s) = find_sort {
            obj.insert("sort".into(), s);
        }
        if let Some(l) = find_limit {
            obj.insert("limit".into(), serde_json::json!(l));
        }
        if let Some(s) = find_skip {
            obj.insert("skip".into(), serde_json::json!(s));
        }
        Some(serde_json::Value::Object(obj))
    } else {
        None
    };

    let code = generate_code_variants(database, &collection, &stages);
    let operation = if find.is_some() {
        SqlOperation::Find
    } else {
        SqlOperation::Aggregate
    };

    Ok(SqlTranslation {
        database: database.to_string(),
        collection,
        operation,
        pipeline: serde_json::Value::Array(stages),
        find,
        warnings: warnings.clone(),
        code,
    })
}

// ---------- UPDATE / INSERT / DELETE ----------

fn assignment_target_to_field(target: &AssignmentTarget) -> AppResult<String> {
    match target {
        AssignmentTarget::ColumnName(name) => Ok(name.to_string()),
        AssignmentTarget::Tuple(_) => Err(AppError::SqlParse(
            "tuple assignment targets are not supported".into(),
        )),
    }
}

fn translate_update(
    database: &str,
    update: &SqlUpdate,
    upsert: bool,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    if !update.table.joins.is_empty() {
        return Err(AppError::SqlParse("UPDATE ... JOIN is not supported".into()));
    }
    let collection = table_factor_name(&update.table.relation)?;

    let mut set_fields = serde_json::Map::new();
    for assignment in &update.assignments {
        let field = assignment_target_to_field(&assignment.target)?;
        let value = expr_to_agg_expr(&assignment.value, warnings)?;
        set_fields.insert(field, value);
    }

    let filter = match &update.selection {
        Some(e) => expr_to_filter(e, warnings)?,
        None => serde_json::Value::Object(serde_json::Map::new()),
    };

    let update_doc = serde_json::json!({ "$set": set_fields });
    let code =
        generate_write_code_variants(database, &collection, "update", &filter, &update_doc, upsert);

    Ok(SqlTranslation {
        database: database.to_string(),
        collection,
        operation: SqlOperation::Update {
            filter: filter.clone(),
            update: update_doc,
            multi: true,
            upsert,
        },
        pipeline: serde_json::Value::Array(vec![]),
        find: None,
        warnings: warnings.clone(),
        code,
    })
}

fn translate_insert(
    database: &str,
    insert: &SqlInsert,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    let collection = match &insert.table {
        TableObject::TableName(name) => table_name_from_object(name)?,
        TableObject::TableFunction(_) => {
            return Err(AppError::SqlParse(
                "table-valued functions are not supported in INSERT".into(),
            ))
        }
        TableObject::TableQuery(_) => {
            return Err(AppError::SqlParse(
                "INSERT INTO (query)-form table targets are not supported".into(),
            ))
        }
    };

    let source = insert
        .source
        .as_ref()
        .ok_or_else(|| AppError::SqlParse("INSERT requires a VALUES clause".into()))?;
    let values = match source.body.as_ref() {
        SetExpr::Values(v) => v,
        _ => {
            return Err(AppError::SqlParse(
                "INSERT ... SELECT is not supported; use VALUES or a raw JSON document".into(),
            ))
        }
    };

    let column_names: Vec<String> = insert.columns.iter().map(|c| c.to_string()).collect();
    if column_names.is_empty() {
        // Without a column list we have no field names to zip the
        // positional values against, and MongoDB has no fixed table
        // schema to fall back on. Point the user at the two forms we
        // do support.
        return Err(AppError::SqlParse(
            "INSERT ... VALUES (...) requires an explicit column list, e.g. \
             INSERT INTO t (a, b) VALUES (1, 2); or paste a document directly: \
             INSERT INTO t VALUES { \"a\": 1, \"b\": 2 }"
                .into(),
        ));
    }

    let mut documents = Vec::new();
    for row in &values.rows {
        if row.content.len() != column_names.len() {
            return Err(AppError::SqlParse(format!(
                "expected {} value(s) but got {}",
                column_names.len(),
                row.content.len()
            )));
        }
        let mut doc = serde_json::Map::new();
        for (name, expr) in column_names.iter().zip(row.content.iter()) {
            doc.insert(name.clone(), expr_to_agg_expr(expr, warnings)?);
        }
        documents.push(serde_json::Value::Object(doc));
    }

    let code = generate_insert_code_variants(database, &collection, &documents);

    Ok(SqlTranslation {
        database: database.to_string(),
        collection,
        operation: SqlOperation::Insert {
            documents: documents.clone(),
        },
        pipeline: serde_json::Value::Array(vec![]),
        find: None,
        warnings: warnings.clone(),
        code,
    })
}

fn translate_delete(
    database: &str,
    delete: &SqlDelete,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    let tables = match &delete.from {
        FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
    };
    if tables.len() != 1 {
        return Err(AppError::SqlParse(
            "DELETE must reference exactly one table".into(),
        ));
    }
    let twj = &tables[0];
    if !twj.joins.is_empty() {
        return Err(AppError::SqlParse("DELETE ... JOIN is not supported".into()));
    }
    let collection = table_factor_name(&twj.relation)?;

    let filter = match &delete.selection {
        Some(e) => expr_to_filter(e, warnings)?,
        None => serde_json::Value::Object(serde_json::Map::new()),
    };

    let code = generate_write_code_variants(
        database,
        &collection,
        "delete",
        &filter,
        &serde_json::Value::Null,
        false,
    );

    Ok(SqlTranslation {
        database: database.to_string(),
        collection,
        operation: SqlOperation::Delete {
            filter: filter.clone(),
            multi: true,
        },
        pipeline: serde_json::Value::Array(vec![]),
        find: None,
        warnings: warnings.clone(),
        code,
    })
}

// ---------- Driver code generation (unchanged; operates on plain JSON) ----------

fn generate_code_variants(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for lang in [
        Language::NodeJs,
        Language::Python,
        Language::Java,
        Language::CSharp,
        Language::Ruby,
        Language::Shell,
    ] {
        let key = serde_json::to_value(lang)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();
        map.insert(
            key,
            query_code::generate(lang, database, collection, pipeline),
        );
    }
    map
}

fn generate_write_code_variants(
    database: &str,
    collection: &str,
    op: &str,
    filter: &serde_json::Value,
    update: &serde_json::Value,
    upsert: bool,
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let filter_str = serde_json::to_string_pretty(filter).unwrap_or_default();
    let update_str = serde_json::to_string_pretty(update).unwrap_or_default();
    let upsert_str = if upsert { ", upsert: true" } else { "" };

    map.insert(
        "node-js".into(),
        format!(
            "const {{ MongoClient }} = require('mongodb');\n\
            async function run() {{\n\
            const client = new MongoClient('mongodb://localhost:27017/{database}');\n\
            await client.connect();\n\
            const db = client.db('{database}');\n\
            const coll = db.collection('{collection}');\n\
            const result = await coll.{op}Many(\n\
            {filter_str},\n\
            {update_str}\n\
            {{ {upsert_str} }}\n\
            );\n\
            console.log(result);\n\
            await client.close();\n\
            }}\n\
            run().catch(console.error);\n"
        ),
    );

    map.insert(
        "python".into(),
        format!(
            "from pymongo import MongoClient\n\
            client = MongoClient('mongodb://localhost:27017/{database}')\n\
            db = client['{database}']\n\
            coll = db['{collection}']\n\
            result = coll.{op}_many(\n\
            {filter_str},\n\
            {update_str}\n\
            )\n\
            print(result)\n"
        ),
    );

    map.insert(
        "shell".into(),
        format!(
            "use {database}\n\
            db.{collection}.{op}Many(\n\
            {filter_str},\n\
            {update_str}\n\
            )\n"
        ),
    );

    for lang in ["java", "c-sharp", "ruby"] {
        map.insert(
            lang.into(),
            format!(
                "// {lang} driver code for {op} on {database}.{collection}\n\
                // Filter: {filter_str}\n\
                // Update: {update_str}\n"
            ),
        );
    }

    map
}

fn generate_insert_code_variants(
    database: &str,
    collection: &str,
    documents: &[serde_json::Value],
) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let docs_str = serde_json::to_string_pretty(documents).unwrap_or_default();

    map.insert(
        "node-js".into(),
        format!(
            "const {{ MongoClient }} = require('mongodb');\n\
            async function run() {{\n\
            const client = new MongoClient('mongodb://localhost:27017/{database}');\n\
            await client.connect();\n\
            const db = client.db('{database}');\n\
            const coll = db.collection('{collection}');\n\
            const result = await coll.insertMany(\n\
            {docs_str}\n\
            );\n\
            console.log(result);\n\
            await client.close();\n\
            }}\n\
            run().catch(console.error);\n"
        ),
    );

    map.insert(
        "python".into(),
        format!(
            "from pymongo import MongoClient\n\
            client = MongoClient('mongodb://localhost:27017/{database}')\n\
            db = client['{database}']\n\
            coll = db['{collection}']\n\
            result = coll.insert_many(\n\
            {docs_str}\n\
            )\n\
            print(result)\n"
        ),
    );

    map.insert(
        "shell".into(),
        format!(
            "use {database}\n\
            db.{collection}.insertMany(\n\
            {docs_str}\n\
            )\n"
        ),
    );

    for lang in ["java", "c-sharp", "ruby"] {
        map.insert(
            lang.into(),
            format!(
                "// {lang} driver code for insert into {database}.{collection}\n\
                // Documents: {docs_str}\n"
            ),
        );
    }

    map
}

// ---------- Expression conversion ----------

/// Classification of a bare identifier, accounting for this dialect's
/// two extensions: `#tag` date tags and double-quoted string literals
/// (see the module doc comment).
enum IdentKind {
    Field(String),
    DateTag(String),
    StringLiteral(String),
}

fn classify_ident(ident: &Ident) -> IdentKind {
    if let Some(tag) = ident.value.strip_prefix('#') {
        IdentKind::DateTag(tag.to_string())
    } else if ident.quote_style == Some('"') {
        IdentKind::StringLiteral(ident.value.clone())
    } else {
        IdentKind::Field(ident.value.clone())
    }
}

fn ident_chain(idents: &[Ident]) -> String {
    idents
        .iter()
        .map(|i| i.value.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

/// `Some(dotted field path)` when `expr` is a bare (possibly
/// dotted/nested) field reference.
fn field_name(e: &SqlExpr) -> Option<String> {
    match e {
        SqlExpr::Identifier(ident) => match classify_ident(ident) {
            IdentKind::Field(f) => Some(f),
            _ => None,
        },
        SqlExpr::CompoundIdentifier(idents) => Some(ident_chain(idents)),
        SqlExpr::Nested(inner) => field_name(inner),
        _ => None,
    }
}

/// `true` when `expr` is a scalar literal (number, string, bool, null,
/// or a date tag that resolves to a concrete value).
fn is_literal(e: &SqlExpr) -> bool {
    match e {
        SqlExpr::Value(_) => true,
        SqlExpr::Identifier(ident) => {
            matches!(classify_ident(ident), IdentKind::DateTag(_) | IdentKind::StringLiteral(_))
        }
        SqlExpr::UnaryOp {
            op: UnaryOperator::Minus | UnaryOperator::Plus,
            expr,
        } => is_literal(expr),
        SqlExpr::Nested(inner) => is_literal(inner),
        _ => false,
    }
}

fn value_to_json(value: &SqlValue, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    Ok(match value {
        SqlValue::Number(s, _) => s
            .parse::<f64>()
            .map(|n| serde_json::json!(n))
            .map_err(|e| AppError::SqlParse(format!("invalid number `{s}`: {e}")))?,
        SqlValue::SingleQuotedString(s)
        | SqlValue::DoubleQuotedString(s)
        | SqlValue::EscapedStringLiteral(s)
        | SqlValue::NationalStringLiteral(s) => serde_json::json!(s),
        SqlValue::Boolean(b) => serde_json::json!(b),
        SqlValue::Null => serde_json::Value::Null,
        other => {
            warnings.push(format!("unsupported literal value, treated as null: {other}"));
            serde_json::Value::Null
        }
    })
}

/// Translate a SQL `LIKE` pattern (`%` = any run of characters, `_` =
/// any single character, `\` escapes the next character) into an
/// anchored regular expression.
fn like_pattern_to_regex(pattern: &str) -> String {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push_str(&regex::escape(&next.to_string()));
                } else {
                    out.push_str("\\\\");
                }
            }
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    out.push('$');
    out
}

fn like_pattern_string(pattern: &SqlExpr, warnings: &mut Vec<String>) -> String {
    match pattern {
        SqlExpr::Value(vws) => match &vws.value {
            SqlValue::SingleQuotedString(s) | SqlValue::DoubleQuotedString(s) => s.clone(),
            _ => {
                warnings.push("LIKE pattern must be a string literal; treated as empty".into());
                String::new()
            }
        },
        SqlExpr::Identifier(ident) if ident.quote_style == Some('"') => ident.value.clone(),
        _ => {
            warnings.push("LIKE pattern must be a string literal; treated as empty".into());
            String::new()
        }
    }
}

fn like_to_agg_expr(
    inner: &SqlExpr,
    pattern: &SqlExpr,
    case_insensitive: bool,
    negated: bool,
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let input = expr_to_agg_expr(inner, warnings)?;
    let regex_pattern = like_pattern_to_regex(&like_pattern_string(pattern, warnings));
    let mut obj = serde_json::Map::new();
    obj.insert("input".into(), input);
    obj.insert("regex".into(), serde_json::json!(regex_pattern));
    if case_insensitive {
        obj.insert("options".into(), serde_json::json!("i"));
    }
    let m = serde_json::json!({ "$regexMatch": serde_json::Value::Object(obj) });
    Ok(if negated {
        serde_json::json!({ "$not": [m] })
    } else {
        m
    })
}

fn function_args(func: &Function) -> AppResult<Vec<SqlExpr>> {
    match &func.args {
        FunctionArguments::None => Ok(vec![]),
        FunctionArguments::Subquery(_) => Err(AppError::SqlParse(
            "subqueries as function arguments are not supported".into(),
        )),
        FunctionArguments::List(list) => {
            let mut out = Vec::new();
            for arg in &list.args {
                match arg {
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(e))
                    | FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    }
                    | FunctionArg::ExprNamed {
                        arg: FunctionArgExpr::Expr(e),
                        ..
                    } => out.push(e.clone()),
                    // `COUNT(*)` and friends: the wildcard itself carries
                    // no data these functions need.
                    _ => {}
                }
            }
            Ok(out)
        }
    }
}

fn function_to_agg_expr(func: &Function, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    let upper = func.name.to_string().to_uppercase();
    let args = function_args(func)?;
    Ok(match upper.as_str() {
        "COUNT" => serde_json::json!({ "$sum": 1 }),
        "SUM" | "AVG" | "MIN" | "MAX" => {
            let inner = args
                .first()
                .ok_or_else(|| AppError::SqlParse(format!("{upper}() needs an argument")))?;
            let op = match upper.as_str() {
                "SUM" => "$sum",
                "AVG" => "$avg",
                "MIN" => "$min",
                _ => "$max",
            };
            serde_json::json!({ op: expr_to_agg_expr(inner, warnings)? })
        }
        "REGEX" | "REGEXMATCH" => {
            // REGEX(input, pattern[, options])
            if args.len() < 2 {
                return Err(AppError::SqlParse(
                    "REGEX requires at least 2 arguments: input, pattern".into(),
                ));
            }
            let input = expr_to_agg_expr(&args[0], warnings)?;
            let pattern = expr_to_agg_expr(&args[1], warnings)?;
            let mut obj = serde_json::Map::new();
            obj.insert("input".into(), input);
            obj.insert("regex".into(), pattern);
            if let Some(opts) = args.get(2) {
                obj.insert("options".into(), expr_to_agg_expr(opts, warnings)?);
            }
            serde_json::json!({ "$regexMatch": serde_json::Value::Object(obj) })
        }
        "COALESCE" => {
            let vals = args
                .iter()
                .map(|a| expr_to_agg_expr(a, warnings))
                .collect::<AppResult<Vec<_>>>()?;
            serde_json::json!({ "$ifNull": vals })
        }
        "LOWER" | "UPPER" | "ABS" | "CEIL" | "FLOOR" | "SQRT" | "TRIM" => {
            let inner = args
                .first()
                .ok_or_else(|| AppError::SqlParse(format!("{upper}() needs an argument")))?;
            let op = match upper.as_str() {
                "LOWER" => "$toLower",
                "UPPER" => "$toUpper",
                "ABS" => "$abs",
                "CEIL" => "$ceil",
                "FLOOR" => "$floor",
                "SQRT" => "$sqrt",
                _ => "$trim",
            };
            if op == "$trim" {
                serde_json::json!({ op: { "input": expr_to_agg_expr(inner, warnings)? } })
            } else {
                serde_json::json!({ op: expr_to_agg_expr(inner, warnings)? })
            }
        }
        _ => {
            warnings.push(format!("unhandled function {upper}"));
            serde_json::Value::Null
        }
    })
}

fn binary_op_to_agg_expr(
    left: &SqlExpr,
    op: &BinaryOperator,
    right: &SqlExpr,
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mongo_op = match op {
        BinaryOperator::Eq | BinaryOperator::Spaceship => "$eq",
        BinaryOperator::NotEq => "$ne",
        BinaryOperator::Lt => "$lt",
        BinaryOperator::LtEq => "$lte",
        BinaryOperator::Gt => "$gt",
        BinaryOperator::GtEq => "$gte",
        BinaryOperator::And => "$and",
        BinaryOperator::Or => "$or",
        BinaryOperator::Plus => "$add",
        BinaryOperator::Minus => "$subtract",
        BinaryOperator::Multiply => "$multiply",
        BinaryOperator::Divide => "$divide",
        BinaryOperator::Modulo => "$mod",
        other => {
            warnings.push(format!("unsupported operator `{other}`, treated as `=`"));
            "$eq"
        }
    };
    Ok(
        serde_json::json!({ mongo_op: [expr_to_agg_expr(left, warnings)?, expr_to_agg_expr(right, warnings)?] }),
    )
}

/// Convert any expression into MongoDB **aggregation expression**
/// shape (`"$field"`, `{$op: [...]}`, literals as-is). Used for
/// projections, `SET` values, `HAVING`, and as the fallback for any
/// WHERE-clause shape that [`expr_to_filter`] can't express as a plain
/// filter document.
fn expr_to_agg_expr(expr: &SqlExpr, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    Ok(match expr {
        SqlExpr::Identifier(ident) => match classify_ident(ident) {
            IdentKind::Field(f) => serde_json::json!(format!("${f}")),
            IdentKind::StringLiteral(s) => serde_json::json!(s),
            IdentKind::DateTag(tag) => {
                if let Some(dt) = resolve_date_tag(&tag) {
                    date_to_extjson(dt)
                } else {
                    warnings.push(format!("unknown date tag #{tag}"));
                    serde_json::Value::String(format!("#{tag}"))
                }
            }
        },
        SqlExpr::CompoundIdentifier(idents) => serde_json::json!(format!("${}", ident_chain(idents))),
        SqlExpr::Nested(inner) => expr_to_agg_expr(inner, warnings)?,
        SqlExpr::Value(vws) => value_to_json(&vws.value, warnings)?,
        SqlExpr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: inner,
        } => {
            let v = expr_to_agg_expr(inner, warnings)?;
            match v.as_f64() {
                Some(n) => serde_json::json!(-n),
                None => serde_json::json!({ "$multiply": [v, -1] }),
            }
        }
        SqlExpr::UnaryOp {
            op: UnaryOperator::Plus,
            expr: inner,
        } => expr_to_agg_expr(inner, warnings)?,
        SqlExpr::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => serde_json::json!({ "$not": [expr_to_agg_expr(inner, warnings)?] }),
        SqlExpr::BinaryOp { left, op, right } => binary_op_to_agg_expr(left, op, right, warnings)?,
        SqlExpr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let arr: Vec<_> = list
                .iter()
                .map(|v| expr_to_agg_expr(v, warnings))
                .collect::<AppResult<Vec<_>>>()?;
            let in_expr = serde_json::json!({ "$in": [expr_to_agg_expr(inner, warnings)?, serde_json::Value::Array(arr)] });
            if *negated {
                serde_json::json!({ "$not": [in_expr] })
            } else {
                in_expr
            }
        }
        SqlExpr::IsNull(inner) => {
            serde_json::json!({ "$eq": [expr_to_agg_expr(inner, warnings)?, serde_json::Value::Null] })
        }
        SqlExpr::IsNotNull(inner) => {
            serde_json::json!({ "$ne": [expr_to_agg_expr(inner, warnings)?, serde_json::Value::Null] })
        }
        SqlExpr::Between {
            expr: inner,
            negated,
            low,
            high,
        } => {
            let e = expr_to_agg_expr(inner, warnings)?;
            let l = expr_to_agg_expr(low, warnings)?;
            let h = expr_to_agg_expr(high, warnings)?;
            let between =
                serde_json::json!({ "$and": [ { "$gte": [e.clone(), l] }, { "$lte": [e, h] } ] });
            if *negated {
                serde_json::json!({ "$not": [between] })
            } else {
                between
            }
        }
        SqlExpr::Like {
            negated,
            expr: inner,
            pattern,
            ..
        } => like_to_agg_expr(inner, pattern, false, *negated, warnings)?,
        SqlExpr::ILike {
            negated,
            expr: inner,
            pattern,
            ..
        } => like_to_agg_expr(inner, pattern, true, *negated, warnings)?,
        SqlExpr::Function(func) => function_to_agg_expr(func, warnings)?,
        other => {
            warnings.push(format!("unsupported expression, treated as null: {other}"));
            serde_json::Value::Null
        }
    })
}

/// Convert a WHERE-clause expression into a MongoDB **filter document**.
///
/// This is distinct from [`expr_to_agg_expr`], which produces aggregation
/// expression shape (`{$op: [a, b]}`). A filter document must never have a
/// top-level `$op` key (MongoDB rejects it with "unknown top level
/// operator"); instead conditions bind to a field name:
/// `{field: {$op: value}}`, or the index-friendlier `{field: value}` for
/// `$eq`.
///
/// Emission rules:
/// - `field <op> literal`  -> `{field: {$op: literal}}` (`{field: literal}`
///   for `$eq`). Keeps single-field indexes usable.
/// - `literal <op> field`  -> swap operands (and flip `$lt`/`$gt`/`$lte`/
///   `$gte` for ordered ops) so the same index-friendly form applies.
/// - `field <op> field`    -> `{$expr: {$op: ["$a", "$b"]}}` (the only case
///   that genuinely requires expression evaluation).
/// - `a AND b`             -> merge into one object when both sides are
///   simple field conditions on disjoint keys (`{x: 1, y: 2}`); otherwise
///   fall back to `{$and: [...]}`. Sharing a key (e.g. two ranges on the
///   same field) is left as `$and` so neither condition is dropped.
/// - `a OR b`              -> `{$or: [...]}` (cannot be merged).
/// - `field IN (...)`      -> `{field: {$in: [...]}}` (`$nin` when negated).
/// - `field IS NULL`       -> `{field: null}`; `IS NOT NULL` ->
///   `{field: {$ne: null}}`.
/// - `field BETWEEN a AND b` -> `{field: {$gte: a, $lte: b}}`.
/// - Functions (`REGEX`, etc.), `LIKE`/`ILIKE`, and any non-scalar shape
///   fall back to `{$expr: <agg expr>}` so they remain valid inside `$match`.
fn expr_to_filter(expr: &SqlExpr, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    Ok(match expr {
        SqlExpr::Nested(inner) => expr_to_filter(inner, warnings)?,
        SqlExpr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            let l = expr_to_filter(left, warnings)?;
            let r = expr_to_filter(right, warnings)?;
            merge_and(l, r)
        }
        SqlExpr::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } => {
            let l = expr_to_filter(left, warnings)?;
            let r = expr_to_filter(right, warnings)?;
            serde_json::json!({ "$or": [l, r] })
        }
        SqlExpr::BinaryOp { left, op, right }
            if matches!(
                op,
                BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
            ) =>
        {
            let mongo_op = match op {
                BinaryOperator::Eq => "$eq",
                BinaryOperator::NotEq => "$ne",
                BinaryOperator::Lt => "$lt",
                BinaryOperator::LtEq => "$lte",
                BinaryOperator::Gt => "$gt",
                _ => "$gte",
            };
            comparison_to_filter(mongo_op, left, right, warnings)?
        }
        SqlExpr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let arr: Vec<_> = list
                .iter()
                .map(|v| expr_to_agg_expr(v, warnings))
                .collect::<AppResult<Vec<_>>>()?;
            if let Some(field) = field_name(inner) {
                let op = if *negated { "$nin" } else { "$in" };
                simple_field_filter(op, &field, serde_json::Value::Array(arr))
            } else {
                let left = expr_to_agg_expr(inner, warnings)?;
                let in_expr =
                    serde_json::json!({ "$in": [left, serde_json::Value::Array(arr)] });
                let value = if *negated {
                    serde_json::json!({ "$not": [in_expr] })
                } else {
                    in_expr
                };
                serde_json::json!({ "$expr": value })
            }
        }
        SqlExpr::IsNull(inner) => match field_name(inner) {
            Some(field) => serde_json::json!({ field: serde_json::Value::Null }),
            None => {
                let inner_expr = expr_to_agg_expr(inner, warnings)?;
                serde_json::json!({ "$expr": { "$eq": [inner_expr, serde_json::Value::Null] } })
            }
        },
        SqlExpr::IsNotNull(inner) => match field_name(inner) {
            Some(field) => serde_json::json!({ field: { "$ne": serde_json::Value::Null } }),
            None => {
                let inner_expr = expr_to_agg_expr(inner, warnings)?;
                serde_json::json!({ "$expr": { "$ne": [inner_expr, serde_json::Value::Null] } })
            }
        },
        SqlExpr::Between {
            expr: inner,
            negated,
            low,
            high,
        } => match field_name(inner) {
            Some(field) => {
                let l = expr_to_agg_expr(low, warnings)?;
                let h = expr_to_agg_expr(high, warnings)?;
                if *negated {
                    let mut lt = serde_json::Map::new();
                    lt.insert(field.clone(), serde_json::json!({ "$lt": l }));
                    let mut gt = serde_json::Map::new();
                    gt.insert(field, serde_json::json!({ "$gt": h }));
                    serde_json::json!({ "$or": [serde_json::Value::Object(lt), serde_json::Value::Object(gt)] })
                } else {
                    serde_json::json!({ field: { "$gte": l, "$lte": h } })
                }
            }
            None => {
                let agg = expr_to_agg_expr(expr, warnings)?;
                serde_json::json!({ "$expr": agg })
            }
        },
        // Functions (REGEX, aggregates, ...), LIKE/ILIKE, arithmetic, and
        // bare scalars are aggregation expressions; wrap them so they are
        // legal in $match.
        _ => {
            let agg = expr_to_agg_expr(expr, warnings)?;
            serde_json::json!({ "$expr": agg })
        }
    })
}

/// Build a single comparison as a filter document. See [`expr_to_filter`]
/// for the shape rules. `op` is the MongoDB operator (`$eq`, `$gt`, ...).
fn comparison_to_filter(
    op: &str,
    l: &SqlExpr,
    r: &SqlExpr,
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    // field <op> literal -> {field: {$op: literal}} (or {field: literal} for $eq)
    if let (Some(field), true) = (field_name(l), is_literal(r)) {
        let lit = expr_to_agg_expr(r, warnings)?;
        return Ok(simple_field_filter(op, &field, lit));
    }
    // literal <op> field -> normalize to field <op> literal.
    if let (true, Some(field)) = (is_literal(l), field_name(r)) {
        let lit = expr_to_agg_expr(l, warnings)?;
        // For ordered ops, swap the operator so the semantics are
        // preserved: `5 < total` == `total > 5`, etc. `$eq`/`$ne` are
        // commutative and need no swap.
        let swapped = match op {
            "$lt" => "$gt",
            "$gt" => "$lt",
            "$lte" => "$gte",
            "$gte" => "$lte",
            other => other,
        };
        return Ok(simple_field_filter(swapped, &field, lit));
    }
    // Both sides are fields, or a non-scalar expression is involved ->
    // genuine aggregation expression; wrap in $expr.
    let left = expr_to_agg_expr(l, warnings)?;
    let right = expr_to_agg_expr(r, warnings)?;
    Ok(serde_json::json!({ "$expr": { op: [left, right] } }))
}

/// Emit `{field: value}` for `$eq`, otherwise `{field: {$op: value}}`.
fn simple_field_filter(op: &str, field: &str, value: serde_json::Value) -> serde_json::Value {
    if op == "$eq" {
        serde_json::json!({ field: value })
    } else {
        serde_json::json!({ field: { op: value } })
    }
}

/// Combine two `$and` children. When both are simple field-condition
/// objects (`{field: ...}` with no top-level `$`-keys) on **disjoint**
/// keys, merge them into one object — this is cheaper for the planner and
/// preserves index intersection. If they share a key (e.g. two ranges on
/// the same field) or either side uses logical/expression operators, fall
/// back to `{$and: [left, right]}` so no condition is silently dropped.
fn merge_and(left: serde_json::Value, right: serde_json::Value) -> serde_json::Value {
    if let (serde_json::Value::Object(l), serde_json::Value::Object(r)) = (&left, &right) {
        let l_simple = l.keys().all(|k| !k.starts_with('$'));
        let r_simple = r.keys().all(|k| !k.starts_with('$'));
        let disjoint = !l.keys().any(|k| r.contains_key(k));
        if l_simple && r_simple && disjoint {
            let mut merged = l.clone();
            for (k, v) in r {
                merged.insert(k.clone(), v.clone());
            }
            return serde_json::Value::Object(merged);
        }
    }
    serde_json::json!({ "$and": [left, right] })
}

fn expr_to_i64(expr: &SqlExpr, warnings: &mut Vec<String>) -> AppResult<i64> {
    expr_to_agg_expr(expr, warnings)?
        .as_f64()
        .map(|n| n as i64)
        .ok_or_else(|| AppError::SqlParse("LIMIT must be a numeric literal".into()))
}

fn expr_to_u64(expr: &SqlExpr, warnings: &mut Vec<String>) -> AppResult<u64> {
    expr_to_agg_expr(expr, warnings)?
        .as_f64()
        .map(|n| n as u64)
        .ok_or_else(|| AppError::SqlParse("OFFSET must be a numeric literal".into()))
}

fn build_project_stage(
    projection: &[ProjItem],
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mut out = serde_json::Map::new();
    for item in projection {
        if let ProjItem::Item { expr, alias } = item {
            let name = projection_name(expr, alias);
            let value = expr_to_agg_expr(expr, warnings)?;
            out.insert(name, value);
        }
    }
    Ok(serde_json::Value::Object(out))
}

/// Build the `_id` key for a DISTINCT group. `SELECT *` is rejected
/// (DISTINCT with no columns is meaningless). Each projected column
/// becomes a sub-field keyed by its column name.
fn build_distinct_group_key(projection: &[ProjItem]) -> AppResult<serde_json::Value> {
    let mut out = serde_json::Map::new();
    let mut has_star = false;
    let mut count = 0;
    for item in projection {
        match item {
            ProjItem::Star => has_star = true,
            ProjItem::Item { expr, alias } => {
                let name = projection_name(expr, alias);
                out.insert(name.clone(), serde_json::json!(format!("${name}")));
                count += 1;
            }
        }
    }
    if has_star || count == 0 {
        return Err(AppError::SqlParse(
            "SELECT DISTINCT requires at least one named column".into(),
        ));
    }
    Ok(serde_json::Value::Object(out))
}

fn build_group_stage(
    keys: &[SqlExpr],
    projection: &[ProjItem],
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mut group = serde_json::Map::new();
    let mut id_doc = serde_json::Map::new();
    for key in keys {
        let name = field_name(key).unwrap_or_else(|| "_key".to_string());
        id_doc.insert(name.clone(), serde_json::json!(format!("${name}")));
    }
    group.insert("_id".into(), serde_json::Value::Object(id_doc));
    for item in projection {
        if let ProjItem::Item { expr, alias } = item {
            let key_name = match (alias, field_name(expr)) {
                (Some(a), _) => a.clone(),
                (None, Some(f)) => f,
                (None, None) => continue,
            };
            if is_grouping_key(expr, keys) {
                continue;
            }
            if let SqlExpr::Function(_) = expr {
                let value = expr_to_agg_expr(expr, warnings)?;
                group.insert(key_name, value);
            } else {
                // For non-aggregate projections, use $first.
                let field = expr_to_agg_expr(expr, warnings)?;
                group.insert(key_name, serde_json::json!({ "$first": field }));
            }
        }
    }
    Ok(serde_json::Value::Object(group))
}

fn is_grouping_key(expr: &SqlExpr, keys: &[SqlExpr]) -> bool {
    match field_name(expr) {
        Some(name) => keys.iter().any(|k| field_name(k).as_deref() == Some(name.as_str())),
        None => false,
    }
}

fn build_sort_stage(order: &[OrderItem]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for item in order {
        let name = field_name(&item.expr).unwrap_or_else(|| "_id".to_string());
        map.insert(name, serde_json::json!(if item.desc { -1 } else { 1 }));
    }
    serde_json::Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_simple_select() {
        let t = translate(
            "shop",
            "SELECT name FROM products WHERE price > 10 ORDER BY price DESC LIMIT 5",
        )
        .expect("translate");
        assert_eq!(t.collection, "products");
        let stages = t.pipeline.as_array().expect("array");
        assert!(stages.iter().any(|s| s.get("$match").is_some()));
        assert!(stages.iter().any(|s| s.get("$sort").is_some()));
        assert!(stages.iter().any(|s| s.get("$limit").is_some()));
        assert!(t.find.is_some());
    }

    #[test]
    fn translates_join_to_lookup() {
        let t = translate(
            "shop",
            "SELECT u.name, o.total FROM users u INNER JOIN orders o ON u._id = o.userId",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let lookup = stages
            .iter()
            .find(|s| s.get("$lookup").is_some())
            .expect("lookup");
        assert_eq!(lookup["$lookup"]["from"], "orders");
        assert_eq!(lookup["$lookup"]["localField"], "_id");
        assert_eq!(lookup["$lookup"]["foreignField"], "userId");
    }

    #[test]
    fn translates_join_with_bare_fields() {
        let t = translate(
            "shop",
            "SELECT name, total FROM products LEFT JOIN orders ON categoryId = _id",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let lookup = stages
            .iter()
            .find(|s| s.get("$lookup").is_some())
            .expect("lookup");
        assert_eq!(lookup["$lookup"]["from"], "orders");
        assert_eq!(lookup["$lookup"]["localField"], "categoryId");
        assert_eq!(lookup["$lookup"]["foreignField"], "_id");
    }

    #[test]
    fn inner_join_adds_non_empty_match_but_left_join_does_not() {
        let inner = translate(
            "shop",
            "SELECT name FROM products INNER JOIN categories ON categoryId = _id",
        )
        .expect("translate");
        let inner_stages = inner.pipeline.as_array().expect("array");
        assert!(inner_stages
            .iter()
            .any(|s| s["$match"].get("categories_joined").is_some()));

        let left = translate(
            "shop",
            "SELECT name FROM products LEFT JOIN categories ON categoryId = _id",
        )
        .expect("translate");
        let left_stages = left.pipeline.as_array().expect("array");
        assert!(!left_stages
            .iter()
            .any(|s| s.get("$match").map(|m| m.get("categories_joined").is_some()).unwrap_or(false)));
    }

    #[test]
    fn translates_group_by() {
        let t = translate(
            "shop",
            "SELECT category, COUNT(*) AS c FROM products GROUP BY category",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let group = stages
            .iter()
            .find(|s| s.get("$group").is_some())
            .expect("group");
        assert!(group["$group"]["c"].is_object());
    }

    #[test]
    fn translation_includes_code_variants() {
        let t = translate("shop", "SELECT name FROM products WHERE price > 10").expect("translate");
        // All six languages present.
        assert!(t.code.contains_key("node-js"));
        assert!(t.code.contains_key("python"));
        assert!(t.code.contains_key("java"));
        assert!(t.code.contains_key("c-sharp"));
        assert!(t.code.contains_key("ruby"));
        assert!(t.code.contains_key("shell"));
        // Each variant is non-empty and mentions the collection.
        for (k, v) in &t.code {
            assert!(!v.is_empty(), "variant {} empty", k);
            assert!(v.contains("products"), "variant {} missing collection", k);
        }
    }

    #[test]
    fn having_clause_emits_post_group_match() {
        let t = translate(
            "shop",
            "SELECT category, COUNT(*) AS c FROM products GROUP BY category HAVING COUNT(*) > 1",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        // The pipeline must contain a $match stage that came from the
        // HAVING clause, positioned after the $group stage.
        let group_pos = stages
            .iter()
            .position(|s| s.get("$group").is_some())
            .expect("$group stage");
        let post_group_matches: Vec<_> = stages[group_pos + 1..]
            .iter()
            .filter(|s| s.get("$match").is_some())
            .collect();
        assert!(
            !post_group_matches.is_empty(),
            "no post-group $match for HAVING"
        );
    }

    #[test]
    fn translates_distinct_with_group_then_replace_root() {
        let t = translate("shop", "SELECT DISTINCT category FROM products").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let group = stages
            .iter()
            .find(|s| s.get("$group").is_some())
            .expect("group");
        let key = &group["$group"]["_id"];
        assert_eq!(key["category"], "$category");
        assert!(stages.iter().any(|s| s.get("$replaceRoot").is_some()));
        // No `find` shortcut for DISTINCT.
        assert!(t.find.is_none());
    }

    #[test]
    fn distinct_rejects_star_and_empty_projection() {
        assert!(translate("shop", "SELECT DISTINCT * FROM products").is_err());
    }

    #[test]
    fn translates_regex_in_where() {
        let t = translate(
            "shop",
            "SELECT name FROM products WHERE REGEX(name, '^foo')",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["$expr"]["$regexMatch"]["regex"], "^foo");
        assert!(m["$match"]["$expr"]["$regexMatch"]["input"].as_str().is_some());
        // options key absent when not supplied
        assert!(m["$match"]["$expr"]["$regexMatch"].get("options").is_none());
    }

    #[test]
    fn translates_regex_in_where_with_options() {
        let t = translate(
            "shop",
            "SELECT name FROM products WHERE REGEX(name, '^FOO', 'i')",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["$expr"]["$regexMatch"]["regex"], "^FOO");
        assert_eq!(m["$match"]["$expr"]["$regexMatch"]["options"], "i");
    }

    #[test]
    fn translates_like_to_anchored_regex() {
        let t = translate("shop", "SELECT * FROM products WHERE name LIKE 'foo%bar_'")
            .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(
            m["$match"]["$expr"]["$regexMatch"]["regex"],
            "^foo.*bar.$"
        );
    }

    #[test]
    fn translates_ilike_case_insensitive() {
        let t = translate("shop", "SELECT * FROM products WHERE name ILIKE 'foo%'")
            .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["$expr"]["$regexMatch"]["options"], "i");
    }

    #[test]
    fn translates_between_on_field_to_range_filter() {
        let t = translate("shop", "SELECT * FROM products WHERE price BETWEEN 10 AND 20")
            .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["price"]["$gte"], 10.0);
        assert_eq!(m["$match"]["price"]["$lte"], 20.0);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn field_comparison_in_where_wraps_in_expr() {
        let t = translate("shop", "SELECT * FROM products WHERE total = max").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        // The match document must be wrapped in $expr because both
        // sides of `=` are bare fields.
        assert_eq!(m["$match"]["$expr"]["$eq"][0], "$total");
        assert_eq!(m["$match"]["$expr"]["$eq"][1], "$max");
    }

    #[test]
    fn literal_eq_collapses_to_field_value_filter() {
        // `field = literal` emits the index-friendly `{field: literal}`
        // form, NOT a top-level `$eq` (which MongoDB rejects) and NOT a
        // `$expr` wrapper (which would defeat single-field indexes).
        let t = translate(
            "shop",
            "SELECT * FROM categories WHERE name=\"Audio\" ORDER BY _id LIMIT 50",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["name"], "Audio");
        assert!(m["$match"].get("$expr").is_none());
        assert!(m["$match"].get("$eq").is_none());
    }

    #[test]
    fn ne_literal_keeps_ne_operator() {
        let t = translate(
            "shop",
            "SELECT * FROM products WHERE status != \"active\"",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["status"]["$ne"], "active");
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn gt_literal_uses_field_op_literal_form() {
        let t = translate("shop", "SELECT * FROM products WHERE price > 10").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["price"]["$gt"], 10.0);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn and_of_disjoint_literal_fields_merges_into_one_object() {
        // Two simple conditions on different fields -> merge into a single
        // flat object instead of `{$and: [...]}`. Cheaper for the planner
        // and preserves index intersection.
        let t = translate("shop", "SELECT * FROM products WHERE x = 5 AND y = 10").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert!(m["$match"].get("$and").is_none());
        assert_eq!(m["$match"]["x"], 5.0);
        assert_eq!(m["$match"]["y"], 10.0);
    }

    #[test]
    fn and_of_same_field_does_not_merge() {
        // Two ranges on the same field must NOT merge — that would
        // overwrite one condition. They stay under `$and`.
        let t = translate(
            "shop",
            "SELECT * FROM products WHERE price >= 5 AND price <= 50",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        let and = m["$match"]["$and"].as_array().expect("and");
        assert_eq!(and.len(), 2);
        assert!(and.iter().all(|b| b.get("price").is_some()));
    }

    #[test]
    fn literal_lt_field_swaps_to_field_gt_literal() {
        // `5 < total` == `total > 5`. Normalizing to the field-on-left
        // form keeps single-field indexes usable.
        let t = translate("shop", "SELECT * FROM products WHERE 5 < total").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["total"]["$gt"], 5.0);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn literal_eq_field_swaps_to_field_eq_literal() {
        let t = translate("shop", "SELECT * FROM products WHERE 5 = total").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["total"], 5.0);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn or_keeps_or_with_branches() {
        let t = translate("shop", "SELECT * FROM products WHERE x = 5 OR y = 10").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        // $or never merges even for disjoint simple branches.
        let or = m["$match"]["$or"].as_array().expect("or");
        assert_eq!(or.len(), 2);
        assert_eq!(or[0]["x"], 5.0);
        assert_eq!(or[1]["y"], 10.0);
    }

    #[test]
    fn in_literal_list_emits_field_in_filter() {
        let t = translate(
            "shop",
            "SELECT * FROM products WHERE category IN (\"a\", \"b\", \"c\")",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        let arr = m["$match"]["category"]["$in"].as_array().expect("in array");
        assert_eq!(arr.len(), 3);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn not_in_literal_list_emits_field_nin_filter() {
        let t = translate(
            "shop",
            "SELECT * FROM products WHERE category NOT IN (\"a\", \"b\")",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        let arr = m["$match"]["category"]["$nin"].as_array().expect("nin array");
        assert_eq!(arr.len(), 2);
    }

    #[test]
    fn is_null_field_emits_field_null_filter() {
        let t = translate("shop", "SELECT * FROM products WHERE deletedAt IS NULL").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert!(m["$match"]["deletedAt"].is_null());
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn is_not_null_field_emits_field_ne_null() {
        let t = translate(
            "shop",
            "SELECT * FROM products WHERE deletedAt IS NOT NULL",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["deletedAt"]["$ne"], serde_json::Value::Null);
        assert!(m["$match"].get("$expr").is_none());
    }

    #[test]
    fn having_clause_wraps_in_expr() {
        // HAVING runs after $group, so its body is an aggregation
        // expression and must be wrapped in $expr to be valid inside a
        // $match stage. A bare `{$gt: [...]}` would reproduce the same
        // "unknown top level operator" bug the WHERE path had.
        let t = translate(
            "shop",
            "SELECT category, COUNT(*) AS c FROM products GROUP BY category HAVING COUNT(*) > 1",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let group_pos = stages
            .iter()
            .position(|s| s.get("$group").is_some())
            .expect("$group stage");
        let m = stages[group_pos + 1..]
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("post-group $match");
        assert!(m["$match"].get("$expr").is_some());
        assert!(m["$match"]["$expr"].get("$gt").is_some());
    }

    #[test]
    fn translates_date_tag_today_to_bson_date() {
        let t =
            translate("shop", "SELECT * FROM events WHERE createdAt >= #today").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        // `field >= literal` -> {field: {$gte: <date>}}
        let date = &m["$match"]["createdAt"]["$gte"];
        assert!(date.get("$date").is_some());
    }

    #[test]
    fn translates_date_tag_lastweek_in_range() {
        let t = translate(
            "shop",
            "SELECT * FROM events WHERE createdAt >= #lastweek AND createdAt < #today",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        // Same field on both branches -> keys collide -> cannot merge,
        // so the result stays under `$and` and neither range is dropped.
        let and = m["$match"]["$and"].as_array().expect("and");
        assert!(and.iter().all(|b| {
            b["createdAt"]["$gte"].get("$date").is_some()
                || b["createdAt"]["$lt"].get("$date").is_some()
        }));
    }

    #[test]
    fn warns_on_unknown_date_tag() {
        let t =
            translate("shop", "SELECT * FROM events WHERE createdAt = #nope").expect("translate");
        assert!(t.warnings.iter().any(|w| w.contains("unknown date tag")));
    }

    // ---- Arithmetic ----

    #[test]
    fn translates_arithmetic_in_projection() {
        let t = translate("shop", "SELECT name, price * stockQuantity AS inventoryValue FROM products").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let project = stages
            .iter()
            .find(|s| s.get("$project").is_some())
            .expect("project");
        assert_eq!(project["$project"]["inventoryValue"]["$multiply"][0], "$price");
        assert_eq!(project["$project"]["inventoryValue"]["$multiply"][1], "$stockQuantity");
    }

    #[test]
    fn translates_arithmetic_in_where() {
        let t = translate("shop", "SELECT * FROM products WHERE price * 2 > 100").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert_eq!(m["$match"]["$expr"]["$gt"][0]["$multiply"][0], "$price");
        assert_eq!(m["$match"]["$expr"]["$gt"][0]["$multiply"][1], 2.0);
        assert_eq!(m["$match"]["$expr"]["$gt"][1], 100.0);
    }

    #[test]
    fn arithmetic_precedence_mul_before_add() {
        let t = translate("shop", "SELECT price + stockQuantity * 2 AS expr FROM products").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let project = stages
            .iter()
            .find(|s| s.get("$project").is_some())
            .expect("project");
        // Should be $add: ["$price", {$multiply: ["$stockQuantity", 2]}]
        assert_eq!(project["$project"]["expr"]["$add"][0], "$price");
        assert_eq!(project["$project"]["expr"]["$add"][1]["$multiply"][0], "$stockQuantity");
        assert_eq!(project["$project"]["expr"]["$add"][1]["$multiply"][1], 2.0);
    }

    #[test]
    fn arithmetic_all_four_operators() {
        let t = translate(
            "shop",
            "SELECT (price + 10) / (stockQuantity - 1) * 2 AS expr FROM products",
        )
        .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let project = stages
            .iter()
            .find(|s| s.get("$project").is_some())
            .expect("project");
        // Top level should be $multiply: [ {$divide: [...]}, 2 ]
        let top = &project["$project"]["expr"];
        assert!(top["$multiply"].is_array());
        assert_eq!(top["$multiply"][1], 2.0);
        let div = &top["$multiply"][0];
        assert_eq!(div["$divide"][0]["$add"][0], "$price");
        assert_eq!(div["$divide"][0]["$add"][1], 10.0);
        assert_eq!(div["$divide"][1]["$subtract"][0], "$stockQuantity");
        assert_eq!(div["$divide"][1]["$subtract"][1], 1.0);
    }

    // ---- Write operation translation tests ----

    #[test]
    fn translates_update_with_set_and_where() {
        let t = translate("shop", "UPDATE products SET name = \"Widget\", price = 10 WHERE status = \"active\"").expect("translate");
        assert_eq!(t.collection, "products");
        match &t.operation {
            SqlOperation::Update { filter, update, multi, upsert } => {
                assert!(filter.get("status").is_some());
                assert_eq!(update["$set"]["name"], "Widget");
                assert_eq!(update["$set"]["price"], 10.0);
                assert!(*multi);
                assert!(!*upsert);
            }
            other => panic!("expected Update, got {:?}", other),
        }
        assert!(t.code.contains_key("node-js"));
        assert!(t.code.contains_key("python"));
    }

    #[test]
    fn translates_update_with_upsert() {
        let t = translate("shop", "UPDATE products SET count = 1 WHERE _id = \"abc\" UPSERT").expect("translate");
        match &t.operation {
            SqlOperation::Update { upsert, .. } => assert!(*upsert),
            other => panic!("expected Update, got {:?}", other),
        }
    }

    #[test]
    fn translates_insert_into_values_object() {
        let t = translate("shop", "INSERT INTO products VALUES {\"name\":\"A\",\"price\":5}").expect("translate");
        assert_eq!(t.collection, "products");
        match &t.operation {
            SqlOperation::Insert { documents } => {
                assert_eq!(documents.len(), 1);
                assert_eq!(documents[0]["name"], "A");
                assert_eq!(documents[0]["price"], 5.0);
            }
            other => panic!("expected Insert, got {:?}", other),
        }
    }

    #[test]
    fn translates_insert_into_columns_values() {
        let t = translate("shop", "INSERT INTO products (name, price) VALUES (\"A\", 5)").expect("translate");
        assert_eq!(t.collection, "products");
        match &t.operation {
            SqlOperation::Insert { documents } => {
                assert_eq!(documents.len(), 1);
                assert_eq!(documents[0]["name"], "A");
                assert_eq!(documents[0]["price"], 5.0);
            }
            other => panic!("expected Insert, got {:?}", other),
        }
    }

    #[test]
    fn translates_delete_with_where() {
        let t = translate("shop", "DELETE FROM products WHERE status = \"archived\"").expect("translate");
        assert_eq!(t.collection, "products");
        match &t.operation {
            SqlOperation::Delete { filter, multi } => {
                assert_eq!(filter["status"], "archived");
                assert!(*multi);
            }
            other => panic!("expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn delete_without_where_uses_empty_filter() {
        let t = translate("shop", "DELETE FROM products").expect("translate");
        match &t.operation {
            SqlOperation::Delete { filter, .. } => {
                assert!(filter.as_object().unwrap().is_empty());
            }
            other => panic!("expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn remove_is_alias_for_delete() {
        let t = translate("shop", "REMOVE FROM products WHERE status = \"archived\"").expect("translate");
        assert_eq!(t.collection, "products");
        assert!(matches!(t.operation, SqlOperation::Delete { .. }));
    }

    #[test]
    fn rejects_unsupported_statement() {
        assert!(translate("shop", "DROP TABLE products").is_err());
    }

    #[test]
    fn update_rejects_trailing_input() {
        assert!(translate("shop", "UPDATE products SET x = 1 WHERE y = 2 trailing").is_err());
    }

    #[test]
    fn insert_without_column_list_requires_json_or_columns() {
        assert!(translate("shop", "INSERT INTO products VALUES (1, 2)").is_err());
    }
}
