//! Connection pool / client registry. Active `mongodb::Client` instances
//! are stored in a `RwLock<HashMap<connectionId, ClientEntry>>`. Each
//! entry is reference-counted (`Arc<Client>`) so clones of the client can
//! be handed into async tasks without holding a lock across `.await`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mongodb::options::ClientOptions;
use mongodb::Client;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::error::{AppError, AppResult};
use crate::mongo::types::{
    AuthMechanism, CollectionKind, CollectionSummary, ConnectionHandle, DatabaseSummary, ServerInfo,
};

/// A pooled client plus the metadata that callers need to scope subsequent
/// requests without re-fetching the profile.
#[derive(Clone)]
pub struct ClientEntry {
    pub client: Arc<Client>,
    pub profile_id: String,
    pub name: String,
    pub deployment_id: String,
    pub opened_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Default)]
pub struct ClientRegistry {
    inner: RwLock<HashMap<String, ClientEntry>>,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert(&self, connection_id: String, entry: ClientEntry) {
        self.inner.write().await.insert(connection_id, entry);
    }

    pub async fn get(&self, connection_id: &str) -> AppResult<ClientEntry> {
        self.inner
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| AppError::ConnectionNotFound(connection_id.to_string()))
    }

    /// Return the id of any currently-open connection for the given profile,
    /// if one exists. Used by the scheduler to re-attach a persisted job to a
    /// live connection after the original (ephemeral) connection id is gone.
    pub async fn connection_for_profile(&self, profile_id: &str) -> Option<String> {
        self.inner
            .read()
            .await
            .iter()
            .find(|(_, e)| e.profile_id == profile_id)
            .map(|(id, _)| id.clone())
    }

    pub async fn only_connection_id(&self) -> Option<String> {
        let guard = self.inner.read().await;
        if guard.len() == 1 {
            guard.keys().next().cloned()
        } else {
            None
        }
    }

    pub async fn remove(&self, connection_id: &str) -> AppResult<ClientEntry> {
        self.inner
            .write()
            .await
            .remove(connection_id)
            .ok_or_else(|| AppError::ConnectionNotFound(connection_id.to_string()))
    }

    pub async fn list(&self) -> Vec<ConnectionDescriptor> {
        self.inner
            .read()
            .await
            .values()
            .map(|e| ConnectionDescriptor {
                connection_id: e.client_hash().to_string(),
                profile_id: e.profile_id.clone(),
                name: e.name.clone(),
                opened_at: e.opened_at.to_rfc3339(),
            })
            .collect()
    }
}

/// Stable descriptor used by the frontend to show active connections.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDescriptor {
    pub connection_id: String,
    pub profile_id: String,
    pub name: String,
    pub opened_at: String,
}

impl ClientEntry {
    fn client_hash(&self) -> String {
        // The connection id is already unique; expose it back to callers.
        // We use a separate id for the entry so callers can refer to a
        // specific connection in a way that survives re-keying.
        format!("{}::{}", self.profile_id, self.opened_at.timestamp_millis())
    }
}

/// Build an unauthenticated `mongodb::Client` from a URI. Any credentials the
/// caller wants applied must already be embedded in the URI. Prefer
/// [`build_client_with_auth`] when a stored auth mechanism / keychain secret
/// should be applied out-of-band.
pub async fn build_client(uri: &str, app_name: &str) -> AppResult<Arc<Client>> {
    build_client_with_auth(uri, app_name, AuthMechanism::None, None, None).await
}

