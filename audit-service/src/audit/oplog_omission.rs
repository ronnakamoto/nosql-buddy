//! Omission detection integration test.
//!
//! This test proves the core completeness guarantee: if even a single
//! oplog entry is omitted from the hash computation, the resulting
//! Merkle root changes and the mismatch is detected.
//!
//! The test:
//! 1. Reads the real oplog from the running replica set.
//! 2. Computes the "honest" oplog hash (all entries).
//! 3. Omits one entry (simulating the operator skipping a write).
//! 4. Computes the "omitted" oplog hash.
//! 5. Asserts the two hashes differ.
//! 6. Also demonstrates that the auditor (reading from the independent
//!    member) would detect the mismatch.
//!
//! Requires: `docker compose up -d` with the 3-member replica set running.
//! Run with: `cargo test --lib audit::oplog::omission -- --ignored --nocapture`

#[cfg(test)]
mod omission {
    use crate::audit::oplog::*;
    use bson::doc;
    use mongodb::Client;
    use mongo_uri::force_direct_connection;

    async fn connect(uri: &str) -> Client {
        let uri = force_direct_connection(uri);
        Client::with_uri_str(&uri)
            .await
            .expect("failed to connect to MongoDB")
    }

    /// Insert a test document to ensure there's oplog activity.
    async fn insert_test_doc(client: &Client, value: i32) {
        client
            .database("omission_test")
            .collection::<bson::Document>("entries")
            .insert_one(doc! {
                "value": value,
                "ts": chrono::Utc::now().timestamp_millis(),
            })
            .await
            .expect("failed to insert test doc");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_omission_detected_by_hash_change() {
        let primary = connect("mongodb://localhost:27017").await;

        // Insert several test documents to create oplog entries.
        for i in 0..5 {
            insert_test_doc(&primary, i).await;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        // Get the latest oplog timestamp.
        let latest_ts = get_latest_oplog_ts(&primary).await.unwrap();
        assert!(latest_ts.time > 0, "oplog should have entries");

        // Read all oplog entries up to the latest timestamp.
        let all_entries = read_oplog_from_beginning(&primary, latest_ts).await.unwrap();
        assert!(
            all_entries.len() >= 5,
            "should have at least 5 oplog entries (from our inserts + setup)"
        );

        // Compute the honest hash (all entries).
        let honest_root = compute_oplog_root_hex(&all_entries);

        // Now omit one entry — simulate the operator skipping a write.
        // We remove an entry from the middle to simulate omission.
        let omit_index = all_entries.len() / 2;
        let mut omitted_entries = all_entries.clone();
        omitted_entries.remove(omit_index);

        let omitted_root = compute_oplog_root_hex(&omitted_entries);

        println!("=== Omission Detection Demo ===");
        println!("Total oplog entries: {}", all_entries.len());
        println!("Omitted entry at index: {} (ts: {:?})",
            omit_index,
            all_entries[omit_index].get("ts")
        );
        println!("Honest root:  {}", honest_root);
        println!("Omitted root: {}", omitted_root);
        println!();

        // THE KEY ASSERTION: omitting even one entry changes the root.
        assert_ne!(
            honest_root, omitted_root,
            "omitting a single oplog entry MUST change the Merkle root — \
            this is the completeness guarantee"
        );

        println!("✓ Omission detected: honest and omitted roots differ");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_auditor_detects_operator_omission() {
        // This test simulates the full three-way compare:
        // 1. The operator commits an oplog hash (but omits an entry).
        // 2. The auditor independently computes the oplog hash from the
        //    independent replica member.
        // 3. The auditor detects the mismatch.

        let primary = connect("mongodb://localhost:27017").await;
        let independent = connect("mongodb://localhost:27019").await;

        // Insert test documents.
        for i in 0..3 {
            insert_test_doc(&primary, 100 + i).await;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let latest_ts = get_latest_oplog_ts(&primary).await.unwrap();

        // The auditor reads from the independent member — gets the FULL oplog.
        let auditor_entries = read_oplog_from_beginning(&independent, latest_ts).await.unwrap();
        let auditor_root = compute_oplog_root_hex(&auditor_entries);

        // The operator "omits" an entry — simulates fraud.
        let mut operator_entries = read_oplog_from_beginning(&primary, latest_ts).await.unwrap();
        if operator_entries.len() > 2 {
            operator_entries.remove(operator_entries.len() - 2); // omit second-to-last
        }
        let operator_root = compute_oplog_root_hex(&operator_entries);

        println!("=== Three-Way Compare: Auditor vs. Operator ===");
        println!("Auditor entries:   {}", auditor_entries.len());
        println!("Operator entries:  {}", operator_entries.len());
        println!("Auditor root:      {}", auditor_root);
        println!("Operator root:     {}", operator_root);
        println!();

        // The auditor's root (from the independent member) should differ
        // from the operator's root (which omitted an entry).
        assert_ne!(
            auditor_root, operator_root,
            "auditor must detect the operator's omission — the independent \
            replica has all entries, the operator's hash is missing one"
        );

        // The auditor's root is the honest one (all entries from the
        // independent replica).
        let honest_entries = read_oplog_from_beginning(&primary, latest_ts).await.unwrap();
        let honest_root = compute_oplog_root_hex(&honest_entries);
        assert_eq!(
            auditor_root, honest_root,
            "auditor's root (from independent member) must match the honest root \
            (all entries from primary) — H2 determinism"
        );

        println!("✓ Auditor detects operator omission: auditor root matches honest, operator root differs");
        println!("✓ H2 determinism: auditor (independent member) and honest (primary) roots match");
    }

    #[tokio::test]
    #[ignore = "requires running 3-member replica set (docker compose up)"]
    async fn test_inclusion_proof_for_oplog_entry() {
        // Verify that we can generate and verify a Merkle inclusion proof
        // for a specific oplog entry. This proves a specific write is in
        // the committed oplog tree.

        let primary = connect("mongodb://localhost:27017").await;
        insert_test_doc(&primary, 42).await;
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        let latest_ts = get_latest_oplog_ts(&primary).await.unwrap();
        let entries = read_oplog_from_beginning(&primary, latest_ts).await.unwrap();

        let tree = build_oplog_tree(&entries);
        let root = tree.root();

        // Generate inclusion proof for the last entry.
        let last_idx = entries.len() - 1;
        let last_hash = crate::audit::oplog_canon::hash_oplog_entry(&entries[last_idx]);
        let proof = tree.prove_inclusion(last_idx).unwrap();

        // Verify the proof.
        assert!(
            OplogMerkleTree::verify_inclusion(last_hash, last_idx, &proof, root),
            "inclusion proof for the last oplog entry should verify"
        );

        // Verify that a wrong leaf is rejected.
        let wrong_hash = [0xff; 32];
        assert!(
            !OplogMerkleTree::verify_inclusion(wrong_hash, last_idx, &proof, root),
            "wrong leaf hash should not verify"
        );

        println!("✓ Inclusion proof for oplog entry {} verifies", last_idx);
        println!("  Root: {}", tree.root_hex());
        println!("  Proof length: {} siblings", proof.len());
    }
}
