/**
 * Cell value formatting helpers shared between ResultsTable and
 * EditableCell. Pure functions only; no React or IPC imports.
 */

const KIND_CLASS = {
  string: "kind-string",
  int: "kind-int",
  long: "kind-long",
  double: "kind-double",
  decimal: "kind-decimal",
  bool: "kind-bool",
  date: "kind-date",
  objectId: "kind-objectId",
  null: "kind-null",
  array: "kind-array",
  object: "kind-object",
  binary: "kind-binary",
};

export function detectKind(value: unknown): string {
  if (value === null) return "null";
  if (Array.isArray(value)) return "array";
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    if (obj._idDisplay) return "objectId";
    if (obj._dateDisplay) return "date";
    if (obj._decimalDisplay) return "decimal";
    if (obj._binaryDisplay) return "binary";
    if ("$oid" in obj) return "objectId";
    if ("$date" in obj) return "date";
    if ("$numberDecimal" in obj) return "decimal";
    if ("$numberInt" in obj) return "int";
    if ("$numberLong" in obj) return "long";
    return "object";
  }
  if (typeof value === "string") return "string";
  if (typeof value === "number") {
    return Number.isInteger(value) ? "long" : "double";
  }
  if (typeof value === "boolean") return "bool";
  return typeof value;
}

export function kindClassName(kind: string): string {
  return KIND_CLASS[kind as keyof typeof KIND_CLASS] ?? "";
}

export function displayValue(value: unknown): string {
  if (value === null || value === undefined) return "null";
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    if (obj._idDisplay) return String(obj._idDisplay);
    if (obj._dateDisplay) return String(obj._dateDisplay);
    if (obj._decimalDisplay) return String(obj._decimalDisplay);
    if (obj._binaryDisplay) return String(obj._binaryDisplay);
    if (obj.$oid) return String(obj.$oid);
    if (obj.$date) {
      const d = obj.$date;
      if (typeof d === "string") return formatIsoDate(d);
      if (typeof d === "object" && d) {
        const inner = d as Record<string, unknown>;
        const epoch = Number(inner.$numberLong ?? inner.$numberInt ?? NaN);
        if (!Number.isNaN(epoch)) return formatIsoDate(new Date(epoch).toISOString());
        return JSON.stringify(d);
      }
    }
    if (obj.$numberDecimal !== undefined) {
      // Canonical/relaxed Extended JSON encodes `$numberDecimal` as a *string*
      // (`{ "$numberDecimal": "9.99" }`). The old code only handled the
      // non-standard `{ $numberDecimal: { $numberString } }` object form and
      // returned an empty string for the standard string form.
      const dec = obj.$numberDecimal;
      if (typeof dec === "string") return dec;
      if (typeof dec === "object" && dec)
        return String((dec as Record<string, unknown>).$numberString ?? "");
      return String(dec);
    }
    if (obj.$numberInt) return String(obj.$numberInt);
    if (obj.$numberLong) return String(obj.$numberLong);
    return JSON.stringify(value);
  }
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return String(value);
}

/**
 * Convert a row's `_id` value back into MongoDB Extended JSON form so
 * it round-trips through the backend's `parse_filter` as the correct
 * BSON type.
 *
 * `find_documents` serializes results via `doc_to_display_json`, which
 * rewrites `{ "$oid": "hex" }` into `{ "_idDisplay": "hex" }` (and
 * similarly for `$date`, `$numberDecimal`, `$binary`). If we send that
 * display form back as a filter `{ _id: { _idDisplay: "hex" } }`, the
 * backend parses it as a subdocument — which never matches the real
 * ObjectId, so updates/deletes silently affect 0 documents.
 *
 * This helper reconstructs the extended-JSON shape from the display
 * form. Values that are already in extended JSON (or are plain
 * scalars) pass through unchanged.
 */
export function toFilterId(row: Record<string, unknown>): unknown {
  const id = row._id;
  if (id === null || typeof id !== "object" || Array.isArray(id)) {
    return id;
  }
  const obj = id as Record<string, unknown>;
  if (typeof obj._idDisplay === "string") {
    return { $oid: obj._idDisplay };
  }
  if (typeof obj._dateDisplay === "string") {
    const ms = Date.parse(obj._dateDisplay);
    if (!Number.isNaN(ms)) {
      return { $date: { $numberLong: String(ms) } };
    }
  }
  if (obj._decimalDisplay !== undefined) {
    const s = String(obj._decimalDisplay);
    return { $numberDecimal: s };
  }
  if (typeof obj._binaryDisplay === "string") {
    return { $binary: { base64: obj._binaryDisplay, subType: "00" } };
  }
  // Already extended JSON (e.g. `{ $oid: ... }`) or a plain subdocument _id.
  return obj;
}

/**
 * Walk a row by a dotted path and return the value at that path, or
 * `undefined` if any intermediate key is missing.
 */
export function getByPath(row: unknown, path: string): unknown {
  if (!path.includes(".")) return (row as Record<string, unknown> | undefined)?.[path];
  let cursor: unknown = row;
  for (const part of path.split(".")) {
    if (cursor === null || typeof cursor !== "object") return undefined;
    cursor = (cursor as Record<string, unknown>)[part];
  }
  return cursor;
}

/**
 * Convert an ISO 8601 string into a slightly more readable UTC form
 * without the awkward `T` separator: `YYYY-MM-DD HH:mm:ss.SSSZ`.
 */
function formatIsoDate(iso: string): string {
  return iso.replace("T", " ");
}
