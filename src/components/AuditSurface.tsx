import { useMemo, useState, useEffect, useCallback, useRef } from "react";
import commands, {
  type AuditStatus,
  type AuditEvent,
  type CommitResult,
  type OnChainRoot,
  type IpfsPublishResult,
  type VerificationReport,
  type Epoch,
  type AuditModeConfig,
  type OplogIntegrityReport,
  type ProofResult,
  type VerificationRecord,
  formatError,
} from "../ipc/commands";
import { Alert } from "./AuditUi";
import { AuditHeader, type HealthState } from "./AuditHeader";
import { AuditStatusSection } from "./AuditStatusSection";
import { AuditChangeFeed } from "./AuditChangeFeed";
import { AuditBatchHistory } from "./AuditBatchHistory";
import { AuditInvestigation } from "./AuditInvestigation";

/**
 * AuditSurface — the unified, adaptive audit interface.
 *
 * Replaces the AuditModeChooser → DevFlow/ProductionFlow → AuditLiveViewV2 chain.
 * A single scrollable surface where sections expand/collapse based on system state.
 *
 * Sections:
 *   1. Header         — always visible (health dot, mode badge, event count, gear)
 *   2. Status         — always expanded (integrity, capture, batch, on-chain, actions)
 *   3. Change Feed    — expanded when capturing, collapsible
 *   4. Batch History  — expanded when batches exist, collapsible
 *   5. Investigation  — collapsed by default, auto-expands on tamper
 *   6. Advanced       — collapsed by default
 */

const POLL_INTERVAL_MS = 2000;

export interface AuditSurfaceProps {
  config: AuditModeConfig;
  connectionId?: string | null;
  onShowSettings: () => void;
}

