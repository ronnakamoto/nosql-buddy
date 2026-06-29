/**
 * Extract `$lookup` relationship signals from the user's query history.
 *
 * Query history lives in browser localStorage (see `features/queryHistory.ts`),
 * keyed per `connectionId::database.collection::mode`. Only `aggregate`-mode
 * entries can contain `$lookup`. This module walks every aggregate history
 * entry for the given connection + database, parses each pipeline, and collects
 * `$lookup` stages into `LookupSignal`s that the backend relationship detector
 * consumes as a high-confidence behavioral signal.
 *
 * The parser is intentionally lenient: malformed JSON or non-array bodies are
 * skipped, and only the classic `$lookup` form (`from`/`localField`/
 * `foreignField`) is recognized. The pipeline-form `$lookup` (which has no
 * local/foreign field) is recorded with empty field names so the backend can
 * still infer a collection-to-collection edge from it.
 */

export interface LookupSignal {
  fromCollection: string;
  toCollection: string;
  localField: string;
  foreignField: string;
  count: number;
}

interface AggregatedLookup {
  key: string;
  fromCollection: string;
  toCollection: string;
  localField: string;
  foreignField: string;
  count: number;
}

/**
 * Collect `$lookup` signals from query history for a connection + database.
 * Returns deduplicated signals keyed by `(fromCollection, toCollection,
 * localField, foreignField)`, with `count` reflecting how many history entries
 * contained that exact lookup.
 */
export function extractLookupSignals(
  connectionId: string,
  database: string,
): LookupSignal[] {
  const prefix = `query-history::${connectionId}::${database}.`;
  const byKey = new Map<string, AggregatedLookup>();

  for (let i = 0; i < window.localStorage.length; i += 1) {
    const k = window.localStorage.key(i);
    if (!k || !k.startsWith(prefix)) continue;
    if (!k.endsWith("::aggregate")) continue;

    // Key shape: query-history::<conn>::<db>.<coll>::aggregate
    // `prefix` already consumes the "<db>." segment, so what remains between
    // it and the "::aggregate" suffix is exactly the collection name (which
    // may itself contain dots, e.g. "system.views"). The previous code ran a
    // `lastIndexOf(".")` guard here that dropped every normal (dotless)
    // collection and truncated dotted ones, making this signal dead.
    const fromCollection = k.slice(prefix.length, -"::aggregate".length);
    if (!fromCollection) continue;

    const raw = window.localStorage.getItem(k);
    if (!raw) continue;
    let entries: unknown;
    try {
      entries = JSON.parse(raw);
    } catch {
      continue;
    }
    if (!Array.isArray(entries)) continue;

    for (const entry of entries) {
      if (!isHistoryEntryLike(entry)) continue;
      const lookups = parseLookups(entry.text);
      for (const lk of lookups) {
        const sigKey = `${fromCollection}\u0001${lk.to}\u0001${lk.localField}\u0001${lk.foreignField}`;
        const existing = byKey.get(sigKey);
        if (existing) {
          existing.count += 1;
        } else {
          byKey.set(sigKey, {
            key: sigKey,
            fromCollection,
            toCollection: lk.to,
            localField: lk.localField,
            foreignField: lk.foreignField,
            count: 1,
          });
        }
      }
    }
  }

  return Array.from(byKey.values()).map(({ key: _key, ...rest }) => {
    void _key;
    return rest;
  });
}

interface ParsedLookup {
  to: string;
  localField: string;
  foreignField: string;
}

/**
 * Parse a single aggregate pipeline text and return every `$lookup` stage's
 * target collection + fields. Returns `[]` for non-JSON or non-array input.
 */
function parseLookups(text: string): ParsedLookup[] {
  let pipeline: unknown;
  try {
    pipeline = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(pipeline)) return [];

  const out: ParsedLookup[] = [];
  for (const stage of pipeline) {
    if (!stage || typeof stage !== "object") continue;
    const lookup = (stage as Record<string, unknown>).$lookup;
    if (!lookup || typeof lookup !== "object") continue;
    const lk = lookup as Record<string, unknown>;
    const to = typeof lk.from === "string" ? lk.from : "";
    if (!to) continue; // a $lookup without `from` is not a cross-collection signal
    out.push({
      to,
      localField: typeof lk.localField === "string" ? lk.localField : "",
      foreignField: typeof lk.foreignField === "string" ? lk.foreignField : "",
    });
  }
  return out;
}

function isHistoryEntryLike(v: unknown): v is { text: string } {
  if (v === null || typeof v !== "object") return false;
  const e = v as Record<string, unknown>;
  return typeof e.text === "string";
}
