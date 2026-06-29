//! Public types for the Mongo domain. DTOs that cross the IPC boundary all
//! derive `Serialize`/`Deserialize` with `rename_all = "camelCase"`. The
//! `ConnectionProfile` keeps the secret in an `Option<String>` so it can be
//! handed in once at save time and stripped before persistence.

use serde::{Deserialize, Serialize};

/// All supported authentication mechanisms. `None` is a no-auth connection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMechanism {
    #[default]
    None,
    // kebab-case would give "scram-sha1" (no dash before digit); the frontend
    // and connection form both use "scram-sha-1" / "scram-sha-256", so we
    // pin the wire name explicitly.
    #[serde(rename = "scram-sha-1")]
    ScramSha1,
    #[serde(rename = "scram-sha-256")]
    ScramSha256,
    X509,
    Ldap,
    Kerberos,
    AwsIam,
}

/// SSH tunnel configuration for a connection. `local_port = 0` means "pick
/// a free port"; the actual port is captured at connection time.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTunnelConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Path to the SSH private key. Optional secret kept in keychain under
    /// `<profileId>:ssh-key`.
    pub private_key_path: Option<String>,
    pub password: Option<String>,
}

/// SOCKS5 proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Socks5Config {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
}

/// Full profile as seen by the command layer. The `secret` is the only
/// place a credential lives in Rust memory, and only briefly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    pub name: String,
    /// May contain credentials in the URI form. The driver will use this
    /// string directly. It is never serialized back to the frontend in
    /// full — only masked.
    pub uri: String,
    #[serde(default)]
    pub auth_mechanism: AuthMechanism,
    #[serde(skip_serializing)]
    pub secret: Option<String>,
    pub group: Option<String>,
    pub color: Option<String>,
    pub notes: Option<String>,
    pub ssh_tunnel: Option<SshTunnelConfig>,
    pub socks5: Option<Socks5Config>,
}

/// Redacted summary returned to the frontend. The raw URI is masked so a
/// password embedded in the URI never crosses the IPC boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub masked_uri: String,
    pub auth_mechanism: AuthMechanism,
    pub has_secret: bool,
    pub group: Option<String>,
    pub color: Option<String>,
    pub notes: Option<String>,
    pub ssh_tunnel: Option<SshTunnelConfig>,
    pub socks5: Option<Socks5Config>,
}

impl ProfileSummary {
    #[allow(clippy::too_many_arguments)]
    pub fn from_stored(
        id: String,
        name: String,
        uri: String,
        auth_mechanism: AuthMechanism,
        has_secret: bool,
        group: Option<String>,
        color: Option<String>,
        notes: Option<String>,
        ssh_tunnel: Option<SshTunnelConfig>,
        socks5: Option<Socks5Config>,
    ) -> Self {
        Self {
            id,
            name,
            masked_uri: mask_uri(&uri),
            auth_mechanism,
            has_secret,
            group,
            color,
            notes,
            ssh_tunnel,
            socks5,
        }
    }
}

/// Replace any userinfo in a Mongo URI with `***:***@`. The scheme, host,
/// port, options, and database name are preserved.
pub fn mask_uri(uri: &str) -> String {
    if let Some(scheme_end) = uri.find("://") {
        let (scheme, rest) = uri.split_at(scheme_end + 3);
        if let Some(at_pos) = rest.find('@') {
            let after_at = &rest[at_pos + 1..];
            return format!("{scheme}***:***@{after_at}");
        }
    }
    uri.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_uri_strips_userinfo() {
        let masked = mask_uri("mongodb://alice:secret@127.0.0.1:27017/?retryWrites=true");
        assert_eq!(
            masked,
            "mongodb://***:***@127.0.0.1:27017/?retryWrites=true"
        );
    }

    #[test]
    fn mask_uri_passthrough_for_no_userinfo() {
        let masked = mask_uri("mongodb://127.0.0.1:27017");
        assert_eq!(masked, "mongodb://127.0.0.1:27017");
    }

    #[test]
    fn collation_dto_serializes_camel_case_with_locale_only() {
        let dto = CollationDto {
            locale: "en_US".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&dto).expect("serialize");
        // locale is always present; every optional field must be omitted,
        // not serialized as null, so the IPC payload stays small and the
        // frontend's `undefined` checks work.
        assert_eq!(json["locale"], "en_US");
        assert!(json.get("strength").is_none() || json["strength"].is_null());
        assert!(json.get("caseLevel").is_none() || json["caseLevel"].is_null());
    }

    #[test]
    fn collation_dto_round_trips_full_fields() {
        let dto = CollationDto {
            locale: "fr".to_string(),
            strength: Some(2),
            case_level: Some(true),
            case_first: Some("upper".to_string()),
            numeric_ordering: Some(true),
            alternate: Some("shifted".to_string()),
            max_variable: Some("punct".to_string()),
            normalization: Some(false),
            backwards: Some(true),
        };
        let json = serde_json::to_string(&dto).expect("serialize");
        let back: CollationDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(dto, back);
        // camelCase must be applied on the wire.
        assert!(json.contains("\"caseLevel\""));
        assert!(json.contains("\"numericOrdering\""));
        assert!(json.contains("\"maxVariable\""));
    }

    #[test]
    fn index_stats_default_has_zero_ops() {
        let s = IndexStats::default();
        assert_eq!(s.ops, 0);
        assert_eq!(s.name, "");
        assert!(s.since_ms.is_none());
    }
}