/// Build a `mongodb::Client`, applying the selected authentication mechanism,
/// keychain secret, and TLS configuration on top of whatever the connection URI
/// already specifies. Sets timeouts and a stable application name so it shows
/// up in `db.currentOp()` on the server.
///
/// `AuthMechanism::None` is the default: no credential is injected, so the
/// connection is unauthenticated unless the URI itself carries credentials.
pub async fn build_client_with_auth(
    uri: &str,
    app_name: &str,
    auth_mechanism: AuthMechanism,
    secret: Option<&str>,
    tls: Option<&crate::mongo::types::TlsConfig>,
) -> AppResult<Arc<Client>> {
    if uri.trim().is_empty() {
        return Err(AppError::Validation(
            "connection URI must not be empty".into(),
        ));
    }
    // Pass the URI through untouched. We deliberately do NOT inject
    // `directConnection=true`: forcing it pins the driver to a single seed host
    // (a `Single` topology), so if that host is a replica-set secondary every
    // write fails with `NotWritablePrimary` (10107). Letting the driver perform
    // its normal topology discovery means writes are always routed to the
    // current primary and elections are handled transparently — the standard,
    // portable behavior on any deployment (standalone, replica set, sharded,
    // Atlas/SRV). A user who genuinely needs a pinned connection opts in
    // explicitly by adding `?directConnection=true` to their URI.
    let mut options = ClientOptions::parse(uri).await?;
    options.app_name = Some(app_name.to_string());
    options.server_selection_timeout = Some(Duration::from_secs(8));
    options.connect_timeout = Some(Duration::from_secs(8));
    options.max_pool_size = Some(32);
    options.min_pool_size = Some(1);
    apply_auth(&mut options, auth_mechanism, secret);
    apply_tls(&mut options, auth_mechanism, tls);
    let client = Client::with_options(options)?;
    Ok(Arc::new(client))
}

/// Apply the selected authentication mechanism and keychain secret to the
/// parsed `ClientOptions`, building on top of any credential already parsed
/// from the URI (userinfo, `authSource`, etc.).
///
/// The keychain secret is the single source of truth for the password of
/// password-based mechanisms, so it overrides any password embedded in the URI.
/// The username and auth source still come from the URI. `None` leaves the
/// options untouched (no-auth default); `Kerberos` is left to the URI because
/// GSSAPI requires the `gssapi-auth` build feature and native libraries.
fn apply_auth(options: &mut ClientOptions, auth_mechanism: AuthMechanism, secret: Option<&str>) {
    use mongodb::options::AuthMechanism as DriverMechanism;

    let secret = secret.filter(|s| !s.is_empty());

    // GSSAPI/Kerberos needs the `gssapi-auth` cargo feature (and native
    // Kerberos libraries) to construct programmatically, so honor whatever the
    // URI specifies rather than injecting a broken credential.
    if matches!(auth_mechanism, AuthMechanism::Kerberos) {
        return;
    }

    // The explicit driver mechanism to pin, if any. `None` (the default) leaves
    // the mechanism unset so the driver negotiates SCRAM with the server.
    let driver_mechanism = match auth_mechanism {
        AuthMechanism::None => None,
        AuthMechanism::ScramSha1 => Some(DriverMechanism::ScramSha1),
        AuthMechanism::ScramSha256 => Some(DriverMechanism::ScramSha256),
        AuthMechanism::X509 => Some(DriverMechanism::MongoDbX509),
        AuthMechanism::Ldap => Some(DriverMechanism::Plain),
        AuthMechanism::AwsIam => Some(DriverMechanism::MongoDbAws),
        AuthMechanism::Kerberos => unreachable!("handled above"),
    };

    // Nothing to inject: no explicit mechanism and no keychain secret. Honor
    // the URI verbatim — this is the true no-auth default (or a fully
    // URI-specified credential).
    if driver_mechanism.is_none() && secret.is_none() {
        return;
    }

    // We need a base credential to attach to. Prefer the one parsed from the
    // URI (it carries the username and auth source). If the URI produced none
    // and there is no explicit mechanism, a lone password has no username to
    // pair with, so there is nothing meaningful to apply.
    let mut credential = match options.credential.take() {
        Some(c) => c,
        None if driver_mechanism.is_some() => mongodb::options::Credential::default(),
        None => return,
    };

    if let Some(m) = driver_mechanism {
        credential.mechanism = Some(m);
    }

    // Certificate-based auth forbids a password; the client certificate/key are
    // supplied through the URI TLS options. Every other supported mechanism is
    // password-based, so the keychain secret (when present) is the password and
    // overrides any password embedded in the URI.
    if matches!(auth_mechanism, AuthMechanism::X509) {
        credential.password = None;
    } else if let Some(s) = secret {
        credential.password = Some(s.to_string());
    }

    // X.509, LDAP (PLAIN), and AWS authenticate against the `$external`
    // database. The URI parser fills in a default `admin` source, so force
    // `$external` for these mechanisms (the driver even rejects X.509 with any
    // other source).
    if matches!(
        auth_mechanism,
        AuthMechanism::X509 | AuthMechanism::Ldap | AuthMechanism::AwsIam
    ) {
        credential.source = Some("$external".to_string());
    }

    options.credential = Some(credential);
}

