import { describe, it, expect } from "vitest";
import { toFilterId, detectKind, displayValue, getByPath } from "./resultsDisplay";

describe("resultsDisplay", () => {
  describe("toFilterId", () => {
    it("preserves flat primitives", () => {
      expect(toFilterId({ _id: "abc" })).toBe("abc");
      expect(toFilterId({ _id: 123 })).toBe(123);
    });

    it("reconstructs ObjectId from Extended JSON structure", () => {
      expect(toFilterId({ _id: { $oid: "507f1f77bcf86cd799439011" } })).toEqual({
        $oid: "507f1f77bcf86cd799439011",
      });
    });

    it("reconstructs ObjectId from display object (_idDisplay)", () => {
      // The frontend augments _id with _idDisplay so ObjectIds aren't nested in the grid.
      // toFilterId needs to peel that away.
      expect(
        toFilterId({
          _id: {
            _idDisplay: "507f1f77bcf86cd799439011",
          }
        })
      ).toEqual({
        $oid: "507f1f77bcf86cd799439011",
      });
    });

    it("returns objects unmodified if not an ObjectId structure", () => {
      const obj = { foo: "bar" };
      expect(toFilterId({ _id: obj })).toEqual({ foo: "bar" });
    });
  });

  describe("detectKind", () => {
    it("identifies null and arrays", () => {
      expect(detectKind(null)).toBe("null");
      expect(detectKind([])).toBe("array");
    });

    it("identifies primitives", () => {
      expect(detectKind("abc")).toBe("string");
      expect(detectKind(123)).toBe("long"); // Number.isInteger
      expect(detectKind(123.4)).toBe("double");
      expect(detectKind(true)).toBe("bool");
    });

    it("identifies Extended JSON types", () => {
      expect(detectKind({ $oid: "abc" })).toBe("objectId");
      expect(detectKind({ $date: "2020-01-01T00:00:00Z" })).toBe("date");
      expect(detectKind({ $binary: { base64: "a", subType: "0" } })).toBe("object"); // Handled manually by some display views
      expect(detectKind({ _idDisplay: "id" })).toBe("objectId");
      expect(detectKind({ _dateDisplay: "date" })).toBe("date");
    });

    it("identifies generic Object", () => {
      expect(detectKind({ foo: "bar" })).toBe("object");
    });
  });

  describe("displayValue", () => {
    it("formats dates and object ids", () => {
      expect(displayValue({ $oid: "123" })).toBe("123");
      expect(displayValue({ _idDisplay: "123" })).toBe("123");
    });

    it("JSONifies arrays and objects", () => {
      expect(displayValue([1, 2])).toBe("[1,2]");
      expect(displayValue({ a: 1 })).toBe('{"a":1}');
    });

    it("renders canonical Extended JSON $numberDecimal (string form) — regression", () => {
      // Old code read `.$numberString` off the string and returned "".
      expect(displayValue({ $numberDecimal: "9.99" })).toBe("9.99");
    });

    it("renders non-standard $numberDecimal object form", () => {
      expect(displayValue({ $numberDecimal: { $numberString: "1.50" } })).toBe("1.50");
    });

    it("renders $numberInt and $numberLong", () => {
      expect(displayValue({ $numberInt: "42" })).toBe("42");
      expect(displayValue({ $numberLong: "9007199254740993" })).toBe("9007199254740993");
    });
  });

  describe("toFilterId reconstruction", () => {
    it("reconstructs date from _dateDisplay", () => {
      const out = toFilterId({ _id: { _dateDisplay: "2026-06-29T10:00:00.000Z" } }) as Record<string, unknown>;
      const inner = out.$date as Record<string, unknown>;
      expect(typeof inner.$numberLong).toBe("string");
      expect(Number(inner.$numberLong)).toBe(Date.parse("2026-06-29T10:00:00.000Z"));
    });

    it("reconstructs decimal from _decimalDisplay", () => {
      expect(toFilterId({ _id: { _decimalDisplay: "9.99" } })).toEqual({ $numberDecimal: "9.99" });
    });

    it("reconstructs binary from _binaryDisplay", () => {
      expect(toFilterId({ _id: { _binaryDisplay: "YWJj" } })).toEqual({
        $binary: { base64: "YWJj", subType: "00" },
      });
    });

    it("falls through to the raw object when _dateDisplay is unparseable", () => {
      const raw = { _dateDisplay: "not-a-date" };
      expect(toFilterId({ _id: raw })).toEqual(raw);
    });
  });

  describe("getByPath", () => {
    it("extracts nested values", () => {
      const doc = { a: { b: { c: 42 } } };
      expect(getByPath(doc, "a.b.c")).toBe(42);
    });

    it("returns undefined for missing paths", () => {
      const doc = { a: { b: 42 } };
      expect(getByPath(doc, "a.x")).toBeUndefined();
      expect(getByPath(doc, "y.z")).toBeUndefined();
    });

    it("handles null gracefully", () => {
      const doc = { a: null };
      expect(getByPath(doc, "a.b")).toBeUndefined();
    });
  });
});
