#!/usr/bin/env bash
#
# ZK Audit — Oplog Completeness Demo
#
# This script demonstrates the complete oplog completeness protocol:
# 1. Start a 3-member MongoDB replica set (operator + independent auditor)
# 2. Insert test data
# 3. Compute the oplog hash from the primary (operator's view)
# 4. Compute the oplog hash from the independent member (auditor's view)
# 5. Verify they match (H2 determinism)
# 6. Simulate an omission and verify it's detected
#
# Requirements:
#   - Docker (for the replica set)
#   - Rust toolchain (for the test binary)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR/.."

echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║     ZK Audit — Oplog Completeness Demo                                ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

# Step 1: Start the replica set
echo "▶ Step 1: Starting 3-member MongoDB replica set..."
cd "$PROJECT_DIR"
# The audit-db compose file brings up the 3-node set AND runs rs-init + seeder.
docker compose -f docker-compose.audit-db.yml up -d
echo "  Waiting for replica set to initialize..."
sleep 15

echo "  ✓ Replica set is running"
echo "    - mongo1 (primary, operator)    : localhost:27017"
echo "    - mongo2 (secondary, operator)  : localhost:27018"
echo "    - mongo3 (secondary, independent): localhost:27019"
echo ""

# Step 2: Insert test data
echo "▶ Step 2: Inserting test data..."
docker compose -f docker-compose.audit-db.yml exec -T mongo1 mongosh --eval '
  db.getSiblingDB("demo").products.insertMany([
    { name: "Widget", price: 9.99 },
    { name: "Gadget", price: 19.99 },
    { name: "Gizmo", price: 29.99 },
  ]);
  print("Inserted 3 products");
'
sleep 2
echo "  ✓ Test data inserted"
echo ""

# Step 3: Run the integration tests
echo "▶ Step 3: Running oplog completeness tests..."
echo ""
cd "$PROJECT_DIR/src-tauri"

echo "  ── H2 Determinism (primary vs. independent member) ──"
cargo test --lib audit::oplog_integration::integration::test_oplog_hash_determinism_across_members -- --ignored --nocapture 2>&1 | grep -E "✓|root:|entries:"

echo ""
echo "  ── Omission Detection ──"
cargo test --lib audit::oplog_omission::omission::test_omission_detected_by_hash_change -- --ignored --nocapture 2>&1 | grep -E "✓|root:|entries:|Omit"

echo ""
echo "  ── Three-Way Compare (auditor detects operator omission) ──"
cargo test --lib audit::oplog_omission::omission::test_auditor_detects_operator_omission -- --ignored --nocapture 2>&1 | grep -E "✓|root:|entries:"

echo ""
echo "  ── Inclusion Proof ──"
cargo test --lib audit::oplog_omission::omission::test_inclusion_proof_for_oplog_entry -- --ignored --nocapture 2>&1 | grep -E "✓|Root:|Proof"

echo ""
echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║  ✓ Demo Complete — Oplog Completeness Protocol Verified              ║"
echo "║                                                                      ║"
echo "║  The oplog hash is deterministic across replica members (H2).        ║"
echo "║  Omitting even a single entry changes the hash (completeness).       ║"
echo "║  The auditor's independent observation detects operator omission.    ║"
echo "║  Inclusion proofs work for individual oplog entries.                 ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