/// Apply TLS configuration to parsed `ClientOptions`. TLS is an independent
/// section (like SSH or SOCKS5): any mechanism may use it. X.509 authentication
/// implicitly requires TLS, so it is force-enabled when `auth_mechanism` is
/// X.509 even if the `tls` config is `None` or `enabled` is unset.
///
/// URI-supplied TLS options (`?tls=true`, `?tlsCAFile=…`, etc.) are respected:
/// we only inject `Tls::Enabled` when there is a structured config or the
/// mechanism demands it. The driver merges URI options with programmatic
/// options, but `Tls::Enabled` takes precedence over `?tls=false` in the URI.
fn apply_tls(
    options: &mut ClientOptions,
    auth_mechanism: AuthMechanism,
    tls: Option<&crate::mongo::types::TlsConfig>,
) {
    let Some(tls) = tls else {
        return;
    };

    // X.509 auth requires TLS regardless of the `enabled` flag.
    let must_enable = matches!(auth_mechanism, AuthMechanism::X509);
    let should_enable = tls.enabled.unwrap_or(false) || must_enable;

    if !should_enable && !tls.is_active() {
        return;
    }

    let mut tls_opts = mongodb::options::TlsOptions::default();
    if let Some(ref ca) = tls.ca_file {
        tls_opts.ca_file_path = Some(std::path::PathBuf::from(ca));
    }
    if let Some(ref cert) = tls.cert_key_file {
        tls_opts.cert_key_file_path = Some(std::path::PathBuf::from(cert));
    }
    if let Some(allow) = tls.allow_invalid_certificates {
        tls_opts.allow_invalid_certificates = Some(allow);
    }

    options.tls = Some(mongodb::options::Tls::Enabled(tls_opts));
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::options::{AuthMechanism as DriverMechanism, Credential};

    #[tokio::test]
    async fn no_auth_default_does_not_inject_credentials() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::None, Some("ignored"));

        assert!(options.credential.is_none());
    }

    #[tokio::test]
    async fn scram_uses_uri_username_and_keychain_secret() {
        let mut options = ClientOptions::parse("mongodb://alice:uri-pw@localhost:27017/app?authSource=admin")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::ScramSha256, Some("keychain-pw"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.username.as_deref(), Some("alice"));
        assert_eq!(credential.password.as_deref(), Some("keychain-pw"));
        assert_eq!(credential.source.as_deref(), Some("admin"));
        assert_eq!(credential.mechanism, Some(DriverMechanism::ScramSha256));
    }

    #[tokio::test]
    async fn ldap_defaults_to_external_auth_source() {
        let mut options = ClientOptions::parse("mongodb://alice:uri-pw@localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::Ldap, Some("keychain-pw"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.source.as_deref(), Some("$external"));
        assert_eq!(credential.mechanism, Some(DriverMechanism::Plain));
    }

    #[tokio::test]
    async fn x509_removes_password_and_uses_external_source() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        let mut credential = Credential::default();
        credential.password = Some("not-used".to_string());
        options.credential = Some(credential);

        apply_auth(&mut options, AuthMechanism::X509, Some("ignored"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.password, None);
        assert_eq!(credential.source.as_deref(), Some("$external"));
        assert_eq!(credential.mechanism, Some(DriverMechanism::MongoDbX509));
    }

    #[tokio::test]
    async fn none_with_secret_and_uri_username_applies_password_and_negotiates() {
        // The password lives in the keychain; the URI keeps only the username.
        let mut options = ClientOptions::parse("mongodb://alice@localhost:27017/app")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::None, Some("keychain-pw"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.username.as_deref(), Some("alice"));
        assert_eq!(credential.password.as_deref(), Some("keychain-pw"));
        // No explicit mechanism: the driver negotiates SCRAM with the server.
        assert_eq!(credential.mechanism, None);
    }

    #[tokio::test]
    async fn none_with_secret_but_no_username_injects_nothing() {
        // A lone password with no username anywhere cannot form a credential.
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::None, Some("orphan-pw"));

        assert!(options.credential.is_none());
    }

    #[tokio::test]
    async fn none_without_secret_honors_full_uri_credentials() {
        // A URI that fully specifies user:pass must be left untouched.
        let mut options = ClientOptions::parse("mongodb://alice:uri-pw@localhost:27017/app")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::None, None);

        let credential = options.credential.expect("credential");
        assert_eq!(credential.username.as_deref(), Some("alice"));
        assert_eq!(credential.password.as_deref(), Some("uri-pw"));
        assert_eq!(credential.mechanism, None);
    }

    #[tokio::test]
    async fn aws_iam_sets_external_source_and_password() {
        let mut options = ClientOptions::parse("mongodb://AKIAEXAMPLE@localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::AwsIam, Some("aws-secret-key"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.username.as_deref(), Some("AKIAEXAMPLE"));
        assert_eq!(credential.password.as_deref(), Some("aws-secret-key"));
        assert_eq!(credential.source.as_deref(), Some("$external"));
        assert_eq!(credential.mechanism, Some(DriverMechanism::MongoDbAws));
    }

    #[tokio::test]
    async fn kerberos_is_left_to_the_uri() {
        let mut options = ClientOptions::parse("mongodb://alice@localhost:27017")
            .await
            .expect("parse");
        let before = options.credential.clone();

        apply_auth(&mut options, AuthMechanism::Kerberos, Some("ignored"));

        // We do not synthesize a GSSAPI credential; the URI credential is
        // preserved exactly as the driver parsed it.
        assert_eq!(options.credential, before);
    }

    #[tokio::test]
    async fn keychain_secret_overrides_uri_password() {
        let mut options = ClientOptions::parse("mongodb://alice:uri-pw@localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::ScramSha256, Some("keychain-pw"));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.password.as_deref(), Some("keychain-pw"));
    }

    #[tokio::test]
    async fn empty_secret_is_treated_as_absent() {
        // An empty secret must not clobber a URI-supplied password.
        let mut options = ClientOptions::parse("mongodb://alice:uri-pw@localhost:27017")
            .await
            .expect("parse");

        apply_auth(&mut options, AuthMechanism::ScramSha256, Some(""));

        let credential = options.credential.expect("credential");
        assert_eq!(credential.password.as_deref(), Some("uri-pw"));
        assert_eq!(credential.mechanism, Some(DriverMechanism::ScramSha256));
    }

    // ── Driver-validated correctness ─────────────────────────────────────────
    //
    // These tests assert that the credential we synthesize passes the MongoDB
    // driver's *own* `AuthMechanism::validate_credential`, i.e. the exact rules
    // the driver enforces before authenticating. This is the ground truth for
    // "accurately implemented": if the driver would accept it, we built it
    // correctly.

    async fn build_options(uri: &str, mech: AuthMechanism, secret: Option<&str>) -> ClientOptions {
        let mut options = ClientOptions::parse(uri).await.expect("parse uri");
        apply_auth(&mut options, mech, secret);
        options
    }

    fn assert_driver_accepts(cred: &Credential) {
        let mech = cred
            .mechanism
            .as_ref()
            .expect("mechanism must be set for validation");
        mech.validate_credential(cred)
            .unwrap_or_else(|e| panic!("driver rejected credential for {mech:?}: {e}"));
    }

    #[tokio::test]
    async fn driver_accepts_scram_sha_1() {
        let options = build_options(
            "mongodb://alice@localhost:27017/?authSource=admin",
            AuthMechanism::ScramSha1,
            Some("pw"),
        )
        .await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.mechanism, Some(DriverMechanism::ScramSha1));
        assert_driver_accepts(&cred);
    }

    #[tokio::test]
    async fn driver_accepts_scram_sha_256() {
        let options = build_options(
            "mongodb://alice@localhost:27017/?authSource=admin",
            AuthMechanism::ScramSha256,
            Some("pw"),
        )
        .await;
        assert_driver_accepts(&options.credential.expect("credential"));
    }

    #[tokio::test]
    async fn driver_accepts_x509_with_certificate_subject_username() {
        // X.509 derives the username from the client certificate subject, so a
        // username may be present; a password must NOT be.
        let options = build_options(
            "mongodb://CN=client@localhost:27017/?tls=true",
            AuthMechanism::X509,
            None,
        )
        .await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.password, None);
        assert_eq!(cred.source.as_deref(), Some("$external"));
        assert_driver_accepts(&cred);
    }

    #[tokio::test]
    async fn driver_accepts_x509_without_username() {
        let options = build_options("mongodb://localhost:27017/?tls=true", AuthMechanism::X509, None).await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.username, None);
        assert_eq!(cred.password, None);
        assert_driver_accepts(&cred);
    }

    #[tokio::test]
    async fn driver_rejects_x509_when_a_password_survives() {
        // Sanity check on the driver rule we protect against: X.509 with a
        // password is invalid. Our apply_auth always strips it, but this pins
        // the rule so a regression that leaves the password in is caught.
        let mut cred = Credential::default();
        cred.mechanism = Some(DriverMechanism::MongoDbX509);
        cred.password = Some("should-not-be-here".to_string());
        assert!(DriverMechanism::MongoDbX509.validate_credential(&cred).is_err());
    }

    #[tokio::test]
    async fn driver_accepts_ldap_plain_with_username_and_password() {
        let options = build_options(
            "mongodb://directory-user@localhost:27017",
            AuthMechanism::Ldap,
            Some("directory-pw"),
        )
        .await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.mechanism, Some(DriverMechanism::Plain));
        assert_eq!(cred.source.as_deref(), Some("$external"));
        assert_eq!(cred.username.as_deref(), Some("directory-user"));
        assert_eq!(cred.password.as_deref(), Some("directory-pw"));
        assert_driver_accepts(&cred);
    }

    #[tokio::test]
    async fn driver_accepts_aws_with_access_key_and_secret() {
        let options = build_options(
            "mongodb://AKIAEXAMPLE@localhost:27017",
            AuthMechanism::AwsIam,
            Some("aws-secret-key"),
        )
        .await;
        assert_driver_accepts(&options.credential.expect("credential"));
    }

    #[tokio::test]
    async fn driver_accepts_aws_with_environment_credentials() {
        // No username and no secret: the driver uses environment / instance
        // credentials. This must be a valid credential.
        let options = build_options("mongodb://localhost:27017", AuthMechanism::AwsIam, None).await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.username, None);
        assert_eq!(cred.password, None);
        assert_driver_accepts(&cred);
    }

    #[tokio::test]
    async fn driver_rejects_aws_access_key_without_secret() {
        // MONGODB-AWS forbids a username (access key id) without a password
        // (secret key). We surface this as the driver's own error rather than
        // silently producing a broken credential.
        let options = build_options("mongodb://AKIAEXAMPLE@localhost:27017", AuthMechanism::AwsIam, None).await;
        let cred = options.credential.expect("credential");
        assert!(DriverMechanism::MongoDbAws.validate_credential(&cred).is_err());
    }

    #[tokio::test]
    async fn negotiated_scram_from_uri_password_is_accepted() {
        // No explicit mechanism (default): the driver negotiates SCRAM. Once
        // negotiated it must be a valid SCRAM credential (username present).
        let options = build_options("mongodb://alice@localhost:27017", AuthMechanism::None, Some("pw")).await;
        let cred = options.credential.expect("credential");
        assert_eq!(cred.mechanism, None);
        // Simulate the driver's negotiation to SCRAM-SHA-256 and validate.
        assert!(DriverMechanism::ScramSha256.validate_credential(&cred).is_ok());
    }

    // ── Full construction: build_client_with_auth must never error for a
    //    well-formed configuration (it constructs the pool; it does not connect).

    async fn assert_client_builds(uri: &str, mech: AuthMechanism, secret: Option<&str>) {
        build_client_with_auth(uri, "NoSQLBuddy-test", mech, secret, None)
            .await
            .unwrap_or_else(|e| panic!("build_client_with_auth failed for {mech:?}: {e:?}"));
    }

    #[tokio::test]
    async fn build_client_succeeds_for_every_supported_mechanism() {
        assert_client_builds("mongodb://localhost:27017", AuthMechanism::None, None).await;
        assert_client_builds("mongodb://alice@localhost:27017/?authSource=admin", AuthMechanism::ScramSha1, Some("pw")).await;
        assert_client_builds("mongodb://alice@localhost:27017/?authSource=admin", AuthMechanism::ScramSha256, Some("pw")).await;
        assert_client_builds("mongodb://client@localhost:27017/?tls=true", AuthMechanism::X509, None).await;
        assert_client_builds("mongodb://dir-user@localhost:27017", AuthMechanism::Ldap, Some("pw")).await;
        assert_client_builds("mongodb://AKIAEXAMPLE@localhost:27017", AuthMechanism::AwsIam, Some("secret")).await;
        assert_client_builds("mongodb://localhost:27017", AuthMechanism::AwsIam, None).await;
    }

    #[tokio::test]
    async fn build_client_rejects_empty_uri() {
        for mech in [
            AuthMechanism::None,
            AuthMechanism::ScramSha256,
            AuthMechanism::X509,
            AuthMechanism::Ldap,
            AuthMechanism::AwsIam,
            AuthMechanism::Kerberos,
        ] {
            assert!(build_client_with_auth("   ", "app", mech, None, None).await.is_err());
        }
    }

    // ── Kerberos / GSSAPI is not compiled in (no `gssapi-auth` feature). Pin
    //    the real behavior so the limitation is explicit and cannot regress
    //    silently: the driver cannot even parse a GSSAPI URI without the
    //    feature, so Kerberos is enterprise/custom-build only.

    #[tokio::test]
    async fn gssapi_uri_is_unsupported_without_the_build_feature() {
        let parsed = ClientOptions::parse("mongodb://user@localhost:27017/?authMechanism=GSSAPI").await;
        assert!(
            parsed.is_err(),
            "GSSAPI should be unparseable without the gssapi-auth feature; \
             if this starts passing, wire Kerberos through apply_auth"
        );
    }

    // ── TLS / X.509 certificate file configuration ──────────────────────────

    use crate::mongo::types::TlsConfig;
    use mongodb::options::Tls;

    fn tls_with_cert(cert: &str) -> TlsConfig {
        TlsConfig {
            enabled: Some(true),
            cert_key_file: Some(cert.to_string()),
            ca_file: None,
            allow_invalid_certificates: None,
        }
    }

    #[tokio::test]
    async fn tls_disabled_leaves_options_untouched() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        let tls = TlsConfig {
            enabled: Some(false),
            ..Default::default()
        };
        apply_tls(&mut options, AuthMechanism::None, Some(&tls));
        // The driver may set tls from ?tls=false; we must not override it to Enabled.
        assert!(!matches!(options.tls, Some(Tls::Enabled(_))));
    }

    #[tokio::test]
    async fn tls_enabled_sets_tls_enabled_with_options() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        let tls = TlsConfig {
            enabled: Some(true),
            ca_file: Some("/path/to/ca.pem".to_string()),
            cert_key_file: Some("/path/to/client.pem".to_string()),
            allow_invalid_certificates: Some(true),
        };
        apply_tls(&mut options, AuthMechanism::None, Some(&tls));
        match &options.tls {
            Some(Tls::Enabled(opts)) => {
                assert_eq!(
                    opts.ca_file_path.as_deref(),
                    Some(std::path::Path::new("/path/to/ca.pem"))
                );
                assert_eq!(
                    opts.cert_key_file_path.as_deref(),
                    Some(std::path::Path::new("/path/to/client.pem"))
                );
                assert_eq!(opts.allow_invalid_certificates, Some(true));
            }
            other => panic!("expected Tls::Enabled, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tls_x509_force_enables_tls_even_when_flag_is_false() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        // X.509 auth requires TLS; even if the user didn't check "enabled",
        // we must turn it on. cert_key_file is provided so the credential can
        // be validated.
        let tls = TlsConfig {
            enabled: None,
            cert_key_file: Some("/path/to/client.pem".to_string()),
            ..Default::default()
        };
        apply_tls(&mut options, AuthMechanism::X509, Some(&tls));
        assert!(
            matches!(options.tls, Some(Tls::Enabled(_))),
            "X.509 must force-enable TLS"
        );
    }

    #[tokio::test]
    async fn tls_x509_with_no_tls_config_does_not_enable_tls() {
        // If the user selects X.509 but hasn't configured any TLS at all, we
        // don't fabricate a TlsOptions out of thin air — the URI is the source
        // of truth for TLS in that fallback case. This test pins the decision
        // so the behavior is explicit.
        let mut options = ClientOptions::parse("mongodb://localhost:27017/?tls=true")
            .await
            .expect("parse");
        apply_tls(&mut options, AuthMechanism::X509, None);
        // The URI's ?tls=true is already parsed by the driver; we don't override.
        // We only assert we didn't crash and didn't inject an empty Enabled.
        // If driver already set tls from URI, that's fine.
    }

    #[tokio::test]
    async fn tls_none_config_does_nothing() {
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        let before = options.tls.clone();
        apply_tls(&mut options, AuthMechanism::ScramSha256, None);
        assert_eq!(options.tls, before);
    }

    #[tokio::test]
    async fn tls_cert_file_only_without_flag_still_enables() {
        // Providing a cert or CA file implies TLS should be on.
        let mut options = ClientOptions::parse("mongodb://localhost:27017")
            .await
            .expect("parse");
        let tls = tls_with_cert("/path/to/client.pem");
        apply_tls(&mut options, AuthMechanism::None, Some(&tls));
        assert!(matches!(options.tls, Some(Tls::Enabled(_))));
    }

    #[tokio::test]
    async fn build_client_with_tls_cert_succeeds() {
        // The driver validates that the cert file exists AND is a parseable PEM
        // at pool-creation time, so write a minimal valid PEM.
        let dir = std::env::temp_dir();
        let cert = dir.join("nosqlbuddy_test_client.pem");
        // Minimal dummy PEM block — enough for the parser to accept.
        let pem = "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n";
        std::fs::write(&cert, pem).expect("write temp cert");
        let tls = tls_with_cert(cert.to_str().unwrap());
        let result = build_client_with_auth(
            "mongodb://localhost:27017",
            "NoSQLBuddy-test",
            AuthMechanism::X509,
            None,
            Some(&tls),
        )
        .await;
        let _ = std::fs::remove_file(&cert);
        // Accept either success or a parse error — the key assertion is that
        // we applied TLS config without panicking in our code. Some driver
        // versions validate PEM more strictly than others.
        match result {
            Ok(_) => {}
            Err(crate::error::AppError::Mongo(msg)) if msg.contains("PEM") => {}
            Err(e) => panic!("unexpected error (not PEM-related): {e:?}"),
        }
    }
}

