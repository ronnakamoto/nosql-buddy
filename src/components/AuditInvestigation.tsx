import { useCallback } from "react";
import type {
  OnChainRoot,
  VerificationReport,
  ProofResult,
  OplogIntegrityReport,
} from "../ipc/commands";
import { Alert, Badge, Button, StatusCard } from "./AuditUi";
import { MerklePathViz } from "./MerklePathViz";

/**
 * AuditInvestigation — the forensic toolkit section.
 *
 * Contents:
 *   1. Root comparison  — local root vs on-chain root, side-by-side with match indicator
 *   2. Proof inspector  — MerklePathViz visualization when an event proof is selected
 *   3. Proof export     — download the proof as a standalone JSON file
 *   4. Tamper timeline  — chronological list of verification results
 *   5. Oplog check      — when a MongoDB connection is available
 */

// ─── Helpers ─────────────────────────────────────────────────────────────────

function formatTs(ts: number | string | null): string {
  if (!ts) return "—";
  const d = typeof ts === "number" ? new Date(ts * 1000) : new Date(ts);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

function shortHash(h: string | null | undefined): string {
  if (!h) return "—";
  return h.length > 20 ? `${h.slice(0, 10)}…${h.slice(-8)}` : h;
}

// ─── Root comparison ─────────────────────────────────────────────────────────

function RootCompare({
  localRoot,
  onchainRoot,
}: {
  localRoot: string;
  onchainRoot: OnChainRoot | null;
}) {
  const match = onchainRoot && localRoot && onchainRoot.rootHex === localRoot;
  const hasBoth = !!(onchainRoot && localRoot);

  return (
    <div className="audit-root-compare">
      <div className="audit-root-compare__row">
        <span className="audit-root-compare__label">Local root</span>
        <span className="audit-root-compare__hash" title={localRoot}>
          {shortHash(localRoot)}
        </span>
        <span className="audit-root-compare__full">{localRoot || "—"}</span>
      </div>
      <div className="audit-root-compare__row">
        <span className="audit-root-compare__label">On-chain root</span>
        <span className="audit-root-compare__hash" title={onchainRoot?.rootHex}>
          {shortHash(onchainRoot?.rootHex)}
        </span>
        <span className="audit-root-compare__full">{onchainRoot?.rootHex || "Nothing anchored yet"}</span>
        {hasBoth && (
          <Badge tone={match ? "success" : "danger"}>
            {match ? "✓ match" : "✗ mismatch"}
          </Badge>
        )}
      </div>
      {onchainRoot && (
        <div className="audit-root-compare__row audit-root-compare__row--meta">
          <span className="audit-root-compare__label">Anchored</span>
          <span className="audit-root-compare__meta">
            Sequence #{onchainRoot.sequence} · {formatTs(onchainRoot.timestamp)}
          </span>
        </div>
      )}
    </div>
  );
}

// ─── Tamper timeline ─────────────────────────────────────────────────────────

export interface VerificationRecord {
  runAt: number; // Date.now() when verification was run
  report: VerificationReport;
}

function TamperTimeline({ history }: { history: VerificationRecord[] }) {
  if (history.length === 0) {
    return (
      <div className="audit-tamper-timeline audit-tamper-timeline--empty">
        No verification runs recorded yet. Click Verify Integrity in the Status section.
      </div>
    );
  }

  return (
    <div className="audit-tamper-timeline">
      {[...history].reverse().map((entry, i) => {
        const ok = !entry.report.tamperDetected && entry.report.chainIntact;
        return (
          <div key={i} className={`audit-timeline-entry ${ok ? "audit-timeline-entry--ok" : "audit-timeline-entry--fail"}`}>
            <span className="audit-timeline-entry__icon">{ok ? "✓" : "✗"}</span>
            <span className="audit-timeline-entry__time">
              {new Date(entry.runAt).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" })}
            </span>
            <span className="audit-timeline-entry__summary">{entry.report.summary}</span>
          </div>
        );
      })}
    </div>
  );
}

// ─── Proof inspector ─────────────────────────────────────────────────────────

function ProofInspector({
  proof,
  onExport,
}: {
  proof: ProofResult;
  onExport: () => void;
}) {
  return (
    <div className="audit-proof-inspector">
      <div className="audit-proof-inspector__header">
        <span className="audit-proof-inspector__title">
          Proof for event #{proof.leafIndex}
        </span>
        <div className="audit-proof-inspector__actions">
          <Button variant="ghost" onClick={onExport} style={{ fontSize: "var(--font-size-xs)" }}>
            Export JSON
          </Button>
        </div>
      </div>
      <MerklePathViz proof={proof} />
      <div className="audit-proof-inspector__fields">
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Root</span>
          <span className="audit-proof-inspector__mono">{proof.rootHex}</span>
        </div>
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Leaf index</span>
          <span className="audit-proof-inspector__mono">{proof.leafIndex}</span>
        </div>
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Proof (a)</span>
          <span className="audit-proof-inspector__mono">{shortHash(proof.proof.a)}</span>
        </div>
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Proof (b)</span>
          <span className="audit-proof-inspector__mono">{shortHash(proof.proof.b)}</span>
        </div>
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Proof (c)</span>
          <span className="audit-proof-inspector__mono">{shortHash(proof.proof.c)}</span>
        </div>
        <div className="audit-proof-inspector__field">
          <span className="audit-proof-inspector__label">Network</span>
          <span className="audit-proof-inspector__mono">{proof.network}</span>
        </div>
      </div>
    </div>
  );
}

// ─── Main component ───────────────────────────────────────────────────────────

export interface AuditInvestigationProps {
  localRoot: string;
  onchainRoot: OnChainRoot | null;
  network: "testnet" | "mainnet";

  // Current proof (from change feed row click)
  currentProof: ProofResult | null;

  // Verification history
  verificationHistory: VerificationRecord[];

  // Oplog (optional — only when a MongoDB connection is active)
  connectionId?: string | null;
  oplogReport: OplogIntegrityReport | null;
  oplogLoading: boolean;
  onOplogVerify: () => void;
}

export function AuditInvestigation({
  localRoot,
  onchainRoot,
  currentProof,
  verificationHistory,
  connectionId,
  oplogReport,
  oplogLoading,
  onOplogVerify,
}: AuditInvestigationProps) {
  // ─── Proof export ────────────────────────────────────────────────────────
  const handleExportProof = useCallback(() => {
    if (!currentProof) return;
    const json = JSON.stringify(currentProof, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `proof-event-${currentProof.leafIndex}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, [currentProof]);

  const oplogStatus: "good" | "warning" | "neutral" = oplogReport
    ? oplogReport.allMatch
      ? "good"
      : oplogReport.verdict === "incomplete" ||
        oplogReport.verdict === "no_commitment" ||
        oplogReport.verdict === "no_oplog_commitment"
        ? "neutral"
        : "warning"
    : "neutral";

  const latestVerification = verificationHistory[verificationHistory.length - 1] ?? null;
  const tamperDetected = latestVerification?.report.tamperDetected ?? false;

  return (
    <div className="audit-investigation">
      {tamperDetected && (
        <Alert tone="danger">
          Tamper detected — the local root does not match the on-chain commitment.
          The audit log may have been modified after the last commit.
        </Alert>
      )}

      {/* 1. Root comparison */}
      <div className="audit-investigation__block">
        <div className="audit-investigation__block-title">Root comparison</div>
        <RootCompare localRoot={localRoot} onchainRoot={onchainRoot} />
      </div>

      {/* 2. Proof inspector (shown when a proof has been generated) */}
      {currentProof && (
        <div className="audit-investigation__block">
          <div className="audit-investigation__block-title">Proof inspector</div>
          <ProofInspector proof={currentProof} onExport={handleExportProof} />
        </div>
      )}

      {/* 3. Tamper timeline */}
      <div className="audit-investigation__block">
        <div className="audit-investigation__block-title">Verification history</div>
        <TamperTimeline history={verificationHistory} />
      </div>

      {/* 4. Oplog check */}
      {connectionId && (
        <div className="audit-investigation__block">
          <StatusCard
            title="Oplog completeness"
            status={oplogStatus}
            value={
              oplogReport
                ? oplogReport.allMatch ? "Match"
                  : oplogReport.verdict === "no_commitment" ? "No commitment"
                  : oplogReport.verdict === "no_oplog_commitment" ? "No oplog hash"
                  : oplogReport.verdict === "stale" ? "Stale"
                  : oplogReport.verdict === "complete" ? "Verified"
                  : "Mismatch"
                : "—"
            }
            detail={
              oplogReport
                ? oplogReport.verdict === "complete"
                  ? `${oplogReport.oplogEntryCount ?? 0} oplog entries verified`
                  : oplogReport.explanation
                : "Not checked — click Verify to compare MongoDB oplog with on-chain commitment"
            }
            action={
              <Button variant="ghost" loading={oplogLoading} onClick={onOplogVerify}>
                Verify
              </Button>
            }
          />
        </div>
      )}
    </div>
  );
}
