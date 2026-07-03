#!/bin/sh
# ─── Custom entrypoint for the audit-db replica set (auth + bootstrap) ────
#
# Why not the official image's docker-entrypoint-initdb.d mechanism? That
# mechanism runs your init scripts against a TEMPORARY mongod instance that
# is deliberately bound to loopback ONLY (ignoring --bind_ip_all), so other
# members (and even this node's own advertised hostname, e.g. "mongo1:27017")
# are unreachable during that phase. rs.initiate() needs to reach every
# member — including verifying "is this host me?" for its own entry — so it
# fails there with "No host described in new configuration ... maps to this
# node". This script instead bootstraps against the REAL, fully-bound mongod
# once it's actually listening.
#
# This script runs as root (the default for an `entrypoint:` override) and
# runs mongod directly as root too (skipping the official image's gosu-based
# privilege drop). MongoDB permits this with a log warning; for a disposable
# local Docker network this is a reasonable simplification — it also avoids
# the keyfile-ownership mismatch that a privilege-dropped process would hit
# against a root-owned, mode-400 keyfile.
set -e

cp /keyfile-src/mongo-keyfile /keyfile-local
chmod 400 /keyfile-local

mongod --replSet rs0 --bind_ip_all --keyFile /keyfile-local &
MONGOD_PID=$!

if [ "$MONGO_ROLE" = "primary" ]; then
  echo "Waiting for local mongod to accept connections..."
  until mongosh --host 127.0.0.1 --port 27017 --quiet --eval "db.runCommand('ping')" >/dev/null 2>&1; do
    sleep 1
  done

  # Idempotency: once rs.initiate() has succeeded, its config persists in
  # the data volume, so rs.status() succeeds immediately on every later
  # restart — that's our signal to skip re-running the bootstrap (which
  # would otherwise fail on "user already exists" / "already initialized").
  if mongosh --host 127.0.0.1 --port 27017 --quiet --eval "rs.status().ok" >/dev/null 2>&1; then
    echo "Replica set already initialized — skipping bootstrap."
  else
    echo "Bootstrapping replica set + auth (first run on this volume)..."
    mongosh --host 127.0.0.1 --port 27017 /scripts/rs-init-audit.js
  fi
fi

wait "$MONGOD_PID"
