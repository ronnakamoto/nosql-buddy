//! SQL -> MongoDB translation. Hand-rolled mini-parser that supports a
//! useful subset: SELECT (with aliases), FROM, WHERE, GROUP BY, HAVING,
//! ORDER BY, LIMIT/OFFSET, and JOIN (translated to $lookup). The output
//! is always an aggregation pipeline the user can edit.
//!
//! We intentionally do not pull in a full SQL parser here. The supported
//! grammar is documented in [`translate`]; anything outside it is
//! rejected with a clear `AppError::SqlParse`.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::error::{AppError, AppResult};
use crate::mongo::bson_json::{date_to_extjson, resolve_date_tag};
use crate::mongo::query_code::{self, Language};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    let mut p = Parser::new(sql);
    p.skip_ws();

    if p.peek_keyword("SELECT") {
        translate_select(database, &mut p, &mut warnings)
    } else if p.peek_keyword("UPDATE") {
        translate_update(database, &mut p, &mut warnings)
    } else if p.peek_keyword("INSERT") || p.peek_keyword("UPSERT") {
        translate_insert(database, &mut p, &mut warnings)
    } else if p.peek_keyword("DELETE") || p.peek_keyword("REMOVE") {
        translate_delete(database, &mut p, &mut warnings)
    } else {
        Err(AppError::SqlParse(
            "expected SELECT, UPDATE, INSERT, or DELETE statement".into(),
        ))
    }
}

