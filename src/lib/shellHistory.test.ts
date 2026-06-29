import { describe, expect, it } from "vitest";
import { HISTORY_CLEAR, nextHistoryIndex, prevHistoryIndex } from "./shellHistory";

describe("shell history navigation", () => {
  it("recalls the newest entry first when pressing Up", () => {
    expect(prevHistoryIndex(3, -1)).toBe(2);
  });

  it("walks older entries and clamps at the oldest entry", () => {
    expect(prevHistoryIndex(3, 2)).toBe(1);
    expect(prevHistoryIndex(3, 1)).toBe(0);
    expect(prevHistoryIndex(3, 0)).toBe(0);
  });

  it("does not recall history when history is empty", () => {
    expect(prevHistoryIndex(0, -1)).toBeNull();
  });

  it("walks newer entries and clears after the newest entry", () => {
    expect(nextHistoryIndex(3, 0)).toBe(1);
    expect(nextHistoryIndex(3, 1)).toBe(2);
    expect(nextHistoryIndex(3, 2)).toBe(HISTORY_CLEAR);
  });

  it("ignores Down when history navigation is inactive", () => {
    expect(nextHistoryIndex(3, -1)).toBeNull();
  });
});
