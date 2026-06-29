import { useMemo, useState, useEffect, useCallback, type ReactNode } from "react";
import commands, {
  type AuditStatus,
  type AuditEvent,
  type CommitResult,
  type OnChainRoot,
  type IpfsPublishResult,
  type VerificationReport,
  type Epoch,
  type OplogIntegrityReport,
  formatError,
} from "../ipc/commands";
import { useToast } from "../context/ToastContext";
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
  TxHashLink,
  StatusCard,
} from "./AuditUi";
import { CircleDashed } from "lucide-react";
import { InfoPopover } from "./InfoPopover";

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
  network = "testnet",
  extraPanels,
  connectionId,
}: {
  commitFn: (metadata?: string) => Promise<CommitResult>;
  badge: ReactNode;
  network?: "testnet" | "mainnet";
  extraPanels?: ReactNode;
  connectionId?: string | null;
  onShowSettings: () => void;
}) {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [epochs, setEpochs] = useState<Epoch[]>([]);
  const [currentEpoch, setCurrentEpoch] = useState<Epoch | null>(null);
  const toast = useToast();

  const [closeEpochLoading, setCloseEpochLoading] = useState(false);
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [pinataResult, setPinataResult] = useState<IpfsPublishResult | null>(null);
  const [commitStep, setCommitStep] = useState("");
  const [pollingOnchain, setPollingOnchain] = useState(false);

  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);
  const [verifyLoading, setVerifyLoading] = useState(false);
  const [verifyReport, setVerifyReport] = useState<VerificationReport | null>(null);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [proofResult, setProofResult] = useState<string | null>(null);

  const [showAdvanced, setShowAdvanced] = useState(false);

  const [oplogReport, setOplogReport] = useState<OplogIntegrityReport | null>(null);
  const [oplogLoading, setOplogLoading] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [s, e, eps, ep] = await Promise.all([
        commands.auditGetStatus(),
        commands.auditListEvents(),
        commands.auditListEpochs(),
        commands.auditCurrentEpoch(),
      ]);
      setStatus(s);
      setEvents(e);
      setEpochs(eps);
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
      return root;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    refreshOnchainRoot();
  }, [refreshOnchainRoot]);

  // Poll on-chain root after commit until it appears (Stellar ledger ~5-10s).
  useEffect(() => {
    if (!pollingOnchain) return;
    let active = true;
    const id = setInterval(async () => {
      const root = await refreshOnchainRoot();
      if (root && active) {
        setPollingOnchain(false);
      }
    }, POLL_INTERVAL_MS);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, [pollingOnchain, refreshOnchainRoot]);

  const handleCloseEpoch = async () => {
    if (!currentEpoch || currentEpoch.eventCount === 0) return;
    if (currentEpoch.endIndex !== null && currentEpoch.endIndex !== undefined) return;
    setCloseEpochLoading(true);

    try {
      await commands.auditCloseEpoch();
      await refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setCloseEpochLoading(false);
    }
  };

  const handleCommit = async () => {
    if (!lastClosedEpoch) return;
    setCommitLoading(true);

    setCommitResult(null);
    setPinataResult(null);
    setCommitStep("Pinning batch to IPFS via Pinata...");

    try {
      const pinata = await commands.auditPublishEpochToPinata(lastClosedEpoch.epochNumber);
      setPinataResult(pinata);

      setCommitStep("Submitting transaction to Stellar...");
      const result = await commitFn(`epoch=${lastClosedEpoch.epochNumber} cid=${pinata.cid}`);
      setCommitResult(result);
      setCommitStep("Confirmed!");

      await commands.auditMarkEpochCommitted(lastClosedEpoch.epochNumber, result.txHash);
      setPollingOnchain(true);
      refreshOnchainRoot();
      refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
      setCommitStep("");
    } finally {
      setCommitLoading(false);
    }
  };

  const handleVerify = async () => {
    setVerifyLoading(true);

    try {
      const report = await commands.auditVerifyReaderMode();
      setVerifyReport(report);
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setVerifyLoading(false);
    }
  };

  const handleOplogVerify = async () => {
    if (!connectionId) return;
    setOplogLoading(true);

    try {
      const report = await commands.auditVerifyOplogIntegrity(connectionId);
      setOplogReport(report);
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setOplogLoading(false);
    }
  };

  const handleProof = async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);

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
      toast.push(formatError(err), "error");
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
  const epochClosed = currentEpoch?.endIndex !== null && currentEpoch?.endIndex !== undefined;
  const lastClosedEpoch = useMemo(() => {
    return (epochs ?? [])
      .filter((e) => e.endIndex !== null && e.endIndex !== undefined && !e.committed)
      .sort((a, b) => b.epochNumber - a.epochNumber)[0] ?? null;
  }, [epochs]);

  const canCloseEpoch = currentEpoch && !epochClosed && epochEventCount > 0 && !closeEpochLoading;
  const closeEpochDisabledReason = !currentEpoch
    ? "No epoch data"
    : epochClosed
      ? "Epoch already closed"
      : epochEventCount === 0
        ? "Write to MongoDB to capture events"
        : null;
  const canCommit = lastClosedEpoch !== null && !commitLoading;
  const commitDisabledReason = !lastClosedEpoch ? "Seal a batch first" : null;

  const oplogStatus: "good" | "warning" | "neutral" = oplogReport
    ? oplogReport.allMatch
      ? "good"
      : oplogReport.verdict === "incomplete" || oplogReport.verdict === "no_commitment" || oplogReport.verdict === "no_oplog_commitment"
        ? "neutral"
        : "warning"
    : "neutral";

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "auto" }}>
      {/* ─── Workflow step guide ──────────────────────────────────── */}
      <div className="audit-step-guide">
        <div className={`audit-step ${epochEventCount > 0 ? "audit-step--done" : "audit-step--active"}`}>
          <span className="audit-step__num">{epochEventCount > 0 ? "✓" : "1"}</span>
          <span className="audit-step__label">Write Data</span>
        </div>
        <div className={`audit-step ${epochEventCount > 0 && !epochClosed ? "audit-step--active" : epochClosed ? "audit-step--done" : ""}`}>
          <span className="audit-step__num">{epochClosed ? "✓" : epochEventCount > 0 ? "2" : ""}</span>
          <span className="audit-step__label">Seal Batch</span>
        </div>
        <div className={`audit-step ${epochClosed || commitResult ? "audit-step--active" : ""} ${commitResult ? "audit-step--done" : ""}`}>
          <span className="audit-step__num">{commitResult ? "✓" : epochClosed || lastClosedEpoch ? "3" : ""}</span>
          <span className="audit-step__label">Commit</span>
        </div>
      </div>

      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
          padding: "var(--space-3)",
          flex: 1,
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
        </div>
      </Card>

      {/* ─── Epoch + commit ─────────────────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-3)" }}>
        <Card>
          <CardHeader
            title={<>Batch {currentEpoch?.epochNumber ?? 0}<InfoPopover label="Help: Audit batch" title="Audit batch"><p>Events are grouped into batches (epochs). Once sealed, the batch fingerprint is committed to the blockchain for permanent verification.</p></InfoPopover></>}
            subtitle={epochClosed ? "Sealed and ready to commit" : `${epochEventCount} / ${epochThreshold} changes captured`}
          />
          <ProgressBar current={epochEventCount} max={epochThreshold} tone={epochClosed ? "success" : "accent"} />
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--space-2)" }}>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {epochClosed ? "Sealed" : "Recording changes"}
            </span>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {Math.round(epochPct)}%
            </span>
          </div>
          {!epochClosed && (
            <div style={{ marginTop: "var(--space-3)" }}>
              <Button
                variant="secondary"
                loading={closeEpochLoading}
                disabled={!canCloseEpoch}
                onClick={handleCloseEpoch}
                style={{ width: "100%" }}
                title={closeEpochDisabledReason ?? "Seal the current batch so it can be committed"}
              >
                Seal Batch
              </Button>
              {closeEpochDisabledReason && (
                <div
                  style={{
                    marginTop: "var(--space-2)",
                    fontSize: "var(--font-size-xs)",
                    color: "var(--ink-faint)",
                    lineHeight: "var(--line-height-tight)",
                  }}
                >
                  {closeEpochDisabledReason}
                </div>
              )}
            </div>
          )}
        </Card>

        <Card>
          <CardHeader
            title={<>Commit to Stellar<InfoPopover label="Help: Commit to Stellar" title="Commit to Stellar"><p>Publishes the sealed batch fingerprint to IPFS and anchors it on the Stellar blockchain. This creates a permanent, verifiable record.</p></InfoPopover></>}
            subtitle={lastClosedEpoch ? `Batch #${lastClosedEpoch.epochNumber} ready` : "Anchor the sealed batch on-chain"}
          />
          {commitLoading && (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-2)" }}>
              <Spinner size={13} />
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>{commitStep}</span>
            </div>
          )}
          <Button
            variant="primary"
            loading={commitLoading}
            disabled={!canCommit}
            onClick={handleCommit}
            style={{ width: "100%" }}
            title={commitDisabledReason ?? "Commit the sealed batch to Stellar"}
          >
            Commit Now
          </Button>
          {commitDisabledReason && (
            <div
              style={{
                marginTop: "var(--space-2)",
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-faint)",
                lineHeight: "var(--line-height-tight)",
              }}
            >
              {commitDisabledReason}
            </div>
          )}
          {commitResult && (
            <div style={{ marginTop: "var(--space-3)", animation: "audit-fade-in 0.2s ease" }}>
              <Badge tone="success" dot>Committed</Badge>
              <div style={{ marginTop: "var(--space-2)" }}>
                <KeyValue label="Tx hash" value={<TxHashLink txHash={commitResult.txHash} network={network} />} />
                {pinataResult && <KeyValue label="IPFS CID" value={pinataResult.cid} />}
              </div>
            </div>
          )}
        </Card>
      </div>

      {/* ─── On-chain root + verify ─────────────────────────────────── */}
      <Card>
        <CardHeader
          title={<>On-Chain Record<InfoPopover label="Help: On-Chain Record" title="On-Chain Record"><p>Shows the latest batch root that has been committed to the Stellar blockchain. Use Refresh to poll for new confirmations and Verify Integrity to detect tampering.</p></InfoPopover></>}
          subtitle="Latest batch fingerprint anchored on Stellar"
          actions={
            <div style={{ display: "flex", gap: "var(--space-2)" }}>
              <Button variant="ghost" onClick={refreshOnchainRoot} loading={pollingOnchain}>Refresh</Button>
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
        ) : pollingOnchain ? (
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", padding: "var(--space-3) 0" }}>
            <Spinner size={14} />
            <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
              Waiting for Stellar confirmation…
            </span>
          </div>
        ) : (
          <EmptyState title="Nothing committed yet" body="Commit a sealed batch to anchor your audit log on Stellar." />
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

      {/* ─── Oplog verification (when a MongoDB connection is active) ── */}
      {connectionId && (
        <StatusCard
          title={<>Oplog verification<InfoPopover label="Help: Oplog verification" title="Oplog verification"><p>Compares the MongoDB oplog against the audit commitment to verify every database operation is accounted for in the audit trail.</p></InfoPopover></>}
          status={oplogStatus}
          value={
            oplogReport
              ? oplogReport.allMatch
                ? "Match"
                : oplogReport.verdict === "no_commitment"
                  ? "No commitment"
                  : oplogReport.verdict === "no_oplog_commitment"
                    ? "No oplog hash"
                    : oplogReport.verdict === "contract_outdated"
                      ? "Contract outdated"
                      : oplogReport.verdict === "stale"
                        ? "Stale"
                        : oplogReport.verdict === "complete"
                          ? "Verified"
                          : "Mismatch"
              : "—"
          }
          detail={
            oplogReport
              ? oplogReport.verdict === "complete"
                ? `${oplogReport.oplogEntryCount ?? 0} oplog entries verified`
                : oplogReport.explanation
              : "Not verified yet — click Verify to check oplog completeness"
          }
          action={
            <Button variant="ghost" loading={oplogLoading} onClick={handleOplogVerify}>
              Verify
            </Button>
          }
        />
      )}

      {/* ─── Event feed ─────────────────────────────────────────────── */}
      <Card>
        <CardHeader
          title={<>Change Feed<InfoPopover label="Help: Change Feed" title="Change Feed"><p>Real-time stream of audited MongoDB operations. Click an event to generate a cryptographic proof, or use filters to narrow by operation type.</p></InfoPopover></>}
          subtitle={`${events.length} event${events.length === 1 ? "" : "s"} captured`}
        />
        {events.length === 0 ? (
          <EmptyState
            icon={<CircleDashed size={28} />}
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
          <CardHeader title={<>Advanced<InfoPopover label="Help: Advanced" title="Advanced audit info"><p>Detailed technical data including the current Merkle root, tree height, on-chain root, transaction hashes, and IPFS CIDs.</p></InfoPopover></>} subtitle="Raw cryptographic details" />
          <KeyValue label="Merkle root (full)" value={rootHex || "—"} />
          <KeyValue label="Tree height" value={status?.treeHeight ?? "—"} />
          {onchainRoot && <KeyValue label="On-chain root (full)" value={onchainRoot.rootHex} />}
          {commitResult && <KeyValue label="Tx hash (full)" value={<TxHashLink txHash={commitResult.txHash} network={network} showExternalIcon={true} />} />}
          {pinataResult && <KeyValue label="IPFS CID (full)" value={pinataResult.cid} />}
        </Card>
      )}
      </div>
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
      <Button variant="ghost" size="sm" loading={proofLoading} onClick={onProof}>
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
