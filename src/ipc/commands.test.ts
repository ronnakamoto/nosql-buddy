import { describe, it, expect } from "vitest";
import { formatError } from "./commands";

describe("formatError", () => {
  it("uses Error.message", () => {
    expect(formatError(new Error("boom"))).toBe("boom");
  });

  it("returns a string as-is", () => {
    expect(formatError("plain error")).toBe("plain error");
  });

  it("prefers message on an object", () => {
    expect(formatError({ message: "from message" })).toBe("from message");
  });

  it("falls back to error field", () => {
    expect(formatError({ error: "from error" })).toBe("from error");
  });

  it("uses message for a Rust { kind, message } payload", () => {
    // `message` is checked before the `{kind, message}` branch, so the bare
    // message is returned (the kind-prefixed branch is only reachable if a
    // future payload omits a string `message`).
    expect(formatError({ kind: "Validation", message: "bad input" })).toBe("bad input");
  });

  it("stringifies an object with only kind (no message)", () => {
    expect(formatError({ kind: "Mongo" })).toBe('{"kind":"Mongo"}');
  });

  it("stringifies primitives that aren't strings/errors", () => {
    expect(formatError(42)).toBe("42");
    expect(formatError(null)).toBe("null");
  });
});
