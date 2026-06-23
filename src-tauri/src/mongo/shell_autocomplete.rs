//! Pure autocomplete logic for the IntelliShell.
//!
//! The IPC command (`commands::shell::shell_autocomplete`) feeds
//! the script text + cursor position into [`autocomplete_context`]
//! to determine what kind of suggestions to fetch (collections,
//! methods, or fields), then calls into the live Mongo connection
//! to populate the suggestion list. The pure logic here is
//! unit-tested without a connection; the I/O lives in the command.

use serde::{Deserialize, Serialize};

/// The kind of completion the shell needs at the current cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompletionKind {
    /// After `db.` — suggest collection names in the active db.
    Collections,
    /// After `db.<coll>.` — suggest collection method names.
    Methods { collection: String },
    /// Inside a method call's argument list, after a `{` or `,`
    /// — suggest field names from the collection's schema.
    Fields { collection: String },
    /// Inside a method call's argument object, when the partial
    /// token starts with `$` — suggest MongoDB operator names
    /// (query or update operators, depending on the method).
    Operators { method: String },
    /// After `use ` — suggest database names.
    Databases,
    /// Typing a bare identifier at statement start — suggest
    /// global utility functions (`print`, `printjson`, `ObjectId`,
    /// `ISODate`, `help`, `db`).
    Globals,
    /// No completions available at this cursor position.
    None,
}

/// One completion suggestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionItem {
    pub label: String,
    /// Short human-readable description (e.g. "collection",
    /// "method", "field").
    pub detail: String,
}

/// The full autocomplete response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutocompleteResponse {
    pub kind: CompletionKind,
    pub items: Vec<CompletionItem>,
}

/// The canonical list of collection methods the shell supports.
/// Kept in sync with `shell::dispatch_sync`. Used for method-name
/// completions and as the source of truth for the help text test.
pub const COLLECTION_METHODS: &[&str] = &[
    "find",
    "findOne",
    "countDocuments",
    "count",
    "aggregate",
    "distinct",
    "insertOne",
    "insertMany",
    "updateOne",
    "updateMany",
    "replaceOne",
    "deleteOne",
    "deleteMany",
    "createIndex",
    "dropIndex",
    "rename",
    "renameCollection",
    "drop",
    "dropDatabase",
    "findOneAndUpdate",
    "findOneAndDelete",
    "findOneAndReplace",
    "bulkWrite",
    "help",
];

/// Common MongoDB query operators, suggested inside filter documents
/// for read/delete/count methods. Sorted for stable, readable dropdowns.
pub const QUERY_OPERATORS: &[&str] = &[
    "$and",
    "$or",
    "$nor",
    "$not",
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$exists",
    "$type",
    "$regex",
    "$mod",
    "$size",
    "$all",
    "$elemMatch",
    "$text",
    "$where",
    "$expr",
    "$jsonSchema",
];

/// Common MongoDB update operators, suggested inside update documents
/// for updateOne/updateMany/findOneAndUpdate update ops.
pub const UPDATE_OPERATORS: &[&str] = &[
    "$set",
    "$unset",
    "$inc",
    "$dec",
    "$mul",
    "$rename",
    "$min",
    "$max",
    "$currentDate",
    "$push",
    "$pop",
    "$pull",
    "$pullAll",
    "$addToSet",
    "$setOnInsert",
];

/// The canonical list of global utility functions the shell
/// registers via `install_host`. Kept in sync with
/// `shell::install_host`. Used for global-name completions when
/// the user is typing a bare identifier at statement start.
pub const GLOBAL_FUNCTIONS: &[&str] = &[
    "print",
    "printjson",
    "ObjectId",
    "ISODate",
    "help",
    "db",
];

/// Parse the text before the cursor and determine what kind of
/// completions to offer. This is the pure, testable core; the
/// command layer fetches the actual names from Mongo.
///
/// Recognized contexts:
///   - `use <partial>` → Databases
///   - `db.<partial>` → Collections
///   - `db.<coll>.<partial>` → Methods
///   - `db.<coll>.<method>(... { <partial>` → Fields
///   - `<partial>` at statement start → Globals
///
/// The "Fields" context is approximate: we detect an open `(` on
/// the current statement and a `{` after it, then suggest field
/// names. This covers the common `find({ <cursor> })` and
/// `updateOne({ <cursor> }, ...)` forms.
pub fn autocomplete_context(text_before_cursor: &str) -> CompletionKind {
    // 1. `use <partial>` at the start of a line / script.
    if let Some(kind) = detect_use_context(text_before_cursor) {
        return kind;
    }

    // 2. Find the last `db.` token and see what follows.
    if let Some(kind) = detect_db_context(text_before_cursor) {
        return kind;
    }

    // 3. Bare identifier at statement start → global functions.
    if let Some(kind) = detect_globals_context(text_before_cursor) {
        return kind;
    }

    CompletionKind::None
}

