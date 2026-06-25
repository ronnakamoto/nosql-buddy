import { useState, useEffect, useCallback, type ReactNode } from "react";
import commands, {
  type AuditStatus,
  type AuditEvent,
  type CommitResult,
  type OnChainRoot,
  type IpfsPublishResult,
  type VerificationReport,
  type Epoch,
  formatError,
} from "../ipc/commands";
import {
  Card,
  CardHeader,
  Badge,
  Button,
  Stat,
  ProgressBar,
  KeyValue,
  Alert,
  Spinner,
  EmptyState,
} from "./AuditUi";

const POLL_INTERVAL_MS = 2000;

/**
 * The redesigned live audit view — shared by Dev and Production flows.
 *
 * Parameterized by:
 * - `commitFn`: the on-chain commit function (native testnet for dev,
 *   production network-aware for production).
 * - `badge`: the mode/network badge shown in the status bar.
 * - `extraPanels`: optional panels appended below (attestation + oplog for
 *   dev mode's full-system view).
 * - `onShowSettings`: opens the settings panel.
 */
export function AuditLiveViewV2({
  commitFn,
  badge,
  extraPanels,
  onShowSettings,
}: {
  commitFn: (metadata?: string) => Promise<CommitResult>;
  badge: ReactNode;
  extraPanels?: ReactNode;
  onShowSettings: () => void;
}) {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [currentEpoch, setCurrentEpoch] = useState<Epoch | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [pinataResult, setPinataResult] = useState<IpfsPublishResult | null>(null);
  const [commitStep, setCommitStep] = useState("");

  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);
  const [verifyLoading, setVerifyLoading] = useState(false);
  const [verifyReport, setVerifyReport] = useState<VerificationReport | null>(null);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [proofResult, setProofResult] = useState<string | null>(null);

  const [showAdvanced, setShowAdvanced] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [s, e, ep] = await Promise.all([
        commands.auditGetStatus(),
        commands.auditListEvents(),
        commands.auditCurrentEpoch(),
      ]);
      setStatus(s);
      setEvents(e);
      setCurrentEpoch(ep);
    } catch {
      // Silent poll failure.
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [refresh]);

  const refreshOnchainRoot = useCallback(async () => {
    try {
      const root = await commands.auditGetOnchainRootRpc();
      setOnchainRoot(root);
    } catch {
      // best-effort
    }
  }, []);

  useEffect(() => {
    refreshOnchainRoot();
  }, [refreshOnchainRoot]);

  const handleCommit = async () => {
    setCommitLoading(true);
    setError(null);
    setCommitResult(null);
    setPinataResult(null);
    setCommitStep("Freezing root...");

    try {
      setCommitStep("Closing epoch...");
      await commands.auditCloseEpoch();
      const epochs = await commands.auditListEpochs();
      const lastEpoch = epochs
        .filter((e) => e.endIndex !== null)
        .sort((a, b) => b.epochNumber - a.epochNumber)[0];

      if (!lastEpoch) throw new Error("No closed epoch to commit");

      setCommitStep("Pinning batch to IPFS via Pinata...");
      const pinata = await commands.auditPublishEpochToPinata(lastEpoch.epochNumber);
      setPinataResult(pinata);

      setCommitStep("Submitting transaction to Stellar...");
      const result = await commitFn(`epoch=${lastEpoch.epochNumber} cid=${pinata.cid}`);
      setCommitResult(result);
      setCommitStep("Confirmed!");

      await commands.auditMarkEpochCommitted(lastEpoch.epochNumber, result.txHash);
      refreshOnchainRoot();
      refresh();
    } catch (err) {
      setError(formatError(err));
      setCommitStep("");
    } finally {
      setCommitLoading(false);
    }
  };

  const handleVerify = async () => {
    setVerifyLoading(true);
    setError(null);
    try {
      const report = await commands.auditVerifyReaderMode();
      setVerifyReport(report);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setVerifyLoading(false);
    }
  };

  const handleProof = async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setError(null);
    try {
      const result = await commands.auditGenerateProof(index);
      setProofResult(
        JSON.stringify(
          { rootHex: result.rootHex, leafIndex: result.leafIndex, proofLength: result.proof.a.length },
          null,
          2,
        ),
      );
    } catch (err) {
      setError(formatError(err));
    } finally {
      setProofLoading(false);
    }
  };

  const leafCount = status?.leafCount ?? 0;
  const eventCount = status?.eventCount ?? 0;
  const rootHex = status?.rootHex ?? "";
  const epochEventCount = currentEpoch?.eventCount ?? 0;
  const epochThreshold = 100;
  const epochPct = epochThreshold > 0 ? Math.min(100, (epochEventCount / epochThreshold) * 100) : 0;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--space-3)",
        padding: "var(--space-4)",
        maxWidth: "880px",
        margin: "0 auto",
        animation: "audit-fade-in 0.2s ease",
      }}
    >
      {/* ─── Status bar ─────────────────────────────────────────────── */}
      <Card padded={false}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-3)",
            padding: "var(--space-3) var(--space-4)",
            flexWrap: "wrap",
          }}
        >
          {badge}
          <div style={{ width: "1px", height: "20px", background: "var(--border)" }} />
          <Stat label="Root" value={rootHex ? `${rootHex.slice(0, 8)}…${rootHex.slice(-6)}` : "—"} mono />
          <Stat label="Events" value={eventCount} />
          <Stat label="Leaves" value={leafCount} />
          <div style={{ flex: 1 }} />
          <Button variant="ghost" onClick={() => setShowAdvanced((v) => !v)}>
            {showAdvanced ? "Hide details" : "Advanced"}
          </Button>
          <Button variant="ghost" onClick={onShowSettings}>
            Settings
          </Button>
        </div>
      </Card>

      {error && <Alert tone="danger">{error}</Alert>}

      {/* ─── Epoch + commit ─────────────────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-3)" }}>
        <Card>
          <CardHeader
            title={`Epoch ${currentEpoch?.epochNumber ?? 0}`}
            subtitle={`${epochEventCount} / ${epochThreshold} events until auto-close`}
          />
          <ProgressBar current={epochEventCount} max={epochThreshold} tone={epochPct >= 100 ? "success" : "accent"} />
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--space-2)" }}>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {currentEpoch?.endIndex !== null && currentEpoch?.endIndex !== undefined
                ? "Closed — ready to commit"
                : "Open — capturing events"}
            </span>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {Math.round(epochPct)}%
            </span>
          </div>
        </Card>

        <Card>
          <CardHeader title="On-Chain Commitment" subtitle="Merkle root → Stellar + IPFS" />
          {commitLoading && (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-2)" }}>
              <Spinner size={13} />
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>{commitStep}</span>
            </div>
          )}
          <Button variant="primary" loading={commitLoading} onClick={handleCommit} style={{ width: "100%" }}>
            Commit Now
          </Button>
          {commitResult && (
            <div style={{ marginTop: "var(--space-3)", animation: "audit-fade-in 0.2s ease" }}>
              <Badge tone="success" dot>Committed</Badge>
              <div style={{ marginTop: "var(--space-2)" }}>
                <KeyValue label="Tx hash" value={shortHash(commitResult.txHash)} />
                {pinataResult && <KeyValue label="IPFS CID" value={pinataResult.cid} />}
              </div>
            </div>
          )}
        </Card>
      </div>

      {/* ─── On-chain root + verify ─────────────────────────────────── */}
      <Card>
        <CardHeader
          title="On-Chain Root"
          subtitle="Latest committed root from the Soroban contract"
          actions={
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <Button variant="ghost" onClick={refreshOnchainRoot}>Refresh</Button>
              <Button variant="secondary" loading={verifyLoading} onClick={handleVerify}>
                Verify Integrity
              </Button>
            </div>
          }
        />
        {onchainRoot ? (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "var(--space-3)" }}>
            <Stat label="Sequence" value={onchainRoot.sequence} mono />
            <Stat label="Root" value={shortHash(onchainRoot.rootHex)} mono />
            <Stat label="Committed" value={formatTs(onchainRoot.timestamp)} />
          </div>
        ) : (
          <EmptyState title="No on-chain commitment yet" body="Commit a root to anchor your audit log on Stellar." />
        )}
        {verifyReport && (
          <div style={{ marginTop: "var(--space-3)" }}>
            <Alert tone={!verifyReport.tamperDetected && verifyReport.chainIntact ? "success" : "danger"}>
              {!verifyReport.tamperDetected && verifyReport.chainIntact
                ? `✓ Integrity verified — ${verifyReport.verifiedEvents} events, chain intact.`
                : `✗ Tamper detected — ${verifyReport.summary}`}
            </Alert>
          </div>
        )}
      </Card>

      {/* ─── Event feed ─────────────────────────────────────────────── */}
      <Card>
        <CardHeader
          title="Event Feed"
          subtitle={`${events.length} event${events.length === 1 ? "" : "s"} captured`}
        />
        {events.length === 0 ? (
          <EmptyState
            icon="○"
            title="No events yet"
            body="Write data to MongoDB (insert, update, delete) to populate the audit log. Events appear here in real time."
          />
        ) : (
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              maxHeight: "320px",
              overflowY: "auto",
              borderRadius: "var(--radius-md)",
              border: "1px solid var(--border)",
            }}
          >
            {events
              .slice()
              .reverse()
              .map((ev) => (
                <EventRow
                  key={ev.index}
                  event={ev}
                  proofLoading={proofLoading && proofIndex === ev.index}
                  onProof={() => handleProof(ev.index)}
                />
              ))}
          </div>
        )}
        {proofResult && (
          <pre
            style={{
              marginTop: "var(--space-3)",
              padding: "var(--space-3)",
              background: "var(--surface-2)",
              borderRadius: "var(--radius-md)",
              fontSize: "var(--font-size-xs)",
              fontFamily: "var(--font-mono)",
              color: "var(--ink-muted)",
              overflow: "auto",
              maxHeight: "180px",
              margin: 0,
            }}
          >
            {proofResult}
          </pre>
        )}
      </Card>

      {/* ─── Extra panels (attestation + oplog for dev mode) ────────── */}
      {extraPanels}

      {/* ─── Advanced drawer ────────────────────────────────────────── */}
      {showAdvanced && (
        <Card style={{ animation: "audit-fade-in 0.2s ease" }}>
          <CardHeader title="Advanced" subtitle="Raw cryptographic details" />
          <KeyValue label="Merkle root (full)" value={rootHex || "—"} />
          <KeyValue label="Tree height" value={status?.treeHeight ?? "—"} />
          {onchainRoot && <KeyValue label="On-chain root (full)" value={onchainRoot.rootHex} />}
          {commitResult && <KeyValue label="Tx hash (full)" value={commitResult.txHash} />}
          {pinataResult && <KeyValue label="IPFS CID (full)" value={pinataResult.cid} />}
        </Card>
      )}
    </div>
  );
}