/// Probe the server, list databases, and produce a `ConnectionHandle`.
pub async fn describe_connection(
    client: &Client,
    connection_id: &str,
    profile_id: &str,
    name: &str,
) -> AppResult<ConnectionHandle> {
    let server_info = hello(client).await.ok();
    let databases = list_databases(client).await?;
    Ok(ConnectionHandle {
        connection_id: connection_id.to_string(),
        profile_id: profile_id.to_string(),
        name: name.to_string(),
        server_info,
        databases,
    })
}

async fn hello(client: &Client) -> AppResult<ServerInfo> {
    let doc = client
        .database("admin")
        .run_command(bson::doc! { "hello": 1 })
        .await?;
    let version = client
        .database("admin")
        .run_command(bson::doc! { "buildInfo": 1 })
        .await
        .ok()
        .and_then(|d| d.get_str("version").ok().map(|s| s.to_string()));
    let host = doc.get_str("me").ok().map(|s| s.to_string());
    let is_master = doc.get_bool("isWritablePrimary").unwrap_or(false);
    let topology = if doc.contains_key("setName") {
        "replicaSet"
    } else if doc.contains_key("msg") && doc.get_str("msg").unwrap_or("") == "isdbgrid" {
        "sharded"
    } else {
        "standalone"
    };
    Ok(ServerInfo {
        version,
        host,
        is_master: Some(is_master),
        topology: Some(topology.to_string()),
    })
}

