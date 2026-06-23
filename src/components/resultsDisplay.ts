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
    if (obj.$numberDecimal)
      return String((obj.$numberDecimal as Record<string, unknown>).$numberString ?? "");
    if (obj.$numberInt) return String(obj.$numberInt);
    if (obj.$numberLong) return String(obj.$numberLong);
    return JSON.stringify(value);
  }
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  return String(value);
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