export function AuditSurface({ config, connectionId, onShowSettings }: AuditSurfaceProps) {
  // ─── Core data ────────────────────────────────────────────────────────
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [epochs, setEpochs] = useState<Epoch[]>([]);
  const [currentEpoch, setCurrentEpoch] = useState<Epoch | null>(null);
  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);

  // ─── Operation loading states ─────────────────────────────────────────
  const [closeEpochLoading, setCloseEpochLoading] = useState(false);
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [pinataResult, setPinataResult] = useState<IpfsPublishResult | null>(null);
  const [commitStep, setCommitStep] = useState("");
  const [pollingOnchain, setPollingOnchain] = useState(false);
  const [verifyLoading, setVerifyLoading] = useState(false);
  const [verifyReport, setVerifyReport] = useState<VerificationReport | null>(null);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [proofResult, setProofResult] = useState<string | null>(null);
  const [currentProof, setCurrentProof] = useState<ProofResult | null>(null);

  // ─── Verification history (tamper timeline) ───────────────────────────
  const [verificationHistory, setVerificationHistory] = useState<VerificationRecord[]>([]);

  // Load persisted verification history once on mount so the tamper timeline
  // survives app restarts (the backend persists every reader-mode run).
  const refreshVerificationHistory = useCallback(async () => {
    try {
      const history = await commands.auditListVerificationHistory();
      setVerificationHistory(history);
      const latest = history[history.length - 1];
      if (latest) setVerifyReport(latest.report);
    } catch {
      // Non-fatal: history is auxiliary. Leave the timeline empty on failure.
    }
  }, []);

  useEffect(() => {
    refreshVerificationHistory();
  }, [refreshVerificationHistory]);

  // ─── Oplog ────────────────────────────────────────────────────────────
  const [oplogReport, setOplogReport] = useState<OplogIntegrityReport | null>(null);
  const [oplogLoading, setOplogLoading] = useState(false);

  // ─── Section collapse state ───────────────────────────────────────────
  const [feedCollapsed, setFeedCollapsed] = useState(false);
  const [historyCollapsed, setHistoryCollapsed] = useState(false);
  const [investigationCollapsed, setInvestigationCollapsed] = useState(true);
  const [advancedCollapsed, setAdvancedCollapsed] = useState(true);

  // ─── Stale batch tracking ─────────────────────────────────────────────
  // Record the wall-clock time when we first observe each epoch with >0 events.
  // Used to compute "filling for Xh" warnings without a server-side startedAt.
  const epochFirstSeenRef = useRef<Map<number, number>>(new Map());

  useEffect(() => {
    if (!currentEpoch || currentEpoch.eventCount === 0) return;
    const n = currentEpoch.epochNumber;
    if (!epochFirstSeenRef.current.has(n)) {
      epochFirstSeenRef.current.set(n, Date.now());
    }
  }, [currentEpoch]);

  const staleBatchMs = useMemo((): number | null => {
    if (!currentEpoch || currentEpoch.eventCount === 0) return null;
    if (currentEpoch.endIndex !== null && currentEpoch.endIndex !== undefined) return null;
    const firstSeen = epochFirstSeenRef.current.get(currentEpoch.epochNumber);
    if (!firstSeen) return null;
    return Date.now() - firstSeen;
  // Re-evaluate on every epoch poll (currentEpoch reference changes each poll).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentEpoch]);

  // ─── Error ────────────────────────────────────────────────────────────
  const [error, setError] = useState<string | null>(null);

  // ─── Polling ─────────────────────────────────────────────────────────
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

  // Poll on-chain root after commit until Stellar confirms (~5-10s).
  useEffect(() => {
    if (!pollingOnchain) return;
    let active = true;
    const id = setInterval(async () => {
      const root = await refreshOnchainRoot();
      if (root && active) setPollingOnchain(false);
    }, POLL_INTERVAL_MS);
    return () => { active = false; clearInterval(id); };
  }, [pollingOnchain, refreshOnchainRoot]);

  // ─── Derived state ────────────────────────────────────────────────────
  const epochEventCount = currentEpoch?.eventCount ?? 0;
  const epochClosed = currentEpoch?.endIndex !== null && currentEpoch?.endIndex !== undefined;

  const lastClosedEpoch = useMemo(() => {
    return (epochs ?? [])
      .filter((e) => e.endIndex !== null && e.endIndex !== undefined && !e.committed)
      .sort((a, b) => b.epochNumber - a.epochNumber)[0] ?? null;
  }, [epochs]);

  const canCloseEpoch = !!(currentEpoch && !epochClosed && epochEventCount > 0 && !closeEpochLoading);
  const closeEpochDisabledReason = !currentEpoch
    ? "No epoch data"
    : epochClosed
      ? "Epoch already sealed"
      : epochEventCount === 0
        ? "Write to MongoDB to capture events"
        : null;
  const canCommit = lastClosedEpoch !== null && !commitLoading;
  const commitDisabledReason = !lastClosedEpoch ? "Seal a batch first" : null;

  // ─── Health state derivation ──────────────────────────────────────────
  const health: HealthState = useMemo(() => {
    if (!status) return "idle";
    if (verifyReport?.tamperDetected) return "tamper";
    if (verifyReport && !verifyReport.tamperDetected && verifyReport.chainIntact) return "healthy";
    return "unverified";
  }, [status, verifyReport]);

  // Auto-expand investigation on tamper detection.
  useEffect(() => {
    if (health === "tamper") setInvestigationCollapsed(false);
  }, [health]);

  // ─── Network ─────────────────────────────────────────────────────────
  const network = config.network;

  // ─── Commit function (mode-aware) ─────────────────────────────────────
  const commitFn = useCallback(
    (metadata?: string): Promise<CommitResult> => {
      if (config.mode === "dev") {
        return commands.auditCommitRootNative(metadata, connectionId ?? undefined);
      }
      return commands.auditCommitRootProduction(metadata, connectionId ?? undefined);
    },
    [config.mode, connectionId],
  );

  // ─── Handlers ─────────────────────────────────────────────────────────
  const handleCloseEpoch = useCallback(async () => {
    if (!canCloseEpoch) return;
    setCloseEpochLoading(true);
    setError(null);
    try {
      await commands.auditCloseEpoch();
      await refresh();
    } catch (err) {
      setError(formatError(err));
    } finally {
      setCloseEpochLoading(false);
    }
  }, [canCloseEpoch, refresh]);

  const handleCommit = useCallback(async () => {
    if (!lastClosedEpoch) return;
    setCommitLoading(true);
    setError(null);
    setCommitResult(null);
    setPinataResult(null);
    setCommitStep("Pinning batch to IPFS via Pinata…");

    try {
      const pinata = await commands.auditPublishEpochToPinata(lastClosedEpoch.epochNumber);
      setPinataResult(pinata);
      setCommitStep("Submitting transaction to Stellar…");
      const result = await commitFn(`epoch=${lastClosedEpoch.epochNumber} cid=${pinata.cid}`);
      setCommitResult(result);
      setCommitStep("Confirmed!");
      await commands.auditMarkEpochCommitted(lastClosedEpoch.epochNumber, result.txHash);
      setPollingOnchain(true);
      refreshOnchainRoot();
      refresh();
    } catch (err) {
      setError(formatError(err));
      setCommitStep("");
    } finally {
      setCommitLoading(false);
    }
  }, [lastClosedEpoch, commitFn, refresh, refreshOnchainRoot]);

  const handleVerify = useCallback(async () => {
    setVerifyLoading(true);
    setError(null);
    try {
      const report = await commands.auditVerifyReaderMode();
      setVerifyReport(report);
      await refreshVerificationHistory();
    } catch (err) {
      setError(formatError(err));
    } finally {
      setVerifyLoading(false);
    }
  }, [refreshVerificationHistory]);

  const handleProof = useCallback(async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setCurrentProof(null);
    setError(null);
    // Auto-expand investigation so the proof viz is visible
    setInvestigationCollapsed(false);
    try {
      const result = await commands.auditGenerateProof(index);
      setCurrentProof(result);
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
  }, []);

  const handleOplogVerify = useCallback(async () => {
    if (!connectionId) return;
    setOplogLoading(true);
    setError(null);
    try {
      const report = await commands.auditVerifyOplogIntegrity(connectionId);
      setOplogReport(report);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setOplogLoading(false);
    }
  }, [connectionId]);

  // ─── Adaptive section collapse ────────────────────────────────────────
  // When batch is sealed/committed, collapse the feed so the commit action leads.
  useEffect(() => {
    if (epochClosed || (currentEpoch?.committed)) {
      setFeedCollapsed(true);
    } else if (events.length > 0) {
      setFeedCollapsed(false);
    }
  }, [epochClosed, currentEpoch?.committed, events.length]);

  // Show history if any batches exist.
  useEffect(() => {
    if (epochs.length > 0) setHistoryCollapsed(false);
  }, [epochs.length]);

  // ─── Advanced details ─────────────────────────────────────────────────
  const rootHex = status?.rootHex ?? "";

  return (
    <div className="audit-surface">
      {/* ─── 1. Header ─────────────────────────────────────────────────── */}
      <AuditHeader
        health={health}
        config={config}
        status={status}
        currentEpoch={currentEpoch}
        onSettings={onShowSettings}
      />

      <div className="audit-surface__body">
        {error && <Alert tone="danger">{error}</Alert>}

        {/* ─── 2. Status section ────────────────────────────────────────── */}
        <AuditStatusSection
          health={health}
          status={status}
          currentEpoch={currentEpoch}
          onchainRoot={onchainRoot}
          pollingOnchain={pollingOnchain}
          network={network}
          verifyLoading={verifyLoading}
          verifyReport={verifyReport}
          onVerify={handleVerify}
          closeEpochLoading={closeEpochLoading}
          onCloseEpoch={handleCloseEpoch}
          commitLoading={commitLoading}
          commitStep={commitStep}
          commitResult={commitResult}
          pinataResult={pinataResult}
          onCommit={handleCommit}
          canCloseEpoch={canCloseEpoch}
          closeEpochDisabledReason={closeEpochDisabledReason}
          canCommit={canCommit}
          commitDisabledReason={commitDisabledReason}
          lastClosedEpoch={lastClosedEpoch}
          staleBatchMs={staleBatchMs}
        />

        {/* ─── 3. Change Feed ───────────────────────────────────────────── */}
        <AuditChangeFeed
          events={events}
          collapsed={feedCollapsed}
          onToggle={() => setFeedCollapsed((v) => !v)}
          proofIndex={proofIndex}
          proofLoading={proofLoading}
          proofResult={proofResult}
          onProof={handleProof}
        />

        {/* ─── 4. Batch History ─────────────────────────────────────────── */}
        <AuditBatchHistory
          epochs={epochs}
          network={network}
          collapsed={historyCollapsed}
          onToggle={() => setHistoryCollapsed((v) => !v)}
          onSealEpoch={canCloseEpoch ? (_n) => handleCloseEpoch() : undefined}
          onCommitEpoch={(_n) => handleCommit()}
          onVerifyEpoch={(_n) => handleVerify()}
        />

        {/* ─── 5. Investigation (collapsed by default, auto-expands on tamper/proof) */}
        <div className="audit-section">
          <div
            className="audit-section-header"
            onClick={() => setInvestigationCollapsed((v) => !v)}
            style={{ cursor: "pointer" }}
          >
            <span className="audit-section-header__title">
              Investigation
              {verificationHistory.some((r) => r.report.tamperDetected) && (
                <span className="audit-section-header__badge audit-section-header__badge--danger">
                  tamper
                </span>
              )}
            </span>
            <span className={`audit-section-header__chevron ${investigationCollapsed ? "" : "audit-section-header__chevron--open"}`}>▶</span>
          </div>

          <div className={`audit-section-body ${investigationCollapsed ? "" : "audit-section-body--open"}`}>
            <div className="audit-section-body__inner">
              <AuditInvestigation
                localRoot={rootHex}
                onchainRoot={onchainRoot}
                network={network}
                currentProof={currentProof}
                verificationHistory={verificationHistory}
                connectionId={connectionId}
                oplogReport={oplogReport}
                oplogLoading={oplogLoading}
                onOplogVerify={handleOplogVerify}
              />
            </div>
          </div>
        </div>

        {/* ─── 6. Advanced (collapsed by default) ──────────────────────── */}
        <div className="audit-section">
          <div
            className="audit-section-header"
            onClick={() => setAdvancedCollapsed((v) => !v)}
            style={{ cursor: "pointer" }}
          >
            <span className="audit-section-header__title">Advanced</span>
            <span className={`audit-section-header__chevron ${advancedCollapsed ? "" : "audit-section-header__chevron--open"}`}>▶</span>
          </div>

          <div className={`audit-section-body ${advancedCollapsed ? "" : "audit-section-body--open"}`}>
            <div className="audit-section-body__inner">
            <div className="audit-advanced">
              <div className="audit-advanced__row">
                <span className="audit-advanced__label">Merkle root</span>
                <span className="audit-advanced__value audit-advanced__value--mono">{rootHex || "—"}</span>
              </div>
              <div className="audit-advanced__row">
                <span className="audit-advanced__label">Tree height</span>
                <span className="audit-advanced__value">{status?.treeHeight ?? "—"}</span>
              </div>
              {onchainRoot && (
                <>
                  <div className="audit-advanced__row">
                    <span className="audit-advanced__label">On-chain root</span>
                    <span className="audit-advanced__value audit-advanced__value--mono">{onchainRoot.rootHex}</span>
                  </div>
                  <div className="audit-advanced__row">
                    <span className="audit-advanced__label">Sequence</span>
                    <span className="audit-advanced__value">{onchainRoot.sequence}</span>
                  </div>
                </>
              )}
              {commitResult && (
                <div className="audit-advanced__row">
                  <span className="audit-advanced__label">Tx hash</span>
                  <span className="audit-advanced__value">
                    <a
                      href={`https://stellar.expert/explorer/${network === "mainnet" ? "public" : "testnet"}/tx/${commitResult.txHash}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      style={{ color: "var(--link)", fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)" }}
                    >
                      {commitResult.txHash}
                    </a>
                  </span>
                </div>
              )}
              {pinataResult && (
                <div className="audit-advanced__row">
                  <span className="audit-advanced__label">IPFS CID</span>
                  <span className="audit-advanced__value audit-advanced__value--mono">{pinataResult.cid}</span>
                </div>
              )}
            </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