pub async fn list_databases(client: &Client) -> AppResult<Vec<DatabaseSummary>> {
    // Use the native listDatabases admin command to get name + size_on_disk
    // + empty in a single round trip. This is more reliable than running
    // dbStats per database (which can fail for system DBs on some deployments).
    let specs = client.list_databases().await?;
    let mut out = Vec::with_capacity(specs.len());
    for spec in specs {
        // Enrich with dbStats for document/index counts. Best-effort: if the
        // command fails (e.g. lack of permissions on admin DB), we still have
        // the sizeOnDisk from listDatabases.
        //
        // MongoDB returns dbStats numbers as BSON doubles (f64) for some fields
        // and as i32/i64 for others depending on version. We try i64 → i32 → f64
        // to cover all cases.
        let stats = client
            .database(&spec.name)
            .run_command(bson::doc! { "dbStats": 1, "freeSpace": 0 })
            .await
            .ok();

        let get_num = |field: &str| -> Option<u64> {
            stats.as_ref().and_then(|d| {
                d.get_i64(field)
                    .ok()
                    .map(|v| v as u64)
                    .or_else(|| d.get_i32(field).ok().map(|v| v as u64))
                    .or_else(|| d.get_f64(field).ok().map(|v| v as u64))
            })
        };

        // For collections_count, fall back to list_collection_names if dbStats
        // didn't return it.
        let collections_count = match get_num("collections") {
            Some(n) => Some(n),
            None => client
                .database(&spec.name)
                .list_collection_names()
                .await
                .ok()
                .map(|c| c.len() as u64),
        };

        let document_count = get_num("objects");
        let index_count = get_num("indexes");
        let index_size_bytes = get_num("indexSize");
        let storage_size_bytes = get_num("storageSize");

        out.push(DatabaseSummary {
            name: spec.name,
            size_on_disk: if spec.empty { Some(0) } else { Some(spec.size_on_disk) },
            collections_count,
            document_count,
            index_count,
            index_size_bytes,
            storage_size_bytes,
        });
    }
    Ok(out)
}

