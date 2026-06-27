//! MongoDB connection-string helper for **deliberately pinning** to one member.
//!
//! General client connections must NOT pin: forcing `directConnection=true`
//! puts the driver in a `Single` topology, so if the seed host is a replica-set
//! secondary every write fails with `NotWritablePrimary` (server error 10107).
//! The desktop app therefore passes connection URIs through untouched and lets
//! the driver discover the topology and route writes to the primary.
//!
//! The one legitimate need for pinning is an auditor/attester that must read a
//! *specific* replica member's own oplog copy. That intent is expressed here, as
//! a single clearly-named function shared by the audit service and its tests so
//! the logic is never re-inlined and never confused with general connections.

/// Force a direct connection to the single seed host.
///
/// Always appends `directConnection=true` (unless already present), pinning
/// the driver to the exact host in the URI and skipping topology discovery.
///
/// Use this **only** when you deliberately want to talk to one specific
/// member — e.g. an auditor/attester reading a particular replica member's
/// own oplog copy. Do not use it for connections that perform writes: pinning
/// to a secondary makes every write fail with `NotWritablePrimary` (10107).
pub fn force_direct_connection(uri: &str) -> String {
    if uri.contains("directConnection=") {
        return uri.to_string();
    }
    append_direct_connection(uri)
}

/// Append `directConnection=true`, choosing `?` or `&` based on whether the
/// URI already has a query string.
fn append_direct_connection(uri: &str) -> String {
    if uri.contains('?') {
        format!("{uri}&directConnection=true")
    } else {
        format!("{uri}?directConnection=true")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_adds_param_when_missing() {
        assert_eq!(
            force_direct_connection("mongodb://localhost:27019"),
            "mongodb://localhost:27019?directConnection=true"
        );
    }

    #[test]
    fn force_adds_even_with_replica_set() {
        // The auditor wants this exact member, even on a replica set.
        assert_eq!(
            force_direct_connection("mongodb://localhost:27019?replicaSet=rs0"),
            "mongodb://localhost:27019?replicaSet=rs0&directConnection=true"
        );
    }

    #[test]
    fn force_is_noop_when_already_present() {
        assert_eq!(
            force_direct_connection("mongodb://localhost:27019?directConnection=true"),
            "mongodb://localhost:27019?directConnection=true"
        );
    }
}
