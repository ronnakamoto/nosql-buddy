//! Query Code generator: translate a SQL translation (pipeline or find
//! document) into idiomatic driver code in JavaScript (Node), Python,
//! Java (sync driver 4.x), C#, and Ruby. Each generator takes the
//! SQL translation plus the original SQL string for context and
//! returns a fenced code string ready to copy.
//!
//! The output is intentionally minimal and self-contained so users
//! can paste it into their own application without further edits.
//!
//! Connection-aware variant: when a `ConnectionInfo` is supplied,
//! the snippet uses the user's real Mongo URI (and embeds the
//! profile name + auth mechanism as a comment) instead of the
//! `mongodb://127.0.0.1:27017` placeholder.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    NodeJs,
    Python,
    Java,
    CSharp,
    Ruby,
    Shell,
}

/// Connection details for code generation. When `None` is supplied
/// to a generator, it falls back to a localhost placeholder so
/// existing callers keep working.
#[derive(Debug, Clone, Default)]
pub struct ConnectionInfo {
    /// Full MongoDB URI, including credentials if applicable.
    pub uri: String,
    /// The active database name (already known from the SQL
    /// translation; included here for completeness).
    pub database: String,
    /// Human-readable profile name (e.g. "production-cluster").
    /// Embedded as a leading comment when present.
    pub profile_name: Option<String>,
    /// Auth mechanism label (e.g. "SCRAM-SHA-256").
    pub auth_mechanism: Option<String>,
}

const PLACEHOLDER_URI: &str = "mongodb://127.0.0.1:27017";

pub fn language_label(lang: Language) -> &'static str {
    match lang {
        Language::NodeJs => "JavaScript (Node.js)",
        Language::Python => "Python",
        Language::Java => "Java",
        Language::CSharp => "C#",
        Language::Ruby => "Ruby",
        Language::Shell => "mongo shell",
    }
}

