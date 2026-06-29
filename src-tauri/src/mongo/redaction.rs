//! Secret redaction for logs, error messages, and IPC payloads.
//!
//! MongoDB driver error messages and connection URI strings may contain
//! credentials. Before any error message leaves the process — to logs,
//! to the frontend, to error reports — strip the sensitive parts.

use regex::Regex;
use std::sync::OnceLock;

/// Apply deterministic, case-insensitive redaction to known sensitive
/// patterns. The redactor is intentionally conservative: when in doubt,
/// it replaces the value with a placeholder.
pub struct Redactor {
    uri_userinfo: OnceLock<Regex>,
    password_kv: OnceLock<Regex>,
    connection_string: OnceLock<Regex>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    pub fn new() -> Self {
        Self {
            uri_userinfo: OnceLock::new(),
            password_kv: OnceLock::new(),
            connection_string: OnceLock::new(),
        }
    }

    fn uri_re(&self) -> &Regex {
        self.uri_userinfo.get_or_init(|| {
            // mongodb://user:password@host, mongodb+srv://user:password@host
            Regex::new(r"(?i)(mongodb(?:\+srv)?://)[^:\s/@]+:[^@\s/]+@").expect("valid regex")
        })
    }

    fn password_kv_re(&self) -> &Regex {
        self.password_kv.get_or_init(|| {
            // password=foo ; PASSWORD="foo bar" ; password: 'multi word'.
            // A quoted value is consumed whole (spaces included) so a
            // multi-word secret can't leak the part after the first space.
            Regex::new(r#"(?i)(password\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^";,\s]+)"#)
                .expect("valid regex")
        })
    }

    fn connection_string_kv_re(&self) -> &Regex {
        self.connection_string.get_or_init(|| {
            // uri=... or connectionString=..., quoted values consumed whole.
            Regex::new(
                r#"(?i)((?:uri|connection\s*string|connectionString)\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^";,\s]+)"#,
            )
            .expect("valid regex")
        })
    }

    pub fn redact(&self, input: &str) -> String {
        let mut out = self.uri_re().replace_all(input, "$1***:***@").to_string();
        out = self
            .password_kv_re()
            .replace_all(&out, "${1}***")
            .to_string();
        out = self
            .connection_string_kv_re()
            .replace_all(&out, "${1}***")
            .to_string();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_uri_userinfo() {
        let r = Redactor::new();
        let out = r.redact("ServerSelectionError: mongodb://alice:secret@127.0.0.1:27017");
        assert!(out.contains("***:***@"));
        assert!(!out.contains("secret"));
    }

    #[test]
    fn redacts_password_kv() {
        let r = Redactor::new();
        let out = r.redact("auth failed, password=supersecret retry");
        assert!(out.contains("password=***"));
        assert!(!out.contains("supersecret"));
    }

    #[test]
    fn leaves_benign_text_alone() {
        let r = Redactor::new();
        let out = r.redact("connection refused on port 27017");
        assert_eq!(out, "connection refused on port 27017");
    }

    #[test]
    fn redacts_multi_word_quoted_password() {
        // Regression: the old `[^";,\s]+` value class stopped at the first
        // space, leaking everything after it in a quoted multi-word secret.
        let r = Redactor::new();
        let out = r.redact(r#"auth failed: password: "super secret value" end"#);
        assert!(!out.contains("super"), "leaked first word: {out}");
        assert!(!out.contains("secret"), "leaked middle word: {out}");
        assert!(!out.contains("value"), "leaked last word: {out}");
        assert!(out.contains("***"));
        assert!(out.contains("end"), "must preserve trailing benign text: {out}");
    }

    #[test]
    fn redacts_single_quoted_password() {
        let r = Redactor::new();
        let out = r.redact("password='a b c'");
        assert!(!out.contains('a') || !out.contains("a b c"));
        assert!(!out.contains("a b c"));
        assert!(out.contains("password=***"));
    }

    #[test]
    fn redacts_srv_uri_userinfo() {
        let r = Redactor::new();
        let out = r.redact("mongodb+srv://alice:hunter2@cluster.example.net/db");
        assert!(!out.contains("hunter2"), "leaked password: {out}");
        assert!(!out.contains("alice"), "leaked user: {out}");
        assert!(out.contains("***:***@"));
    }

    #[test]
    fn redacts_connection_string_kv() {
        let r = Redactor::new();
        let out = r.redact("connectionString=mongodb://u:p@h:27017/db retry=1");
        assert!(!out.contains("u:p@h"), "leaked credentials: {out}");
        assert!(out.contains("connectionString=***"));
    }

    #[test]
    fn redaction_is_idempotent() {
        let r = Redactor::new();
        let once = r.redact("mongodb://u:p@h/db password=secret");
        let twice = r.redact(&once);
        assert_eq!(once, twice, "redacting twice must equal redacting once");
    }

    #[test]
    fn leaves_plain_mongo_uri_without_userinfo_alone() {
        let r = Redactor::new();
        let out = r.redact("mongodb://127.0.0.1:27017/db");
        assert_eq!(out, "mongodb://127.0.0.1:27017/db");
    }
}
