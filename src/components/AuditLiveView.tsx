import { useState, useEffect, useCallback, type ReactNode, type CSSProperties } from "react";
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

/**
 * Live audit view — the simplified one-view experience for dev mode.
 *
 * Replaces the 4-tab AuditPanel with a single real-time view:
 * - Live event feed (auto-refreshing)
 * - Merkle root display
 * - Epoch progress bar
 * - Commit button (native signing + Pinata)
 * - Verify button
 * - Proof generation per event
 * - Advanced drawer (collapsed)
 */

const POLL_INTERVAL_MS = 2000;

export function AuditLiveView({
  onShowSettings,
}: {
  onShowSettings: () => void;
}) {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [currentEpoch, setCurrentEpoch] = useState<Epoch | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Commit state
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [pinataResult, setPinataResult] = useState<IpfsPublishResult | null>(null);
  const [commitStep, setCommitStep] = useState<string>("");

  // On-chain state
  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);

  // Verify state
  const [verifyLoading, setVerifyLoading] = useState(false);
  const [verifyReport, setVerifyReport] = useState<VerificationReport | null>(null);

  // Proof state
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [proofResult, setProofResult] = useState<string | null>(null);

  // Advanced drawer
  const [showAdvanced, setShowAdvanced] = useState(false);

  // ─── Auto-refresh ────────────────────────────────────────────────

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
    } catch (err) {
      // Silent fail on poll — don't spam errors every 2s.
      // Only show errors from explicit user actions.
    }
  }, []);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [refresh]);

  // ─── Actions ─────────────────────────────────────────────────────

  const handleCommit = async () => {
    setCommitLoading(true);
    setError(null);
    setCommitResult(null);
    setPinataResult(null);
    setCommitStep("Freezing root...");

    try {
      // Step 1: Close the current epoch (freezes the root).
      setCommitStep("Closing epoch...");
      await commands.auditCloseEpoch();
      const epochs = await commands.auditListEpochs();
      const lastEpoch = epochs
        .filter((e) => e.endIndex !== null)
        .sort((a, b) => b.epochNumber - a.epochNumber)[0];

      if (!lastEpoch) {
        throw new Error("No closed epoch to commit");
      }

      // Step 2: Pin the batch to IPFS via Pinata.
      setCommitStep("Pinning batch to IPFS via Pinata...");
      const pinata = await commands.auditPublishEpochToPinata(lastEpoch.epochNumber);
      setPinataResult(pinata);

      // Step 3: Commit the root on-chain via native signing.
      setCommitStep("Submitting transaction to Stellar testnet...");
      const result = await commands.auditCommitRootNative(
        `epoch=${lastEpoch.epochNumber} cid=${pinata.cid}`,
      );
      setCommitResult(result);
      setCommitStep("Confirmed!");

      // Mark the epoch as committed.
      await commands.auditMarkEpochCommitted(lastEpoch.epochNumber, result.txHash);

      // Refresh on-chain root.
      refreshOnchainRoot();
    } catch (err) {
      setError(formatError(err));
      setCommitStep("");
    } finally {
      setCommitLoading(false);
    }
  };

  const refreshOnchainRoot = async () => {
    try {
      const root = await commands.auditGetOnchainRootRpc();
      setOnchainRoot(root);
    } catch {
      // Silent — on-chain query is best-effort.
    }
  };

  useEffect(() => {
    refreshOnchainRoot();
  }, []);

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

  const handleGenerateProof = async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setError(null);
    try {
      const result = await commands.auditGenerateProof(index);
      setProofResult(
        JSON.stringify(
          {
            rootHex: result.rootHex,
            leafIndex: result.leafIndex,
            proofLength: result.proof.a.length,
          },
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

  // ─── Render ──────────────────────────────────────────────────────

  const leafCount = status?.leafCount ?? 0;
  const eventCount = status?.eventCount ?? 0;
  const rootHex = status?.rootHex ?? "";
  const epochEventCount = currentEpoch?.eventCount ?? 0;
  const epochThreshold = 10; // from epoch.rs default

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "12px", padding: "12px" }}>
      {/* Status bar */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: "12px",
          padding: "8px 12px",
          background: "var(--surface-2)",
          border: "1px solid var(--border)",
          borderRadius: "var(--radius-md)",
          flexWrap: "wrap",
        }}
      >
        <DevModeBadge />
        <Stat label="Root" value={rootHex ? `${rootHex.slice(0, 8)}...${rootHex.slice(-8)}` : "—"} />
        <Stat label="Events" value={eventCount} />
        <Stat label="Leaves" value={leafCount} />
        <div style={{ flex: 1 }} />
        <button
          onClick={() => setShowAdvanced((v) => !v)}
          style={btnSecondaryStyle}
        >
          {showAdvanced ? "Hide Details" : "Advanced"}
        </button>
        <button onClick={onShowSettings} style={btnSecondaryStyle}>
          Settings
        </button>
      </div>

      {/* Epoch progress */}
      <SectionCard>
        <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "8px" }}>
          <strong style={{ fontSize: "12px", fontFamily: "var(--font-sans)" }}>
            Epoch {currentEpoch?.epochNumber ?? 0}
          </strong>
          <span style={{ fontSize: "11px", color: "var(--ink-faint)" }}>
            {epochEventCount} / {epochThreshold} events until auto-close
          </span>
        </div>
        <ProgressBar current={epochEventCount} max={epochThreshold} />
      </SectionCard>

      {/* Commit section */}
      <SectionCard>
        <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "8px" }}>
          <strong style={{ fontSize: "12px", fontFamily: "var(--font-sans)" }}>
            On-Chain Commitment
          </strong>
        </div>

        {commitLoading && (
          <div style={{ fontSize: "11px", color: "var(--ink-faint)", marginBottom: "8px" }}>
            {commitStep}
          </div>
        )}

        {commitResult && (
          <CommitSuccessView
            result={commitResult}
            pinataResult={pinataResult}
          />
        )}

        <div style={{ display: "flex", gap: "8px", marginTop: "8px" }}>
          <button
            onClick={handleCommit}
            disabled={commitLoading || leafCount === 0}
            style={btnPrimaryStyle}
          >
            {commitLoading ? "Committing..." : "Commit to Stellar"}
          </button>
          <button
            onClick={handleVerify}
            disabled={verifyLoading}
            style={btnSecondaryStyle}
          >
            {verifyLoading ? "Verifying..." : "Verify Integrity"}
          </button>
        </div>

        {verifyReport && (
          <div style={{ marginTop: "8px" }}>
            <VerifyResultView report={verifyReport} />
          </div>
        )}
      </SectionCard>

      {/* On-chain status */}
      {onchainRoot && (
        <SectionCard>
          <strong style={{ fontSize: "12px", fontFamily: "var(--font-sans)" }}>
            Latest On-Chain Root
          </strong>
          <div style={{ fontSize: "11px", fontFamily: "var(--font-mono)", marginTop: "6px" }}>
            <div>Sequence: {onchainRoot.sequence}</div>
            <div>Root: {onchainRoot.rootHex.slice(0, 16)}...{onchainRoot.rootHex.slice(-16)}</div>
            <div>Timestamp: {new Date(onchainRoot.timestamp * 1000).toLocaleString()}</div>
            {onchainRoot.metadata && <div>Metadata: {onchainRoot.metadata}</div>}
          </div>
        </SectionCard>
      )}

      {/* Event feed */}
      <SectionCard>
        <strong style={{ fontSize: "12px", fontFamily: "var(--font-sans)", marginBottom: "8px", display: "block" }}>
          Live Event Feed ({events.length})
        </strong>
        <EventFeed
          events={events}
          proofIndex={proofIndex}
          proofLoading={proofLoading}
          proofResult={proofResult}
          onGenerateProof={handleGenerateProof}
        />
      </SectionCard>

      {/* Advanced drawer */}
      {showAdvanced && (
        <SectionCard>
          <strong style={{ fontSize: "12px", fontFamily: "var(--font-sans)", marginBottom: "8px", display: "block" }}>
            Technical Details
          </strong>
          <div style={{ fontSize: "11px", fontFamily: "var(--font-mono)", color: "var(--ink-faint)", lineHeight: 1.6 }}>
            <div>Full root: {rootHex || "—"}</div>
            <div>Tree height: {status?.treeHeight ?? 20}</div>
            {commitResult && <div>Tx hash: {commitResult.txHash}</div>}
            {commitResult && <div>On-chain sequence: {commitResult.sequence}</div>}
            {pinataResult && (
              <div>
                IPFS CID:{" "}
                <a
                  href={pinataResult.gatewayUrl}
                  target="_blank"
                  rel="noreferrer"
                  style={{ color: "var(--accent-500)" }}
                >
                  {pinataResult.cid.slice(0, 16)}...
                </a>
              </div>
            )}
            {pinataResult && <div>Batch size: {pinataResult.batchSizeBytes} bytes</div>}
            {pinataResult && <div>Event count: {pinataResult.eventCount}</div>}
          </div>
        </SectionCard>
      )}

      {/* Error display */}
      {error && (
        <div style={errorBannerStyle}>
          {error}
        </div>
      )}
    </div>
  );
}