/// Detect `use <partial>` (database completion).
fn detect_use_context(text: &str) -> Option<CompletionKind> {
    // Walk backwards from the end to find the start of the
    // current line.
    let line_start = text.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line = &text[line_start..];
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("use ") {
        // The partial is whatever follows `use ` on this line,
        // up to the cursor. It must not contain a `;` or newline
        // (those would close the statement).
        let partial = rest.trim();
        if !partial.contains(';') && !partial.contains('\n') {
            // Only offer database completions if the partial
            // looks like an identifier prefix (or is empty).
            let ok = partial.is_empty()
                || partial
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
            if ok {
                return Some(CompletionKind::Databases);
            }
        }
    }
    None
}

/// Detect `db.<partial>` (collections), `db.<coll>.<partial>`
/// (methods), or `db.<coll>.<method>(... { <partial>` (fields).
fn detect_db_context(text: &str) -> Option<CompletionKind> {
    // Find the last occurrence of `db.` in the text.
    let db_pos = text.rfind("db.")?;
    let after_db = &text[db_pos + 3..];

    // Split into the collection part and the rest.
    // The collection name is `[A-Za-z_][A-Za-z0-9_]*`.
    let coll_end = after_db
        .char_indices()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let collection = &after_db[..coll_end];

    if collection.is_empty() {
        // `db.` with nothing after → collections.
        // But check we're not inside a string or comment.
        if !is_inside_string_or_comment(text, db_pos) {
            return Some(CompletionKind::Collections);
        }
        return None;
    }

    let after_coll = &after_db[coll_end..];

    // If there's no `.` after the collection, we're still typing
    // the collection name → Collections.
    if !after_coll.starts_with('.') {
        // Could be `db.users` with cursor right after — still
        // completing the collection name.
        if !is_inside_string_or_comment(text, db_pos) {
            return Some(CompletionKind::Collections);
        }
        return None;
    }

    // After the `.`: method name or method call.
    let after_dot = &after_coll[1..];

    // Check if we're inside a method call's argument list.
    // Find the first `(` after the method name.
    if let Some(paren_pos) = after_dot.find('(') {
        // The method name is everything before the `(`.
        let method_name = after_dot[..paren_pos]
            .trim()
            .trim_end_matches('.');
        // If the method name is empty, we're still completing it.
        if method_name.is_empty() {
            return Some(CompletionKind::Methods {
                collection: collection.to_string(),
            });
        }
        // We're inside the argument list. Check if there's an
        // open `{` after the `(` (and no closing `}` after it
        // before the cursor) → field-name context.
        let after_paren = &after_dot[paren_pos + 1..];
        // First check: are the parentheses already closed? If
        // the `)` that matches the `(` has been typed, the
        // `db.X.Y(...)` statement is finished and we should let
        // globals detection run instead of shadowing it.
        if !is_in_paren_context(after_paren) {
            return None;
        }
        if is_in_field_context(after_paren) {
            // If the partial token being typed starts with `$`,
            // the user is typing a MongoDB operator name (e.g.
            // `$gt`, `$set`) rather than a field name — offer
            // operator completions instead of field names.
            let partial = partial_token(text);
            if partial.starts_with('$') {
                return Some(CompletionKind::Operators {
                    method: method_name.to_string(),
                });
            }
            return Some(CompletionKind::Fields {
                collection: collection.to_string(),
            });
        }
        // Inside the call but not in a field context → no
        // completions (we don't autocomplete operator names or
        // values yet).
        return Some(CompletionKind::None);
    }

    // No `(` after the method name → we're still typing the
    // method name → Methods.
    // But verify the text after the dot looks like an identifier
    // prefix (no spaces, no operators).
    let method_partial = after_dot;
    let looks_like_ident = method_partial.is_empty()
        || method_partial
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_');
    if looks_like_ident && !is_inside_string_or_comment(text, db_pos) {
        return Some(CompletionKind::Methods {
            collection: collection.to_string(),
        });
    }

    None
}

