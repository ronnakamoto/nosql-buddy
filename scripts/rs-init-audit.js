// 3-node replica set initializer for the AUDIT trust-anchor demo.
// Started with `mongosh --nodb` so we can control connection timing.
//
// Members (advertised by their internal Docker hostnames, all on port 27017):
//   mongo1:27017 — primary (operator)
//   mongo2:27017 — secondary (operator)
//   mongo3:27017 — secondary (independent — auditor/regulator)
//
// These names resolve inside the Docker network, where the audit services run.
// The host does NOT consume this set directly via topology discovery. For app
// writes that should be audited, connect to the primary through:
//   mongodb://127.0.0.1:27020/?directConnection=true

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
const hosts = ["mongo1:27017", "mongo2:27017", "mongo3:27017"];

for (const h of hosts) {
  if (!waitForHost(h, maxAttempts)) {
    printjson({ error: "could not reach " + h });
    quit(1);
  }
}

print("All members reachable. Initiating replica set rs0...");
const conn = new Mongo("mongo1:27017");
const db = conn.getDB("admin");

try {
  const result = db.adminCommand({
    replSetInitiate: {
      _id: "rs0",
      members: [
        // mongo1 is pinned as the preferred primary (priority 2) so the
        // host-published port 27020 (directConnection=true) always reaches a
        // writable node. Without this, an election can move PRIMARY to
        // mongo2/mongo3 and direct writes to mongo1 fail with NotWritablePrimary.
        { _id: 0, host: "mongo1:27017", priority: 2 },
        { _id: 1, host: "mongo2:27017", priority: 1 },
        { _id: 2, host: "mongo3:27017", priority: 1 },
      ],
    },
  });
  printjson(result);
} catch (e) {
  print("rs.initiate returned: " + e);
}

print("Waiting for primary election...");
for (let i = 0; i < 30; i++) {
  try {
    const status = conn.getDB("admin").adminCommand({ replSetGetStatus: 1 });
    if (status.ok === 1) {
      const primary = status.members.find((m) => m.stateStr === "PRIMARY");
      if (primary) {
        printjson({ primary: primary.name, ok: true });
        break;
      }
    }
  } catch (e) {
    // rs.status() may fail before initiation completes
  }
  sleep(1000);
}

print("Replica set initialization complete.");
