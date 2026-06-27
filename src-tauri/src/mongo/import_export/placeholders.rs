//! Path-placeholder resolution for export destinations.
//!
//! Path-template placeholders let a user declare a target path template
//! (`backups/${db}/${collection}_${date}.json`) that the backend resolves at
//! export time against the live connection context. Resolution happens *before*
//! path-safety validation, so an invalid expanded path is still rejected by
//! [`crate::mongo::import_export::io_util::validate_target_path`].
//!
//! Only a small, fixed token set is supported, and unknown tokens are left
//! intact (rather than silently dropped) so the user sees the literal in the
//! resulting filename and can fix the template. Tokens are case-sensitive and
//! matched as `${name}` only — no `${name:format}` syntax, to keep parsing
//! trivial and predictable.

use chrono::Utc;

/// Context required to resolve a path template.
#[derive(Debug, Clone)]
pub struct PlaceholderContext<'a> {
    pub database: &'a str,
    pub collection: &'a str,
    /// Profile display name (e.g. "Local RS0"). Falls back to the empty string
    /// when lookup failed, in which case `${profile}` resolves to "".
    pub profile: &'a str,
}

/// Resolve every supported token in `path`.
///
/// Supported tokens:
/// - `${date}`     -> `YYYY-MM-DD` (UTC, lexicographically sortable)
/// - `${time}`     -> `HHmmss`     (UTC, no separators so it's filename-safe)
/// - `${db}`       -> database name
/// - `${collection}` -> collection name
/// - `${profile}`  -> profile display name (sanitized)
///
/// Unknown `${...}` tokens are left untouched so the user can spot the typo.
pub fn resolve_path(path: &str, ctx: &PlaceholderContext<'_>) -> String {
    let now = Utc::now();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H%M%S").to_string();

    resolve_with_values(path, ctx, &date, &time)
}

/// Same as [`resolve_path`] but with injected date/time strings — extracted so
/// unit tests are deterministic and the live clock is never read.
fn resolve_with_values(
    path: &str,
    ctx: &PlaceholderContext<'_>,
    date: &str,
    time: &str,
) -> String {
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Find the matching close brace.
            if let Some(end_rel) = path[i + 2..].find('}') {
                let token = &path[i + 2..i + 2 + end_rel];
                let replacement = lookup_token(token, ctx, date, time);
                match replacement {
                    Some(value) => {
                        out.push_str(&value);
                        i = i + 2 + end_rel + 1; // skip `${...}`
                        continue;
                    }
                    None => {
                        // Unknown token: copy the literal `${token}` through.
                        out.push('$');
                        out.push('{');
                        out.push_str(token);
                        out.push('}');
                        i = i + 2 + end_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn lookup_token(
    token: &str,
    ctx: &PlaceholderContext<'_>,
    date: &str,
    time: &str,
) -> Option<String> {
    match token {
        "date" => Some(date.to_string()),
        "time" => Some(time.to_string()),
        "db" => Some(sanitize_filename(ctx.database)),
        "collection" => Some(sanitize_filename(ctx.collection)),
        "profile" => Some(sanitize_filename(ctx.profile)),
        _ => None,
    }
}

/// Replace characters that are illegal in filenames on common platforms with
/// `_`. Keeps the result readable: only path separators and control chars are
/// rewritten, so `My DB` stays `My DB`. Empty input becomes `untitled` so a
/// missing profile never produces an empty path segment.
fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "untitled".to_string();
    }
    trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> PlaceholderContext<'static> {
        PlaceholderContext {
            database: "shop",
            collection: "orders",
            profile: "Local RS0",
        }
    }

    #[test]
    fn resolves_all_known_tokens() {
        let out = resolve_with_values(
            "/home/user/backups/${db}/${collection}_${profile}_${date}_${time}.json",
            &ctx(),
            "2026-06-27",
            "143052",
        );
        assert_eq!(
            out,
            "/home/user/backups/shop/orders_Local RS0_2026-06-27_143052.json"
        );
    }

    #[test]
    fn leaves_unknown_tokens_intact() {
        let out =
            resolve_with_values("${db}/${unknown}/${date}.json", &ctx(), "2026-06-27", "143052");
        assert_eq!(out, "shop/${unknown}/2026-06-27.json");
    }

    #[test]
    fn empty_profile_becomes_untitled() {
        let ctx = PlaceholderContext {
            database: "shop",
            collection: "orders",
            profile: "",
        };
        let out = resolve_with_values("${profile}.json", &ctx, "2026-06-27", "143052");
        assert_eq!(out, "untitled.json");
    }

    #[test]
    fn sanitizes_path_separators_in_profile() {
        let ctx = PlaceholderContext {
            database: "shop",
            collection: "orders",
            profile: "../etc/passwd",
        };
        let out = resolve_with_values("${profile}.json", &ctx, "2026-06-27", "143052");
        assert_eq!(out, ".._etc_passwd.json");
    }

    #[test]
    fn no_tokens_returns_input_unchanged() {
        let out =
            resolve_with_values("/home/user/orders.json", &ctx(), "2026-06-27", "143052");
        assert_eq!(out, "/home/user/orders.json");
    }

    #[test]
    fn unterminated_brace_is_copied_literally() {
        let out = resolve_with_values("${db}/${unterminated", &ctx(), "2026-06-27", "143052");
        assert_eq!(out, "shop/${unterminated");
    }

    #[test]
    fn adjacent_tokens_resolve_without_separator() {
        let out = resolve_with_values("${db}${collection}.json", &ctx(), "2026-06-27", "143052");
        assert_eq!(out, "shoporders.json");
    }

    #[test]
    fn dollar_sign_without_brace_is_literal() {
        let out = resolve_with_values("price_$5_${db}.json", &ctx(), "2026-06-27", "143052");
        assert_eq!(out, "price_$5_shop.json");
    }
}
