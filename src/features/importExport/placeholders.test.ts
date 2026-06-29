import { describe, it, expect } from "vitest";
import { resolvePathPreview } from "./placeholders";

// Pinned clock: 2026-06-29T07:05:09Z -> date 2026-06-29, time 070509 (UTC).
const NOW = new Date("2026-06-29T07:05:09.000Z");
const CTX = { database: "shop", collection: "orders", profile: "Prod DB" };

describe("resolvePathPreview", () => {
  it("expands db and date tokens", () => {
    expect(resolvePathPreview("${db}_${date}.json", CTX, NOW)).toBe("shop_2026-06-29.json");
  });

  it("expands time with zero-padding (UTC)", () => {
    expect(resolvePathPreview("dump_${time}", CTX, NOW)).toBe("dump_070509");
  });

  it("expands collection and profile (sanitized)", () => {
    expect(resolvePathPreview("${collection}-${profile}", CTX, NOW)).toBe("orders-Prod DB");
  });

  it("sanitizes illegal filename characters in tokens", () => {
    const ctx = { database: "a/b:c*?", collection: "x", profile: "y" };
    expect(resolvePathPreview("${db}", ctx, NOW)).toBe("a_b_c__");
  });

  it("returns 'untitled' for empty token values", () => {
    expect(resolvePathPreview("${profile}", { database: "d", collection: "c", profile: "" }, NOW)).toBe("untitled");
  });

  it("leaves unknown tokens intact", () => {
    expect(resolvePathPreview("${nope}.json", CTX, NOW)).toBe("${nope}.json");
  });

  it("copies an unterminated ${ literally", () => {
    expect(resolvePathPreview("file_${db", CTX, NOW)).toBe("file_${db");
  });

  it("handles back-to-back tokens and literal text", () => {
    expect(resolvePathPreview("x${db}${collection}y", CTX, NOW)).toBe("xshopordersy");
  });
});