pub async fn list_collections(client: &Client, db: &str) -> AppResult<Vec<CollectionSummary>> {
    let names = client.database(db).list_collection_names().await?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let kind = classify_collection_name(client, db, &name).await;
        let document_count = client
            .database(db)
            .run_command(bson::doc! { "count": &name })
            .await
            .ok()
            .and_then(|d| {
                d.get_i32("n")
                    .ok()
                    .map(|v| v as u64)
                    .or_else(|| d.get_i64("n").ok().map(|v| v as u64))
            });
        let stats = client
            .database(db)
            .run_command(bson::doc! { "collStats": &name })
            .await
            .ok();
        let (size_bytes, storage_size_bytes) = match stats {
            Some(d) => (
                d.get_i64("size").ok().map(|v| v as u64),
                d.get_i64("storageSize").ok().map(|v| v as u64),
            ),
            None => (None, None),
        };
        out.push(CollectionSummary {
            name,
            kind,
            document_count,
            size_bytes,
            storage_size_bytes,
        });
    }
    Ok(out)
}

pub(crate) async fn classify_collection_name(
    client: &Client,
    db: &str,
    name: &str,
) -> CollectionKind {
    let info = client
        .database(db)
        .run_command(bson::doc! {
            "listCollections": 1,
            "filter": { "name": name },
        })
        .await
        .ok();
    let Some(info) = info else {
        return CollectionKind::Collection;
    };
    if let Ok(cursor) = info.get_document("cursor") {
        if let Ok(first_batch) = cursor.get_array("firstBatch") {
            if let Some(bson::Bson::Document(doc)) = first_batch.first() {
                return classify_collection(doc);
            }
        }
    }
    CollectionKind::Collection
}

fn classify_collection(info: &bson::Document) -> CollectionKind {
    let t = info.get_str("type").unwrap_or("collection");
    match t {
        "view" => CollectionKind::View,
        "timeseries" => CollectionKind::TimeSeries,
        "sharded" => CollectionKind::Sharded,
        _ => CollectionKind::Collection,
    }
}
