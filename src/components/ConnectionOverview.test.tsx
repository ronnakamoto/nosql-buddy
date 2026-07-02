import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConnectionOverview } from "./ConnectionOverview";
import type {
  CollectionSummary,
  ConnectionHandle,
  DatabaseSummary,
  ProfileSummary,
} from "../ipc/commands";

function makeHandle(over: Partial<ConnectionHandle> = {}): ConnectionHandle {
  return {
    connectionId: "conn-1",
    profileId: "p1",
    name: "Test Cluster",
    serverInfo: {
      version: "7.0.12",
      host: "localhost:27017",
      isMaster: true,
      topology: "replicaSet",
    },
    databases: [],
    ...over,
  };
}

function makeProfile(over: Partial<ProfileSummary> = {}): ProfileSummary {
  return {
    id: "p1",
    name: "Test Cluster",
    maskedUri: "mongodb://localhost",
    authMechanism: "none",
    hasSecret: false,
    group: null,
    color: null,
    notes: null,
    sshTunnel: null,
    socks5: null,
    tls: null,
    ...over,
  };
}

function makeDb(over: Partial<DatabaseSummary> = {}): DatabaseSummary {
  return {
    name: "testdb",
    sizeOnDisk: 1048576,
    collectionsCount: 3,
    documentCount: 500,
    indexCount: 4,
    indexSizeBytes: 2048,
    storageSizeBytes: 4096,
    ...over,
  };
}

function makeColl(over: Partial<CollectionSummary> = {}): CollectionSummary {
  return {
    name: "coll",
    type: "collection",
    documentCount: 100,
    sizeBytes: 1024,
    storageSizeBytes: 768,
    ...over,
  };
}

const defaultProps = {
  active: {
    handle: makeHandle(),
    profile: makeProfile(),
    databases: [makeDb()],
    collections: { testdb: [makeColl(), makeColl({ name: "c2" })] },
  },
  onOpenDatabase: vi.fn(),
};

describe("ConnectionOverview - server header", () => {
  it("renders the connection name", () => {
    render(<ConnectionOverview {...defaultProps} />);
    expect(screen.getByText("Test Cluster")).toBeInTheDocument();
  });

  it("renders the host address", () => {
    render(<ConnectionOverview {...defaultProps} />);
    expect(screen.getByText("localhost:27017")).toBeInTheDocument();
  });

  it("renders topology badge", () => {
    render(<ConnectionOverview {...defaultProps} />);
    expect(screen.getByText("Replica Set")).toBeInTheDocument();
  });

  it("renders MongoDB version", () => {
    render(<ConnectionOverview {...defaultProps} />);
    expect(screen.getByText("MongoDB 7.0.12")).toBeInTheDocument();
  });

  it("renders writable primary status", () => {
    render(<ConnectionOverview {...defaultProps} />);
    expect(screen.getByText("Writable Primary")).toBeInTheDocument();
  });

  it("renders read-only for non-primary", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          handle: makeHandle({ serverInfo: { version: "7.0", host: "h", isMaster: false, topology: "replicaSet" } }),
        }}
      />,
    );
    expect(screen.getByText("Read-Only")).toBeInTheDocument();
  });

  it("handles null serverInfo", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{ ...defaultProps.active, handle: makeHandle({ serverInfo: null }) }}
      />,
    );
    expect(screen.getByText("unknown host")).toBeInTheDocument();
    expect(screen.getByText("unknown")).toBeInTheDocument();
  });

  it("handles standalone topology", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          handle: makeHandle({ serverInfo: { version: "6.0", host: "h", isMaster: true, topology: "standalone" } }),
        }}
      />,
    );
    expect(screen.getByText("Standalone")).toBeInTheDocument();
  });

  it("handles sharded topology", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          handle: makeHandle({ serverInfo: { version: "6.0", host: "h", isMaster: true, topology: "sharded" } }),
        }}
      />,
    );
    expect(screen.getByText("Sharded Cluster")).toBeInTheDocument();
  });

  it("handles unknown topology", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          handle: makeHandle({ serverInfo: { version: "6.0", host: "h", isMaster: true, topology: "weird" } }),
        }}
      />,
    );
    expect(screen.getByText("weird")).toBeInTheDocument();
  });
});

