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
  type DomainRootInfo,
  type DomainSuperProofResult,
  type OnChainAttestationVerification,
  formatError,
} from "../ipc/commands";
import { useToast } from "../context/ToastContext";
import { AuditHeader, type HealthState } from "./AuditHeader";
import { AuditStatusSection } from "./AuditStatusSection";
import { AuditChangeFeed } from "./AuditChangeFeed";
import { AuditBatchHistory } from "./AuditBatchHistory";
import { AuditInvestigation } from "./AuditInvestigation";
import { InfoPopover } from "./InfoPopover";
import { ChevronRight, RotateCcw } from "lucide-react";

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
  const [selectedDeploymentId, setSelectedDeploymentId] = useState<string | null>(null);
  const [selectedDatabase, setSelectedDatabase] = useState<string | null>(null);
  const [domainInfos, setDomainInfos] = useState<DomainRootInfo[]>([]);
  const [domainInfo, setDomainInfo] = useState<DomainRootInfo | null>(null);
  const [domainBusy, setDomainBusy] = useState(false);
  const [superRootHex, setSuperRootHex] = useState<string | null>(null);
  const [superProof, setSuperProof] = useState<DomainSuperProofResult | null>(null);
  const [epochs, setEpochs] = useState<Epoch[]>([]);
  const [currentEpoch, setCurrentEpoch] = useState<Epoch | null>(null);
  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);

  // ─── Operation loading states ─────────────────────────────────────────
  const [closeEpochLoading, setCloseEpochLoading] = useState(false);
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [pinataResult, setPinataResult] = useState<IpfsPublishResult | null>(null);
  const [commitStep, setCommitStep] = useState("");
  const [onchainAttestation, setOnchainAttestation] =
    useState<OnChainAttestationVerification | null>(null);
  const [contractId, setContractId] = useState<string>(config.testnetContractId ?? "");
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

  // ─── Reset audit data ─────────────────────────────────────────────────
  const [resetBusy, setResetBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);
  const toast = useToast();

  // ─── Polling ─────────────────────────────────────────────────────────
  const refresh = useCallback(async () => {
    try {
      const [s, e, eps, ep, domainList, superRoot] = await Promise.all([
        commands.auditGetStatus(selectedDeploymentId, selectedDatabase),
        commands.auditListEvents(selectedDeploymentId, selectedDatabase),
        commands.auditListEpochs(),
        commands.auditCurrentEpoch(),
        commands.auditListDomains(),
        commands.auditGetDomainSuperRoot(),
      ]);
      setStatus(s);
      setEvents(e);
      setEpochs(eps);
      setCurrentEpoch(ep);
      setDomainInfos(domainList);
      setSuperRootHex(superRoot.superRootHex);
    } catch {
      // Silent poll failure.
    }
  }, [selectedDatabase, selectedDeploymentId]);

  useEffect(() => {
    refresh();
    const interval = setInterval(refresh, POLL_INTERVAL_MS);
    return () => clearInterval(interval);
  }, [refresh]);

  useEffect(() => {
    if (selectedDeploymentId === null || selectedDatabase === null) {
      setDomainInfo(null);
      return;
    }
    setDomainInfo(
      domainInfos.find(
        (domain) =>
          domain.deploymentId === selectedDeploymentId &&
          domain.database === selectedDatabase,
      ) ?? null,
    );
  }, [domainInfos, selectedDatabase, selectedDeploymentId]);

  // Reset any stale super-root proof when the selected domain changes.
  useEffect(() => {
    setSuperProof(null);
  }, [selectedDatabase, selectedDeploymentId]);

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

    try {
      await commands.auditCloseEpoch();
      await refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setCloseEpochLoading(false);
    }
  }, [canCloseEpoch, refresh]);

  const handleCommit = useCallback(async () => {
    if (!lastClosedEpoch) return;
    setCommitLoading(true);

    setCommitResult(null);
    setPinataResult(null);
    setOnchainAttestation(null);
    setCommitStep("Pinning batch to IPFS via Pinata…");

    try {
      // On testnet, ensure a commitment contract owned by this app's key
      // exists before committing. The bundled shared contract is admin-gated
      // to a key we don't hold, so an unprovisioned commit traps on-chain.
      // This call is idempotent: it reuses an already-deployed contract.
      if (network === "testnet") {
        setCommitStep("Provisioning your audit contract…");
        const provision = await commands.auditProvisionTestnetContract();
        setContractId(provision.contractId);
        setCommitStep("Pinning batch to IPFS via Pinata…");
      }
      const pinata = await commands.auditPublishEpochToPinata(lastClosedEpoch.epochNumber);
      setPinataResult(pinata);
      setCommitStep("Submitting transaction to Stellar…");
      const result = await commitFn(`epoch=${lastClosedEpoch.epochNumber} cid=${pinata.cid}`);
      setCommitResult(result);
      setCommitStep("Confirmed!");
      await commands.auditMarkEpochCommitted(lastClosedEpoch.epochNumber, result.txHash);
      // Query the contract's independent attestation verdict (no self-attest).
      // The operator key commits but cannot produce a "verified" verdict on
      // its own; that requires K-of-N distinct authorized attester signatures.
      try {
        const verification = await commands.auditVerifyOnchainAttestation(
          result.sequence,
        );
        setOnchainAttestation(verification);
      } catch {
        // Non-fatal: contract may not support verify_attestation yet.
      }
      setPollingOnchain(true);
      refreshOnchainRoot();
      refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
      setCommitStep("");
    } finally {
      setCommitLoading(false);
    }
  }, [lastClosedEpoch, commitFn, network, refresh, refreshOnchainRoot, toast]);

  const handleVerify = useCallback(async () => {
    setVerifyLoading(true);

    try {
      const report = await commands.auditVerifyReaderMode();
      setVerifyReport(report);
      await refreshVerificationHistory();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setVerifyLoading(false);
    }
  }, [refreshVerificationHistory]);

  const handleProof = useCallback(async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setCurrentProof(null);

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
      toast.push(formatError(err), "error");
    } finally {
      setProofLoading(false);
    }
  }, []);

  const handleOplogVerify = useCallback(async () => {
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
  }, [connectionId]);

  const handleResetData = useCallback(async () => {
    setResetBusy(true);

    try {
      await commands.auditResetData();
      setConfirmReset(false);
      setVerifyReport(null);
      setVerificationHistory([]);
      setCommitResult(null);
      setPinataResult(null);
      setCurrentProof(null);
      setOplogReport(null);
      setDomainInfos([]);
      setDomainInfo(null);
      await refresh();
      await refreshOnchainRoot();
      await refreshVerificationHistory();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setResetBusy(false);
    }
  }, [refresh, refreshOnchainRoot, refreshVerificationHistory]);

  const handleSetLegalHold = useCallback(async (hold: boolean) => {
    if (selectedDeploymentId === null || selectedDatabase === null) return;
    setDomainBusy(true);
    try {
      await commands.auditSetLegalHold(selectedDeploymentId, selectedDatabase, hold);
      await refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setDomainBusy(false);
    }
  }, [refresh, selectedDatabase, selectedDeploymentId]);

  const handleProveInSuperRoot = useCallback(async () => {
    if (selectedDeploymentId === null || selectedDatabase === null) return;
    setDomainBusy(true);
    try {
      const proof = await commands.auditGenerateDomainSuperProof(
        selectedDeploymentId,
        selectedDatabase,
      );
      setSuperProof(proof);
      setSuperRootHex(proof.superRootHex);
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setDomainBusy(false);
    }
  }, [selectedDatabase, selectedDeploymentId, toast]);

  const handlePruneDomain = useCallback(async () => {
    if (selectedDeploymentId === null || selectedDatabase === null) return;
    setDomainBusy(true);
    try {
      await commands.auditPruneDomain(selectedDeploymentId, selectedDatabase);
      setProofIndex(null);
      setProofResult(null);
      setCurrentProof(null);
      await refresh();
    } catch (err) {
      toast.push(formatError(err), "error");
    } finally {
      setDomainBusy(false);
    }
  }, [refresh, selectedDatabase, selectedDeploymentId]);

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
          onchainAttestation={onchainAttestation}
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
          domains={domainInfos.length > 0 ? domainInfos : (status?.domains ?? [])}
          selectedDeploymentId={selectedDeploymentId}
          selectedDatabase={selectedDatabase}
          onDomainChange={(deploymentId, database) => {
            setSelectedDeploymentId(deploymentId);
            setSelectedDatabase(database);
            setDomainInfo(null);
            setProofIndex(null);
            setProofResult(null);
            setCurrentProof(null);
          }}
          domainInfo={domainInfo}
          domainBusy={domainBusy}
          onSetLegalHold={handleSetLegalHold}
          onPruneDomain={handlePruneDomain}
          superRootHex={superRootHex}
          superProof={superProof}
          onProveInSuperRoot={handleProveInSuperRoot}
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
          >
            <span className="audit-section-header__title">
              Investigation
              <InfoPopover label="Help: Investigation" title="Investigation"><p>Forensic tools for deep verification: root comparison, Merkle proof inspection, verification history, and oplog completeness checks.</p></InfoPopover>
              {verificationHistory.some((r) => r.report.tamperDetected) && (
                <span className="audit-section-header__badge audit-section-header__badge--danger">
                  tamper
                </span>
              )}
            </span>
            <span className={`audit-section-header__chevron ${investigationCollapsed ? "" : "audit-section-header__chevron--open"}`}>
              <ChevronRight size={15} aria-hidden="true" />
            </span>
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
          >
            <span className="audit-section-header__title">Advanced<InfoPopover label="Help: Advanced" title="Advanced audit info"><p>Detailed technical data including the current Merkle root, tree height, on-chain root, transaction hashes, and IPFS CIDs.</p></InfoPopover></span>
            <span className={`audit-section-header__chevron ${advancedCollapsed ? "" : "audit-section-header__chevron--open"}`}>
              <ChevronRight size={15} aria-hidden="true" />
            </span>
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
              {network === "testnet" && contractId && (
                <div className="audit-advanced__row">
                  <span className="audit-advanced__label">Your contract</span>
                  <span className="audit-advanced__value audit-advanced__value--mono">{contractId}</span>
                </div>
              )}
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
                      className="audit-advanced__link"
                    >
                      {commitResult.txHash}
                    </a>
                  </span>
                </div>
              )}
              {pinataResult && (
                <div className="audit-advanced__row">
                  <span className="audit-advanced__label">IPFS CID</span>
                  <span className="audit-advanced__value audit-advanced__value--mono">
                    <a
                      href={pinataResult.gatewayUrl || `https://ipfs.io/ipfs/${pinataResult.cid}`}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="audit-advanced__link"
                    >
                      {pinataResult.cid}
                    </a>
                  </span>
                </div>
              )}
            </div>
            </div>
          </div>
        </div>

        {/* ─── Maintenance: reset local audit data (destructive, kept last) ── */}
        <div className="audit-surface__maintenance">
          {confirmReset ? (
            <div className="audit-reset">
              <span className="audit-reset__prompt">
                Clear all local audit data? On-chain history is unaffected.
              </span>
              <button className="audit-mode-tab" onClick={() => setConfirmReset(false)} disabled={resetBusy}>
                Cancel
              </button>
              <button
                className="audit-mode-tab audit-reset__confirm"
                onClick={handleResetData}
                disabled={resetBusy}
              >
                {resetBusy ? "Resetting…" : "Confirm reset"}
              </button>
            </div>
          ) : (
            <div className="audit-reset">
              <button
                className="audit-mode-tab audit-reset__trigger"
                onClick={() => setConfirmReset(true)}
                title="Clear local events, batches, and verification history (on-chain history is unaffected)"
              >
                <RotateCcw size={13} aria-hidden="true" />
                Reset audit data
              </button>
              <InfoPopover label="Help: Reset audit data" title="Reset audit data"><p>Clears all local audit events, batches, and verification history. On-chain commitments and IPFS data are unaffected and remain verifiable.</p></InfoPopover>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
