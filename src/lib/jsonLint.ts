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

/** Replace the *contents* of every double-quoted string with spaces, keeping
 *  the surrounding quotes and the overall length (and therefore every index)
 *  intact. Scanning this masked copy means structural regexes never see commas,
 *  colons, or apostrophes that live *inside* string values, which would
 *  otherwise produce false "unquoted key" / "single-quoted string" hits on
 *  perfectly valid JSON like `{ "note": "hello, world: foo" }`. */
function maskStringContents(text: string): string {
  const chars = text.split("");
  let inString = false;
  let escaped = false;
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    if (inString) {
      if (escaped) {
        chars[i] = " ";
        escaped = false;
      } else if (c === "\\") {
        chars[i] = " ";
        escaped = true;
      } else if (c === '"') {
        inString = false; // keep the closing quote
      } else {
        chars[i] = " ";
      }
    } else if (c === '"') {
      inString = true; // keep the opening quote
    }
  }
  return chars.join("");
}

/** Auto-fix known JSON syntax issues (unquoted keys, single quotes,
 *  trailing commas). Returns `null` if nothing was fixable. */
export function autoFixJson(text: string): string | null {
  if (!text.trim()) return null;
  // Locate every fix on a string-masked copy so we never rewrite content that
  // lives inside a double-quoted string, then apply the edits to the original.
  const masked = maskStringContents(text);
  const edits: { from: number; to: number; insert: string }[] = [];
  let m: RegExpExecArray | null;

  // 1. Quote bare keys.
  UNQUOTED_KEY.lastIndex = 0;
  while ((m = UNQUOTED_KEY.exec(masked)) !== null) {
    const keyStart = m.index + m[0].indexOf(m[1]);
    edits.push({ from: keyStart, to: keyStart + m[1].length, insert: `"${m[1]}"` });
  }

  // 2. Replace single-quoted strings with double-quoted.
  SINGLE_QUOTED.lastIndex = 0;
  while ((m = SINGLE_QUOTED.exec(masked)) !== null) {
    const quoteStart = m.index + (m[1]?.length ?? 0);
    const quoteEnd = quoteStart + m[2].length + 2;
    const content = text.slice(quoteStart + 1, quoteEnd - 1);
    edits.push({ from: quoteStart, to: quoteEnd, insert: `"${content}"` });
  }

  // 3. Remove trailing commas.
  TRAILING_COMMA.lastIndex = 0;
  while ((m = TRAILING_COMMA.exec(masked)) !== null) {
    edits.push({ from: m.index, to: m.index + 1, insert: "" });
  }

  if (edits.length === 0) return null;
  edits.sort((a, b) => a.from - b.from);
  let out = "";
  let cursor = 0;
  for (const e of edits) {
    if (e.from < cursor) continue; // skip any overlapping edit defensively
    out += text.slice(cursor, e.from) + e.insert;
    cursor = e.to;
  }
  out += text.slice(cursor);
  return out !== text ? out : null;
}

/** Lint a JSON text and return diagnostics with user-friendly messages.
 *  Returns an empty array for valid JSON or empty/whitespace-only input. */
export function lintJsonText(text: string): LintDiagnostic[] {
  if (!text.trim()) return [];

  const diagnostics: LintDiagnostic[] = [];
  // Scan a string-masked copy so commas/colons/quotes inside string *values*
  // can't masquerade as structural errors. Indices line up with the original.
  const masked = maskStringContents(text);

  // Pass 1: unquoted keys — the most common MongoDB shell → JSON mistake.
  let m: RegExpExecArray | null;
  UNQUOTED_KEY.lastIndex = 0;
  while ((m = UNQUOTED_KEY.exec(masked)) !== null) {
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
  while ((m = SINGLE_QUOTED.exec(masked)) !== null) {
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
  while ((m = TRAILING_COMMA.exec(masked)) !== null) {
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
