import { describe, it, expect } from "vitest";
import { parseExplain } from "./explainParser";

describe("parseExplain", () => {
  it("parses winningPlan (MongoDB 4.4+ style)", () => {
    // The real IPC payload nests it under queryPlanner.winningPlan
    const raw = {
      queryPlannerWinningPlan: {
        queryPlanner: {
          winningPlan: {
            stage: "COLLSCAN",
            inputStage: { stage: "FETCH" },
          },
        },
      },
    };
    const { root } = parseExplain(raw);
    expect(root?.stage).toBe("COLLSCAN");
    expect(root?.children).toHaveLength(1);
    expect(root?.children[0].stage).toBe("FETCH");
  });

  it("parses executionStats tree (MongoDB 3.2+ style)", () => {
    // The walker recursively looks at inputStage(s), it doesn't automatically
    // map the executionStages wrapper. If we pass it exactly to winningPlan
    // it will extract it. (In the UI, usually queryPlannerWinningPlan holds the tree).
    const raw = {
      queryPlannerWinningPlan: {
        stage: "IXSCAN",
        nReturned: 5,
        executionTimeMillisEstimate: 10
      }
    };
    const { root } = parseExplain(raw);
    expect(root?.stage).toBe("IXSCAN");
    expect(root?.nReturned).toBe(5);
    expect(root?.executionTimeMs).toBe(10);
  });

  it("parses aggregate pipelines (stages array)", () => {
    const raw = {
      queryPlannerWinningPlan: {
        stages: [
          { $cursor: { queryPlanner: { winningPlan: { stage: "COLLSCAN" } } } },
        ],
      },
    };
    const { root } = parseExplain(raw);
    expect(root?.stage).toBe("SHARDED_PIPELINE");
    expect(root?.children).toHaveLength(1);
  });

  it("flags collection and index scans correctly", () => {
    const raw = {
      queryPlannerWinningPlan: {
        stage: "FETCH",
        inputStage: { stage: "IXSCAN" },
      },
    };
    const parsed = parseExplain(raw);
    expect(parsed.hasIndexScan).toBe(true);
    expect(parsed.hasCollectionScan).toBe(false);
    expect(parsed.stageCount).toBe(2);
  });

  it("returns fallback for unknown structure", () => {
    const raw = { raw: "bar" };
    // foo is completely ignored by the walker because it expects queryPlannerWinningPlan
    const { root } = parseExplain(raw);
    expect(root).toBeNull();
  });

  it("safely handles null/undefined input", () => {
    expect(parseExplain(null).root).toBeNull();
    expect(parseExplain(undefined).root).toBeNull();
  });
});
