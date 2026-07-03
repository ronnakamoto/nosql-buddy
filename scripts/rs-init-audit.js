// ─── 3-node replica set + auth bootstrap for the AUDIT trust-anchor demo ──
//
// Runs ONCE, automatically, via the official mongo image's
// docker-entrypoint-initdb.d mechanism, against a temporary bootstrap
// instance on mongo1 — but ONLY on a genuinely empty /data/db (i.e. the
// first time a fresh volume is brought up). On restart with existing data,
// the official entrypoint skips this file entirely, so this script is
// naturally idempotent per-volume without needing its own guard logic.
//
// Because no users exist yet anywhere in the deployment when this runs,
// MongoDB's "localhost exception" lets this script (connecting from
// mongosh, unauthenticated) both initiate the replica set AND create the
// first users. Once the root user below is created, that exception closes
// for the whole deployment (the admin.system.users collection replicates
// to mongo2/mongo3 once they join).
//
// Members (advertised by their internal Docker hostnames, all on port 27017):
//   mongo1:27017 — primary (operator), priority 2 (preferred primary so the
//                  host-published port 27020 always reaches a writable node)
//   mongo2:27017 — secondary (operator), priority 1
//   mongo3:27017 — secondary (INDEPENDENT — auditor/regulator), priority 0,
//                  hidden: true. It still has votes: 1 (the default), which
//                  matters: the oplog-completeness trust model relies on
//                  w:"majority" forcing replication to this member before a
//                  write acks, so it must count toward the majority. It's
//                  just structurally ineligible to become primary and
//                  invisible to normal driver topology discovery / default
//                  read preference — a real "audit replica" should never
//                  serve app traffic or become primary.
//
// ── Credentials (DEV/DEMO ONLY — fixed, not secret, not for production) ──
// Two users are created:
//   root      — full admin access. Used by the operator (publisher) and the
//               seed script. In production this would be the operator's own
//               credential, scoped far more narrowly than `root`.
//   auditor   — minimal privilege: can only `find` on local.oplog.rs (to
//               independently recompute the oplog hash) and run
//               replSetGetStatus (to see replication topology). It CANNOT
//               read shopkeeper.orders or any other application data, and
//               cannot write anything. This is the credential a real
//               auditor/regulator would be handed — read-only, oplog-only.
//
// These are fixed strings (not generated per-run) purely to keep the local
// Dev Mode stack self-contained and reproducible. They protect nothing of
// real value (an isolated Docker bridge network on a developer's own
// machine); a real deployment must generate and rotate its own credentials
// out of band, exactly like the Stellar/age keys the setup wizard already
// generates fresh per run.

const ROOT_USERNAME = "root";
const ROOT_PASSWORD = "nosqlbuddy-dev-root-pw";
const AUDITOR_USERNAME = "auditor";
const AUDITOR_PASSWORD = "nosqlbuddy-dev-auditor-pw";

function waitForHost(host, maxAttempts) {
  for (let i = 0; i < maxAttempts; i++) {
    try {
      const conn = new Mongo(host);
      conn.close();
      printjson({ ok: true, host, attempt: i + 1 });
      return true;
    } catch (e) {
      if (i % 5 === 0) print("Waiting for " + host + " (attempt " + (i + 1) + ")...");
      sleep(1000);
    }
  }
  return false;
}

const maxAttempts = 60;
// Only mongo2/mongo3 need waiting for here — this script itself already
// runs against mongo1's own (temporary, local) bootstrap instance.
for (const h of ["mongo2:27017", "mongo3:27017"]) {
  if (!waitForHost(h, maxAttempts)) {
    printjson({ error: "could not reach " + h });
    quit(1);
  }
}

print("All members reachable. Initiating replica set rs0...");

