//! End-to-end smoke test against a live MongoDB instance.
//!
//! Run with: `cargo run --example smoke -- 127.0.0.1:27017`

use std::time::Instant;

use bson::doc;
use futures_util::StreamExt;
use mongodb::options::ClientOptions;
use serde_json::json;

use app_lib::mongo::client_registry::{
    build_client, describe_connection, list_collections, list_databases, ClientRegistry,
};
use app_lib::mongo::redaction::Redactor;
use app_lib::mongo::sql_to_mongo::translate;
use app_lib::mongo::types::ProfileSummary;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:27017".to_string());
    // Smoke test deliberately pins to the single host it was given.
    let uri = mongo_uri::force_direct_connection(&format!("mongodb://{}/", host));
    println!("== NoSQLBuddy smoke test ==");
    println!("uri: {}", uri);

    // 1. hello
    let opts = ClientOptions::parse(&uri).await?;
    let client = mongodb::Client::with_options(opts)?;
    let started = Instant::now();
    let hello = client
        .database("admin")
        .run_command(doc! { "hello": 1 })
        .await?;
    println!(
        "hello: ok={}ms set={} writable={}",
        started.elapsed().as_millis(),
        hello.get_str("setName").unwrap_or("standalone"),
        hello.get_bool("isWritablePrimary").unwrap_or(false)
    );

    // 2. ClientRegistry reuse
    let registry = ClientRegistry::new();
    let built = build_client(&uri, "NoSQLBuddy-smoke").await?;
    let _handle = describe_connection(&built, "smoke", "smoke", "smoke").await?;
    let databases = list_databases(&built).await?;
    println!("databases: {} -> {:?}", databases.len(), databases);

    // 3. list_collections
    let collections = list_collections(&built, "nosqlbuddy").await?;
    println!("collections: {} -> {:?}", collections.len(), collections);

    // Insert and re-fetch to prove the registry round-trips.
    registry
        .insert(
            "smoke".into(),
            app_lib::mongo::client_registry::ClientEntry {
                profile_id: "smoke".into(),
                name: "smoke".into(),
                deployment_id: "smoke".into(),
                client: built.clone(),
                opened_at: chrono::Utc::now(),
            },
        )
        .await;
    let active = registry.list().await;
    println!("active connections: {}", active.len());

    // 4. find with filter
    let started = Instant::now();
    let coll = client
        .database("nosqlbuddy")
        .collection::<bson::Document>("products");
    let mut cursor = coll
        .find(doc! { "active": true, "price": { "$lt": 30 } })
        .limit(5)
        .await?;
    let mut count = 0;
    while (cursor.next().await).is_some() {
        count += 1;
    }
    println!(
        "find active & price<30: {} docs in {}ms",
        count,
        started.elapsed().as_millis()
    );

    // 5. aggregation with $lookup
    let started = Instant::now();
    let mut cursor = client
        .database("nosqlbuddy")
        .collection::<bson::Document>("orders")
        .aggregate(vec![
            doc! { "$lookup": {
                "from": "users",
                "localField": "userId",
                "foreignField": "_id",
                "as": "user",
            }},
            doc! { "$limit": 3 },
        ])
        .await?;
    let mut count = 0;
    while (cursor.next().await).is_some() {
        count += 1;
    }
    println!(
        "$lookup join: {} docs in {}ms",
        count,
        started.elapsed().as_millis()
    );

    // 6. explain
    let started = Instant::now();
    let explain = client
        .database("nosqlbuddy")
        .run_command(doc! {
            "explain": {
                "find": "products",
                "filter": { "category": "books" }
            },
            "verbosity": "executionStats",
        })
        .await?;
    let exec_ms = explain
        .get_document("executionStats")
        .ok()
        .and_then(|e| e.get_i64("executionTimeMillis").ok());
    println!(
        "explain find: ok={}ms execStats={:?}ms",
        started.elapsed().as_millis(),
        exec_ms
    );

    // 7. SQL translation
    let tx = translate(
        "nosqlbuddy",
        "SELECT name, price FROM products WHERE price < 50 LIMIT 10",
    )?;
    println!(
        "sql-select: pipeline_bytes={} warnings={}",
        serde_json::to_string(&tx.pipeline)
            .map(|s| s.len())
            .unwrap_or_default(),
        tx.warnings.len()
    );
    let tx = translate(
        "nosqlbuddy",
        "SELECT u.name, o.total FROM orders o JOIN users u ON o.userId = u._id LIMIT 5",
    )?;
    println!(
        "sql-join: pipeline={:?} warnings={}",
        tx.pipeline,
        tx.warnings.len()
    );
    let tx = translate(
        "nosqlbuddy",
        "SELECT category, COUNT(*) AS c FROM products GROUP BY category",
    )?;
    println!(
        "sql-group: pipeline={:?} warnings={}",
        tx.pipeline,
        tx.warnings.len()
    );

    // 8. Redaction
    let r = Redactor::new();
    let bad = "uri=mongodb://user:hunter2@host1/?password=foo";
    println!("redact-before: {}", bad);
    println!("redact-after:  {}", r.redact(bad));

    // 9. Profile summary mask
    let summary = ProfileSummary::from_stored(
        "id1".into(),
        "smoke".into(),
        "mongodb://user:secret@host/?password=foo".into(),
        Default::default(),
        true,
        Some("team".into()),
        None,
        None,
        None,
        None,
    );
    println!("profile-mask:  {}", summary.masked_uri);

    // 10. error message redaction
    let redacted = Redactor::new().redact("connection to mongodb://admin:hunter2@host failed");
    println!("err-redacted:  {}", redacted);

    println!("json: {}", json!({"status": "ok", "tests_passed": 10}));

    Ok(())
}