/// Build a short comment line identifying the connection. Returns an
/// empty string when no profile metadata is present so callers can
/// unconditionally prepend it.
fn connection_comment(conn: &ConnectionInfo, comment_syntax: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(name) = &conn.profile_name {
        parts.push(format!("profile: {name}"));
    }
    if let Some(mech) = &conn.auth_mechanism {
        parts.push(format!("auth: {mech}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    let line = parts.join(" · ");
    match comment_syntax {
        "//" => format!("// {line}\n"),
        "#" => format!("# {line}\n"),
        _ => format!("// {line}\n"),
    }
}

fn uri_for(conn: &ConnectionInfo) -> &str {
    if conn.uri.trim().is_empty() {
        PLACEHOLDER_URI
    } else {
        &conn.uri
    }
}

/// Indent each line of `s` by `n` spaces (joined).
fn indent(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| format!("{}{}", pad, l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render a BSON value into the closest literal in the target language.
/// Nested objects and arrays are rendered as inline JSON (works for
/// all five languages with at most a single tweak).
fn json_literal(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".to_string())
}

/// Quote a string in the target language's string-literal syntax.
fn str_in(lang: Language, s: &str) -> String {
    match lang {
        Language::NodeJs | Language::Shell => {
            format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
        }
        Language::Python => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Language::Java => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Language::CSharp => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Language::Ruby => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}

/// Build a JS / Node snippet using the official `mongodb` driver.
pub fn node_js(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    node_js_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn node_js_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(json_literal)
            .collect::<Vec<_>>()
            .join(", ")
    );
    let uri = json_escape(uri_for(conn));
    format!(
        "{comment}import {{ MongoClient }} from \"mongodb\";\n\n\
const client = new MongoClient(\"{uri}\");\n\
await client.connect();\n\
const cursor = client\n\
  .db({db})\n\
  .collection({coll})\n\
  .aggregate({pipeline});\n\
const docs = await cursor.toArray();\n\
console.log(docs);\n",
        comment = connection_comment(conn, "//"),
        uri = uri,
        db = str_in(Language::NodeJs, database),
        coll = str_in(Language::NodeJs, collection),
        pipeline = pipeline_lit,
    )
}

/// Build a Python snippet using `pymongo`.
pub fn python(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    python_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn python_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(json_literal)
            .collect::<Vec<_>>()
            .join(", ")
    );
    let uri_dq = double_quote(uri_for(conn));
    format!(
        "{comment}from pymongo import MongoClient\n\
import json\n\n\
client = MongoClient({uri})\n\
cursor = client[{db}][{coll}].aggregate({pipeline})\n\
for doc in cursor:\n\
{indent}print(doc)\n",
        comment = connection_comment(conn, "#"),
        uri = uri_dq,
        db = double_quote(database),
        coll = double_quote(collection),
        pipeline = pipeline_lit,
        indent = indent("print(doc)", 4),
    )
}

/// Build a Java snippet using the sync driver 4.x.
pub fn java(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    java_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn java_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(|v| format!("new Document(Document.parse({}))", json_literal(v)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let uri_dq = double_quote(uri_for(conn));
    let db_dq = double_quote(database);
    let coll_dq = double_quote(collection);
    format!(
        "{comment}import com.mongodb.client.*;\n\
import org.bson.Document;\n\
import java.util.Arrays;\n\n\
public class Query {{\n\
{indent}public static void main(String[] args) {{\n\
{indent2}MongoClient client = MongoClients.create({uri});\n\
{indent2}MongoCollection<Document> coll = client\n\
{indent3}.getDatabase({db})\n\
{indent3}.getCollection({coll});\n\
{indent2}AggregateIterable<Document> cursor = coll.aggregate({pipeline});\n\
{indent2}cursor.forEach((Block<? super Document>) System.out::println);\n\
{indent}}}\n\
}}\n",
        comment = connection_comment(conn, "//"),
        uri = uri_dq,
        db = db_dq,
        coll = coll_dq,
        pipeline = pipeline_lit,
        indent = indent("public static void main(String[] args) {", 4),
        indent2 = indent("public static void main(String[] args) {", 8),
        indent3 = indent("public static void main(String[] args) {", 12),
    )
}

/// Build a C# snippet using the official `MongoDB.Driver`.
pub fn csharp(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    csharp_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn csharp_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(|v| format!("BsonDocument.Parse({})", json_literal(v)))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let uri_dq = double_quote(uri_for(conn));
    let db_dq = double_quote(database);
    let coll_dq = double_quote(collection);
    format!(
        "{comment}using MongoDB.Bson;\n\
using MongoDB.Driver;\n\n\
var client = new MongoClient({uri});\n\
var cursor = client\n\
  .GetDatabase({db})\n\
  .GetCollection<BsonDocument>({coll})\n\
  .Aggregate<BsonDocument>({pipeline});\n\
foreach (var doc in cursor.ToList()) {{\n\
{indent}Console.WriteLine(doc);\n\
}}\n",
        comment = connection_comment(conn, "//"),
        uri = uri_dq,
        db = db_dq,
        coll = coll_dq,
        pipeline = pipeline_lit,
        indent = indent("Console.WriteLine(doc);", 4),
    )
}

/// Build a Ruby snippet using the official `mongo` gem.
pub fn ruby(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    ruby_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn ruby_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(json_literal)
            .collect::<Vec<_>>()
            .join(", ")
    );
    format!(
        "{comment}require 'mongo'\n\n\
Mongo::Logger.logger.level = Logger::WARN\n\
client = Mongo::Client.new({uri_arr})\n\
cursor = client[{db}][{coll}].aggregate({pipeline})\n\
cursor.each do |doc|\n\
{indent}puts doc\n\
end\n",
        comment = connection_comment(conn, "#"),
        uri_arr = ruby_uri_array(uri_for(conn)),
        db = str_in(Language::Ruby, database),
        coll = str_in(Language::Ruby, collection),
        pipeline = pipeline_lit,
        indent = indent("puts doc", 4),
    )
}

/// Build a mongo shell snippet.
pub fn shell(database: &str, collection: &str, pipeline: &[serde_json::Value]) -> String {
    shell_with(database, collection, pipeline, &ConnectionInfo::default())
}

pub fn shell_with(
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    let pipeline_lit = format!(
        "[{}]",
        pipeline
            .iter()
            .map(json_literal)
            .collect::<Vec<_>>()
            .join(", ")
    );
    format!(
        "{comment}use {db};\n\
db.{coll}.aggregate({pipeline}).forEach(printjson);\n",
        comment = connection_comment(conn, "//"),
        db = database,
        coll = collection,
        pipeline = pipeline_lit,
    )
}

/// Top-level: render code for the requested language. Falls back to
/// `shell` if the language is unrecognised.
pub fn generate(
    lang: Language,
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
) -> String {
    generate_with(
        lang,
        database,
        collection,
        pipeline,
        &ConnectionInfo::default(),
    )
}

pub fn generate_with(
    lang: Language,
    database: &str,
    collection: &str,
    pipeline: &[serde_json::Value],
    conn: &ConnectionInfo,
) -> String {
    match lang {
        Language::NodeJs => node_js_with(database, collection, pipeline, conn),
        Language::Python => python_with(database, collection, pipeline, conn),
        Language::Java => java_with(database, collection, pipeline, conn),
        Language::CSharp => csharp_with(database, collection, pipeline, conn),
        Language::Ruby => ruby_with(database, collection, pipeline, conn),
        Language::Shell => shell_with(database, collection, pipeline, conn),
    }
}

// ---------- Helpers for connection-aware string emission ----------

/// Wrap a string in double quotes with backslash and quote escapes,
/// suitable for embedding into a JSON-like literal.
fn json_escape(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Wrap a string in double quotes with backslash and quote escapes,
/// suitable for embedding into a Python/Java/C# string literal.
fn double_quote(s: &str) -> String {
    json_escape(s)
}

/// Render a Mongo URI as a Ruby array literal. We accept whatever
/// scheme the URI starts with (`mongodb://`, `mongodb+srv://`) and
/// keep the rest as a single element so the user can paste
/// `Mongo::Client.new(['mongodb+srv://...'])` directly.
fn ruby_uri_array(uri: &str) -> String {
    format!("['{uri}']")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_pipeline() -> Vec<serde_json::Value> {
        vec![
            json!({ "$match": { "active": true } }),
            json!({ "$limit": 5 }),
        ]
    }

    #[test]
    fn node_js_emits_aggregate_pipeline() {
        let code = node_js("shop", "products", &sample_pipeline());
        assert!(code.contains("MongoClient"));
        assert!(code.contains("\"shop\""));
        assert!(code.contains("\"products\""));
        assert!(code.contains("\"$match\""));
        assert!(code.contains("\"$limit\""));
        assert!(code.contains(".aggregate("));
    }

    #[test]
    fn python_emits_aggregate_pipeline() {
        let code = python("shop", "products", &sample_pipeline());
        assert!(code.contains("pymongo"));
        assert!(code.contains("client[\"shop\"][\"products\"]"));
        assert!(code.contains("\"$match\""));
    }

    #[test]
    fn java_emits_aggregate_pipeline() {
        let code = java("shop", "products", &sample_pipeline());
        assert!(code.contains("MongoClient"));
        assert!(code.contains("getDatabase(\"shop\")"));
        assert!(code.contains("getCollection(\"products\")"));
        assert!(code.contains("new Document(Document.parse("));
    }

    #[test]
    fn csharp_emits_aggregate_pipeline() {
        let code = csharp("shop", "products", &sample_pipeline());
        assert!(code.contains("MongoClient"));
        assert!(code.contains("GetDatabase(\"shop\")"));
        assert!(code.contains("GetCollection<BsonDocument>(\"products\")"));
        assert!(code.contains("BsonDocument.Parse("));
    }

    #[test]
    fn ruby_emits_aggregate_pipeline() {
        let code = ruby("shop", "products", &sample_pipeline());
        assert!(code.contains("require 'mongo'"));
        assert!(code.contains("client[\"shop\"][\"products\"]"));
        assert!(code.contains(".aggregate("));
    }

    #[test]
    fn shell_emits_use_and_aggregate() {
        let code = shell("shop", "products", &sample_pipeline());
        assert!(code.contains("use shop"));
        assert!(code.contains("db.products.aggregate("));
        assert!(code.contains("printjson"));
    }

    #[test]
    fn generate_dispatches_on_language() {
        let pipeline = sample_pipeline();
        for lang in [
            Language::NodeJs,
            Language::Python,
            Language::Java,
            Language::CSharp,
            Language::Ruby,
            Language::Shell,
        ] {
            let s = generate(lang, "shop", "products", &pipeline);
            assert!(!s.is_empty(), "{:?} returned empty", lang);
        }
    }

    #[test]
    fn json_in_handles_string_escaping() {
        let v = serde_json::json!({ "name": "O'Reilly" });
        let s = json_literal(&v);
        // The single quote in O'Reilly does not need escaping inside JSON.
        assert!(s.contains("O'Reilly"));
    }

    fn conn_for_test() -> ConnectionInfo {
        ConnectionInfo {
            uri: "mongodb://user:pw@cluster.example.com:27017/?authSource=admin".into(),
            database: "shop".into(),
            profile_name: Some("production".into()),
            auth_mechanism: Some("SCRAM-SHA-256".into()),
        }
    }

    #[test]
    fn node_js_with_injects_uri_and_profile_comment() {
        let conn = conn_for_test();
        let code = node_js_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("cluster.example.com:27017"));
        assert!(!code.contains("127.0.0.1:27017"));
        // Both profile and auth are joined into a single comment line.
        assert!(code.contains("// profile: production · auth: SCRAM-SHA-256"));
    }

    #[test]
    fn python_with_injects_uri() {
        let conn = conn_for_test();
        let code = python_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("cluster.example.com:27017"));
        assert!(code.contains("# profile: production"));
    }

    #[test]
    fn java_with_injects_uri() {
        let conn = conn_for_test();
        let code = java_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("cluster.example.com:27017"));
        assert!(code.contains("// profile: production"));
    }

    #[test]
    fn csharp_with_injects_uri() {
        let conn = conn_for_test();
        let code = csharp_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("cluster.example.com:27017"));
    }

    #[test]
    fn ruby_with_injects_uri() {
        let conn = conn_for_test();
        let code = ruby_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("cluster.example.com:27017"));
        assert!(code.contains("# profile: production"));
    }

    #[test]
    fn shell_with_injects_comment_only() {
        // Shell is connected interactively; we don't rewrite the
        // URI into the snippet, but the profile / auth comment
        // should still appear so the user has context.
        let conn = conn_for_test();
        let code = shell_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("// profile: production"));
    }

    #[test]
    fn default_connection_falls_back_to_placeholder() {
        let code = node_js("shop", "products", &sample_pipeline());
        assert!(code.contains("mongodb://127.0.0.1:27017"));
        assert!(!code.contains("// profile:"));
    }

    #[test]
    fn empty_uri_falls_back_to_placeholder() {
        let conn = ConnectionInfo {
            uri: "  ".into(),
            ..ConnectionInfo::default()
        };
        let code = node_js_with("shop", "products", &sample_pipeline(), &conn);
        assert!(code.contains("mongodb://127.0.0.1:27017"));
    }

    #[test]
    fn generate_with_dispatches_all_languages() {
        let conn = conn_for_test();
        let pipeline = sample_pipeline();
        for lang in [
            Language::NodeJs,
            Language::Python,
            Language::Java,
            Language::CSharp,
            Language::Ruby,
            Language::Shell,
        ] {
            let s = generate_with(lang, "shop", "products", &pipeline, &conn);
            assert!(!s.is_empty(), "{:?} returned empty", lang);
        }
    }
}