try {
  const result = rs.initiate({
    _id: "rs0",
    members: [
      // mongo1 is pinned as the preferred primary (priority 2) so the
      // host-published port 27020 (directConnection=true) always reaches a
      // writable node. Without this, an election can move PRIMARY to
      // mongo2/mongo3 and direct writes to mongo1 fail with NotWritablePrimary.
      { _id: 0, host: "mongo1:27017", priority: 2 },
      { _id: 1, host: "mongo2:27017", priority: 1 },
      // Independent audit member: never primary, hidden from normal
      // topology discovery, but still a full voting member (votes: 1 by
      // default) so majority writes are still forced to replicate here.
      { _id: 2, host: "mongo3:27017", priority: 0, hidden: true },
    ],
  });
  printjson(result);
} catch (e) {
  print("rs.initiate returned: " + e);
}

// Wait until THIS node (mongo1, the one this script is connected to over
// its own loopback) is itself the writable primary — not just "some member
// somewhere is primary". createUser is a write; it must run against the
// primary, and during initial election churn a different member can
// transiently hold PRIMARY before settling on mongo1 (which has the
// highest priority and should win once the dust settles).
print("Waiting for this node to become writable primary...");
let isPrimary = false;
for (let i = 0; i < 60; i++) {
  try {
    if (db.hello().isWritablePrimary) {
      isPrimary = true;
      break;
    }
  } catch (e) {
    // may fail during election churn
  }
  sleep(1000);
}
if (!isPrimary) {
  print("timed out waiting to become primary");
  quit(1);
}
print("This node is primary.");

print("Creating root user (closes the localhost exception for this deployment)...");
// Retry the write itself too: a stepdown can still land right between the
// hello() check above and this call.
let userCreated = false;
for (let i = 0; i < 10; i++) {
  try {
    db.getSiblingDB("admin").createUser({
      user: ROOT_USERNAME,
      pwd: ROOT_PASSWORD,
      roles: [{ role: "root", db: "admin" }],
    });
    userCreated = true;
    break;
  } catch (e) {
    print("createUser attempt " + (i + 1) + " failed: " + e);
    sleep(1000);
  }
}
if (!userCreated) {
  print("failed to create root user after retries");
  quit(1);
}

// Retry helper: transient stepdowns can hit any of these writes, not just
// the root user creation above.
function retryWrite(label, fn) {
  for (let i = 0; i < 10; i++) {
    try {
      fn();
      return;
    } catch (e) {
      print(label + " attempt " + (i + 1) + " failed: " + e);
      sleep(1000);
    }
  }
  print("failed: " + label + " after retries");
  quit(1);
}

print("Authenticating as root to create the minimal-privilege auditor role...");
db.getSiblingDB("admin").auth(ROOT_USERNAME, ROOT_PASSWORD);

retryWrite("createRole(auditorOplogReader)", () => {
  db.getSiblingDB("admin").createRole({
    role: "auditorOplogReader",
    privileges: [
      // Read-only access to the replication log itself — enough to
      // independently recompute the oplog Merkle hash. Nothing else.
      { resource: { db: "local", collection: "oplog.rs" }, actions: ["find"] },
      // Needed to check replication/member status (majority commit point,
      // topology) but grants no data access.
      { resource: { db: "admin", collection: "" }, actions: ["replSetGetStatus"] },
      // listDatabases (nameOnly) is needed so the auditor can connect
      // through NoSQLBuddy's connection test, which uses it as the
      // handshake probe. nameOnly=true reveals only database names, not
      // contents, collections, or documents.
      { resource: { cluster: true }, actions: ["listDatabases"] },
    ],
    roles: [],
  });
});

retryWrite("createUser(auditor)", () => {
  db.getSiblingDB("admin").createUser({
    user: AUDITOR_USERNAME,
    pwd: AUDITOR_PASSWORD,
    roles: [{ role: "auditorOplogReader", db: "admin" }],
  });
});

print("Replica set + auth bootstrap complete.");
print("  root credentials:    " + ROOT_USERNAME + " / (dev-only, see script)");
print("  auditor credentials: " + AUDITOR_USERNAME + " / (dev-only, see script) — oplog read-only");