// ─── Sub-components ───────────────────────────────────────────────────

function DevModeBadge() {
  return (
    <span
      title="Dev Mode uses Stellar testnet with auto-funded accounts. Switch to mainnet in Settings."
      style={{
        fontSize: "10px",
        fontWeight: 600,
        padding: "2px 8px",
        borderRadius: "var(--radius-sm)",
        background: "var(--accent-500)",
        color: "#fff",
        whiteSpace: "nowrap",
      }}
    >
      Dev Mode · Testnet
    </span>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <span style={{ display: "inline-flex", gap: "4px", alignItems: "baseline" }}>
      <span style={{ fontSize: "10px", color: "var(--ink-faint)" }}>{label}</span>
      <span style={{ fontSize: "11px", fontWeight: 600, fontFamily: "var(--font-mono)" }}>{value}</span>
    </span>
  );
}

function ProgressBar({ current, max }: { current: number; max: number }) {
  const pct = Math.min(100, (current / max) * 100);
  return (
    <div
      style={{
        height: "4px",
        background: "var(--surface-1)",
        borderRadius: "2px",
        overflow: "hidden",
      }}
    >
      <div
        style={{
          width: `${pct}%`,
          height: "100%",
          background: "var(--accent-500)",
          transition: "width 0.3s ease",
        }}
      />
    </div>
  );
}

