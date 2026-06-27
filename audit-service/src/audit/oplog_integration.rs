//! Integration test: oplog hash determinism across replica set members.
//!
//! This test connects to the running 3-member replica set (docker-compose)
//! and verifies that:
//! 1. We can read the oplog from both the primary and the independent secondary.
//! 2. The canonical oplog hash computed from the primary matches the hash
//!    computed from the independent secondary (H2 determinism).
//! 3. The hash changes when a new write is added.
//!
//! Requires: `docker compose up -d` with the 3-member replica set running.
//! Run with: `cargo test --lib audit::oplog::integration -- --ignored --nocapture`

#[cfg(test)]
mod integration {
    use crate::audit::oplog::*;
    use bson::doc;
    use mongodb::Client;
    use mongo_uri::force_direct_connection;

    async fn connect(uri: &str) -> Client {
        // Pin to the specified member: the rs is configured with internal Docker
        // hostnames (mongo1, mongo2, mongo3) that aren't resolvable from the host,
        // and an auditor deliberately reads one specific member's own oplog copy.
        let uri = force_direct_connection(uri);
        Client::with_uri_str(&uri)
            .await
            .expect("failed to connect to MongoDB")
    }

    async fn insert_test_doc(client: &Client, db: &str, coll: &str, value: i32) {
        client
            .database(db)
            .collection::<bson::Document>(coll)
            .insert_one(doc! { "value": value, "ts": chrono::Utc::now().timestamp_millis() })
            .await
            .expect("failed to insert test doc");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_oplog_hash_determinism_across_members() {
        let primary = connect("mongodb://localhost:27017").await;
        let independent = connect("mongodb://localhost:27019").await;

        // Insert a test document to ensure there's at least one oplog entry.
        insert_test_doc(&primary, "oplog_test", "determinism", 42).await;

        // Give replication a moment to propagate.
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Get the latest oplog timestamp from the primary.
        let latest_ts = get_latest_oplog_ts(&primary).await.unwrap();
        assert!(
            latest_ts.time > 0,
            "oplog should have entries after insert"
        );

        // Read oplog entries from the primary up to latest_ts.
        let primary_entries = read_oplog_from_beginning(&primary, latest_ts).await.unwrap();
        assert!(
            !primary_entries.is_empty(),
            "primary should have oplog entries"
        );

        // Read the same range from the independent secondary.
        // We need to set secondary read preference.
        let independent_entries = read_oplog_from_beginning(&independent, latest_ts).await.unwrap();

        // Compute hashes from both members.
        let primary_root = compute_oplog_root_hex(&primary_entries);
        let independent_root = compute_oplog_root_hex(&independent_entries);

        println!("Primary oplog entries: {}", primary_entries.len());
        println!("Independent oplog entries: {}", independent_entries.len());
        println!("Primary root:         {}", primary_root);
        println!("Independent root:     {}", independent_root);

        // THE KEY ASSERTION: hashes must match across members.
        // This is the H2 determinism guarantee — two honest parties
        // reading the same oplog range from replicated members produce
        // the same hash.
        assert_eq!(
            primary_root, independent_root,
            "oplog hash from primary must match hash from independent member (H2 determinism)"
        );

        println!("✓ H2 determinism verified: primary and independent member produce the same oplog hash");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_oplog_hash_changes_on_new_write() {
        let primary = connect("mongodb://localhost:27017").await;

        // Get the current latest oplog timestamp.
        let ts_before = get_latest_oplog_ts(&primary).await.unwrap();
        let entries_before = read_oplog_from_beginning(&primary, ts_before).await.unwrap();
        let root_before = compute_oplog_root_hex(&entries_before);

        // Insert a new document.
        insert_test_doc(&primary, "oplog_test", "changes", 99).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Get the new latest oplog timestamp.
        let ts_after = get_latest_oplog_ts(&primary).await.unwrap();
        let entries_after = read_oplog_from_beginning(&primary, ts_after).await.unwrap();
        let root_after = compute_oplog_root_hex(&entries_after);

        println!("Root before: {}", root_before);
        println!("Root after:  {}", root_after);
        println!("Entries before: {}", entries_before.len());
        println!("Entries after:  {}", entries_after.len());

        // The root must change when a new write is added.
        assert_ne!(
            root_before, root_after,
            "oplog hash must change when a new write is added"
        );
        assert!(
            entries_after.len() > entries_before.len(),
            "entry count must increase after a new write"
        );

        println!("✓ Hash sensitivity verified: new write produces different root");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_majority_commit_ts_is_available() {
        let primary = connect("mongodb://localhost:27017").await;
        let majority_ts = get_majority_commit_ts(&primary).await.unwrap();

        println!("Majority commit ts: {}({})", majority_ts.time, majority_ts.increment);
        assert!(
            majority_ts.time > 0,
            "majority commit timestamp should be non-zero on a running replica set"
        );

        println!("✓ Majority commit timestamp available from hello command");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_oplog_range_hash_with_majority_clamp() {
        let primary = connect("mongodb://localhost:27017").await;

        // Insert a test doc.
        insert_test_doc(&primary, "oplog_test", "clamp", 1).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let majority_ts = get_majority_commit_ts(&primary).await.unwrap();
        let start = OplogTimestamp::zero();
        let end = majority_ts;

        let range = compute_oplog_range_hash(&primary, 0, start, end).await.unwrap();

        println!("Epoch 0 oplog range:");
        println!("  start_ts: {}({})", range.start_ts.time, range.start_ts.increment);
        println!("  end_ts: {}({})", range.end_ts.time, range.end_ts.increment);
        println!("  entry_count: {}", range.entry_count);
        println!("  oplog_merkle_root: {}", range.oplog_merkle_root_hex);
        println!("  majority_commit_ts: {}({})", range.majority_commit_ts.time, range.majority_commit_ts.increment);

        assert!(range.entry_count > 0, "should have oplog entries");
        assert!(!range.oplog_merkle_root_hex.is_empty(), "root should be non-empty");
        assert!(range.end_ts <= range.majority_commit_ts, "end_ts should be clamped to majority");

        println!("✓ Range hash with majority clamp works correctly");
    }
}
