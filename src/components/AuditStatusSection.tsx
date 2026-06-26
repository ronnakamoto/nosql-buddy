import type {
  AuditStatus,
  Epoch,
  OnChainRoot,
  VerificationReport,
  CommitResult,
  IpfsPublishResult,
} from "../ipc/commands";
import {
  Alert,
  Badge,
  Button,
  KeyValue,
  Spinner,
  TxHashLink,
} from "./AuditUi";
import type { HealthState } from "./AuditHeader";

/**
 * AuditStatusSection — the always-expanded operational summary.
 *
 * Replaces the current scattered cards (status bar + epoch card + commit card +
 * on-chain card) with a single cohesive block.
 *
 * Four information rows:
 *   Integrity · Capture · Batch · On-chain
 *
 * Followed by primary actions: [Verify] [Seal Batch] [Commit]
 * Only actions that make sense in the current state are shown.
 */

const EPOCH_THRESHOLD = 100;

function formatTs(ts: string | number | null): string {
  if (!ts) return "";
  const d = typeof ts === "number" ? new Date(ts * 1000) : new Date(ts);
  if (isNaN(d.getTime())) return "";
  const now = Date.now();
  const diff = Math.floor((now - d.getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
}

function staleBatchWarning(staleBatchMs: number | null): string | null {
  if (staleBatchMs === null) return null;
  const hours = staleBatchMs / 3_600_000;
  if (hours < 2) return null;
  if (hours < 24) return `Filling for ${Math.floor(hours)}h — consider sealing`;
  return `Filling for ${Math.floor(hours / 24)}d — batch may be stale`;
}

export interface AuditStatusSectionProps {
  health: HealthState;
  status: AuditStatus | null;
  currentEpoch: Epoch | null;
  onchainRoot: OnChainRoot | null;
  pollingOnchain: boolean;
  network: "testnet" | "mainnet";

  // Verification
  verifyLoading: boolean;
  verifyReport: VerificationReport | null;
  onVerify: () => void;

  // Seal
  closeEpochLoading: boolean;
  onCloseEpoch: () => void;

  // Commit
  commitLoading: boolean;
  commitStep: string;
  commitResult: CommitResult | null;
  pinataResult: IpfsPublishResult | null;
  onCommit: () => void;

  // Derived
  canCloseEpoch: boolean;
  closeEpochDisabledReason: string | null;
  canCommit: boolean;
  commitDisabledReason: string | null;
  lastClosedEpoch: Epoch | null;
  /** Milliseconds the current epoch has been filling (null if N/A). */
  staleBatchMs: number | null;
}

export function AuditStatusSection({
  health,
  status,
  currentEpoch,
  onchainRoot,
  pollingOnchain,
  network,
  verifyLoading,
  verifyReport,
  onVerify,
  closeEpochLoading,
  onCloseEpoch,
  commitLoading,
  commitStep,
  commitResult,
  pinataResult,
  onCommit,
  canCloseEpoch,
  closeEpochDisabledReason,
  canCommit,
  commitDisabledReason,
  lastClosedEpoch,
  staleBatchMs,
}: AuditStatusSectionProps) {
  const epochEventCount = currentEpoch?.eventCount ?? 0;
  const epochClosed = currentEpoch?.endIndex !== null && currentEpoch?.endIndex !== undefined;
  const epochPct = EPOCH_THRESHOLD > 0 ? Math.min(100, (epochEventCount / EPOCH_THRESHOLD) * 100) : 0;
  const staleWarning = staleBatchWarning(staleBatchMs);

  // Integrity row
  let integrityContent: React.ReactNode;
  if (health === "tamper") {
    integrityContent = (
      <span className="audit-status-row__value audit-status-row__value--danger">
        ✗ Tamper detected
        {verifyReport && <span className="audit-status-row__detail">{verifyReport.summary}</span>}
      </span>
    );
  } else if (health === "healthy") {
    integrityContent = (
      <span className="audit-status-row__value audit-status-row__value--success">
        ✓ Verified
        {verifyReport && (
          <span className="audit-status-row__detail">
            {verifyReport.verifiedEvents} events · chain intact
          </span>
        )}
      </span>
    );
  } else {
    integrityContent = (
      <span className="audit-status-row__value audit-status-row__value--muted">
        ⚠ Not verified
        <span className="audit-status-row__detail">Run Verify Integrity to check</span>
      </span>
    );
  }

  // Capture row
  const captureContent = status ? (
    <span className="audit-status-row__value">
      <span className="audit-status-dot audit-status-dot--live" />
      Live · {status.eventCount} events · {status.leafCount} leaves
    </span>
  ) : (
    <span className="audit-status-row__value audit-status-row__value--muted">○ Not started</span>
  );

  // Batch row
  let batchContent: React.ReactNode;
  if (!currentEpoch) {
    batchContent = <span className="audit-status-row__value audit-status-row__value--muted">No batch yet</span>;
  } else if (currentEpoch.committed) {
    batchContent = (
      <span className="audit-status-row__value audit-status-row__value--success">
        Batch #{currentEpoch.epochNumber} · committed · {epochEventCount} events
      </span>
    );
  } else if (epochClosed) {
    batchContent = (
      <span className="audit-status-row__value">
        Batch #{currentEpoch.epochNumber} · sealed · {epochEventCount}/{EPOCH_THRESHOLD}
      </span>
    );
  } else {
    batchContent = (
      <div className="audit-status-row__batch">
        <span className="audit-status-row__value">
          Batch #{currentEpoch.epochNumber} · filling · {epochEventCount}/{EPOCH_THRESHOLD}
        </span>
        <div className="audit-status-mini-bar">
          <div
            className="audit-status-mini-bar__fill"
            style={{ width: `${Math.max(epochPct, 1)}%` }}
          />
        </div>
        {staleWarning && (
          <span className="audit-status-row__detail audit-status-row__detail--warn">{staleWarning}</span>
        )}
      </div>
    );
  }

  // On-chain row
  let onchainContent: React.ReactNode;
  if (pollingOnchain) {
    onchainContent = (
      <span className="audit-status-row__value audit-status-row__value--muted">
        <Spinner size={12} /> Waiting for Stellar confirmation…
      </span>
    );
  } else if (onchainRoot) {
    onchainContent = (
      <span className="audit-status-row__value">
        Sequence #{onchainRoot.sequence} anchored
        <span className="audit-status-row__detail">
          {formatTs(onchainRoot.timestamp)} · <TxHashLink txHash={onchainRoot.rootHex.slice(0, 20)} network={network} showExternalIcon={false} />
        </span>
      </span>
    );
  } else {
    onchainContent = (
      <span className="audit-status-row__value audit-status-row__value--muted">Nothing anchored yet</span>
    );
  }

  const tamperDetected = health === "tamper";

  return (
    <div className="audit-section">
      {tamperDetected && (
        <Alert tone="danger">
          Tamper detected — the local audit log does not match the on-chain commitment.
          Expand the Investigation section below to inspect the discrepancy.
        </Alert>
      )}

      {/* Four status rows */}
      <div className="audit-status-grid">
        <div className="audit-status-row">
          <span className="audit-status-row__label">Integrity</span>
          {integrityContent}
        </div>
        <div className="audit-status-row">
          <span className="audit-status-row__label">Capture</span>
          {captureContent}
        </div>
        <div className="audit-status-row">
          <span className="audit-status-row__label">Batch</span>
          {batchContent}
        </div>
        <div className="audit-status-row">
          <span className="audit-status-row__label">On-chain</span>
          {onchainContent}
        </div>
      </div>

      {/* Primary actions */}
      <div className="audit-status-actions">
        <Button
          variant="secondary"
          loading={verifyLoading}
          onClick={onVerify}
          title="Verify the integrity of the local audit log against the on-chain commitment"
        >
          Verify Integrity
        </Button>

        {/* Only show Seal if not already sealed */}
        {!epochClosed && (
          <Button
            variant="secondary"
            loading={closeEpochLoading}
            disabled={!canCloseEpoch}
            onClick={onCloseEpoch}
            title={closeEpochDisabledReason ?? "Seal the current batch so it can be committed"}
          >
            Seal Batch
          </Button>
        )}

        {/* Only show Commit if there's a sealed batch waiting */}
        {lastClosedEpoch && (
          <Button
            variant="primary"
            loading={commitLoading}
            disabled={!canCommit}
            onClick={onCommit}
            title={commitDisabledReason ?? `Commit batch #${lastClosedEpoch.epochNumber} to Stellar`}
          >
            {commitLoading ? "Committing…" : `Commit Batch #${lastClosedEpoch.epochNumber}`}
          </Button>
        )}
      </div>

      {/* Commit progress feedback */}
      {commitLoading && commitStep && (
        <div className="audit-status-commit-step">
          <Spinner size={12} />
          <span>{commitStep}</span>
        </div>
      )}

      {/* Commit result */}
      {commitResult && !commitLoading && (
        <div className="audit-status-commit-result" style={{ animation: "audit-fade-in 0.2s ease" }}>
          <Badge tone="success" dot>Committed</Badge>
          <KeyValue label="Tx hash" value={<TxHashLink txHash={commitResult.txHash} network={network} />} />
          {pinataResult && <KeyValue label="IPFS CID" value={pinataResult.cid} />}
        </div>
      )}

      {/* Verify result */}
      {verifyReport && !verifyLoading && (
        <Alert tone={!verifyReport.tamperDetected && verifyReport.chainIntact ? "success" : "danger"}>
          {!verifyReport.tamperDetected && verifyReport.chainIntact
            ? `✓ Integrity verified — ${verifyReport.verifiedEvents} events, chain intact.`
            : `✗ Tamper detected — ${verifyReport.summary}`}
        </Alert>
      )}

      {/* Disabled action hints */}
      {closeEpochDisabledReason && !epochClosed && (
        <div className="audit-status-hint">{closeEpochDisabledReason}</div>
      )}
      {commitDisabledReason && !lastClosedEpoch && (
        <div className="audit-status-hint">{commitDisabledReason}</div>
      )}
    </div>
  );
}