/// A connect request. Includes the credential once, in memory only.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub profile_id: String,
    /// Optional override; if absent, the stored secret is used.
    pub secret_override: Option<String>,
}

/// Result of opening a connection. The runtime connection id is what the
/// frontend references on every subsequent request; it is not the profile
/// id so the frontend never has to know how connections are pooled.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionHandle {
    pub connection_id: String,
    pub profile_id: String,
    pub name: String,
    pub server_info: Option<ServerInfo>,
    pub databases: Vec<DatabaseSummary>,
}

/// Server metadata returned by the `hello` command.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: Option<String>,
    pub host: Option<String>,
    pub is_master: Option<bool>,
    pub topology: Option<String>,
}

/// Database summary for the connection tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSummary {
    pub name: String,
    pub size_on_disk: Option<u64>,
    pub collections_count: Option<u64>,
}

/// Collection summary used in the tree navigator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: CollectionKind,
    pub document_count: Option<u64>,
    pub size_bytes: Option<u64>,
    pub storage_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CollectionKind {
    Collection,
    View,
    TimeSeries,
    Sharded,
    Bucketed,
}

/// Result of a find / aggregate query, encoded as MongoDB Extended JSON so
/// the frontend preserves ObjectId, Date, Decimal128, and Binary types.
///
/// Paging: the backend uses skip/limit paging (`skip = (page - 1) *
/// page_size`). `has_more` is true when the page was full
/// (`docs.len() == page_size`), signalling more rows likely exist.
/// `total_count_approx` marks `total_count` as coming from the collection
/// metadata (`estimatedDocumentCount`, ~constant time) rather than a real
/// scan, so the UI can render it with a leading "≈".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPage {
    pub documents: Vec<serde_json::Value>,
    pub limit: u32,
    pub skip: u64,
    pub has_more: bool,
    pub execution_ms: Option<u64>,
    pub total_count: Option<u64>,
    /// True when `total_count` came from `estimatedDocumentCount` (metadata,
    /// fast, approximate) rather than a filtered `countDocuments` (scan).
    #[serde(default, skip_serializing_if = "is_false")]
    pub total_count_approx: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Collation configuration. Mirrors the subset of MongoDB `Collation`
/// fields that are useful from a GUI: locale is mandatory; the rest are
/// optional. Strength is sent as an integer (1=Primary … 5=Identical) to
/// keep the IPC surface a flat JSON object.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CollationDto {
    pub locale: String,
    pub strength: Option<i32>,
    pub case_level: Option<bool>,
    pub case_first: Option<String>,
    pub numeric_ordering: Option<bool>,
    pub alternate: Option<String>,
    pub max_variable: Option<String>,
    pub normalization: Option<bool>,
    pub backwards: Option<bool>,
}

/// Index descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub key: serde_json::Value,
    pub unique: bool,
    pub sparse: bool,
    pub hidden: bool,
    pub ttl_seconds: Option<i32>,
    pub partial_filter_expression: Option<serde_json::Value>,
    pub collation: Option<CollationDto>,
    pub wildcard_projection: Option<serde_json::Value>,
    pub is_text: bool,
    pub is_geo: bool,
    pub is_id: bool,
}

/// Per-index usage statistics from `$indexStats`. `ops` is the operation
/// count; `since` is the server-side timestamp (milliseconds since epoch)
/// when stats collection started. Missing fields are `None` when the
/// server omits them or the value is not a valid date.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub name: String,
    pub ops: i64,
    pub since_ms: Option<i64>,
    pub accesses: Option<i64>,
    pub size_bytes: Option<i64>,
    pub building: Option<bool>,
    pub metadata: Option<serde_json::Value>,
}

/// Collection statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionStats {
    pub name: String,
    pub document_count: u64,
    pub size_bytes: u64,
    pub storage_size_bytes: u64,
    pub index_count: u32,
    pub total_index_size_bytes: u64,
    pub avg_obj_size_bytes: u64,
}

/// Explain output for find / aggregate.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResult {
    pub query_planner_winning_plan: serde_json::Value,
    pub execution_stats: Option<serde_json::Value>,
    pub raw: serde_json::Value,
}
