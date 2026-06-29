import { describe, it, expect } from "vitest";
import { lintJsonText, autoFixJson } from "./jsonLint";

/** Helper: extract just the messages from diagnostics. */
function messages(text: string): string[] {
  return lintJsonText(text).map((d) => d.message);
}

/** Helper: extract [from, to] ranges. */
function ranges(text: string): [number, number][] {
  return lintJsonText(text).map((d) => [d.from, d.to]);
}

// ─── Valid JSON (should produce zero diagnostics) ─────────────────

describe("valid JSON", () => {
  it("empty string", () => {
    expect(lintJsonText("")).toEqual([]);
  });

  it("whitespace only", () => {
    expect(lintJsonText("   \n  ")).toEqual([]);
  });

  it("empty object", () => {
    expect(lintJsonText("{}")).toEqual([]);
  });

  it("simple object with quoted keys", () => {
    expect(lintJsonText('{ "sku": "WH-1000XM5" }')).toEqual([]);
  });

  it("nested object", () => {
    expect(lintJsonText('{ "$set": { "specs.battery": 32 } }')).toEqual([]);
  });

  it("array value", () => {
    expect(lintJsonText('{ "tags": ["a", "b"] }')).toEqual([]);
  });

  it("complex MongoDB filter", () => {
    expect(
      lintJsonText('{ "$and": [{ "price": { "$gte": 300 } }, { "stock": { "$lt": 10 } }] }'),
    ).toEqual([]);
  });

  it("object with numeric values", () => {
    expect(lintJsonText('{ "count": 42, "ratio": 3.14, "neg": -1 }')).toEqual([]);
  });

  it("null, true, false values", () => {
    expect(lintJsonText('{ "a": null, "b": true, "c": false }')).toEqual([]);
  });
});

// ─── Unquoted keys ────────────────────────────────────────────────

describe("unquoted keys", () => {
  it("single unquoted key", () => {
    const diags = lintJsonText('{ sku: "WH-1000XM5" }');
    expect(diags).toHaveLength(1);
    expect(diags[0].severity).toBe("error");
    expect(diags[0].message).toContain("Unquoted key: sku");
    expect(diags[0].message).toContain('"sku"');
  });

  it("highlights the exact key span", () => {
    const text = '{ sku: "val" }';
    const r = ranges(text);
    expect(r).toHaveLength(1);
    // "sku" starts at index 2, length 3
    expect(r[0]).toEqual([2, 5]);
    expect(text.slice(r[0][0], r[0][1])).toBe("sku");
  });

  it("multiple unquoted keys", () => {
    const diags = lintJsonText('{ name: "x", price: 10 }');
    expect(diags).toHaveLength(2);
    expect(diags[0].message).toContain("name");
    expect(diags[1].message).toContain("price");
  });

  it("MongoDB operators as unquoted keys", () => {
    const diags = lintJsonText('{ $set: { stock: 5 } }');
    expect(diags).toHaveLength(2);
    expect(diags[0].message).toContain("$set");
    expect(diags[1].message).toContain("stock");
  });

  it("nested unquoted keys", () => {
    const diags = lintJsonText('{ $inc: { stock: -5 }, $set: { "specs.battery_life_hrs": 32 } }');
    expect(diags).toHaveLength(3);
    expect(messages(diags.map((d) => d.message).join("|"))).toBeTruthy(); // sanity
    const keys = diags.map((d) => {
      const m = d.message.match(/Unquoted key: (\S+)/);
      return m?.[1];
    });
    expect(keys).toEqual(["$inc", "stock", "$set"]);
  });

  it("dotted unquoted key", () => {
    const diags = lintJsonText('{ specs.battery: 30 }');
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain("specs.battery");
  });

  it("does NOT flag quoted keys", () => {
    expect(lintJsonText('{ "$set": { "name": "x" } }')).toEqual([]);
  });

  it("does NOT flag values that look like identifiers after colon", () => {
    // `true`, `false`, `null` after a colon should not trigger
    expect(lintJsonText('{ "flag": true }')).toEqual([]);
  });
});

// ─── Single-quoted strings ────────────────────────────────────────

describe("single-quoted strings", () => {
  it("detects single-quoted value", () => {
    const diags = lintJsonText("{ \"name\": 'hello' }");
    expect(diags).toHaveLength(1);
    expect(diags[0].severity).toBe("error");
    expect(diags[0].message).toContain("Single-quoted string");
    expect(diags[0].message).toContain('"hello"');
  });

  it("detects single-quoted string in array", () => {
    const diags = lintJsonText("[\"a\", 'b']");
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain('"b"');
  });
});

// ─── Trailing commas ──────────────────────────────────────────────

describe("trailing commas", () => {
  it("trailing comma before }", () => {
    const diags = lintJsonText('{ "a": 1, }');
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain("Trailing comma");
  });

  it("trailing comma before ]", () => {
    const diags = lintJsonText('["a", "b",]');
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain("Trailing comma");
  });

  it("highlights just the comma", () => {
    const text = '{ "a": 1, }';
    const r = ranges(text);
    const commaIdx = text.indexOf(",");
    expect(r[0]).toEqual([commaIdx, commaIdx + 1]);
    expect(text[r[0][0]]).toBe(",");
  });
});

// ─── Combined errors ──────────────────────────────────────────────

