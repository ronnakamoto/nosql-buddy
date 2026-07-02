import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  StorageByDatabaseChart,
  DocumentsByDatabaseChart,
  DataVsIndexChart,
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
  it("StorageByDatabaseChart renders empty state with no databases", () => {
    render(<StorageByDatabaseChart databases={[]} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("StorageByDatabaseChart renders empty state with null", () => {
    render(<StorageByDatabaseChart databases={null as unknown as DatabaseSummary[]} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("StorageByDatabaseChart filters databases with zero or null sizeOnDisk", () => {
    const dbs = [
      makeDb({ name: "a", sizeOnDisk: 0 }),
      makeDb({ name: "b", sizeOnDisk: null }),
      makeDb({ name: "c", sizeOnDisk: undefined }),
      makeDb({ name: "d", sizeOnDisk: -100 }),
    ];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart renders empty state when no document counts", () => {
    const dbs = [makeDb({ name: "a", documentCount: 0 }), makeDb({ name: "b", documentCount: null })];
    render(<DocumentsByDatabaseChart databases={dbs} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("DataVsIndexChart renders empty state with no size data", () => {
    render(<DataVsIndexChart databases={[]} />);
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
  it("StorageByDatabaseChart handles NaN sizeOnDisk", () => {
    const dbs = [makeDb({ name: "a", sizeOnDisk: NaN })];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("StorageByDatabaseChart handles Infinity sizeOnDisk", () => {
    const dbs = [makeDb({ name: "a", sizeOnDisk: Infinity })];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("DocumentsByDatabaseChart handles NaN documentCount", () => {
    const dbs = [makeDb({ name: "a", documentCount: NaN })];
    render(<DocumentsByDatabaseChart databases={dbs} />);
    expect(screen.getByText("No data available")).toBeInTheDocument();
  });

  it("DataVsIndexChart handles NaN storage/index sizes by treating as zero", () => {
    const dbs = [
      makeDb({ name: "big", storageSizeBytes: 2048, indexSizeBytes: 1024 }),
      makeDb({ name: "bad", storageSizeBytes: NaN, indexSizeBytes: NaN }),
    ];
    const { container } = render(<DataVsIndexChart databases={dbs} />);
    const rows = container.querySelectorAll(".overview__bar-row");
    expect(rows).toHaveLength(1);
  });

  it("handles databases array with null entries", () => {
    const dbs = [null, makeDb({ name: "real", sizeOnDisk: 1024 }), undefined] as unknown as DatabaseSummary[];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("real")).toBeInTheDocument();
    expect(screen.getByText("1.0 KB")).toBeInTheDocument();
  });

  it("handles databases with missing name", () => {
    const dbs = [
      makeDb({ name: "", sizeOnDisk: 500 }),
      makeDb({ name: "valid", sizeOnDisk: 1000 }),
    ];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("valid")).toBeInTheDocument();
  });
});

describe("OverviewCharts - correct rendering with valid data", () => {
  it("StorageByDatabaseChart renders bars sorted by size descending", () => {
    const dbs = [
      makeDb({ name: "small", sizeOnDisk: 1024 }),
      makeDb({ name: "big", sizeOnDisk: 1048576 }),
      makeDb({ name: "medium", sizeOnDisk: 102400 }),
    ];
    const { container } = render(<StorageByDatabaseChart databases={dbs} />);
    const labels = container.querySelectorAll(".overview__bar-label");
    expect(labels).toHaveLength(3);
    expect(labels[0].textContent).toBe("big");
    expect(labels[1].textContent).toBe("medium");
    expect(labels[2].textContent).toBe("small");
  });

  it("StorageByDatabaseChart formats bytes correctly", () => {
    const dbs = [makeDb({ name: "db", sizeOnDisk: 1048576 })];
    render(<StorageByDatabaseChart databases={dbs} />);
    expect(screen.getByText("1.0 MB")).toBeInTheDocument();
  });

  it("StorageByDatabaseChart limits to 8 databases", () => {
    const dbs = Array.from({ length: 12 }, (_, i) =>
      makeDb({ name: `db${i}`, sizeOnDisk: 1024 * (12 - i) }),
    );
    const { container } = render(<StorageByDatabaseChart databases={dbs} />);
    const rows = container.querySelectorAll(".overview__bar-row");
    expect(rows).toHaveLength(8);
  });

  it("DocumentsByDatabaseChart formats counts correctly", () => {
    const dbs = [makeDb({ name: "db", documentCount: 1500000 })];
    render(<DocumentsByDatabaseChart databases={dbs} />);
    expect(screen.getByText("1.5M")).toBeInTheDocument();
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

  it("DataVsIndexChart renders stacked bars with legend", () => {
    const dbs = [
      makeDb({ name: "db1", storageSizeBytes: 4096, indexSizeBytes: 1024 }),
      makeDb({ name: "db2", storageSizeBytes: 2048, indexSizeBytes: 2048 }),
    ];
    const { container } = render(<DataVsIndexChart databases={dbs} />);
    const rows = container.querySelectorAll(".overview__bar-row--stacked");
    expect(rows).toHaveLength(2);
    expect(screen.getByText("Data")).toBeInTheDocument();
    expect(screen.getByText("Indexes")).toBeInTheDocument();
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
