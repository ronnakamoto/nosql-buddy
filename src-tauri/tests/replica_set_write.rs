//! Regression test for the `NotWritablePrimary` (server error 10107) bug.
//!
//! History: `build_client` used to append `directConnection=true` to the URI.
//! On a replica set that pins the driver to the single seed host (a `Single`
//! topology), so writes never reach the primary. When the seed was a
//! *secondary*, every write failed with `NotWritablePrimary`. The fix is that
//! `build_client` now passes the URI through untouched and lets the driver
//! discover the topology and route writes to the primary.
//!
//! This test connects through the real `build_client` path to a replica-set
//! seed (ideally a **secondary**) and performs an update. It must succeed,
//! proving writes are routed to the primary and that `build_client` does not
//! silently pin the connection.
//!
//! Env-gated so offline `cargo test` stays green. Point it at a secondary of
//! any real multi-node replica set with resolvable hostnames:
//!
//! ```sh
//! NOSQLBUDDY_TEST_RS_SECONDARY_URI="mongodb://<secondary-host>/?replicaSet=<name>" \
//!   cargo test -p nosql-buddy --test replica_set_write -- --nocapture
//! ```

use app_lib::mongo::client_registry::build_client;
use bson::doc;

#[tokio::test]
async fn write_to_replica_set_secondary_seed_routes_to_primary() {
    let uri = match std::env::var("NOSQLBUDDY_TEST_RS_SECONDARY_URI") {
        Ok(uri) if !uri.trim().is_empty() => uri,
        _ => {
            eprintln!(
                "skipping: set NOSQLBUDDY_TEST_RS_SECONDARY_URI to a replica-set \
                 secondary seed (e.g. mongodb://localhost:27018/?replicaSet=rs0) to run"
            );
            return;
        }
    };

    let client = build_client(&uri, "NoSQLBuddy-regression")
        .await
        .expect("build_client should connect to the replica set");

    let coll = client
        .database("nosqlbuddy_regression")
        .collection::<bson::Document>("primary_routing");

    // This update would fail with NotWritablePrimary (10107) if the driver were
    // pinned to the secondary seed via directConnection=true.
    let result = coll
        .update_one(
            doc! { "_id": "canary" },
            doc! { "$set": { "ts": chrono::Utc::now().timestamp_millis() } },
        )
        .upsert(true)
        .await
        .expect("write must succeed by routing to the primary, not NotWritablePrimary");

    assert!(
        result.matched_count == 1 || result.upserted_id.is_some(),
        "expected the upsert to match or insert the canary document"
    );

    // Clean up so reruns stay deterministic.
    let _ = coll.delete_one(doc! { "_id": "canary" }).await;
}