describe("ConnectionOverview - database grid", () => {
  it("renders database cards", () => {
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", sizeOnDisk: 0 }), makeDb({ name: "logs", sizeOnDisk: 0 })],
          collections: {},
        }}
      />,
    );
    const cardNames = container.querySelectorAll(".overview__db-name");
    const names = Array.from(cardNames).map((el) => el.textContent);
    expect(names).toContain("app");
    expect(names).toContain("logs");
  });

  it("renders empty state when no databases", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{ ...defaultProps.active, databases: [], collections: {} }}
      />,
    );
    expect(screen.getByText("No databases found on this server.")).toBeInTheDocument();
  });

  it("calls onOpenDatabase with the database name when a card is clicked", () => {
    const onOpenDatabase = vi.fn();
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        onOpenDatabase={onOpenDatabase}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", sizeOnDisk: 0 })],
          collections: {},
        }}
      />,
    );
    const card = container.querySelector(".overview__db-card") as HTMLElement;
    fireEvent.click(card);
    expect(onOpenDatabase).toHaveBeenCalledTimes(1);
    expect(onOpenDatabase).toHaveBeenCalledWith("app");
  });

  it("calls onOpenDatabase with the database name on Enter key press", () => {
    const onOpenDatabase = vi.fn();
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        onOpenDatabase={onOpenDatabase}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", sizeOnDisk: 0 })],
          collections: {},
        }}
      />,
    );
    const card = container.querySelector(".overview__db-card") as HTMLElement;
    fireEvent.keyDown(card, { key: "Enter" });
    expect(onOpenDatabase).toHaveBeenCalledTimes(1);
    expect(onOpenDatabase).toHaveBeenCalledWith("app");
  });

  it("calls onOpenDatabase on Space key press for accessibility", () => {
    const onOpenDatabase = vi.fn();
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        onOpenDatabase={onOpenDatabase}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "shopkeeper", sizeOnDisk: 1024 })],
          collections: {},
        }}
      />,
    );
    const card = container.querySelector(".overview__db-card") as HTMLElement;
    fireEvent.keyDown(card, { key: " " });
    expect(onOpenDatabase).toHaveBeenCalledWith("shopkeeper");
  });

  it("handles database with null sizeOnDisk", () => {
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", sizeOnDisk: null })],
          collections: {},
        }}
      />,
    );
    expect(container.textContent).toContain("0 B");
  });

  it("handles database with NaN documentCount", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", documentCount: NaN })],
          collections: {},
        }}
      />,
    );
    expect(screen.getByText("0 docs")).toBeInTheDocument();
  });
});

describe("ConnectionOverview - no footer (handled by app status bar)", () => {
  it("does not render overview totals footer", () => {
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [
            makeDb({ name: "a", documentCount: 100, indexCount: 2, sizeOnDisk: 1024 }),
            makeDb({ name: "b", documentCount: 200, indexCount: 3, sizeOnDisk: 2048 }),
          ],
          collections: {},
        }}
      />,
    );
    expect(container.querySelector(".overview__totals")).not.toBeInTheDocument();
  });

  it("renders empty state for databases with no data", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{ ...defaultProps.active, databases: [], collections: {} }}
      />,
    );
    expect(screen.getByText("No databases found on this server.")).toBeInTheDocument();
  });
});

describe("ConnectionOverview - malformed data", () => {
  it("handles null databases array", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: null as unknown as DatabaseSummary[],
          collections: {},
        }}
      />,
    );
    expect(screen.getByText("No databases found on this server.")).toBeInTheDocument();
  });

  it("handles null collections record", () => {
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "app", sizeOnDisk: 0 })],
          collections: null as unknown as Record<string, CollectionSummary[]>,
        }}
      />,
    );
    const cardNames = container.querySelectorAll(".overview__db-name");
    expect(Array.from(cardNames).some((el) => el.textContent === "app")).toBe(true);
  });

  it("handles database entry with null in array", () => {
    const { container } = render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [
            null,
            makeDb({ name: "valid", sizeOnDisk: 0 }),
            undefined,
          ] as unknown as DatabaseSummary[],
          collections: {},
        }}
      />,
    );
    const cardNames = container.querySelectorAll(".overview__db-name");
    const names = Array.from(cardNames).map((el) => el.textContent);
    expect(names).toContain("valid");
    expect(names).not.toContain(null);
  });

  it("handles all charts returning empty states", () => {
    render(
      <ConnectionOverview
        {...defaultProps}
        active={{
          ...defaultProps.active,
          databases: [makeDb({ name: "empty", sizeOnDisk: 0, documentCount: 0 })],
          collections: {},
        }}
      />,
    );
    const emptyMessages = screen.getAllByText("No data available");
    expect(emptyMessages.length).toBeGreaterThanOrEqual(2);
  });
});
