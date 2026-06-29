/** Pure JSON linting for MongoDB query editors.
 *
 *  Detects common mistakes when users type MongoDB shell syntax (unquoted
 *  keys, single-quoted strings, trailing commas) and returns diagnostics
 *  with actionable messages instead of the generic errors from JSON.parse.
 *
 *  The return type is compatible with CodeMirror's `Diagnostic`, but this
 *  module has no CodeMirror dependency — it operates on plain strings.
 */

export interface LintDiagnostic {
  from: number;
  to: number;
  severity: "error" | "warning";
  message: string;
}

/** Unquoted key: after `{` or `,`, optional whitespace/newlines, then a
 *  bare identifier (JS-style, including `$` and `.`) followed by `:`.
 *  Uses a lookbehind so `"already": "quoted"` is never matched. */
const UNQUOTED_KEY = /(?<=[{,])\s*([a-zA-Z_$][a-zA-Z0-9_$.]*)(\s*):/g;

/** Single-quoted string value (not inside a double-quoted string). Naive
 *  pattern — good enough for one-liner JSON objects. */
const SINGLE_QUOTED = /(?<=[:{,[\s])(\s*)'((?:[^'\\]|\\.)*)'/g;

/** Trailing comma before a closing `}` or `]`. */
const TRAILING_COMMA = /,(\s*[}\]])/g;

/** Auto-fix known JSON syntax issues (unquoted keys, single quotes,
 *  trailing commas). Returns `null` if nothing was fixable. */
export function autoFixJson(text: string): string | null {
  if (!text.trim()) return null;
  let fixed = text;

  // 1. Quote bare keys.
  fixed = fixed.replace(UNQUOTED_KEY, (match, key, spacing) => {
    // Preserve the leading whitespace that was part of the match.
    const leading = match.slice(0, match.indexOf(key));
    return `${leading}"${key}"${spacing}:`;
  });

  // 2. Replace single-quoted strings with double-quoted.
  fixed = fixed.replace(SINGLE_QUOTED, (_match, spacing, content) => {
    return `${spacing ?? ""}"${content}"`;
  });

  // 3. Remove trailing commas.
  fixed = fixed.replace(TRAILING_COMMA, "$1");

  return fixed !== text ? fixed : null;
}

/** Lint a JSON text and return diagnostics with user-friendly messages.
 *  Returns an empty array for valid JSON or empty/whitespace-only input. */
export function lintJsonText(text: string): LintDiagnostic[] {
  if (!text.trim()) return [];

  const diagnostics: LintDiagnostic[] = [];

  // Pass 1: unquoted keys — the most common MongoDB shell → JSON mistake.
  let m: RegExpExecArray | null;
  UNQUOTED_KEY.lastIndex = 0;
  while ((m = UNQUOTED_KEY.exec(text)) !== null) {
    const keyStart = m.index + m[0].indexOf(m[1]);
    const keyEnd = keyStart + m[1].length;
    diagnostics.push({
      from: keyStart,
      to: keyEnd,
      severity: "error",
      message: `Unquoted key: ${m[1]} — use "${m[1]}" (JSON requires double-quoted keys)`,
    });
  }

  // Pass 2: single-quoted strings.
  SINGLE_QUOTED.lastIndex = 0;
  while ((m = SINGLE_QUOTED.exec(text)) !== null) {
    const quoteStart = m.index + (m[1]?.length ?? 0);
    const quoteEnd = quoteStart + m[2].length + 2; // include both quotes
    diagnostics.push({
      from: quoteStart,
      to: quoteEnd,
      severity: "error",
      message: `Single-quoted string — use double quotes: "${m[2]}"`,
    });
  }

  // Pass 3: trailing commas.
  TRAILING_COMMA.lastIndex = 0;
  while ((m = TRAILING_COMMA.exec(text)) !== null) {
    diagnostics.push({
      from: m.index,
      to: m.index + 1,
      severity: "error",
      message: "Trailing comma — remove the comma before the closing bracket",
    });
  }

  if (diagnostics.length > 0) return diagnostics;

  // Pass 4: fall back to JSON.parse for any remaining syntax errors.
  try {
    JSON.parse(text);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    diagnostics.push({ from: 0, to: text.length, severity: "error", message: msg });
  }
  return diagnostics;
}
