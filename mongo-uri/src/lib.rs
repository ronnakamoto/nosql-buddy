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

/// A connection URI with the password removed from its userinfo, plus the
/// (percent-decoded) password that was extracted, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrippedUri {
    /// The URI with the password removed. The username (if any) is preserved.
    pub uri: String,
    /// The decoded password that was embedded in the URI, if one was present.
    pub password: Option<String>,
}

/// Remove a password embedded in a MongoDB connection URI's userinfo so it is
/// never persisted in plaintext, returning the sanitized URI and the decoded
/// password.
///
/// The username, scheme (`mongodb://` / `mongodb+srv://`), hosts, database,
/// and query options are all preserved. Per the MongoDB connection-string
/// spec, any `@` or `:` inside a username or password must be percent-encoded,
/// so the first raw `@` terminates the userinfo and the first raw `:` inside
/// the userinfo separates username from password. The extracted password is
/// percent-decoded so it matches a password typed into a dedicated field
/// (which the driver treats as a raw credential).
///
/// A URI with no userinfo, or with a username but no password, is returned
/// unchanged with `password: None`. An empty password (e.g. `user:@host`) is
/// treated as absent.
pub fn strip_password(uri: &str) -> StrippedUri {
    let Some(scheme_end) = uri.find("://") else {
        return StrippedUri {
            uri: uri.to_string(),
            password: None,
        };
    };
    let (scheme, rest) = uri.split_at(scheme_end + 3);
    // The first raw '@' terminates the userinfo section.
    let Some(at) = rest.find('@') else {
        return StrippedUri {
            uri: uri.to_string(),
            password: None,
        };
    };
    let userinfo = &rest[..at];
    let after_at = &rest[at + 1..];
    match userinfo.find(':') {
        Some(colon) => {
            let user = &userinfo[..colon];
            let raw_password = &userinfo[colon + 1..];
            let sanitized = if user.is_empty() {
                // Neither a real username nor keep the empty userinfo.
                format!("{scheme}{after_at}")
            } else {
                format!("{scheme}{user}@{after_at}")
            };
            let password = if raw_password.is_empty() {
                None
            } else {
                Some(percent_decode(raw_password))
            };
            StrippedUri {
                uri: sanitized,
                password,
            }
        }
        // Username only, no password delimiter: nothing to strip.
        None => StrippedUri {
            uri: uri.to_string(),
            password: None,
        },
    }
}

/// Percent-decode a URI component into its raw UTF-8 form.
fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .into_owned()
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

    // ── strip_password ──────────────────────────────────────────────────────

    #[test]
    fn strip_password_noop_without_userinfo() {
        let s = strip_password("mongodb://127.0.0.1:27017/db?retryWrites=true");
        assert_eq!(s.uri, "mongodb://127.0.0.1:27017/db?retryWrites=true");
        assert_eq!(s.password, None);
    }

    #[test]
    fn strip_password_noop_with_username_only() {
        let s = strip_password("mongodb://alice@127.0.0.1:27017/db");
        assert_eq!(s.uri, "mongodb://alice@127.0.0.1:27017/db");
        assert_eq!(s.password, None);
    }

    #[test]
    fn strip_password_extracts_and_keeps_username() {
        let s = strip_password("mongodb://alice:hunter2@127.0.0.1:27017/db?authSource=admin");
        assert_eq!(s.uri, "mongodb://alice@127.0.0.1:27017/db?authSource=admin");
        assert_eq!(s.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn strip_password_drops_userinfo_when_no_username() {
        let s = strip_password("mongodb://:hunter2@127.0.0.1:27017/db");
        assert_eq!(s.uri, "mongodb://127.0.0.1:27017/db");
        assert_eq!(s.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn strip_password_empty_password_is_absent() {
        let s = strip_password("mongodb://alice:@127.0.0.1:27017/db");
        assert_eq!(s.uri, "mongodb://alice@127.0.0.1:27017/db");
        assert_eq!(s.password, None);
    }

    #[test]
    fn strip_password_percent_decodes() {
        // Password "p@ss:w/rd" fully percent-encoded in the URI.
        let s = strip_password("mongodb://alice:p%40ss%3Aw%2Frd@127.0.0.1:27017/db");
        assert_eq!(s.uri, "mongodb://alice@127.0.0.1:27017/db");
        assert_eq!(s.password.as_deref(), Some("p@ss:w/rd"));
    }

    #[test]
    fn strip_password_srv_scheme() {
        let s = strip_password("mongodb+srv://alice:hunter2@cluster.example.com/db");
        assert_eq!(s.uri, "mongodb+srv://alice@cluster.example.com/db");
        assert_eq!(s.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn strip_password_preserves_multiple_hosts_and_options() {
        let s = strip_password(
            "mongodb://alice:hunter2@h1:27017,h2:27017,h3:27017/db?replicaSet=rs0&authSource=admin",
        );
        assert_eq!(
            s.uri,
            "mongodb://alice@h1:27017,h2:27017,h3:27017/db?replicaSet=rs0&authSource=admin"
        );
        assert_eq!(s.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn strip_password_is_idempotent() {
        let once = strip_password("mongodb://alice:hunter2@127.0.0.1:27017/db");
        let twice = strip_password(&once.uri);
        assert_eq!(twice.uri, once.uri);
        assert_eq!(twice.password, None);
    }
}
