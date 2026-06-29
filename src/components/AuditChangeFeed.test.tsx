import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import type { ComponentProps } from "react";
import { AuditChangeFeed } from "./AuditChangeFeed";
import type { AuditDomain, AuditEvent, DomainRootInfo } from "../ipc/commands";

const events: AuditEvent[] = [
  event(0, "rs:rs0", "sales", "orders", 0),
  event(1, "rs:rs0", "sales", "orders", 1),
  event(2, "rs:rs0", "billing", "invoices", 0),
];

const domains: AuditDomain[] = [
  { deploymentId: "rs:rs0", database: "sales", eventCount: 2 },
  { deploymentId: "rs:rs0", database: "billing", eventCount: 1 },
];

const domainInfo: DomainRootInfo = {
  deploymentId: "rs:rs0",
  database: "sales",
  rootHex: "abcdef0123456789abcdef0123456789",
  eventCount: 2,
  legalHold: false,
  retainedRoots: [],
};

describe("AuditChangeFeed domain segmentation", () => {
  it("renders deployment/database domain groups and changes domain filter", () => {
    const onDomainChange = vi.fn();

    renderFeed({ onDomainChange });

    expect(screen.getByLabelText("Audit domain groups")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "All domains · 3" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "rs:rs0 · sales · 2" }));
    expect(onDomainChange).toHaveBeenCalledWith("rs:rs0", "sales");

    fireEvent.change(screen.getByLabelText("Audit domain"), {
      target: { value: "rs:rs0\u0000billing" },
    });
    expect(onDomainChange).toHaveBeenCalledWith("rs:rs0", "billing");
  });

  it("filters visible rows to the selected deployment/database domain", () => {
    renderFeed({
      selectedDeploymentId: "rs:rs0",
      selectedDatabase: "sales",
    });

    expect(screen.getAllByText(/rs:rs0 · sales\.orders/)).toHaveLength(2);
    expect(screen.queryByText(/rs:rs0 · billing\.invoices/)).not.toBeInTheDocument();
    expect(screen.getByText("Domain: rs:rs0 · sales")).toBeInTheDocument();
  });

  it("renders selected domain root status and invokes lifecycle actions", () => {
    const onSetLegalHold = vi.fn();
    const onPruneDomain = vi.fn();

    renderFeed({
      selectedDeploymentId: "rs:rs0",
      selectedDatabase: "sales",
      domainInfo,
      onSetLegalHold,
      onPruneDomain,
    });

    expect(screen.getByLabelText("Domain segment")).toBeInTheDocument();
    expect(screen.getByText(/Domain root/)).toHaveTextContent("abcdef0123456789");

    fireEvent.click(screen.getByRole("button", { name: "Legal hold" }));
    expect(onSetLegalHold).toHaveBeenCalledWith(true);

    fireEvent.click(screen.getByRole("button", { name: "Prune" }));
    expect(onPruneDomain).toHaveBeenCalled();
  });

  it("shows legal-hold and pruned badges and disables pruning while held", () => {
    renderFeed({
      selectedDeploymentId: "rs:rs0",
      selectedDatabase: "sales",
      domainInfo: {
        ...domainInfo,
        legalHold: true,
        retainedRoots: [
          {
            rootHex: "retained-root",
            eventCount: 2,
            maxIndex: 1,
            prunedAt: "2026-01-01T00:00:00Z",
          },
        ],
      },
    });

    expect(screen.getByText("legal hold")).toBeInTheDocument();
    expect(screen.getByText("1 pruned")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Lift legal hold" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Prune" })).toBeDisabled();
  });

  it("proves a domain root in the aggregation super-root", () => {
    const onProveInSuperRoot = vi.fn();

    const { rerender } = renderFeed({
      selectedDeploymentId: "rs:rs0",
      selectedDatabase: "sales",
      domainInfo,
      superRootHex: "0123456789abcdef0123456789abcdef",
      onProveInSuperRoot,
    });

    expect(screen.getByLabelText("Domain super-root")).toHaveTextContent(
      "0123456789abcdef",
    );

    fireEvent.click(screen.getByRole("button", { name: "Prove in super-root" }));
    expect(onProveInSuperRoot).toHaveBeenCalled();

    rerender(
      <AuditChangeFeed
        events={events}
        domains={domains}
        selectedDeploymentId="rs:rs0"
        selectedDatabase="sales"
        domainInfo={domainInfo}
        superRootHex="0123456789abcdef0123456789abcdef"
        superProof={{
          deploymentId: "rs:rs0",
          database: "sales",
          domainRootHex: "abcdef0123456789abcdef0123456789",
          superRootHex: "0123456789abcdef0123456789abcdef",
          position: 2,
          leafHex: "fedcba9876543210fedcba9876543210",
          pathElements: ["a"],
          pathIndices: [0],
        }}
        collapsed={false}
        onToggle={vi.fn()}
        proofIndex={null}
        proofLoading={false}
        proofResult={null}
        onProof={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Domain super-root")).toHaveTextContent(
      "Included at position 2",
    );
  });
});

function renderFeed(
  overrides: Partial<ComponentProps<typeof AuditChangeFeed>> = {},
) {
  return render(
    <AuditChangeFeed
      events={events}
      domains={domains}
      collapsed={false}
      onToggle={vi.fn()}
      proofIndex={null}
      proofLoading={false}
      proofResult={null}
      onProof={vi.fn()}
      {...overrides}
    />,
  );
}

function event(
  index: number,
  deploymentId: string,
  database: string,
  collection: string,
  sequence: number,
): AuditEvent {
  return {
    index,
    leafHex: `leaf-${index}`,
    operation: "insert",
    database,
    collection,
    deploymentId,
    sequence,
    timestamp: "2026-01-01T00:00:00Z",
  };
}