/// Detect a bare identifier at statement start → global functions.
///
/// We look at the text before the cursor and check:
/// 1. It ends with an identifier-like partial (or is empty).
/// 2. The partial is not preceded by a `.` (that would be a
///    method or property access, handled by `detect_db_context`).
/// 3. The partial is not inside a string or comment.
/// 4. The partial is at a statement boundary — the character
///    before the partial (if any) is a statement separator
///    (`;`, `{`, `}`, newline, or start of text) or an operator
///    (`=`, `(`, `,`, ` `). This prevents triggering in the
///    middle of a dotted path like `foo.bar.print`.
fn detect_globals_context(text: &str) -> Option<CompletionKind> {
    // Extract the partial token at the end.
    let partial = partial_token(text);
    // If the partial is empty, we need to check what's before
    // the cursor to see if we're at a position where a global
    // name would be valid. If there's a partial, it must look
    // like an identifier prefix.
    if !partial.is_empty()
        && !partial
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
    {
        return None;
    }

    // Find where the partial starts in the text.
    let partial_start = text.len() - partial.len();

    // Check what's immediately before the partial.
    // If there's a `.` right before, this is a property access,
    // not a global — skip (handled by detect_db_context).
    if partial_start > 0 {
        let before = &text[..partial_start];
        let last_char = before.chars().last()?;
        // A `.` means property access — not globals.
        if last_char == '.' {
            return None;
        }
        // If we're inside a string or comment, skip.
        if is_inside_string_or_comment(text, partial_start) {
            return None;
        }
        // Only offer globals at statement-like boundaries.
        // Allow: start of text, `;`, `{`, `}`, `(`, `,`, `=`,
        // whitespace, newline. Disallow: alphanumeric or `_`
        // (would be inside another identifier).
        let ok_boundary = matches!(
            last_char,
            ';' | '{' | '}' | '(' | ',' | '=' | ' ' | '\t' | '\n' | '\r' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '!' | '<' | '>' | '?' | ':' | '~' | '^'
        );
        if !ok_boundary {
            return None;
        }
    }

    Some(CompletionKind::Globals)
}

/// Heuristic: are we inside a `{ ... }` that's an argument to a
/// method call? We look at the text after the opening `(` and
/// check for an unclosed `{`.
fn is_in_field_context(text_after_paren: &str) -> bool {
    // Track brace depth, ignoring braces inside strings.
    let mut depth: i32 = 0;
    let mut in_string: Option<char> = None;
    let mut prev = '\0';
    for c in text_after_paren.chars() {
        if let Some(quote) = in_string {
            if c == quote && prev != '\\' {
                in_string = None;
            }
        } else if c == '"' || c == '\'' {
            in_string = Some(c);
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth < 0 {
                return false;
            }
        }
        prev = c;
    }
    // If we end with depth > 0, we're inside an unclosed `{`.
    depth > 0
}

/// Heuristic: are we inside an unclosed `(...)` ? Used to check
/// whether the `db.X.Y(...)` call is still open (cursor inside
/// the argument list) or already closed (cursor past the `)`).
fn is_in_paren_context(text_after_open_paren: &str) -> bool {
    let mut depth: i32 = 1; // we start after the opening `(`
    let mut in_string: Option<char> = None;
    let mut prev = '\0';
    for c in text_after_open_paren.chars() {
        if let Some(quote) = in_string {
            if c == quote && prev != '\\' {
                in_string = None;
            }
        } else if c == '"' || c == '\'' {
            in_string = Some(c);
        } else if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                // Parentheses are closed — we're past the call.
                return false;
            }
        }
        prev = c;
    }
    // If depth > 0 at the end, the `(` is still unclosed.
    depth > 0
}

/// Heuristic: is the position `pos` inside a string literal or
/// line comment? We scan from the start of the text to `pos`.
fn is_inside_string_or_comment(text: &str, pos: usize) -> bool {
    let mut in_string: Option<char> = None;
    let mut prev = '\0';
    for (i, c) in text.char_indices() {
        if i >= pos {
            break;
        }
        if let Some(quote) = in_string {
            if c == quote && prev != '\\' {
                in_string = None;
            }
        } else if c == '"' || c == '\'' {
            in_string = Some(c);
        } else if c == '/' && prev == '/' {
            // Line comment — everything until newline is inert.
            // Skip to end of line.
            if let Some(nl) = text[i..].find('\n') {
                // We can't easily skip in this iterator; just
                // mark that we're in a comment and clear it on
                // the next newline. For simplicity, return false
                // here since we'd need more state. The common
                // case (cursor inside a string) is handled.
                let _ = nl;
            }
        }
        prev = c;
    }
    in_string.is_some()
}

