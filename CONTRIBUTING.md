# Contributing

## Don't let fixed bugs come back

This project has been bitten by regressions of already-fixed bugs (most
memorably: connection logic that broke replica-set writes with
`NotWritablePrimary`). The rules below exist to make that class of failure hard
to reintroduce.

### 1. Every bug fix ships with a regression test

When you fix a bug, add a test that **fails before your fix and passes after**.
Name it after the symptom so the intent is obvious later (e.g.
`ensure_skips_when_replica_set_specified`). A fix without a test is not done.

- Pure-logic bugs → a unit test next to the code.
- Bugs that only show up against a real database → an env-gated integration
  test (see `src-tauri/tests/replica_set_write.rs` for the pattern). Gate it on
  an env var so the default `cargo test` stays offline and green.

### 2. Never silently inject `directConnection`

The desktop app passes connection URIs to the driver **untouched**
(`build_client` does not modify them). Forcing `directConnection=true` pins the
driver to a single seed host (`Single` topology); if that host is a replica-set
secondary, every write fails with `NotWritablePrimary` (10107). Letting the
driver discover the topology means writes are always routed to the current
primary on any deployment, with no host configuration. A user who wants a
pinned connection opts in explicitly via `?directConnection=true`.

The one legitimate place to force a pin is the auditor/attester reading a
*specific* replica member's own oplog copy. That intent lives in **one** place:
`mongo_uri::force_direct_connection`. Do **not** re-inline `directConnection=true`
string handling anywhere else — the original regression happened precisely
because that logic was copy-pasted and the copies drifted.

### 3. Run the pre-push gate locally

Enable the shared git hooks once per clone:

```sh
git config core.hooksPath .githooks
```

On `git push` this runs `cargo test --workspace` and the frontend build as hard
gates, plus `cargo fmt`/`cargo clippy` as advisory checks.

## Local databases

- `docker compose up -d` — **single-node** replica set for general dev. Always
  primary, reachable at `mongodb://localhost:27017/?replicaSet=rs0` with no
  `/etc/hosts` alias and no `directConnection`.
- `docker compose -f docker-compose.audit-db.yml up -d` then
  `docker compose -f docker-compose.audit.yml up -d` — the 3-node replica set
  plus the audit services, for the audit trust-anchor demo.

### Running the replica-set write regression test

This guards that `build_client` never pins to a secondary. Point it at a
secondary member of any real multi-node replica set (resolvable hostnames):

```sh
NOSQLBUDDY_TEST_RS_SECONDARY_URI="mongodb://<secondary-host>/?replicaSet=<name>" \
  cargo test -p nosql-buddy --test replica_set_write -- --nocapture
```

## Releasing the audit service image

The audit daemon ships as a Docker image so users and servers never need the
Rust toolchain or a source checkout.

- Pushing a `v*` tag (e.g. `v0.1.0`) triggers `.github/workflows/docker-publish.yml`,
  which builds `audit-service/Dockerfile.audit` and pushes
  `ghcr.io/ronnakamoto/nosqlbuddy-audit` tagged with the semver, `MAJOR.MINOR`,
  and `latest`. It also attaches `deploy/docker-compose.audit.yml` and
  `deploy/audit-stack.env.example` as release assets.
- Keep the desktop app's crate version (`src-tauri/Cargo.toml`) in step with the
  release tag: packaged Dev Mode pulls `ghcr.io/ronnakamoto/nosqlbuddy-audit:<crate-version>`
  (see `published_image_ref` in `src-tauri/src/audit/dev_stack.rs`), so the tag
  must exist in the registry for that version.
- The source `docker-compose.audit.yml` builds locally by default
  (`AUDIT_IMAGE` unset → `nosqlbuddy-audit:dev`); set `AUDIT_IMAGE` to run the
  published image instead. The server-oriented `deploy/docker-compose.audit.yml`
  always uses the published image.
