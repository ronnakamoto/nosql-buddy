import type {
  AuditStatus,
  Epoch,
  OnChainRoot,
  VerificationReport,
  CommitResult,
  IpfsPublishResult,
  OnChainAttestationVerification,
} from "../ipc/commands";
import {
  Alert,
  Badge,
  Button,
  IpfsCidLink,
  KeyValue,
  Spinner,
  TxHashLink,
} from "./AuditUi";
import type { HealthState } from "./AuditHeader";
import { InfoPopover } from "./InfoPopover";
import {
  Anchor,
  ArrowRight,
  Check,
  CircleCheckBig,
  CircleDashed,
  Layers,
  Lock,
  ShieldAlert,
  ShieldCheck,
  ShieldQuestion,
} from "lucide-react";

const ROW_ICON = 15;

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
  /** Independent on-chain K-of-N attestation verdict from the contract. */
  onchainAttestation: OnChainAttestationVerification | null;

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
  onchainAttestation,
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

  const tamperDetected = health === "tamper";

  // Three stages mirror the operator's task flow: capture, seal, anchor.
  const writeDone = epochEventCount > 0 || (status?.eventCount ?? 0) > 0;
  const committed = !!currentEpoch?.committed || !!commitResult;
  const sealDone = epochClosed || !!lastClosedEpoch || committed;
  const sealActive = writeDone && !sealDone;
  const commitActive = sealDone && !committed;

  let nextStep: string | null = null;
  if (tamperDetected) {
    nextStep = "Open Investigation to compare local and on-chain roots.";
  } else if (!writeDone) {
    nextStep = "Write to the audited MongoDB endpoint to capture changes.";
  } else if (sealActive && canCloseEpoch) {
    nextStep = "Seal the current batch to lock its fingerprint.";
  } else if (commitActive && lastClosedEpoch) {
    nextStep = `Commit batch #${lastClosedEpoch.epochNumber} to anchor it on-chain.`;
  }

  // Integrity row
  let integrityContent: React.ReactNode;
  if (health === "tamper") {
    integrityContent = (
      <span className="audit-status-row__value audit-status-row__value--danger">
        <ShieldAlert size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Tamper detected
      </span>
    );
  } else if (health === "healthy") {
    integrityContent = (
      <span className="audit-status-row__value audit-status-row__value--success">
        <ShieldCheck size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Verified
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
        <ShieldQuestion size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Not verified
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
    <span className="audit-status-row__value audit-status-row__value--muted">
      <CircleDashed size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
      Not started
    </span>
  );

  // Batch row
  let batchContent: React.ReactNode;
  if (!currentEpoch) {
    batchContent = (
      <span className="audit-status-row__value audit-status-row__value--muted">
        <CircleDashed size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        No batch yet
      </span>
    );
  } else if (currentEpoch.committed) {
    batchContent = (
      <span className="audit-status-row__value audit-status-row__value--success">
        <CircleCheckBig size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Batch #{currentEpoch.epochNumber} · committed · {epochEventCount} events
      </span>
    );
  } else if (epochClosed) {
    batchContent = (
      <span className="audit-status-row__value">
        <Lock size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Batch #{currentEpoch.epochNumber} · sealed · {epochEventCount}/{EPOCH_THRESHOLD}
      </span>
    );
  } else {
    batchContent = (
      <span className="audit-status-row__value audit-status-row__value--top">
        <Layers size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        <div className="audit-status-row__batch">
          <span className="audit-status-row__batch-line">
            Batch #{currentEpoch.epochNumber} · filling
            <span className="audit-status-row__batch-count">{epochEventCount}/{EPOCH_THRESHOLD}</span>
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
      </span>
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
        <Anchor size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Sequence #{onchainRoot.sequence} anchored
        <span className="audit-status-row__detail">
          {formatTs(onchainRoot.timestamp)} · <TxHashLink txHash={onchainRoot.rootHex.slice(0, 20)} network={network} showExternalIcon={false} />
        </span>
      </span>
    );
  } else {
    onchainContent = (
      <span className="audit-status-row__value audit-status-row__value--muted">
        <CircleDashed size={ROW_ICON} className="audit-status-row__icon" aria-hidden="true" />
        Nothing anchored yet
      </span>
    );
  }

  return (
    <div className="audit-section">
      {tamperDetected && (
        <Alert tone="danger">
          {verifyReport?.summary
            ? `${verifyReport.summary} Open the Investigation section below to inspect the discrepancy.`
            : "The local audit log does not match the on-chain commitment. Open the Investigation section below to inspect the discrepancy."}
        </Alert>
      )}

      {/* Workflow stepper — capture → seal → anchor */}
      <div className="audit-status-workflow" aria-label="Audit workflow progress">
        <div className={`audit-step ${writeDone ? "audit-step--done" : "audit-step--active"}`}>
          <span className="audit-step__num">{writeDone ? <Check size={13} aria-hidden="true" /> : "1"}</span>
          <span className="audit-step__label">Write Data</span>
        </div>
        <div className={`audit-step ${sealDone ? "audit-step--done" : sealActive ? "audit-step--active" : ""}`}>
          <span className="audit-step__num">{sealDone ? <Check size={13} aria-hidden="true" /> : "2"}</span>
          <span className="audit-step__label">Seal Batch</span>
        </div>
        <div className={`audit-step ${committed ? "audit-step--done" : commitActive ? "audit-step--active" : ""}`}>
          <span className="audit-step__num">{committed ? <Check size={13} aria-hidden="true" /> : "3"}</span>
          <span className="audit-step__label">Commit to Chain</span>
        </div>
      </div>

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
        <InfoPopover label="Help: Verify integrity" title="Verify integrity"><p>Recomputes the audit log Merkle root and compares it against the on-chain commitment. Run this to detect if local data was tampered with.</p></InfoPopover>

        {/* Only show Seal if not already sealed */}
        {!epochClosed && (
          <>
            <Button
              variant="secondary"
              loading={closeEpochLoading}
              disabled={!canCloseEpoch}
              onClick={onCloseEpoch}
              title={closeEpochDisabledReason ?? "Seal the current batch so it can be committed"}
            >
              Seal Batch
            </Button>
            <InfoPopover label="Help: Seal batch" title="Seal batch"><p>Freezes the current batch of events and computes its cryptographic fingerprint (Merkle root). Sealed batches can no longer accept new events.</p></InfoPopover>
          </>
        )}

        {/* Only show Commit if there's a sealed batch waiting */}
        {lastClosedEpoch && (
          <>
            <Button
              variant="primary"
              loading={commitLoading}
              disabled={!canCommit}
              onClick={onCommit}
              title={commitDisabledReason ?? `Commit batch #${lastClosedEpoch.epochNumber} to Stellar`}
            >
              {commitLoading ? "Committing…" : `Commit Batch #${lastClosedEpoch.epochNumber}`}
            </Button>
            <InfoPopover label="Help: Commit to blockchain" title="Commit to blockchain"><p>Publishes the sealed batch fingerprint to IPFS and anchors it on the Stellar blockchain. This creates a permanent, verifiable record.</p></InfoPopover>
          </>
        )}
      </div>

      {/* Next-step guidance — one clear instruction, not an error */}
      {nextStep && !commitLoading && (
        <div className="audit-status-next">
          <ArrowRight size={13} className="audit-status-next__icon" aria-hidden="true" />
          <span><span className="audit-status-next__lead">Next:</span> {nextStep}</span>
        </div>
      )}

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
          {pinataResult && (
            <KeyValue
              label="IPFS CID"
              value={<IpfsCidLink cid={pinataResult.cid} gatewayUrl={pinataResult.gatewayUrl} encrypted={pinataResult.encrypted} />}
            />
          )}
        </div>
      )}

      {/* Independent on-chain K-of-N attestation verdict */}
      {onchainAttestation && !commitLoading && (
        <div className="audit-status-commit-result" style={{ animation: "audit-fade-in 0.2s ease" }}>
          <Badge tone={onchainAttestation.verdict === "verified" ? "success" : "warning"} dot>
            {onchainAttestation.verdict === "verified"
              ? "Independently verified"
              : onchainAttestation.verdict === "no_attestations"
                ? "Awaiting attesters"
                : onchainAttestation.verdict === "threshold_not_met"
                  ? "Threshold not met"
                  : "Attestation issue"}
            {` · ${onchainAttestation.authorizedCount}/${onchainAttestation.threshold}`}
          </Badge>
          <span className="audit-status-row__detail">
            {onchainAttestation.authorizedCount} of {onchainAttestation.attestationCount} attestation(s)
            from authorized auditor(s) · K={onchainAttestation.threshold} required
          </span>
          <InfoPopover label="Help: On-chain attestation" title="On-chain attestation">
            <p>
              This verdict is computed by the Soroban contract, not by the operator's
              app. The contract counts how many <strong>distinct, currently-authorized</strong>
              auditors signed the exact oplog root with their own ed25519 key, and compares
              that count against the on-chain K-of-N threshold. The operator cannot produce
              a "verified" verdict alone.
            </p>
            <p>
              {onchainAttestation.verdict === "verified"
                ? "The threshold is met. The committed root is independently certified."
                : onchainAttestation.verdict === "no_attestations"
                  ? "No auditor has attested this batch yet. The commit is anchored but not independently certified."
                  : "Not enough independent auditor signatures yet. Ask your auditor(s) to run attest_oplog for this sequence."}
            </p>
          </InfoPopover>
        </div>
      )}
    </div>
  );
}