function EventRow({
  event,
  proofLoading,
  onProof,
}: {
  event: AuditEvent;
  proofLoading: boolean;
  onProof: () => void;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "var(--space-3)",
        padding: "var(--space-2) var(--space-3)",
        borderBottom: "1px solid var(--border)",
        fontSize: "var(--font-size-sm)",
      }}
    >
      <Badge tone={opTone(event.operation)}>{event.operation}</Badge>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
        {event.database}.{event.collection}
      </span>
      <span style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
        leaf {event.leafHex.slice(0, 10)}…
      </span>
      <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
        {new Date(event.timestamp).toLocaleTimeString()}
      </span>
      <Button variant="ghost" loading={proofLoading} onClick={onProof} style={{ padding: "3px 8px", fontSize: "var(--font-size-xs)" }}>
        Proof
      </Button>
    </div>
  );
}

function opTone(op: string): "success" | "warning" | "danger" | "info" {
  if (op.toLowerCase().includes("insert")) return "success";
  if (op.toLowerCase().includes("update")) return "warning";
  if (op.toLowerCase().includes("delete")) return "danger";
  return "info";
}

function shortHash(h: string): string {
  if (!h) return "—";
  return h.length > 20 ? `${h.slice(0, 10)}…${h.slice(-8)}` : h;
}

function formatTs(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return new Date(unixSeconds * 1000).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}
