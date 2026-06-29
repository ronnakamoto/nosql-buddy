import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  listTimeline,
  operationKindLabel,
  approvalStatusLabel,
  rollbackLevelLabel,
} from "./timeline";

// Mock Tauri IPC
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("timeline IPC", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("listTimeline", () => {
    it("maps filter params correctly to backend defaults", async () => {
      vi.mocked(invoke).mockResolvedValue([]);

      await listTimeline({
        profileId: "conn-1",
        // Only passing profileId; the rest should map to null
      });

      expect(invoke).toHaveBeenCalledWith("list_timeline", {
        request: {
          profileId: "conn-1",
          database: null,
          collection: null,
          kind: null,
          errored: null,
          limit: null,
          from: null,
          to: null,
        }
      });
    });

    it("passes all filter params when provided", async () => {
      vi.mocked(invoke).mockResolvedValue([{ id: "123", kind: "insert" }]);

      const result = await listTimeline({
        profileId: "conn-2",
        database: "mydb",
        collection: "users",
        kind: "updateOne",
        limit: 50,
      });

      expect(invoke).toHaveBeenCalledWith("list_timeline", {
        request: {
          profileId: "conn-2",
          database: "mydb",
          collection: "users",
          kind: "updateOne",
          errored: null,
          limit: 50,
          from: null,
          to: null,
        }
      });
      expect(result).toHaveLength(1);
    });
  });
});

describe("label functions", () => {
  it("maps known operation kinds to display labels", () => {
    expect(operationKindLabel("updateMany")).toBe("Update Many");
    expect(operationKindLabel("restore")).toBe("Restore");
  });

  it("falls back to the raw value for unknown operation kinds", () => {
    expect(operationKindLabel("futureKind" as never)).toBe("futureKind");
  });

  it("maps known approval statuses and falls back on drift", () => {
    expect(approvalStatusLabel("approved")).toBe("Approved");
    expect(approvalStatusLabel("escalated" as never)).toBe("escalated");
  });

  it("maps known rollback levels and falls back on drift", () => {
    expect(rollbackLevelLabel("changedFields")).toBe("Changed Fields");
    expect(rollbackLevelLabel("partial" as never)).toBe("partial");
  });
});