function CommitSuccessView({
  result,
  pinataResult,
}: {
  result: CommitResult;
  pinataResult: IpfsPublishResult | null;
}) {
  return (
    <div
      style={{
        padding: "10px",
        background: "var(--surface-1)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
        fontSize: "11px",
        fontFamily: "var(--font-mono)",
        lineHeight: 1.6,
      }}
    >
      <div style={{ color: "var(--ink)", fontWeight: 600, marginBottom: "4px" }}>
        ✓ Epoch committed to Stellar testnet
      </div>
      <div>
        Tx:{" "}
        <a
          href={`https://stellar.expert/explorer/testnet/tx/${result.txHash}`}
          target="_blank"
          rel="noreferrer"
          style={{ color: "var(--accent-500)" }}
        >
          {result.txHash.slice(0, 12)}...{result.txHash.slice(-8)}
        </a>
      </div>
      {pinataResult && (
        <div>
          IPFS:{" "}
          <a
            href={pinataResult.gatewayUrl}
            target="_blank"
            rel="noreferrer"
            style={{ color: "var(--accent-500)" }}
          >
            {pinataResult.cid.slice(0, 16)}...
          </a>
        </div>
      )}
      <div>Sequence: {result.sequence}</div>
    </div>
  );
}

function VerifyResultView({ report }: { report: VerificationReport }) {
  const isMatch = report.chainIntact && !report.tamperDetected;
  return (
    <div
      style={{
        padding: "8px 10px",
        background: "var(--surface-1)",
        border: `1px solid ${isMatch ? "var(--accent-500)" : "var(--danger-500, #c00)"}`,
        borderRadius: "var(--radius-sm)",
        fontSize: "11px",
        fontFamily: "var(--font-mono)",
      }}
    >
      <div style={{ fontWeight: 600, color: isMatch ? "var(--accent-500)" : "var(--danger-500, #c00)" }}>
        {isMatch ? "✓ Audit log verified" : "✗ Verification failed"}
      </div>
      <div style={{ color: "var(--ink-faint)", marginTop: "4px" }}>{report.summary}</div>
      <div style={{ color: "var(--ink-faint)", marginTop: "4px" }}>
        Events verified: {report.verifiedEvents} / {report.totalEvents}
      </div>
    </div>
  );
}

