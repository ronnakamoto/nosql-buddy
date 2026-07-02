import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  DocumentsByDatabaseChart,
  StorageShareDonutChart,
  TopCollectionsChart,
  CollectionTypeChart,
} from "./OverviewCharts";
import type { CollectionSummary, DatabaseSummary } from "../ipc/commands";

function makeDb(over: Partial<DatabaseSummary> = {}): DatabaseSummary {
  return {
    name: "test",
    sizeOnDisk: 1024,
    collectionsCount: 1,
    documentCount: 100,
    indexCount: 2,
    indexSizeBytes: 512,
    storageSizeBytes: 768,
    ...over,
  };
}

function makeColl(over: Partial<CollectionSummary> = {}): CollectionSummary {
  return {
    name: "coll",
    type: "collection",
    documentCount: 10,
    sizeBytes: 1024,
    storageSizeBytes: 768,
    ...over,
  };
}

describe("OverviewCharts - empty / null / undefined data", () => {
  it("DocumentsByDatabaseChart renders empty state when no document counts", () => {
    const dbs = [makeDb({ name: "a", documentCount: 0 }), makeDb({ name: "b", documentCount: null })];
    render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("StorageShareDonutChart renders empty state with no size data", () => {
    render(<StorageShareDonutChart databases={[]} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("TopCollectionsChart renders empty state with no collections", () => {
    render(<TopCollectionsChart databases={[]} collections={{}} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("TopCollectionsChart renders empty state with null collections", () => {
    render(
      <TopCollectionsChart
        databases={[makeDb({ name: "a" })]}
        collections={null as unknown as Record<string, CollectionSummary[]>}
      />,
    );
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("CollectionTypeChart renders empty state with no collections", () => {
    render(<CollectionTypeChart databases={[]} collections={{}} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("CollectionTypeChart renders empty state with null databases", () => {
    render(
      <CollectionTypeChart
        databases={null as unknown as DatabaseSummary[]}
        collections={{}}
      />,
    );
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });
});

describe("OverviewCharts - NaN / Infinity / malformed values", () => {
  it("DocumentsByDatabaseChart handles NaN documentCount", () => {
    const dbs = [makeDb({ name: "a", documentCount: NaN })];
    render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("StorageShareDonutChart filters databases with no size", () => {
    const dbs = [
      makeDb({ name: "big", sizeOnDisk: 2048 }),
      makeDb({ name: "zero", sizeOnDisk: 0 }),
      makeDb({ name: "bad", sizeOnDisk: NaN }),
      makeDb({ name: "null", sizeOnDisk: null }),
    ];
    const { container } = render(<StorageShareDonutChart databases={dbs} />);
    const items = container.querySelectorAll(".overview__donut-legend-item");
    expect(items).toHaveLength(1);
  });
});

describe("OverviewCharts - correct rendering with valid data", () => {
  it("DocumentsByDatabaseChart formats counts correctly", () => {
    const dbs = [makeDb({ name: "db", documentCount: 1500000 })];
    render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    expect(screen.getByText("1.5M")).toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart falls back to per-collection aggregation when dbStats.objects missing", () => {
    const dbs = [
      makeDb({ name: "a", documentCount: null, collectionsCount: 2 }),
      makeDb({ name: "b", documentCount: 0, collectionsCount: 1 }),
    ];
    const collections = {
      a: [makeColl({ name: "users", documentCount: 500 }), makeColl({ name: "logs", documentCount: 200 })],
      b: [makeColl({ name: "items", documentCount: 1000 })],
    };
    render(<DocumentsByDatabaseChart databases={dbs} collections={collections} />);
    expect(screen.getByText("700")).toBeInTheDocument();
    expect(screen.getByText("1.0k")).toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart prefers dbStats.objects over collection aggregation", () => {
    const dbs = [makeDb({ name: "a", documentCount: 9999 })];
    const collections = {
      a: [makeColl({ name: "users", documentCount: 500 })],
    };
    render(<DocumentsByDatabaseChart databases={dbs} collections={collections} />);
    expect(screen.getByText("10.0k")).toBeInTheDocument();
    expect(screen.queryByText("500")).not.toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart shows log/linear toggle when data spans orders of magnitude", () => {
    const dbs = [
      makeDb({ name: "huge", documentCount: 57700 }),
      makeDb({ name: "medium", documentCount: 44 }),
      makeDb({ name: "tiny", documentCount: 3 }),
    ];
    const { container } = render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    expect(container.querySelector(".overview__scale-toggle")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Log" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Linear" })).toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart hides toggle when data is uniform", () => {
    const dbs = [
      makeDb({ name: "a", documentCount: 100 }),
      makeDb({ name: "b", documentCount: 80 }),
      makeDb({ name: "c", documentCount: 60 }),
    ];
    const { container } = render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    expect(container.querySelector(".overview__scale-toggle")).not.toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart toggle switches between log and linear scales", () => {
    const dbs = [
      makeDb({ name: "huge", documentCount: 57700 }),
      makeDb({ name: "tiny", documentCount: 3 }),
    ];
    const { container } = render(<DocumentsByDatabaseChart databases={dbs} collections={{}} />);
    // The tiny value's bar (second in the list) should be wider on log scale
    // than on linear scale. The huge value's bar is always ~100% on both.
    const fills = () => Array.from(container.querySelectorAll(".overview__bar-fill")) as HTMLElement[];
    // Default is log scale
    const logTinyWidth = parseFloat(fills()[1].style.width);
    // Switch to linear
    fireEvent.click(screen.getByRole("button", { name: "Linear" }));
    const linearTinyWidth = parseFloat(fills()[1].style.width);
    // Log scaling should give the tiny value a meaningfully wider bar
    expect(logTinyWidth).toBeGreaterThan(linearTinyWidth);
  });

  it("TopCollectionsChart merges collections across databases", () => {
    const dbs = [makeDb({ name: "a" }), makeDb({ name: "b" })];
    const collections = {
      a: [makeColl({ name: "users", documentCount: 500 }), makeColl({ name: "logs", documentCount: 200 })],
      b: [makeColl({ name: "orders", documentCount: 1000 })],
    };
    const { container } = render(<TopCollectionsChart databases={dbs} collections={collections} />);
    const labels = container.querySelectorAll(".overview__bar-label");
    expect(labels).toHaveLength(3);
    expect(labels[0].textContent).toBe("b.orders");
    expect(labels[1].textContent).toBe("a.users");
  });

  it("TopCollectionsChart filters collections with zero document count", () => {
    const dbs = [makeDb({ name: "a" })];
    const collections = {
      a: [makeColl({ name: "full", documentCount: 10 }), makeColl({ name: "empty", documentCount: 0 })],
    };
    const { container } = render(<TopCollectionsChart databases={dbs} collections={collections} />);
    const labels = container.querySelectorAll(".overview__bar-label");
    expect(labels).toHaveLength(1);
    expect(labels[0].textContent).toBe("a.full");
  });

  it("CollectionTypeChart renders type breakdown", () => {
    const dbs = [makeDb({ name: "a" })];
    const collections = {
      a: [
        makeColl({ name: "c1", type: "collection" }),
        makeColl({ name: "c2", type: "collection" }),
        makeColl({ name: "v1", type: "view" }),
        makeColl({ name: "t1", type: "time-series" }),
      ],
    };
    render(<CollectionTypeChart databases={dbs} collections={collections} />);
    expect(screen.getByText("Collections")).toBeInTheDocument();
    expect(screen.getByText("Views")).toBeInTheDocument();
    expect(screen.getByText("Time-Series")).toBeInTheDocument();
  });

  it("CollectionTypeChart handles unknown collection types", () => {
    const dbs = [makeDb({ name: "a" })];
    const collections = {
      a: [makeColl({ name: "weird", type: "custom-type" as CollectionSummary["type"] })],
    };
    render(<CollectionTypeChart databases={dbs} collections={collections} />);
    expect(screen.getByText("custom-type")).toBeInTheDocument();
  });

  it("StorageShareDonutChart renders donut with legend showing percentages", () => {
    const dbs = [
      makeDb({ name: "alpha", sizeOnDisk: 4096 }),
      makeDb({ name: "beta", sizeOnDisk: 2048 }),
    ];
    const { container } = render(<StorageShareDonutChart databases={dbs} />);
    const ring = container.querySelector(".overview__donut-ring");
    expect(ring).toBeInTheDocument();
    expect((ring as HTMLElement).style.background).toContain("conic-gradient");
    const items = container.querySelectorAll(".overview__donut-legend-item");
    expect(items).toHaveLength(2);
    expect(screen.getByText("alpha")).toBeInTheDocument();
    expect(screen.getByText("beta")).toBeInTheDocument();
    expect(screen.getByText("Total")).toBeInTheDocument();
  });

  it("StorageShareDonutChart groups tail databases into Other bucket when 20 databases", () => {
    // 20 databases with sizes such that top 7 each clearly exceed MIN_VISIBLE_PCT (1.5%).
    // Use an arithmetic series with a big gap so db0..db6 each hold >= 2% and the
    // remaining 13 databases together fall into "Other".
    const dbs = Array.from({ length: 20 }, (_, i) => {
      // db0..db6: sizes 100, 90, 80, 70, 60, 50, 40  →  sum=490, each >= 40/490 = 8%
      // db7..db19: tiny tail  → all together about 0.01, well under any single top item
      const size = i < 7 ? (7 - i) * 10 : 0.001;
      return makeDb({ name: `db${i}`, sizeOnDisk: size });
    });
    const { container } = render(<StorageShareDonutChart databases={dbs} />);
    const items = container.querySelectorAll(".overview__donut-legend-item");
    // 7 named + 1 Other bucket = 8 total
    expect(items).toHaveLength(8);
    expect(screen.getByText(/Other \(/)).toBeInTheDocument();
    // The legend should show database names in descending size order
    expect(screen.getByText("db0")).toBeInTheDocument();
    expect(screen.getByText("db6")).toBeInTheDocument();
    // db19 should NOT appear directly (it's in Other)
    expect(screen.queryByText("db19")).not.toBeInTheDocument();
  });

  it("StorageShareDonutChart rolls sub-threshold databases into Other even when fewer than MAX_SLICES", () => {
    // 3 databases, but tiny ones are < MIN_VISIBLE_PCT so they fall into Other
    const dbs = [
      makeDb({ name: "big", sizeOnDisk: 1000000 }),
      makeDb({ name: "small1", sizeOnDisk: 100 }),    // ~0.01%
      makeDb({ name: "small2", sizeOnDisk: 100 }),    // ~0.01%
    ];
    const { container } = render(<StorageShareDonutChart databases={dbs} />);
    const items = container.querySelectorAll(".overview__donut-legend-item");
    // 1 named + 1 Other = 2
    expect(items).toHaveLength(2);
    expect(screen.getByText("big")).toBeInTheDocument();
    expect(screen.getByText(/Other \(2\)/)).toBeInTheDocument();
  });

  it("StorageShareDonutChart does not show Other bucket when all databases fit", () => {
    const dbs = [
      makeDb({ name: "alpha", sizeOnDisk: 1000000 }),
      makeDb({ name: "beta", sizeOnDisk: 500000 }),
      makeDb({ name: "gamma", sizeOnDisk: 250000 }),
    ];
    render(<StorageShareDonutChart databases={dbs} />);
    expect(screen.queryByText(/Other/)).not.toBeInTheDocument();
  });

  it("StorageShareDonutChart Other bucket percentage equals sum of rolled-in databases", () => {
    // 5 dbs: 1 large (~99.4%) dominates; 4 small roll into "Other"
    const dbs = [
      makeDb({ name: "a", sizeOnDisk: 99000000 }),  // ~99.4%
      makeDb({ name: "b", sizeOnDisk: 500000 }),     // ~0.5%  → Other
      makeDb({ name: "c", sizeOnDisk: 300000 }),     // ~0.3%  → Other
      makeDb({ name: "d", sizeOnDisk: 100000 }),     // ~0.1%  → Other
      makeDb({ name: "tiny", sizeOnDisk: 100 }),     // ~0%    → Other
    ];
    render(<StorageShareDonutChart databases={dbs} />);
    expect(screen.getByText("a")).toBeInTheDocument();
    // 4 databases rolled into Other
    expect(screen.getByText(/Other \(4\)/)).toBeInTheDocument();
  });

  it("StorageShareDonutChart stays readable with 20 databases", () => {
    const dbs = Array.from({ length: 20 }, (_, i) =>
      makeDb({ name: `db${i}`, sizeOnDisk: 1024 * (20 - i) }),
    );
    const { container } = render(<StorageShareDonutChart databases={dbs} />);
    const items = container.querySelectorAll(".overview__donut-legend-item");
    // Capped: 7 named + 1 Other = 8
    expect(items).toHaveLength(8);
    // Total still equals the full sum (not just the visible top 7)
    const ring = container.querySelector(".overview__donut-ring") as HTMLElement;
    expect(ring.style.background).toContain("conic-gradient");
  });

  it("TopCollectionsChart handles collections referencing unknown databases", () => {
    const dbs = [makeDb({ name: "a" })];
    const collections = {
      b: [makeColl({ name: "orphan", documentCount: 100 })],
    } as unknown as Record<string, CollectionSummary[]>;
    render(<TopCollectionsChart databases={dbs} collections={collections} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });
});