describe("combined errors", () => {
  it("unquoted key takes priority over JSON.parse fallback", () => {
    const diags = lintJsonText('{ sku: "val" }');
    // Should get the specific unquoted-key message, not a generic JSON.parse error
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain("Unquoted key");
    // The message should mention the specific key, not be a generic parse error
    expect(diags[0].message).toContain("sku");
    expect(diags[0].message).not.toContain("Expected");
  });

  it("unquoted keys and trailing commas together", () => {
    const diags = lintJsonText('{ name: "x", }');
    expect(diags.length).toBeGreaterThanOrEqual(2);
    const msgs = diags.map((d) => d.message);
    expect(msgs.some((m) => m.includes("Unquoted key"))).toBe(true);
    expect(msgs.some((m) => m.includes("Trailing comma"))).toBe(true);
  });
});

// ─── JSON.parse fallback ──────────────────────────────────────────

describe("JSON.parse fallback", () => {
  it("malformed JSON without identifiable pattern", () => {
    const diags = lintJsonText('{ "a": }');
    expect(diags).toHaveLength(1);
    // Falls through to JSON.parse — message varies by engine but should exist
    expect(diags[0].message.length).toBeGreaterThan(0);
    expect(diags[0].from).toBe(0);
  });

  it("unclosed string", () => {
    const diags = lintJsonText('{ "a": "unclosed }');
    expect(diags).toHaveLength(1);
    expect(diags[0].severity).toBe("error");
  });

  it("completely invalid input", () => {
    const diags = lintJsonText("not json at all");
    expect(diags).toHaveLength(1);
  });
});

// ─── Edge cases ───────────────────────────────────────────────────

describe("edge cases", () => {
  it("colon inside a quoted string value does not trigger false positive", () => {
    expect(lintJsonText('{ "url": "http://example.com" }')).toEqual([]);
  });

  it("empty array", () => {
    expect(lintJsonText("[]")).toEqual([]);
  });

  it("top-level number", () => {
    expect(lintJsonText("42")).toEqual([]);
  });

  it("top-level string", () => {
    expect(lintJsonText('"hello"')).toEqual([]);
  });

  it("multiline object with unquoted keys", () => {
    const text = `{
  name: "test",
  price: 10
}`;
    const diags = lintJsonText(text);
    expect(diags).toHaveLength(2);
    expect(diags[0].message).toContain("name");
    expect(diags[1].message).toContain("price");
    // Verify the highlighted text matches the key
    expect(text.slice(diags[0].from, diags[0].to)).toBe("name");
    expect(text.slice(diags[1].from, diags[1].to)).toBe("price");
  });

  it("key starting with underscore", () => {
    const diags = lintJsonText('{ _id: "abc" }');
    expect(diags).toHaveLength(1);
    expect(diags[0].message).toContain("_id");
  });
});

// ─── autoFixJson ──────────────────────────────────────────────────

describe("autoFixJson", () => {
  it("returns null for valid JSON", () => {
    expect(autoFixJson('{ "a": 1 }')).toBeNull();
  });

  it("returns null for empty string", () => {
    expect(autoFixJson("")).toBeNull();
  });

  it("returns null for whitespace-only", () => {
    expect(autoFixJson("   ")).toBeNull();
  });

  it("quotes a single bare key", () => {
    expect(autoFixJson('{ sku: "WH-1000XM5" }')).toBe('{ "sku": "WH-1000XM5" }');
  });

  it("quotes multiple bare keys", () => {
    expect(autoFixJson('{ name: "x", price: 10 }')).toBe('{ "name": "x", "price": 10 }');
  });

  it("quotes MongoDB operators", () => {
    expect(autoFixJson('{ $set: { stock: 5 } }')).toBe('{ "$set": { "stock": 5 } }');
  });

  it("quotes nested bare keys", () => {
    const input = '{ $inc: { stock: -5 }, $set: { "specs.battery": 32 } }';
    const fixed = autoFixJson(input);
    expect(fixed).toBe('{ "$inc": { "stock": -5 }, "$set": { "specs.battery": 32 } }');
  });

  it("fixes single-quoted strings", () => {
    expect(autoFixJson("{ \"name\": 'hello' }")).toBe('{ "name": "hello" }');
  });

  it("removes trailing commas in objects", () => {
    expect(autoFixJson('{ "a": 1, }')).toBe('{ "a": 1 }');
  });

  it("removes trailing commas in arrays", () => {
    expect(autoFixJson('["a", "b",]')).toBe('["a", "b"]');
  });

  it("fixes all issues at once", () => {
    const input = "{ name: 'hello', }";
    const fixed = autoFixJson(input);
    expect(fixed).toBe('{ "name": "hello" }');
  });

  it("result is valid JSON", () => {
    const cases = [
      '{ sku: "WH-1000XM5" }',
      '{ $inc: { stock: -5 }, $set: { "specs.battery": 32 } }',
      "{ name: 'test', }",
      '{ _id: "abc", tags: ["a", "b",] }',
      '{ $and: [{ price: { $gte: 300 } }, { stock: { $lt: 10 } }] }',
    ];
    for (const input of cases) {
      const fixed = autoFixJson(input);
      if (fixed !== null) {
        expect(() => JSON.parse(fixed), `Failed to parse fixed output for: ${input}`).not.toThrow();
      }
    }
  });

  it("preserves already-quoted keys", () => {
    const input = '{ "$set": { name: "x" } }';
    const fixed = autoFixJson(input);
    expect(fixed).toBe('{ "$set": { "name": "x" } }');
  });

  it("quotes dotted bare keys", () => {
    const fixed = autoFixJson('{ specs.battery: 30 }');
    expect(fixed).toBe('{ "specs.battery": 30 }');
  });

  it("handles multiline input", () => {
    const input = `{
  name: "test",
  price: 10
}`;
    const fixed = autoFixJson(input);
    expect(fixed).not.toBeNull();
    expect(() => JSON.parse(fixed!)).not.toThrow();
    const parsed = JSON.parse(fixed!);
    expect(parsed.name).toBe("test");
    expect(parsed.price).toBe(10);
  });
});
