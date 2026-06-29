import { describe, it, expect, beforeEach } from "vitest";
import { extractLookupSignals } from "./lookupSignals";

const CONN = "c1";
const DB = "shop";

function seedAggregateHistory(collection: string, pipelines: string[]) {
  const key = `query-history::${CONN}::${DB}.${collection}::aggregate`;
  const entries = pipelines.map((text, i) => ({
    ts: i,
    text,
    durationMs: null,
    docCount: null,
    errored: false,
  }));
  window.localStorage.setItem(key, JSON.stringify(entries));
}

const LOOKUP_PIPELINE = JSON.stringify([
  { $match: { active: true } },
  {
    $lookup: {
      from: "customers",
      localField: "customerId",
      foreignField: "_id",
      as: "customer",
    },
  },
]);

describe("extractLookupSignals", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("extracts $lookup from a normal (dotless) collection — regression", () => {
    // Previously the lastIndexOf('.') guard dropped every dotless collection,
    // making this signal return [] for typical collections like "orders".
    seedAggregateHistory("orders", [LOOKUP_PIPELINE]);
    const signals = extractLookupSignals(CONN, DB);
    expect(signals).toHaveLength(1);
    expect(signals[0]).toMatchObject({
      fromCollection: "orders",
      toCollection: "customers",
      localField: "customerId",
      foreignField: "_id",
      count: 1,
    });
  });

  it("deduplicates identical lookups and increments count", () => {
    seedAggregateHistory("orders", [LOOKUP_PIPELINE, LOOKUP_PIPELINE]);
    const signals = extractLookupSignals(CONN, DB);
    expect(signals).toHaveLength(1);
    expect(signals[0].count).toBe(2);
  });

  it("handles dotted collection names (e.g. system.views)", () => {
    seedAggregateHistory("system.views", [LOOKUP_PIPELINE]);
    const signals = extractLookupSignals(CONN, DB);
    expect(signals).toHaveLength(1);
    expect(signals[0].fromCollection).toBe("system.views");
  });

  it("ignores non-aggregate history keys", () => {
    window.localStorage.setItem(
      `query-history::${CONN}::${DB}.orders::find`,
      JSON.stringify([{ ts: 0, text: LOOKUP_PIPELINE, durationMs: null, docCount: null, errored: false }]),
    );
    expect(extractLookupSignals(CONN, DB)).toHaveLength(0);
  });

  it("skips malformed JSON and pipelines without $lookup.from", () => {
    seedAggregateHistory("orders", [
      "{not json",
      JSON.stringify([{ $lookup: { localField: "x" } }]), // no `from`
      JSON.stringify([{ $match: {} }]),
    ]);
    expect(extractLookupSignals(CONN, DB)).toHaveLength(0);
  });

  it("scopes to the requested connection + database", () => {
    seedAggregateHistory("orders", [LOOKUP_PIPELINE]);
    window.localStorage.setItem(
      `query-history::other::${DB}.orders::aggregate`,
      JSON.stringify([{ ts: 0, text: LOOKUP_PIPELINE, durationMs: null, docCount: null, errored: false }]),
    );
    const signals = extractLookupSignals(CONN, DB);
    expect(signals).toHaveLength(1);
  });
});