/// Filter and rank a list of candidate labels by a prefix.
/// Case-insensitive prefix match; exact prefix matches sorted
/// first, then substring matches.
pub fn filter_by_prefix<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    prefix: &str,
) -> Vec<CompletionItem> {
    let prefix_lower = prefix.to_lowercase();
    let mut exact: Vec<&str> = Vec::new();
    let mut substring: Vec<&str> = Vec::new();
    for c in candidates {
        if c.to_lowercase().starts_with(&prefix_lower) {
            exact.push(c);
        } else if c.to_lowercase().contains(&prefix_lower) {
            substring.push(c);
        }
    }
    // Case-insensitive sort so "find" comes before "FindOne"
    // instead of "FIND" coming before "find".
    exact.sort_by_key(|a| a.to_lowercase());
    substring.sort_by_key(|a| a.to_lowercase());
    exact
        .into_iter()
        .chain(substring)
        .map(|label| CompletionItem {
            label: label.to_string(),
            detail: String::new(),
        })
        .collect()
}

/// Extract the partial token being typed at the end of
/// `text_before_cursor`. For `db.us`, this returns `us`. For
/// `db.users.findO`, returns `findO`. For `db.users.find(`,
/// returns `` (empty). For `db.users.find({ $gt`, returns `$gt`
/// (the `$` is included so operator-name completions can match).
pub fn partial_token(text_before_cursor: &str) -> String {
    // Walk backwards from the end, collecting identifier chars.
    // `$` is included so MongoDB operator prefixes (`$gt`, `$set`)
    // are captured for operator-context detection and filtering.
    let mut chars: Vec<char> = Vec::new();
    for c in text_before_cursor.chars().rev() {
        if c.is_alphanumeric() || c == '_' || c == '$' {
            chars.push(c);
        } else {
            break;
        }
    }
    chars.reverse();
    chars.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_after_db_dot_is_collections() {
        assert_eq!(
            autocomplete_context("db."),
            CompletionKind::Collections
        );
    }

    #[test]
    fn context_after_db_partial_collection_is_collections() {
        assert_eq!(
            autocomplete_context("db.us"),
            CompletionKind::Collections
        );
    }

    #[test]
    fn context_after_db_coll_dot_is_methods() {
        assert_eq!(
            autocomplete_context("db.users."),
            CompletionKind::Methods {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn context_after_db_coll_partial_method_is_methods() {
        assert_eq!(
            autocomplete_context("db.users.findO"),
            CompletionKind::Methods {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn context_inside_find_filter_is_fields() {
        assert_eq!(
            autocomplete_context("db.users.find({ na"),
            CompletionKind::Fields {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn context_inside_update_filter_is_fields() {
        assert_eq!(
            autocomplete_context("db.users.updateOne({ sta"),
            CompletionKind::Fields {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn context_after_closed_filter_is_none() {
        // The filter `{ }` is closed and there's no open `{`
        // after it — we're between arguments, no field
        // completions.
        assert_eq!(
            autocomplete_context("db.users.find({ name: 'a' }, "),
            CompletionKind::None
        );
    }

    #[test]
    fn context_inside_projection_is_fields() {
        // The projection `{ ` is open — field names are valid
        // here too.
        assert_eq!(
            autocomplete_context("db.users.find({ name: 'a' }, { na"),
            CompletionKind::Fields {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn context_use_directive_is_databases() {
        assert_eq!(
            autocomplete_context("use ad"),
            CompletionKind::Databases
        );
    }

    #[test]
    fn context_use_after_newline_is_databases() {
        assert_eq!(
            autocomplete_context("db.users.find();\nuse "),
            CompletionKind::Databases
        );
    }

    #[test]
    fn context_use_with_semicolon_is_globals() {
        // After `use admin;` the statement is complete. The
        // cursor is at a statement boundary — globals are valid
        // for the next statement.
        assert_eq!(
            autocomplete_context("use admin;"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_plain_text_is_globals() {
        // `var x = 1;` — statement complete, cursor at boundary.
        // Globals are valid for the next statement.
        assert_eq!(
            autocomplete_context("var x = 1;"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_inside_string_is_none() {
        assert_eq!(
            autocomplete_context("var x = \"db.users."),
            CompletionKind::None
        );
    }

    #[test]
    fn partial_token_extracts_identifier_suffix() {
        assert_eq!(partial_token("db.users.findO"), "findO");
        assert_eq!(partial_token("db.us"), "us");
        assert_eq!(partial_token("db.users.find("), "");
        assert_eq!(partial_token("use ad"), "ad");
    }

    #[test]
    fn filter_by_prefix_exact_before_substring() {
        let items = filter_by_prefix(
            ["find", "findOne", "findMany", "aggregate", "count"],
            "find",
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Exact prefix matches first, sorted.
        assert_eq!(labels, vec!["find", "findMany", "findOne"]);
    }

    #[test]
    fn filter_by_prefix_case_insensitive() {
        let items = filter_by_prefix(["FindOne", "find", "FIND"], "find");
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["find", "FIND", "FindOne"]);
    }

    #[test]
    fn filter_by_prefix_empty_returns_all_sorted() {
        let items = filter_by_prefix(["deleteOne", "aggregate", "find"], "");
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["aggregate", "deleteOne", "find"]);
    }

    #[test]
    fn collection_methods_list_includes_all_write_methods() {
        for m in [
            "insertMany",
            "updateOne",
            "updateMany",
            "replaceOne",
            "deleteOne",
            "deleteMany",
            "createIndex",
            "dropIndex",
            "rename",
            "drop",
        ] {
            assert!(
                COLLECTION_METHODS.contains(&m),
                "COLLECTION_METHODS missing: {m}"
            );
        }
    }

    // --- Globals context tests ---

    #[test]
    fn context_bare_print_is_globals() {
        assert_eq!(
            autocomplete_context("print"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_bare_printjson_partial_is_globals() {
        assert_eq!(
            autocomplete_context("printj"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_empty_at_start_is_globals() {
        // Empty text — at statement start, globals are valid.
        assert_eq!(
            autocomplete_context(""),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_after_semicolon_is_globals() {
        assert_eq!(
            autocomplete_context("db.users.find();\nprint"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_after_var_equals_is_globals() {
        // `var x = print` — globals valid after `=`.
        assert_eq!(
            autocomplete_context("var x = print"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_after_open_paren_is_globals() {
        // `printjson(print` — globals valid inside a call arg.
        assert_eq!(
            autocomplete_context("printjson(print"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_dotted_access_is_not_globals() {
        // `foo.bar.print` — the last `.` means property access,
        // not a global. detect_db_context won't match (no `db.`),
        // and detect_globals_context must not match either.
        assert_ne!(
            autocomplete_context("foo.bar.print"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn context_dotted_property_access_is_not_globals() {
        // `foo.print` — the `.` before `print` means property
        // access, not a global. Neither detect_db_context nor
        // detect_globals_context should return Globals.
        assert_ne!(
            autocomplete_context("foo.print"),
            CompletionKind::Globals
        );
    }

    #[test]
    fn global_functions_list_includes_all_host_functions() {
        for f in ["print", "printjson", "ObjectId", "ISODate", "help", "db"] {
            assert!(
                GLOBAL_FUNCTIONS.contains(&f),
                "GLOBAL_FUNCTIONS missing: {f}"
            );
        }
    }

    // --- Operator context tests ---

    #[test]
    fn autocomplete_offers_operators_when_partial_starts_with_dollar() {
        assert_eq!(
            autocomplete_context("db.users.find({ $"),
            CompletionKind::Operators {
                method: "find".to_string()
            }
        );
    }

    #[test]
    fn autocomplete_offers_fields_when_partial_does_not_start_with_dollar() {
        assert_eq!(
            autocomplete_context("db.users.find({ na"),
            CompletionKind::Fields {
                collection: "users".to_string()
            }
        );
    }

    #[test]
    fn autocomplete_operators_for_update_method() {
        assert_eq!(
            autocomplete_context("db.users.updateOne({ $s"),
            CompletionKind::Operators {
                method: "updateOne".to_string()
            }
        );
    }

    #[test]
    fn autocomplete_operators_inside_nested_object() {
        // The `$` is inside a nested `{`, still field/operator
        // context — operators should be offered.
        assert_eq!(
            autocomplete_context("db.users.find({ a: { $"),
            CompletionKind::Operators {
                method: "find".to_string()
            }
        );
    }

    #[test]
    fn query_operator_list_includes_common_ops() {
        for op in ["$gt", "$or", "$elemMatch", "$expr"] {
            assert!(
                QUERY_OPERATORS.contains(&op),
                "QUERY_OPERATORS missing: {op}"
            );
        }
    }

    #[test]
    fn update_operator_list_includes_common_ops() {
        for op in ["$set", "$inc", "$push", "$unset"] {
            assert!(
                UPDATE_OPERATORS.contains(&op),
                "UPDATE_OPERATORS missing: {op}"
            );
        }
    }
}
