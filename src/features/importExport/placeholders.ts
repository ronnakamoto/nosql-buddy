/**
 * Pure-frontend mirror of the backend path-placeholder resolver
 * (`src-tauri/src/mongo/import_export/placeholders.rs`). Used to show a live
 * preview of the resolved export filename in the wizard *before* the user
 * submits, so they can see `${db}_${date}.json` expand to `shop_2026-06-27.json`
 * without a round-trip. The backend remains authoritative at submit time.
 *
 * Supported tokens (case-sensitive, `${name}` only):
 * - `${date}`       -> `YYYY-MM-DD` (UTC, lexicographically sortable)
 * - `${time}`       -> `HHmmss`     (UTC, no separators, filename-safe)
 * - `${db}`         -> database name
 * - `${collection}` -> collection name
 * - `${profile}`    -> profile display name (sanitized)
 *
 * Unknown `${...}` tokens are left intact so the user sees the literal in the
 * preview and can fix the template.
 */

export interface PlaceholderPreviewContext {
  database: string;
  collection: string;
  /** Profile display name; empty string when unknown. */
  profile: string;
}

/** Resolve every supported token in `path`. Deterministic for a given ctx +
 * injected date/time (extracted so callers/tests can pin the clock). */
export function resolvePathPreview(
  path: string,
  ctx: PlaceholderPreviewContext,
  now: Date = new Date(),
): string {
  const date = formatDate(now);
  const time = formatTime(now);
  return resolveWithValues(path, ctx, date, time);
}

function resolveWithValues(
  path: string,
  ctx: PlaceholderPreviewContext,
  date: string,
  time: string,
): string {
  let out = "";
  let i = 0;
  while (i < path.length) {
    const two = path.slice(i, i + 2);
    if (two === "${") {
      const close = path.indexOf("}", i + 2);
      if (close === -1) {
        // Unterminated: copy the rest literally.
        out += path.slice(i);
        break;
      }
      const token = path.slice(i + 2, close);
      const replacement = lookupToken(token, ctx, date, time);
      if (replacement !== null) {
        out += replacement;
      } else {
        out += `${two}${token}}`;
      }
      i = close + 1;
      continue;
    }
    out += path[i];
    i += 1;
  }
  return out;
}

function lookupToken(
  token: string,
  ctx: PlaceholderPreviewContext,
  date: string,
  time: string,
): string | null {
  switch (token) {
    case "date":
      return date;
    case "time":
      return time;
    case "db":
      return sanitizeFilename(ctx.database);
    case "collection":
      return sanitizeFilename(ctx.collection);
    case "profile":
      return sanitizeFilename(ctx.profile);
    default:
      return null;
  }
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

/** UTC `YYYY-MM-DD` to match the backend's `chrono::Utc::now().format("%Y-%m-%d")`. */
function formatDate(d: Date): string {
  return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(
    d.getUTCDate(),
  )}`;
}

/** UTC `HHmmss` to match the backend's `chrono::Utc::now().format("%H%M%S")`. */
function formatTime(d: Date): string {
  return `${pad2(d.getUTCHours())}${pad2(d.getUTCMinutes())}${pad2(
    d.getUTCSeconds(),
  )}`;
}

function sanitizeFilename(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "untitled";
  let out = "";
  for (const ch of trimmed) {
    if (
      ch === "/" ||
      ch === "\\" ||
      ch === ":" ||
      ch === "*" ||
      ch === "?" ||
      ch === '"' ||
      ch === "<" ||
      ch === ">" ||
      ch === "|"
    ) {
      out += "_";
    } else if (ch.charCodeAt(0) < 0x20) {
      out += "_";
    } else {
      out += ch;
    }
  }
  return out;
}