function EventFeed({
  events,
  proofIndex,
  proofLoading,
  proofResult,
  onGenerateProof,
}: {
  events: AuditEvent[];
  proofIndex: number | null;
  proofLoading: boolean;
  proofResult: string | null;
  onGenerateProof: (index: number) => void;
}) {
  if (events.length === 0) {
    return (
      <div style={{ fontSize: "11px", color: "var(--ink-faint)", padding: "16px", textAlign: "center" }}>
        No events yet. Write data to MongoDB to see audit events appear here.
      </div>
    );
  }

  // Show most recent first, limit to 50 for performance.
  const recent = [...events].reverse().slice(0, 50);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "2px", maxHeight: "300px", overflowY: "auto" }}>
      {recent.map((event) => (
        <div
          key={event.index}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "8px",
            padding: "4px 8px",
            fontSize: "11px",
            fontFamily: "var(--font-mono)",
            borderBottom: "1px solid var(--border)",
          }}
        >
          <span style={{ color: "var(--ink-faint)", minWidth: "32px" }}>#{event.index}</span>
          <span
            style={{
              color: event.operation === "delete" ? "var(--danger-500, #c00)" : "var(--ink)",
              minWidth: "48px",
            }}
          >
            {event.operation}
          </span>
          <span style={{ color: "var(--ink-faint)", flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
            {event.database}.{event.collection}
          </span>
          <span style={{ color: "var(--ink-faint)", fontSize: "10px" }}>{event.timestamp}</span>
          <button
            onClick={() => onGenerateProof(event.index)}
            disabled={proofLoading && proofIndex === event.index}
            style={{
              fontSize: "10px",
              padding: "2px 6px",
              background: "transparent",
              color: "var(--accent-500)",
              border: "1px solid var(--border-strong)",
              borderRadius: "var(--radius-sm)",
              cursor: "pointer",
              whiteSpace: "nowrap",
            }}
          >
            {proofLoading && proofIndex === event.index ? "..." : "Proof"}
          </button>
        </div>
      ))}
      {proofResult && proofIndex !== null && (
        <pre
          style={{
            margin: "8px 0 0",
            padding: "8px",
            fontSize: "10px",
            fontFamily: "var(--font-mono)",
            background: "var(--surface-1)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-sm)",
            overflow: "auto",
            maxHeight: "120px",
          }}
        >
          {proofResult}
        </pre>
      )}
    </div>
  );
}

function SectionCard({ children }: { children: ReactNode }) {
  return (
    <div
      style={{
        padding: "12px",
        background: "var(--surface-2)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
      }}
    >
      {children}
    </div>
  );
}

// ─── Shared styles ────────────────────────────────────────────────────

const btnPrimaryStyle: CSSProperties = {
  padding: "4px 12px",
  fontSize: "11px",
  fontFamily: "var(--font-sans)",
  fontWeight: 600,
  cursor: "pointer",
  background: "var(--accent-500)",
  color: "#fff",
  border: "none",
  borderRadius: "var(--radius-sm)",
  whiteSpace: "nowrap",
};

const btnSecondaryStyle: CSSProperties = {
  padding: "4px 12px",
  fontSize: "11px",
  fontFamily: "var(--font-sans)",
  cursor: "pointer",
  background: "transparent",
  color: "var(--ink)",
  border: "1px solid var(--border-strong)",
  borderRadius: "var(--radius-sm)",
  whiteSpace: "nowrap",
};

const errorBannerStyle: CSSProperties = {
  padding: "8px 12px",
  fontSize: "11px",
  color: "var(--danger-500, #c00)",
  background: "var(--surface-2)",
  border: "1px solid var(--danger-500, #c00)",
  borderRadius: "var(--radius-sm)",
  fontFamily: "var(--font-mono)",
};