fn translate_select(
    database: &str,
    p: &mut Parser,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    let stmt = p.parse_select()?;
    let collection = stmt.collection.ok_or_else(|| {
        AppError::SqlParse("FROM clause with at least one table is required".into())
    })?;

    let mut stages: Vec<serde_json::Value> = Vec::new();
    let mut find_filter: Option<serde_json::Value> = None;
    let mut find_projection: Option<serde_json::Value> = None;
    let mut find_sort: Option<serde_json::Value> = None;
    let mut find_limit: Option<i64> = None;
    let mut find_skip: Option<u64> = None;

    if let Some(filter) = stmt.where_clause {
        // WHERE is a pre-group filter document, so emit MongoDB filter
        // shape (`{field: {$op: value}}` or `{field: value}` for `$eq`)
        // and only fall back to `{$expr: ...}` when both sides are field
        // references or the comparison is non-scalar. This keeps indexes
        // usable for the common `field <op> literal` case.
        let value = expr_to_filter(&filter, warnings)?;
        find_filter = Some(value.clone());
        stages.push(serde_json::json!({ "$match": value }));
    }

    for join in &stmt.joins {
        stages.push(join_to_lookup(join, warnings)?);
    }

    if stmt.distinct && stmt.group_by.is_empty() {
        // DISTINCT: collapse duplicates by grouping on the projected
        // fields, then replace the root so the output shape matches
        // what the user asked for (instead of `_id: {...}`).
        let key = build_distinct_group_key(&stmt.projection, warnings)?;
        stages.push(serde_json::json!({ "$group": { "_id": key } }));
        stages.push(serde_json::json!({ "$replaceRoot": { "newRoot": "$_id" } }));
        // No `find` shortcut: distinct always runs through aggregate.
    } else if !stmt.group_by.is_empty() {
        let group = build_group_stage(&stmt.group_by, &stmt.projection, warnings)?;
        stages.push(serde_json::json!({ "$group": group }));
    } else {
        let project = build_project_stage(&stmt.projection, warnings)?;
        if !project.as_object().map(|m| m.is_empty()).unwrap_or(true) {
            stages.push(serde_json::json!({ "$project": project }));
            find_projection = Some(project);
        }
    }

    if let Some(having) = &stmt.having {
        // HAVING runs after $group, so every reference is an aggregation
        // expression and must be wrapped in $expr to be valid inside a
        // $match stage. Bare `{$gt: [...]}` would hit the same
        // "unknown top level operator" error as the WHERE bug.
        let value = expr_to_agg_expr(having, warnings)?;
        stages.push(serde_json::json!({ "$match": { "$expr": value } }));
    }

    if !stmt.order_by.is_empty() {
        let sort = build_sort_stage(&stmt.order_by);
        stages.push(serde_json::json!({ "$sort": sort }));
        find_sort = Some(sort);
    }

    if let Some(limit) = stmt.limit {
        stages.push(serde_json::json!({ "$limit": limit }));
        find_limit = Some(limit);
    }

    if let Some(offset) = stmt.offset {
        stages.push(serde_json::json!({ "$skip": offset }));
        find_skip = Some(offset);
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

    let collection_for_code = collection.clone();
    let pipeline_arr = stages.clone();
    let code = generate_code_variants(database, &collection_for_code, &pipeline_arr);

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

fn translate_update(
    database: &str,
    p: &mut Parser,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    p.expect_keyword("UPDATE")?;
    p.skip_ws();
    let collection = p.parse_identifier()?;
    p.skip_ws();

    let mut filter = serde_json::Value::Object(serde_json::Map::new());
    let mut set_fields = serde_json::Map::new();
    let mut upsert = false;

    if p.peek_keyword("SET") {
        p.consume_keyword("SET")?;
        p.skip_ws();
        loop {
            let field = p.parse_identifier()?;
            p.skip_ws();
            p.expect_char('=')?;
            p.skip_ws();
            let value = expr_to_agg_expr(&p.parse_expr()?, warnings)?;
            set_fields.insert(field, value);
            p.skip_ws();
            if p.peek_char() == Some(',') {
                p.pos += 1;
                p.skip_ws();
                continue;
            }
            break;
        }
    }
    p.skip_ws();

    if p.peek_keyword("WHERE") {
        p.consume_keyword("WHERE")?;
        p.skip_ws();
        filter = expr_to_filter(&p.parse_expr()?, warnings)?;
    }
    p.skip_ws();

    if p.peek_keyword("UPSERT") {
        p.consume_keyword("UPSERT")?;
        upsert = true;
    }

    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(AppError::SqlParse(format!(
            "unexpected trailing input at position {}",
            p.pos
        )));
    }

    let update_doc = serde_json::json!({ "$set": set_fields });
    let code = generate_write_code_variants(database, &collection, "update", &filter, &update_doc, upsert);

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
    p: &mut Parser,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    p.expect_keyword("INSERT")?;
    p.skip_ws();
    p.expect_keyword("INTO")?;
    p.skip_ws();
    let collection = p.parse_identifier()?;
    p.skip_ws();

    let mut documents = Vec::new();

    if p.peek_keyword("VALUES") {
        p.consume_keyword("VALUES")?;
        p.skip_ws();
        loop {
            if p.peek_char() == Some('{') {
                let doc = p.parse_json_value()?;
                documents.push(doc);
            } else {
                let expr = p.parse_expr()?;
                let doc = expr_to_agg_expr(&expr, warnings)?;
                documents.push(doc);
            }
            p.skip_ws();
            if p.peek_char() == Some(',') {
                p.pos += 1;
                p.skip_ws();
                continue;
            }
            break;
        }
    } else if p.peek_char() == Some('{') {
        let doc = p.parse_json_value()?;
        documents.push(doc);
    } else {
        p.expect_char('(')?;
        p.skip_ws();
        let mut fields = Vec::new();
        loop {
            fields.push(p.parse_identifier()?);
            p.skip_ws();
            if p.peek_char() == Some(',') {
                p.pos += 1;
                p.skip_ws();
                continue;
            }
            break;
        }
        p.expect_char(')')?;
        p.skip_ws();
        p.expect_keyword("VALUES")?;
        p.skip_ws();
        loop {
            p.expect_char('(')?;
            p.skip_ws();
            let mut doc = serde_json::Map::new();
            for (i, field) in fields.iter().enumerate() {
                let expr = p.parse_expr()?;
                let value = expr_to_agg_expr(&expr, warnings)?;
                doc.insert(field.clone(), value);
                p.skip_ws();
                if i < fields.len() - 1 {
                    p.expect_char(',')?;
                    p.skip_ws();
                }
            }
            p.expect_char(')')?;
            documents.push(serde_json::Value::Object(doc));
            p.skip_ws();
            if p.peek_char() == Some(',') {
                p.pos += 1;
                p.skip_ws();
                continue;
            }
            break;
        }
    }

    p.skip_ws();
    if p.pos < p.src.len() {
        return Err(AppError::SqlParse(format!(
            "unexpected trailing input at position {}",
            p.pos
        )));
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
    p: &mut Parser,
    warnings: &mut Vec<String>,
) -> AppResult<SqlTranslation> {
    p.expect_keyword("DELETE")?;
    p.skip_ws();
    if p.peek_keyword("FROM") {
        p.consume_keyword("FROM")?;
        p.skip_ws();
    }
    let collection = p.parse_identifier()?;
    p.skip_ws();

    let mut filter = serde_json::Value::Object(serde_json::Map::new());
    if p.peek_keyword("WHERE") {
        p.consume_keyword("WHERE")?;
        p.skip_ws();
        filter = expr_to_filter(&p.parse_expr()?, warnings)?;
    }
    p.skip_ws();

    if p.pos < p.src.len() {
        return Err(AppError::SqlParse(format!(
            "unexpected trailing input at position {}",
            p.pos
        )));
    }

    let code = generate_write_code_variants(database, &collection, "delete", &filter, &serde_json::Value::Null, false);

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

// ---------- Parser ----------

#[derive(Debug)]
struct Stmt {
    projection: Vec<SelectItem>,
    distinct: bool,
    collection: Option<String>,
    joins: Vec<JoinClause>,
    where_clause: Option<Expr>,
    group_by: Vec<Expr>,
    having: Option<Expr>,
    order_by: Vec<OrderItem>,
    limit: Option<i64>,
    offset: Option<u64>,
}

#[derive(Debug, Clone)]
enum SelectItem {
    Star,
    Expr { expr: Expr, alias: Option<String> },
}

#[derive(Debug, Clone)]
struct JoinClause {
    collection: String,
    local_field: String,
    foreign_field: String,
    #[allow(dead_code)]
    kind: JoinKind,
}

#[derive(Debug, Clone, Copy)]
enum JoinKind {
    Inner,
    Left,
}

#[derive(Debug, Clone)]
enum Expr {
    Field(String),
    Number(f64),
    Str(String),
    Bool(bool),
    Null,
    DateTag(String),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    In(Box<Expr>, Vec<Expr>),
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),
    Func { name: String, args: Vec<Expr> },
}

#[derive(Debug, Clone)]
struct OrderItem {
    expr: Expr,
    desc: bool,
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn parse_select(&mut self) -> AppResult<Stmt> {
        self.skip_ws();
        self.expect_keyword("SELECT")?;
        self.skip_ws();
        let distinct = if self.peek_keyword("DISTINCT") {
            self.consume_keyword("DISTINCT")?;
            self.skip_ws();
            true
        } else {
            false
        };
        let projection = self.parse_projection_list()?;
        self.skip_ws();
        self.expect_keyword("FROM")?;
        self.skip_ws();
        let collection = Some(self.parse_identifier()?);
        self.skip_ws();
        // Optional alias: an identifier that is not a known SQL keyword.
        if self.pos < self.src.len()
            && !self.peek_keyword("WHERE")
            && !self.peek_keyword("GROUP")
            && !self.peek_keyword("HAVING")
            && !self.peek_keyword("ORDER")
            && !self.peek_keyword("LIMIT")
            && !self.peek_keyword("OFFSET")
            && !self.peek_keyword("JOIN")
            && !self.peek_keyword("INNER")
            && !self.peek_keyword("LEFT")
            && self
                .peek_char()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            let _ = self.parse_identifier();
            self.skip_ws();
        }

        let mut joins = Vec::new();
        while self.peek_keyword("JOIN") || self.peek_keyword("INNER") || self.peek_keyword("LEFT") {
            joins.push(self.parse_join()?);
            self.skip_ws();
        }

        let where_clause = if self.peek_keyword("WHERE") {
            self.consume_keyword("WHERE")?;
            self.skip_ws();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.skip_ws();

        let group_by = if self.peek_keyword("GROUP") {
            self.consume_keyword("GROUP")?;
            self.skip_ws();
            self.expect_keyword("BY")?;
            self.skip_ws();
            self.parse_expr_list()?
        } else {
            Vec::new()
        };
        self.skip_ws();

        let having = if self.peek_keyword("HAVING") {
            self.consume_keyword("HAVING")?;
            self.skip_ws();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.skip_ws();

        let order_by = if self.peek_keyword("ORDER") {
            self.consume_keyword("ORDER")?;
            self.skip_ws();
            self.expect_keyword("BY")?;
            self.skip_ws();
            self.parse_order_list()?
        } else {
            Vec::new()
        };
        self.skip_ws();

        let limit = if self.peek_keyword("LIMIT") {
            self.consume_keyword("LIMIT")?;
            self.skip_ws();
            let n = self.parse_number()? as i64;
            Some(n)
        } else {
            None
        };
        self.skip_ws();

        let offset = if self.peek_keyword("OFFSET") {
            self.consume_keyword("OFFSET")?;
            self.skip_ws();
            Some(self.parse_number()? as u64)
        } else {
            None
        };
        self.skip_ws();

        if self.pos < self.src.len() {
            return Err(AppError::SqlParse(format!(
                "unexpected trailing input at position {}",
                self.pos
            )));
        }

        Ok(Stmt {
            projection,
            distinct,
            collection,
            joins,
            where_clause,
            group_by,
            having,
            order_by,
            limit,
            offset,
        })
    }

    fn parse_projection_list(&mut self) -> AppResult<Vec<SelectItem>> {
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('*') {
                self.pos += 1;
                items.push(SelectItem::Star);
            } else {
                let expr = self.parse_expr()?;
                let alias = if self.peek_keyword("AS") {
                    self.consume_keyword("AS")?;
                    self.skip_ws();
                    Some(self.parse_identifier()?)
                } else {
                    None
                };
                items.push(SelectItem::Expr { expr, alias });
            }
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(items)
    }

    /// Parse a raw JSON value (object or array) by consuming balanced
    /// braces/brackets, then delegating to `serde_json::from_str`.
    fn parse_json_value(&mut self) -> AppResult<serde_json::Value> {
        self.skip_ws();
        let start = self.pos;
        let open = match self.peek_char() {
            Some('{') => '{',
            Some('[') => '[',
            other => {
                return Err(AppError::SqlParse(format!(
                    "expected JSON object or array, found {:?} at position {}",
                    other, self.pos
                )));
            }
        };
        let close = match open {
            '{' => '}',
            '[' => ']',
            _ => unreachable!(),
        };
        let mut depth = 1;
        let mut in_string = false;
        let mut escape = false;
        self.pos += 1;
        while self.pos < self.src.len() {
            let c = self.src[self.pos..].chars().next().unwrap();
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
                            self.pos += 1;
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

    fn parse_join(&mut self) -> AppResult<JoinClause> {
        let kind = if self.peek_keyword("INNER") {
            self.consume_keyword("INNER")?;
            self.skip_ws();
            JoinKind::Inner
        } else if self.peek_keyword("LEFT") {
            self.consume_keyword("LEFT")?;
            self.skip_ws();
            JoinKind::Left
        } else {
            JoinKind::Inner
        };
        self.expect_keyword("JOIN")?;
        self.skip_ws();
        let collection = self.parse_identifier()?;
        self.skip_ws();
        // Optional alias
        if self.pos < self.src.len()
            && !self.peek_keyword("ON")
            && self
                .peek_char()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            let _ = self.parse_identifier();
            self.skip_ws();
        }
        self.expect_keyword("ON")?;
        self.skip_ws();
        // Expect a.col = b.col
        let _left = self.parse_identifier()?;
        self.expect_char('.')?;
        let lfield = self.parse_identifier()?;
        self.skip_ws();
        self.expect_char('=')?;
        self.skip_ws();
        let _right = self.parse_identifier()?;
        self.expect_char('.')?;
        let rfield = self.parse_identifier()?;
        let _ = kind;
        Ok(JoinClause {
            collection,
            local_field: lfield,
            foreign_field: rfield,
            kind,
        })
    }

    fn parse_order_list(&mut self) -> AppResult<Vec<OrderItem>> {
        let mut items = Vec::new();
        loop {
            let expr = self.parse_expr()?;
            self.skip_ws();
            let desc = if self.peek_keyword("DESC") {
                self.consume_keyword("DESC")?;
                true
            } else if self.peek_keyword("ASC") {
                self.consume_keyword("ASC")?;
                false
            } else {
                false
            };
            items.push(OrderItem { expr, desc });
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(items)
    }

    fn parse_expr_list(&mut self) -> AppResult<Vec<Expr>> {
        let mut items = Vec::new();
        loop {
            items.push(self.parse_expr()?);
            self.skip_ws();
            if self.peek_char() == Some(',') {
                self.pos += 1;
                self.skip_ws();
                continue;
            }
            break;
        }
        Ok(items)
    }

    fn parse_expr(&mut self) -> AppResult<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> AppResult<Expr> {
        let mut left = self.parse_and()?;
        loop {
            self.skip_ws();
            if self.peek_keyword("OR") {
                self.consume_keyword("OR")?;
                let right = self.parse_and()?;
                left = Expr::Or(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> AppResult<Expr> {
        let mut left = self.parse_cmp()?;
        loop {
            self.skip_ws();
            if self.peek_keyword("AND") {
                self.consume_keyword("AND")?;
                let right = self.parse_cmp()?;
                left = Expr::And(Box::new(left), Box::new(right));
            } else {
                break;
            }
        }
        Ok(left)
    }

    fn parse_cmp(&mut self) -> AppResult<Expr> {
        let left = self.parse_primary()?;
        self.skip_ws();
        let op_char = self.peek_char();
        let op = match op_char {
            Some('=') => {
                self.pos += 1;
                "="
            }
            Some('<') => {
                self.pos += 1;
                match self.peek_char() {
                    Some('=') => {
                        self.pos += 1;
                        "<="
                    }
                    Some('>') => {
                        self.pos += 1;
                        "<>"
                    }
                    _ => "<",
                }
            }
            Some('>') => {
                self.pos += 1;
                match self.peek_char() {
                    Some('=') => {
                        self.pos += 1;
                        ">="
                    }
                    _ => ">",
                }
            }
            Some('!') => {
                self.pos += 1;
                if self.peek_char() == Some('=') {
                    self.pos += 1;
                    "!="
                } else {
                    return Err(AppError::SqlParse(format!(
                        "expected `!=` after `!` at position {}",
                        self.pos
                    )));
                }
            }
            _ => return Ok(left),
        };
        self.skip_ws();
        let right = self.parse_primary()?;
        Ok(match op {
            "=" => Expr::Eq(Box::new(left), Box::new(right)),
            "<" => Expr::Lt(Box::new(left), Box::new(right)),
            "<=" => Expr::Le(Box::new(left), Box::new(right)),
            ">" => Expr::Gt(Box::new(left), Box::new(right)),
            ">=" => Expr::Ge(Box::new(left), Box::new(right)),
            _ => Expr::Ne(Box::new(left), Box::new(right)),
        })
    }

    fn parse_primary(&mut self) -> AppResult<Expr> {
        self.skip_ws();
        let c = self.peek_char();
        match c {
            Some('(') => {
                self.pos += 1;
                // Could be a parenthesized expression OR a subquery OR a *
                // special-case like COUNT(*).
                if self.peek_char() == Some('*') {
                    self.pos += 1;
                    self.expect_char(')')?;
                    return Ok(Expr::Func {
                        name: "COUNT".into(),
                        args: vec![Expr::Field("*".into())],
                    });
                }
                let expr = self.parse_expr()?;
                self.skip_ws();
                self.expect_char(')')?;
                Ok(expr)
            }
            Some('\'') => {
                let s = self.parse_string()?;
                Ok(Expr::Str(s))
            }
            Some('"') => {
                let s = self.parse_string()?;
                Ok(Expr::Str(s))
            }
            Some(ch) if ch.is_ascii_digit() || (ch == '-' || ch == '+') => {
                let n = self.parse_number()?;
                Ok(Expr::Number(n))
            }
            Some('#') => {
                self.pos += 1;
                let tag = self.parse_identifier()?;
                Ok(Expr::DateTag(tag))
            }
            Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {
                let ident = self.parse_identifier()?;
                self.skip_ws();
                if self.peek_char() == Some('(') {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek_char() == Some('*') {
                        self.pos += 1;
                        args.push(Expr::Field("*".into()));
                    } else if self.peek_char() != Some(')') {
                        loop {
                            args.push(self.parse_expr()?);
                            self.skip_ws();
                            if self.peek_char() == Some(',') {
                                self.pos += 1;
                                self.skip_ws();
                                continue;
                            }
                            break;
                        }
                    }
                    self.expect_char(')')?;
                    Ok(Expr::Func {
                        name: ident.to_uppercase(),
                        args,
                    })
                } else if ident.eq_ignore_ascii_case("NULL") {
                    Ok(Expr::Null)
                } else if ident.eq_ignore_ascii_case("TRUE") {
                    Ok(Expr::Bool(true))
                } else if ident.eq_ignore_ascii_case("FALSE") {
                    Ok(Expr::Bool(false))
                } else {
                    // May be a column name possibly followed by IS NULL / IS NOT NULL / IN
                    let mut field = ident;
                    // Optional qualifier
                    if self.peek_char() == Some('.') {
                        self.pos += 1;
                        let next = self.parse_identifier()?;
                        field = format!("{field}.{next}");
                    }
                    self.skip_ws();
                    if self.peek_keyword("IS") {
                        self.consume_keyword("IS")?;
                        self.skip_ws();
                        if self.peek_keyword("NOT") {
                            self.consume_keyword("NOT")?;
                            self.skip_ws();
                            self.expect_keyword("NULL")?;
                            return Ok(Expr::IsNotNull(Box::new(Expr::Field(field))));
                        }
                        self.expect_keyword("NULL")?;
                        return Ok(Expr::IsNull(Box::new(Expr::Field(field))));
                    }
                    if self.peek_keyword("IN") {
                        self.consume_keyword("IN")?;
                        self.skip_ws();
                        self.expect_char('(')?;
                        let mut values = Vec::new();
                        if self.peek_char() != Some(')') {
                            loop {
                                values.push(self.parse_expr()?);
                                self.skip_ws();
                                if self.peek_char() == Some(',') {
                                    self.pos += 1;
                                    self.skip_ws();
                                    continue;
                                }
                                break;
                            }
                        }
                        self.expect_char(')')?;
                        return Ok(Expr::In(Box::new(Expr::Field(field)), values));
                    }
                    Ok(Expr::Field(field))
                }
            }
            _ => Err(AppError::SqlParse(format!(
                "unexpected character `{}` at position {}",
                c.unwrap_or('?'),
                self.pos
            ))),
        }
    }

    fn parse_string(&mut self) -> AppResult<String> {
        let quote = self.src[self.pos..].chars().next().unwrap();
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src[self.pos..].chars().next().unwrap();
            if c == quote {
                let s = self.src[start..self.pos].to_string();
                self.pos += 1;
                return Ok(s);
            }
            self.pos += 1;
        }
        Err(AppError::SqlParse("unterminated string literal".into()))
    }

    fn parse_number(&mut self) -> AppResult<f64> {
        self.skip_ws();
        let start = self.pos;
        if matches!(self.peek_char(), Some('+') | Some('-')) {
            self.pos += 1;
        }
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() || c == '.' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(AppError::SqlParse("expected a number".into()));
        }
        self.src[start..self.pos]
            .parse::<f64>()
            .map_err(|e| AppError::SqlParse(format!("invalid number: {e}")))
    }

    fn parse_identifier(&mut self) -> AppResult<String> {
        let start = self.pos;
        while let Some(c) = self.peek_char() {
            if c.is_ascii_alphanumeric() || c == '_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(AppError::SqlParse(format!(
                "expected identifier at position {}",
                self.pos
            )));
        }
        Ok(self.src[start..self.pos].to_string())
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn expect_char(&mut self, ch: char) -> AppResult<()> {
        match self.peek_char() {
            Some(c) if c == ch => {
                self.pos += 1;
                Ok(())
            }
            Some(c) => Err(AppError::SqlParse(format!(
                "expected `{ch}` but found `{c}` at position {}",
                self.pos
            ))),
            None => Err(AppError::SqlParse(format!(
                "expected `{ch}` but reached end of input"
            ))),
        }
    }

    fn peek_keyword(&self, kw: &str) -> bool {
        let rest = &self.src[self.pos..];
        if rest.len() < kw.len() {
            return false;
        }
        if !rest[..kw.len()].eq_ignore_ascii_case(kw) {
            return false;
        }
        // Must be a word boundary
        !matches!(rest[kw.len()..].chars().next(), Some(c) if c.is_ascii_alphanumeric() || c == '_')
    }

    fn consume_keyword(&mut self, kw: &str) -> AppResult<()> {
        if self.peek_keyword(kw) {
            self.pos += kw.len();
            Ok(())
        } else {
            Err(AppError::SqlParse(format!(
                "expected keyword `{kw}` at position {}",
                self.pos
            )))
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> AppResult<()> {
        if self.peek_keyword(kw) {
            self.pos += kw.len();
            Ok(())
        } else {
            Err(AppError::SqlParse(format!(
                "expected keyword `{kw}` at position {}",
                self.pos
            )))
        }
    }
}

fn expr_to_agg_expr(expr: &Expr, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    Ok(match expr {
        Expr::Field(name) => serde_json::json!(format!("${name}")),
        Expr::Number(n) => serde_json::json!(n),
        Expr::Str(s) => serde_json::json!(s),
        Expr::Bool(b) => serde_json::json!(b),
        Expr::Null => serde_json::Value::Null,
        Expr::DateTag(tag) => {
            if let Some(dt) = resolve_date_tag(tag) {
                date_to_extjson(dt)
            } else {
                warnings.push(format!("unknown date tag #{tag}"));
                serde_json::Value::String(format!("#{tag}"))
            }
        }
        Expr::Eq(l, r) => {
            serde_json::json!({ "$eq": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Ne(l, r) => {
            serde_json::json!({ "$ne": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Lt(l, r) => {
            serde_json::json!({ "$lt": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Le(l, r) => {
            serde_json::json!({ "$lte": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Gt(l, r) => {
            serde_json::json!({ "$gt": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Ge(l, r) => {
            serde_json::json!({ "$gte": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::And(l, r) => {
            serde_json::json!({ "$and": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::Or(l, r) => {
            serde_json::json!({ "$or": [expr_to_agg_expr(l, warnings)?, expr_to_agg_expr(r, warnings)?] })
        }
        Expr::In(l, values) => {
            let arr: Vec<_> = values
                .iter()
                .map(|v| expr_to_agg_expr(v, warnings))
                .collect::<AppResult<Vec<_>>>()?;
            serde_json::json!({ "$in": [expr_to_agg_expr(l, warnings)?, serde_json::Value::Array(arr)] })
        }
        Expr::IsNull(inner) => {
            serde_json::json!({ "$eq": [expr_to_agg_expr(inner, warnings)?, serde_json::Value::Null] })
        }
        Expr::IsNotNull(inner) => {
            serde_json::json!({ "$ne": [expr_to_agg_expr(inner, warnings)?, serde_json::Value::Null] })
        }
        Expr::Func { name, args } => {
            let upper = name.to_uppercase();
            match upper.as_str() {
                "COUNT" => {
                    let _ = args;
                    serde_json::json!({ "$sum": 1 })
                }
                "SUM" | "AVG" | "MIN" | "MAX" => {
                    let inner = args.first().ok_or_else(|| {
                        AppError::SqlParse(format!("{upper}() needs an argument"))
                    })?;
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
                _ => {
                    warnings.push(format!("unhandled function {upper}"));
                    serde_json::Value::Null
                }
            }
        }
    })
}

fn build_project_stage(
    projection: &[SelectItem],
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mut out = serde_json::Map::new();
    for item in projection {
        match item {
            SelectItem::Star => {}
            SelectItem::Expr { expr, alias } => {
                let name = match (alias, expr) {
                    (Some(a), _) => a.clone(),
                    (None, Expr::Field(f)) => f.clone(),
                    _ => "expr".to_string(),
                };
                let value = expr_to_agg_expr(expr, warnings)?;
                out.insert(name, value);
            }
        }
    }
    Ok(serde_json::Value::Object(out))
}

/// Build the `_id` key for a DISTINCT group. `SELECT *` is rejected
/// (DISTINCT with no columns is meaningless). Each projected column
/// becomes a sub-field keyed by its column name.
fn build_distinct_group_key(
    projection: &[SelectItem],
    #[allow(clippy::ptr_arg)] _warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mut out = serde_json::Map::new();
    let mut has_star = false;
    let mut count = 0;
    for item in projection {
        match item {
            SelectItem::Star => has_star = true,
            SelectItem::Expr { expr, alias } => {
                let name = match (alias, expr) {
                    (Some(a), _) => a.clone(),
                    (None, Expr::Field(f)) => f.clone(),
                    _ => "expr".to_string(),
                };
                out.insert(name.clone(), format!("${}", name).into());
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
/// - `field IN (...)`      -> `{field: {$in: [...]}}`.
/// - `field IS NULL`       -> `{field: null}`; `IS NOT NULL` ->
///   `{field: {$ne: null}}`.
/// - Functions (`REGEX`, etc.) and any non-scalar shape fall back to
///   `{$expr: <agg expr>}` so they remain valid inside `$match`.
fn expr_to_filter(expr: &Expr, warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    Ok(match expr {
        Expr::Eq(l, r) => comparison_to_filter("$eq", l, r, warnings)?,
        Expr::Ne(l, r) => comparison_to_filter("$ne", l, r, warnings)?,
        Expr::Lt(l, r) => comparison_to_filter("$lt", l, r, warnings)?,
        Expr::Le(l, r) => comparison_to_filter("$lte", l, r, warnings)?,
        Expr::Gt(l, r) => comparison_to_filter("$gt", l, r, warnings)?,
        Expr::Ge(l, r) => comparison_to_filter("$gte", l, r, warnings)?,
        Expr::And(l, r) => {
            let left = expr_to_filter(l, warnings)?;
            let right = expr_to_filter(r, warnings)?;
            merge_and(left, right)
        }
        Expr::Or(l, r) => {
            let left = expr_to_filter(l, warnings)?;
            let right = expr_to_filter(r, warnings)?;
            serde_json::json!({ "$or": [left, right] })
        }
        Expr::In(l, values) => {
            let arr: Vec<_> = values
                .iter()
                .map(|v| expr_to_agg_expr(v, warnings))
                .collect::<AppResult<Vec<_>>>()?;
            if let Some(field) = field_name(l) {
                serde_json::json!({ field: { "$in": serde_json::Value::Array(arr) } })
            } else {
                let left = expr_to_agg_expr(l, warnings)?;
                serde_json::json!({ "$expr": { "$in": [left, serde_json::Value::Array(arr)] } })
            }
        }
        Expr::IsNull(inner) => match field_name(inner) {
            Some(field) => serde_json::json!({ field: serde_json::Value::Null }),
            None => {
                let inner_expr = expr_to_agg_expr(inner, warnings)?;
                serde_json::json!({ "$expr": { "$eq": [inner_expr, serde_json::Value::Null] } })
            }
        },
        Expr::IsNotNull(inner) => match field_name(inner) {
            Some(field) => serde_json::json!({ field: { "$ne": serde_json::Value::Null } }),
            None => {
                let inner_expr = expr_to_agg_expr(inner, warnings)?;
                serde_json::json!({ "$expr": { "$ne": [inner_expr, serde_json::Value::Null] } })
            }
        },
        // Functions (REGEX, aggregates, ...) and bare scalars are
        // aggregation expressions; wrap them so they are legal in $match.
        Expr::Func { .. } | Expr::Field(_) | Expr::Number(_) | Expr::Str(_)
        | Expr::Bool(_) | Expr::Null | Expr::DateTag(_) => {
            let agg = expr_to_agg_expr(expr, warnings)?;
            serde_json::json!({ "$expr": agg })
        }
    })
}

/// Build a single comparison as a filter document. See [`expr_to_filter`]
/// for the shape rules. `op` is the MongoDB operator (`$eq`, `$gt`, ...).
fn comparison_to_filter(
    op: &str,
    l: &Expr,
    r: &Expr,
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    // field <op> literal -> {field: {$op: literal}} (or {field: literal} for $eq)
    if let (Some(field), true) = (field_name(l), is_literal(r)) {
        let lit = expr_to_agg_expr(r, warnings)?;
        return Ok(simple_field_filter(op, field, lit));
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
        return Ok(simple_field_filter(swapped, field, lit));
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

/// `Some(name)` when `expr` is a bare field reference.
fn field_name(e: &Expr) -> Option<&str> {
    if let Expr::Field(name) = e {
        Some(name)
    } else {
        None
    }
}

/// `true` when `expr` is a scalar literal (number, string, bool, null, or
/// a date tag that resolves to a concrete value).
fn is_literal(e: &Expr) -> bool {
    matches!(
        e,
        Expr::Number(_) | Expr::Str(_) | Expr::Bool(_) | Expr::Null | Expr::DateTag(_)
    )
}

fn build_group_stage(
    keys: &[Expr],
    projection: &[SelectItem],
    warnings: &mut Vec<String>,
) -> AppResult<serde_json::Value> {
    let mut group = serde_json::Map::new();
    let mut id_doc = serde_json::Map::new();
    for key in keys {
        let name = match key {
            Expr::Field(f) => f.clone(),
            _ => "_key".to_string(),
        };
        id_doc.insert(name.clone(), serde_json::json!(format!("${name}")));
    }
    group.insert("_id".into(), serde_json::Value::Object(id_doc));
    for item in projection {
        if let SelectItem::Expr { expr, alias } = item {
            let key_name = match (alias, expr) {
                (Some(a), _) => a.clone(),
                (None, Expr::Field(f)) => f.clone(),
                _ => continue,
            };
            if is_grouping_key(expr, keys) {
                continue;
            }
            if let Expr::Func { .. } = expr {
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

fn is_grouping_key(expr: &Expr, keys: &[Expr]) -> bool {
    keys.iter()
        .any(|k| matches!((k, expr), (Expr::Field(name), Expr::Field(other)) if other == name))
}

fn build_sort_stage(order: &[OrderItem]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for item in order {
        let name = match &item.expr {
            Expr::Field(f) => f.clone(),
            _ => "_id".to_string(),
        };
        map.insert(name, serde_json::json!(if item.desc { -1 } else { 1 }));
    }
    serde_json::Value::Object(map)
}

#[allow(clippy::ptr_arg)]
fn join_to_lookup(join: &JoinClause, _warnings: &mut Vec<String>) -> AppResult<serde_json::Value> {
    let as_name = format!("{}_joined", join.collection);
    Ok(serde_json::json!({
        "$lookup": {
            "from": join.collection,
            "localField": join.local_field,
            "foreignField": join.foreign_field,
            "as": as_name,
        }
    }))
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
        let t = translate("shop", "SELECT * FROM products WHERE total = 5").expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        assert!(m["$match"].get("$expr").is_none());
        assert!(m["$match"].get("$eq").is_none());
        assert_eq!(m["$match"]["total"], 5.0);
    }

    #[test]
    fn mixed_and_wraps_each_field_comparison_branch() {
        // Mixed: literal comparison + field comparison. The literal branch
        // becomes `{total: {$gt: 5}}`; the field-vs-field branch needs
        // `$expr` so it stays legal in a filter document. The two cannot
        // merge (one has a `$expr` top-level key), so they remain under
        // `$and`.
        let t = translate("shop", "SELECT * FROM products WHERE total > 5 AND x = y")
            .expect("translate");
        let stages = t.pipeline.as_array().expect("array");
        let m = stages
            .iter()
            .find(|s| s.get("$match").is_some())
            .expect("match");
        let and = m["$match"]["$and"].as_array().expect("and array");
        // Literal branch: {total: {$gt: 5}}
        let literal_branch = and
            .iter()
            .find(|b| b.get("total").is_some())
            .expect("literal total branch");
        assert_eq!(literal_branch["total"]["$gt"], 5.0);
        // Field branch: {$expr: {$eq: ["$x", "$y"]}}
        let field_branch = and
            .iter()
            .find(|b| b.get("$expr").is_some())
            .expect("field $expr branch");
        assert_eq!(field_branch["$expr"]["$eq"][0], "$x");
        assert_eq!(field_branch["$expr"]["$eq"][1], "$y");
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

    // ---- Regression + optimization coverage for the filter-shape fix ----

    /// The exact query that originally surfaced
    /// "unknown top level operator: $eq". Must now emit a legal filter.
    #[test]
    fn user_query_eq_string_literal_emits_field_filter() {
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
    fn rejects_unsupported_statement() {
        assert!(translate("shop", "DROP TABLE products").is_err());
    }

    #[test]
    fn update_rejects_trailing_input() {
        assert!(translate("shop", "UPDATE products SET x = 1 WHERE y = 2 trailing").is_err());
    }
}
