/**
 * Types and labels for the driver code panel. Mirrors the Rust
 * `Language` enum in `src-tauri/src/mongo/query_code.rs`. The
 * kebab-case serialisation matches the Rust serde rename so the
 * string round-trips through the IPC layer unchanged.
 *
 * `SqlLanguage` is kept around for the SQL Query Code panel which
 * is a separate feature; both panels live in the same vocabulary
 * so the dropdown can be shared in future refactors.
 */

export type Language = "node-js" | "python" | "java" | "c-sharp" | "ruby" | "shell";

export type SqlLanguage = Language;

export function languageLabel(lang: Language): string {
  switch (lang) {
    case "node-js":
      return "JavaScript (Node.js)";
    case "python":
      return "Python";
    case "java":
      return "Java";
    case "c-sharp":
      return "C#";
    case "ruby":
      return "Ruby";
    case "shell":
      return "mongo shell";
  }
}
