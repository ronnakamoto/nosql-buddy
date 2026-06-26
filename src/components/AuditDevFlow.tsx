import { useMemo, useState, useEffect, useCallback } from "react";
import commands, {
  type DevPrerequisites,
  type DevStackStatus,
  formatError,
} from "../ipc/commands";
import {
  Card,
  CardHeader,
  Badge,
  Button,
  ProgressBar,
  KeyValue,
  Alert,
  Spinner,
  StatusCard,
  InlineEmpty,
  EmptyState,
  TxHashLink,
  ContractLink,
  LogsModal,
} from "./AuditUi";
import type { ProofResult } from "../ipc/commands";
import { FlaskConical, CircleDashed, X, CheckCircle, ExternalLink } from "lucide-react";

/**
 * Dev Mode — a guided, step-based audit control surface.
 *
 * The job is simple: see the stack is healthy, watch the epoch fill,
 * and commit it to Stellar. Everything else is secondary detail.
 */

const PUBLISHER_PORT = 9173;
const ATTESTER_PORT = 9174;
const READER_PORT = 9175;
const POLL_MS = 2500;
const EPOCH_THRESHOLD = 100;

// ─── Proxy response shapes (loosely typed; the daemon returns camelCase) ──
interface DaemonStatus {
  mode: string;
  listening: boolean;
  audit: { rootHex: string; leafCount: number; eventCount: number; treeHeight: number };
}
interface DevEvent {
  index: number;
  leafHex: string;
  operation: string;
  database: string;
  collection: string;
  timestamp: string;
}
interface DevEpoch {
  epochNumber: number;
  startIndex: number;
  endIndex: number | null;
  rootHex: string | null;
  eventCount: number;
  committed: boolean;
}
interface DevOnChainRoot {
  sequence: number;
  rootHex: string;
  timestamp: number;
  metadata: string;
}

interface OplogReport {
  sequence: number;
  onChainOplogRoot: string;
  auditorOplogRoot: string | null;
  oplogEntryCount: number | null;
  allMatch: boolean;
  onChainMatchesAuditor: boolean;
  verdict: string;
  explanation: string;
  alerts: string[];
}

export function AuditDevFlow(_: { onShowSettings: () => void; onSwitchMode: () => void }) {
  const [prereqs, setPrereqs] = useState<DevPrerequisites | null>(null);
  const [stack, setStack] = useState<DevStackStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [resetBusy, setResetBusy] = useState(false);
  const [confirmReset, setConfirmReset] = useState(false);
  const [logsBusy, setLogsBusy] = useState(false);
  const [logs, setLogs] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refreshInfra = useCallback(async () => {
    try {
      const [p, s] = await Promise.all([
        commands.auditCheckDevPrerequisites(),
        commands.auditDevStackStatus(),
      ]);
      setPrereqs(p);
      setStack(s);
    } catch (e) {
      setError(formatError(e));
    }
  }, []);

  useEffect(() => {
    refreshInfra();
  }, [refreshInfra]);

  const stackUp = async () => {
    setBusy(true);
    setError(null);
    setLogs(null);
    try {
      await commands.auditDevStackUp();
      await refreshInfra();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  // Poll refreshInfra until the stack reports as stopped (up to ~8s).
  const pollUntilDown = useCallback(async () => {
    for (let i = 0; i < 10; i++) {
      await new Promise((r) => setTimeout(r, 800));
      try {
        const [p, s] = await Promise.all([
          commands.auditCheckDevPrerequisites(),
          commands.auditDevStackStatus(),
        ]);
        setPrereqs(p);
        setStack(s);
        if (!p.auditStackRunning) return;
      } catch {
        // best-effort
      }
    }
  }, []);

  const stackDown = async () => {
    setBusy(true);
    setError(null);
    try {
      await commands.auditDevStackDown();
      await pollUntilDown();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const stackResetData = async () => {
    setResetBusy(true);
    setConfirmReset(false);
    setError(null);
    try {
      await commands.auditDevStackResetData();
      await pollUntilDown();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setResetBusy(false);
    }
  };

  const showLogs = async () => {
    setLogsBusy(true);
    try {
      const l = await commands.auditDevStackLogs(120);
      setLogs(l);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setLogsBusy(false);
    }
  };

  const ready = prereqs?.auditStackRunning ?? false;

  // Determine workflow step state for the step guide
  const stepStatus: ("done" | "active" | "todo")[] = [
    ready ? "done" : "active",
    ready ? "active" : "todo",
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "auto" }}>
      {/* ─── Step guide ─────────────────────────────────────────────── */}
      <div className="audit-step-guide">
        <div className={`audit-step audit-step--${stepStatus[0]}`}>
          <span className="audit-step__num">{stepStatus[0] === "done" ? "✓" : "1"}</span>
          <span className="audit-step__label">Start Stack</span>
        </div>
        <div className={`audit-step audit-step--${stepStatus[1]}`}>
          <span className="audit-step__num">{stepStatus[0] === "done" ? "2" : ""}</span>
          <span className="audit-step__label">Audit & Commit</span>
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
        {error && <Alert tone="danger">{error}</Alert>}

        {/* ─── Stack status bar ───────────────────────────────────────── */}
        <StackStatusBar
          prereqs={prereqs}
          stack={stack}
          ready={ready}
          busy={busy}
          resetBusy={resetBusy}
          confirmReset={confirmReset}
          onRequestReset={() => setConfirmReset(true)}
          onCancelReset={() => setConfirmReset(false)}
          onConfirmReset={stackResetData}
          logsBusy={logsBusy}
          onStart={stackUp}
          onStop={stackDown}
          onToggleLogs={showLogs}
        />

        {logs !== null && (
          <LogsModal
            open={logs !== null}
            onClose={() => setLogs(null)}
            logs={logs}
            loading={logsBusy}
            title="Dev Stack Logs"
          />
        )}

        {/* ─── Live view or empty state ──────────────────────────────── */}
        {ready ? (
          <DevLiveView />
        ) : (
          <Card>
            <EmptyState
              icon={<FlaskConical size={28} />}
              title="Start the audit stack"
              body="The dev stack runs three Docker containers locally (publisher, attester, reader). Once started, every MongoDB insert, update, and delete is captured into a tamper-evident log that you can anchor to the Stellar blockchain."
              action={
                prereqs && !prereqs.dockerInstalled ? (
                  <Alert tone="warning">Install Docker Desktop to run the audit stack locally.</Alert>
                ) : undefined
              }
            />
          </Card>
        )}
      </div>
    </div>
  );
}

function StackStatusBar({
  prereqs,
  stack,
  ready,
  busy,
  resetBusy,
  confirmReset,
  onRequestReset,
  onCancelReset,
  onConfirmReset,
  logsBusy,
  onStart,
  onStop,
  onToggleLogs,
}: {
  prereqs: DevPrerequisites | null;
  stack: DevStackStatus | null;
  ready: boolean;
  busy: boolean;
  resetBusy: boolean;
  confirmReset: boolean;
  onRequestReset: () => void;
  onCancelReset: () => void;
  onConfirmReset: () => void;
  logsBusy: boolean;
  onStart: () => void;
  onStop: () => void;
  onToggleLogs: () => void;
}) {
  if (!prereqs) {
    return (
      <Card compact>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", padding: "var(--space-1) 0" }}>
          <Spinner size={16} />
          <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>Checking stack status…</span>
        </div>
      </Card>
    );
  }

  const missingPrereq = !prereqs.dockerInstalled
    ? "Docker not installed"
    : !prereqs.dockerComposeAvailable
      ? "docker compose not available"
      : !prereqs.dockerDaemonRunning
        ? "Docker daemon not running"
        : !prereqs.composeFilePresent
          ? "docker-compose.audit.yml missing"
          : null;

  const canStart = !missingPrereq && !ready;

  return (
    <Card compact>
      <div style={{ display: "flex", alignItems: "center", gap: "var(--space-3)", flexWrap: "wrap" }}>
        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          <Badge tone={ready ? "success" : "neutral"} dot={ready}>
            {ready ? "Stack running" : "Stack stopped"}
          </Badge>
          {stack && stack.services.length > 0 && (
            <div style={{ display: "flex", alignItems: "center", gap: "6px" }}>
              {stack.services.filter((s) => ["publisher", "attester", "reader"].includes(s.name)).map((s) => (
                <span
                  key={s.name}
                  style={{
                    fontSize: "var(--font-size-xs)",
                    color: isRunning(s.state) ? "var(--success-500)" : "var(--ink-faint)",
                    fontWeight: 500,
                  }}
                >
                  {s.name}
                </span>
              ))}
            </div>
          )}
        </div>

        <div style={{ flex: 1 }} />

        <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
          {missingPrereq && (
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--warning-500)" }}>{missingPrereq}</span>
          )}
          {!ready && !missingPrereq && (
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              Start the MongoDB replica set first
            </span>
          )}
          <Button variant="ghost" onClick={onToggleLogs} loading={logsBusy} disabled={logsBusy}>
            View Logs
          </Button>
          {ready ? (
            <Button variant="danger" loading={busy} onClick={onStop}>
              Stop
            </Button>
          ) : confirmReset ? (
            // ─── Inline confirmation ──────────────────────────────────
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
                Wipe all local data?
              </span>
              <Button variant="ghost" onClick={onCancelReset} disabled={resetBusy}>
                Cancel
              </Button>
              <Button variant="danger" loading={resetBusy} onClick={onConfirmReset}>
                Confirm Reset
              </Button>
            </div>
          ) : (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
              <Button
                variant="ghost"
                loading={resetBusy}
                disabled={busy || resetBusy}
                onClick={onRequestReset}
                style={{ color: "var(--warning-500)" }}
              >
                Reset Data
              </Button>
              <Button variant="primary" loading={busy} disabled={!canStart} onClick={onStart}>
                Start Stack
              </Button>
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}

function DevLiveView() {
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [events, setEvents] = useState<DevEvent[]>([]);
  const [epochs, setEpochs] = useState<DevEpoch[]>([]);
  const [current, setCurrent] = useState<DevEpoch | null>(null);
  const [onchain, setOnchain] = useState<DevOnChainRoot | null>(null);
  const [onChainAttesters, setOnChainAttesters] = useState<string[]>([]);
  const [oplog, setOplog] = useState<OplogReport | null>(null);
  const [proofBusy, setProofBusy] = useState<number | null>(null);
  const [proofResult, setProofResult] = useState<ProofResult | null>(null);
  const [proofError, setProofError] = useState<string | null>(null);
  const [provenIndex, setProvenIndex] = useState<number | null>(null);
  const [showProofDetails, setShowProofDetails] = useState(false);
  const [copyProofHint, setCopyProofHint] = useState(false);
  const [verifyTxHash, setVerifyTxHash] = useState<string | null>(null);
  const [verifyBusy, setVerifyBusy] = useState(false);
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const [closeBusy, setCloseBusy] = useState(false);
  const [commitBusy, setCommitBusy] = useState(false);
  const [commitStep, setCommitStep] = useState("");
  const [commitResult, setCommitResult] = useState<{ txHash: string; cid: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Track the most recently committed epoch so attestation queries the
  // right epoch number (not `current`, which becomes the new open epoch).
  const [committedEpochNum, setCommittedEpochNum] = useState<number | null>(null);
  // Track whether we should poll on-chain root until it appears.
  const [pollingOnchain, setPollingOnchain] = useState(false);

  const poll = useCallback(async () => {
    try {
      const [s, e, ep, c] = await Promise.all([
        commands.auditDevProxyGet(PUBLISHER_PORT, "status"),
        commands.auditDevProxyGet(PUBLISHER_PORT, "events"),
        commands.auditDevProxyGet(PUBLISHER_PORT, "epochs"),
        commands.auditDevProxyGet(PUBLISHER_PORT, "epoch/current"),
      ]);
      setStatus(s as DaemonStatus);
      setEvents((e as DevEvent[]) ?? []);
      setEpochs((ep as DevEpoch[]) ?? []);
      setCurrent(c as DevEpoch);
    } catch {
      // publisher may be mid-restart; stay silent on poll
    }
  }, []);

  useEffect(() => {
    poll();
    const id = setInterval(poll, POLL_MS);
    return () => clearInterval(id);
  }, [poll]);

  const refreshOnchain = useCallback(async () => {
    try {
      const r = await commands.auditDevProxyGet(PUBLISHER_PORT, "onchain-root");
      const root = r as DevOnChainRoot | null;
      setOnchain(root ?? null);
      return root ?? null;
    } catch {
      return null;
    }
  }, []);



  const refreshOnChainAttesters = useCallback(async (sequence: number | null) => {
    if (sequence === null) return;
    try {
      const r = await commands.auditDevProxyGet(ATTESTER_PORT, `attest/attestations/${sequence}`) as { attesters?: string[] };
      setOnChainAttesters(r?.attesters ?? []);
    } catch {
      /* best-effort */
    }
  }, []);

  const refreshOplog = useCallback(async () => {
    try {
      const r = await commands.auditDevProxyGet(READER_PORT, "reader/verify-oplog");
      setOplog(r as OplogReport);
    } catch {
      /* best-effort */
    }
  }, []);

  // On mount: load on-chain root, attestation, and oplog so previously-committed
  // data is visible immediately without requiring a new commit this session.
  useEffect(() => {
    (async () => {
      const root = await refreshOnchain();
      if (root) {
        refreshOplog();
        refreshOnChainAttesters(root.sequence);
      }
    })();
  }, [refreshOnchain, refreshOplog, refreshOnChainAttesters]);

  // Poll on-chain root every few seconds after a commit until it appears.
  // Stellar transactions take ~5-10s to be confirmed and visible via RPC.
  // Stop after 60s (12 attempts) regardless so the spinner never hangs forever.
  useEffect(() => {
    if (!pollingOnchain) return;
    let active = true;
    let attempts = 0;
    const MAX_ATTEMPTS = 12;
    const id = setInterval(async () => {
      attempts += 1;
      const root = await refreshOnchain();
      if (!active) return;
      if (root || attempts >= MAX_ATTEMPTS) {
        setPollingOnchain(false);
        if (root) refreshOplog();
      }
    }, POLL_MS);
    return () => {
      active = false;
      clearInterval(id);
    };
  }, [pollingOnchain, refreshOnchain, refreshOplog]);



  // Poll on-chain attesters using the current on-chain sequence.
  useEffect(() => {
    if (!onchain) return;
    refreshOnChainAttesters(onchain.sequence);
    const id = setInterval(() => refreshOnChainAttesters(onchain.sequence), 15000);
    return () => clearInterval(id);
  }, [onchain, refreshOnChainAttesters]);

  const handleCloseEpoch = async () => {
    if (!current || current.eventCount === 0) return;
    if (current.endIndex !== null && current.endIndex !== undefined) return;
    setCloseBusy(true);
    setError(null);
    try {
      await commands.auditDevProxyPost(PUBLISHER_PORT, "epoch/close", {});
      await poll();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setCloseBusy(false);
    }
  };

  const handleCommit = async () => {
    if (!lastClosedEpoch) return;
    setCommitBusy(true);
    setError(null);
    setCommitResult(null);
    let cid = "";
    try {
      const num = lastClosedEpoch.epochNumber;

      setCommitStep("Pinning to IPFS…");
      try {
        const pub = (await commands.auditDevProxyPost(
          PUBLISHER_PORT,
          `epoch/${num}/publish-ipfs`,
          {},
        )) as { cid?: string };
        cid = pub?.cid ?? "";
      } catch (e) {
        // IPFS publishing is optional. Continue to on-chain commit without a
        // CID so the root is still anchored even if no IPFS backend is up.
        cid = "";
      }

      setCommitStep("Committing on-chain…");
      const res = (await commands.auditDevProxyPost(
        PUBLISHER_PORT,
        `epoch/${num}/commit`,
        {},
      )) as { txHash?: string };

      setCommitResult({ txHash: res?.txHash ?? "", cid });
      setCommitStep("Confirmed");
      setCommittedEpochNum(num);
      setPollingOnchain(true);
      poll();
      refreshOnchain();
    } catch (e) {
      setError(formatError(e));
      setCommitStep("");
    } finally {
      setCommitBusy(false);
    }
  };

  const handleGenerateProof = async (index: number) => {
    setProofBusy(index);
    setProofResult(null);
    setProofError(null);
    setProvenIndex(null);
    setShowProofDetails(false);
    setCopyProofHint(false);
    setVerifyTxHash(null);
    setVerifyError(null);
    try {
      const res = await commands.auditDevProxyPost(
        PUBLISHER_PORT,
        `proof/${index}`,
        {},
      );
      setProofResult(res as ProofResult);
      setProvenIndex(index);
    } catch (e) {
      setProofError(formatError(e));
    } finally {
      setProofBusy(null);
    }
  };

  const handleVerifyOnchain = async () => {
    if (!proofResult) return;
    setVerifyBusy(true);
    setVerifyError(null);
    setVerifyTxHash(null);
    try {
      const res = await commands.auditDevProxyPost(PUBLISHER_PORT, "verify-onchain", {
        rootHex: proofResult.rootHex,
        proofA: proofResult.proof.a,
        proofB: proofResult.proof.b,
        proofC: proofResult.proof.c,
        vkAlpha: proofResult.vk.alpha,
        vkBeta: proofResult.vk.beta,
        vkGamma: proofResult.vk.gamma,
        vkDelta: proofResult.vk.delta,
        vkIc: proofResult.vk.ic,
      });
      const result = res as { txHash: string; verified: boolean };
      setVerifyTxHash(result.txHash);
    } catch (e) {
      setVerifyError(formatError(e));
    } finally {
      setVerifyBusy(false);
    }
  };

  const handleOplogVerify = async () => {
    try {
      const r = await commands.auditDevProxyGet(READER_PORT, "reader/verify-oplog");
      setOplog(r as OplogReport);
    } catch (e) {
      setError(formatError(e));
    }
  };

  const leafCount = status?.audit.leafCount ?? 0;
  const epochEvents = current?.eventCount ?? 0;
  const closed = current?.endIndex !== null && current?.endIndex !== undefined;
  const lastClosedEpoch = useMemo(() => {
    return epochs
      .filter((e) => e.endIndex !== null && e.endIndex !== undefined && !e.committed)
      .sort((a, b) => b.epochNumber - a.epochNumber)[0] ?? null;
  }, [epochs]);

  // Initialize committedEpochNum from the last committed epoch on first load.
  const lastCommittedEpochNum = useMemo(() => {
    return epochs
      .filter((e) => e.committed)
      .sort((a, b) => b.epochNumber - a.epochNumber)[0]?.epochNumber ?? null;
  }, [epochs]);

  useEffect(() => {
    if (committedEpochNum === null && lastCommittedEpochNum !== null) {
      setCommittedEpochNum(lastCommittedEpochNum);
    }
  }, [lastCommittedEpochNum, committedEpochNum]);

  const canClose = current && !closed && epochEvents > 0 && !closeBusy;
  const closeDisabledReason = !current
    ? "No epoch data"
    : closed
      ? "Epoch already closed"
      : epochEvents === 0
        ? "Write to MongoDB to capture events"
        : null;

  const canCommit = lastClosedEpoch !== null && !commitBusy;
  const commitDisabledReason = !lastClosedEpoch ? "Close an epoch first" : null;

  const onChainStatus: "good" | "neutral" = onchain ? "good" : "neutral";
  // On-chain attester count drives sign-off status (≥1 = warning/yellow, shows real data)
  const attestationStatus: "good" | "warning" | "neutral" =
    onChainAttesters.length > 0 ? "warning" : "neutral";
  const oplogStatus: "good" | "warning" | "neutral" = oplog
    ? oplog.onChainMatchesAuditor
      ? "good"
      : oplog.verdict === "incomplete"
        ? "neutral"
        : "warning"
    : "neutral";

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
      {error && <Alert tone="danger">{error}</Alert>}

      {/* ─── Workflow step guide ──────────────────────────────────── */}
      <div className="audit-step-guide">
        <div className={`audit-step ${epochEvents > 0 ? "audit-step--done" : "audit-step--active"}`}>
          <span className="audit-step__num">{epochEvents > 0 ? "✓" : "1"}</span>
          <span className="audit-step__label">Write Data</span>
        </div>
        <div className={`audit-step ${epochEvents > 0 && !closed ? "audit-step--active" : closed ? "audit-step--done" : ""}`}>
          <span className="audit-step__num">{closed ? "✓" : epochEvents > 0 ? "2" : ""}</span>
          <span className="audit-step__label">Close Batch</span>
        </div>
        <div className={`audit-step ${closed || commitResult ? "audit-step--active" : ""} ${commitResult ? "audit-step--done" : ""}`}>
          <span className="audit-step__num">{commitResult ? "✓" : closed || lastClosedEpoch ? "3" : ""}</span>
          <span className="audit-step__label">Commit to Chain</span>
        </div>
      </div>

      {/* ─── Main stage: epoch + commit ───────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-4)" }}>
        <Card>
          <CardHeader
            title={`Batch ${current?.epochNumber ?? 0}`}
            subtitle={closed ? "Sealed and ready to commit" : `${epochEvents} / ${EPOCH_THRESHOLD} changes captured`}
          />
          <ProgressBar current={epochEvents} max={EPOCH_THRESHOLD} tone={closed ? "success" : "accent"} />
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--space-3)" }}>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {closed ? "Sealed" : "Recording MongoDB changes"}
            </span>
            <span
              style={{
                fontSize: "var(--font-size-xs)",
                color: "var(--ink-faint)",
                fontFamily: "var(--font-mono)",
              }}
            >
              fingerprint {current?.rootHex ? shortHash(current.rootHex) : "—"}
            </span>
          </div>
          {epochEvents === 0 && !closed && (
            <div style={{ marginTop: "var(--space-3)" }}>
              <Alert tone="info">
                Insert, update, or delete a document in any MongoDB collection. Changes are captured automatically. The batch seals itself at {EPOCH_THRESHOLD} events, or close it manually.
              </Alert>
            </div>
          )}
          {!closed && (
            <div style={{ marginTop: "var(--space-3)" }}>
              <Button
                variant="secondary"
                loading={closeBusy}
                disabled={!canClose}
                onClick={handleCloseEpoch}
                style={{ width: "100%" }}
                title={closeDisabledReason ?? "Seal the current batch so it can be committed"}
              >
                Seal Batch
              </Button>
              {closeDisabledReason && (
                <div
                  style={{
                    marginTop: "var(--space-2)",
                    fontSize: "var(--font-size-xs)",
                    color: "var(--ink-faint)",
                    lineHeight: "var(--line-height-tight)",
                  }}
                >
                  {closeDisabledReason}
                </div>
              )}
            </div>
          )}
        </Card>

        <Card>
          <CardHeader
            title="Commit to Stellar"
            subtitle={lastClosedEpoch ? `Batch #${lastClosedEpoch.epochNumber} ready` : "Anchor the sealed batch on-chain"}
          />
          {commitBusy && (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-3)" }}>
              <Spinner size={13} />
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>{commitStep}</span>
            </div>
          )}
          {commitResult && !commitBusy && (
            <div style={{ marginBottom: "var(--space-3)", display: "flex", flexDirection: "column", gap: "var(--space-2)" }}>
              <Badge tone="success" dot>Committed</Badge>
              {commitResult.txHash && <KeyValue label="Tx hash" value={<TxHashLink txHash={commitResult.txHash} network="testnet" />} />}
              {commitResult.cid && <KeyValue label="IPFS CID" value={commitResult.cid} />}
            </div>
          )}
          <Button
            variant="primary"
            loading={commitBusy}
            disabled={!canCommit}
            onClick={handleCommit}
            style={{ width: "100%" }}
            title={commitDisabledReason ?? "Commit the closed epoch to Stellar"}
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
        </Card>
      </div>

      {/* ─── Integrity row ──────────────────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: "var(--space-4)" }}>
        <StatusCard
          title="On-chain root"
          status={onChainStatus}
          value={onchain ? shortHash(onchain.rootHex) : pollingOnchain ? "…" : "—"}
          detail={onchain ? `Batch ${onchain.sequence} · ${formatTs(onchain.timestamp)}` : pollingOnchain ? "Waiting for Stellar confirmation…" : "No batch committed yet"}
          action={<Button variant="ghost" onClick={refreshOnchain} loading={pollingOnchain}>Refresh</Button>}
        />

        <StatusCard
          title="Multi-party sign-off"
          status={attestationStatus}
          value={onchain ? `${onChainAttesters.length} attester${onChainAttesters.length !== 1 ? "s" : ""}` : "—"}
          detail={onChainAttesters.length > 0 ? `Signed batch ${onchain?.sequence}` : onchain ? "Awaiting attestation" : "Seal a batch to begin"}
          action={<Button variant="ghost" onClick={() => onchain && refreshOnChainAttesters(onchain.sequence)}>Refresh</Button>}
        />

        <StatusCard
          title="Oplog verification"
          status={oplogStatus}
          value={oplog ? (oplog.onChainMatchesAuditor ? "Match" : oplog.verdict === "incomplete" ? "Incomplete" : "Mismatch") : "—"}
          detail={oplog ? (oplog.verdict === "incomplete" ? (oplog.explanation ?? "Cannot verify") : `${oplog.oplogEntryCount ?? 0} entries checked`) : "Not verified yet"}
          action={<Button variant="ghost" onClick={handleOplogVerify}>Verify</Button>}
        />
      </div>

      {/* ─── Proof result ─────────────────────────────────────────────── */}
      {proofError && (
        <Alert tone="danger">
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", gap: "var(--space-2)" }}>
            <span>Proof generation failed: {proofError}</span>
            <button
              className="audit-proof-dismiss"
              onClick={() => setProofError(null)}
              aria-label="Dismiss"
            >
              <X size={14} />
            </button>
          </div>
        </Alert>
      )}
      {proofResult && (
        <Card>
          <CardHeader
            title="Inclusion Proof"
            subtitle={`Leaf #${proofResult.leafIndex} · batch root ${shortHash(proofResult.rootHex)}`}
            actions={
              <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
                <Badge tone="success" dot>Verified</Badge>
                <button
                  className="audit-proof-dismiss"
                  onClick={() => { setProofResult(null); setProvenIndex(null); setShowProofDetails(false); }}
                  aria-label="Close proof"
                >
                  <X size={14} />
                </button>
              </div>
            }
          />
          <div className="audit-proof-card">
            <div className="audit-proof-card__summary">
              <div className="audit-proof-card__check">
                <CheckCircle size={32} />
              </div>
              <div className="audit-proof-card__summary-text">
                <div className="audit-proof-card__summary-title">
                  This change is provably included in the audit batch.
                </div>
                <p className="audit-proof-card__summary-body">
                  A zero-knowledge proof confirms event #{proofResult.leafIndex} is part of the
                  batch rooted at {shortHash(proofResult.rootHex)}. The same root is anchored on the
                  Stellar blockchain, so anyone can verify it independently.
                </p>
              </div>
            </div>

            <div className="audit-proof-card__actions">
              {verifyTxHash ? (
                <a
                  className="audit-proof-card__primary-link"
                  href={`https://stellar.expert/explorer/${proofResult.network === "mainnet" ? "public" : proofResult.network}/tx/${verifyTxHash}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  title="View the on-chain verification transaction in Stellar Expert"
                >
                  <ExternalLink size={14} />
                  View verification tx
                </a>
              ) : (
                <button
                  className="audit-proof-card__primary-link"
                  onClick={handleVerifyOnchain}
                  disabled={verifyBusy || !proofResult.txHash}
                  title={
                    proofResult.txHash
                      ? "Submit the proof to the Soroban contract for on-chain verification"
                      : "Batch root must be committed on-chain first"
                  }
                >
                  {verifyBusy ? (
                    <>
                      <Spinner size={14} />
                      Verifying on-chain…
                    </>
                  ) : (
                    <>
                      <ExternalLink size={14} />
                      Verify on-chain
                    </>
                  )}
                </button>
              )}
              {verifyError && (
                <span className="audit-proof-card__error">{verifyError}</span>
              )}
              <button
                className="audit-proof-card__secondary-btn"
                onClick={() => {
                  const payload = {
                    rootHex: proofResult.rootHex,
                    leafIndex: proofResult.leafIndex,
                    proof: proofResult.proof,
                    pubSignals: proofResult.pubSignals,
                    network: proofResult.network,
                    contractId: proofResult.contractId,
                    txHash: proofResult.txHash,
                  };
                  navigator.clipboard.writeText(JSON.stringify(payload, null, 2));
                  setCopyProofHint(true);
                  setTimeout(() => setCopyProofHint(false), 1500);
                }}
                aria-label="Copy proof data to clipboard"
              >
                {copyProofHint ? "Copied" : "Copy proof"}
              </button>
              <button
                className="audit-proof-card__secondary-btn"
                onClick={() => setShowProofDetails((s) => !s)}
                aria-expanded={showProofDetails}
                aria-controls="audit-proof-advanced"
              >
                {showProofDetails ? "Hide details" : "Show cryptographic details"}
              </button>
            </div>

            {showProofDetails && (
              <div id="audit-proof-advanced" className="audit-proof-card__advanced">
                <div className="audit-proof-card__advanced-header">
                  <span>Cryptographic proof</span>
                  <span className="audit-proof-card__advanced-hint">Anyone with this data can verify the inclusion.</span>
                </div>
                <div className="audit-proof-fields">
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Batch root</span>
                    <span className="audit-proof-field__value" title={proofResult.rootHex}>
                      {proofResult.rootHex}
                    </span>
                  </div>
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Proof A</span>
                    <span className="audit-proof-field__value" title={proofResult.proof.a}>
                      {proofResult.proof.a}
                    </span>
                  </div>
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Proof B</span>
                    <span className="audit-proof-field__value" title={proofResult.proof.b}>
                      {proofResult.proof.b}
                    </span>
                  </div>
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Proof C</span>
                    <span className="audit-proof-field__value" title={proofResult.proof.c}>
                      {proofResult.proof.c}
                    </span>
                  </div>
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Public signal</span>
                    <span className="audit-proof-field__value" title={proofResult.pubSignals.join(", ")}>
                      {proofResult.pubSignals.join(", ")}
                    </span>
                  </div>
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Root commitment tx</span>
                    <span className="audit-proof-field__value">
                      {proofResult.txHash ? (
                        <TxHashLink txHash={proofResult.txHash} network={proofResult.network} />
                      ) : (
                        <span style={{ color: "var(--ink-faint)" }}>Not yet committed</span>
                      )}
                    </span>
                  </div>
                  {verifyTxHash && (
                    <div className="audit-proof-field">
                      <span className="audit-proof-field__label">Verification tx</span>
                      <span className="audit-proof-field__value">
                        <TxHashLink txHash={verifyTxHash} network={proofResult.network} />
                      </span>
                    </div>
                  )}
                  <div className="audit-proof-field">
                    <span className="audit-proof-field__label">Contract</span>
                    <span className="audit-proof-field__value">
                      <ContractLink
                        contractId={proofResult.contractId}
                        network={proofResult.network}
                      />
                    </span>
                  </div>
                </div>
              </div>
            )}
          </div>
        </Card>
      )}

      {/* ─── Event feed + history ───────────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "2fr 1fr", gap: "var(--space-4)" }}>
        <Card compact>
          <CardHeader
            title="Change Feed"
            subtitle={`${events.length} captured · ${leafCount} leaves`}
            compact
          />
          {events.length === 0 ? (
            <InlineEmpty
              icon={<CircleDashed size={22} />}
              title="No changes captured yet"
              body="Insert, update, or delete a document in MongoDB to populate the audit log."
            />
          ) : (
            <div className="audit-event-list">
              {events.slice().reverse().map((ev, i, arr) => (
                <div
                  key={ev.index}
                  className={`audit-event-row-grid audit-event-row-grid--with-action${
                    provenIndex === ev.index ? " audit-event-row--proven" : ""
                  }`}
                  style={i >= arr.length - 1 ? { borderBottom: "none" } : undefined}
                >
                  <Badge tone={opTone(ev.operation)}>{ev.operation}</Badge>
                  <span className="audit-event-db">{ev.database}.{ev.collection}</span>
                  <span className="audit-event-leaf">leaf {ev.leafHex.slice(0, 10)}…</span>
                  <span className="audit-event-time">{new Date(ev.timestamp).toLocaleTimeString()}</span>
                  <Button
                    variant="ghost"
                    loading={proofBusy === ev.index}
                    disabled={proofBusy !== null}
                    onClick={() => handleGenerateProof(ev.index)}
                    style={{ fontSize: "var(--font-size-xs)", padding: "3px 10px", minHeight: "auto" }}
                    title={provenIndex === ev.index ? "Regenerate proof" : "Generate ZK inclusion proof"}
                  >
                    {provenIndex === ev.index ? "Re-prove" : "Prove"}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </Card>

        <Card compact>
          <CardHeader
            title="Batches"
            subtitle={`${epochs.length} sealed`}
            compact
          />
          {epochs.length === 0 ? (
            <InlineEmpty title="No batches yet" body="Seal a batch to see it here." />
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
              {epochs
                .slice()
                .reverse()
                .map((ep) => (
                  <div
                    key={ep.epochNumber}
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "var(--space-2)",
                      padding: "var(--space-2) var(--space-3)",
                      borderRadius: "var(--radius-md)",
                      background: "var(--surface-2)",
                      fontSize: "var(--font-size-sm)",
                      border: "1px solid var(--border)",
                    }}
                  >
                    <span
                      style={{
                        fontFamily: "var(--font-mono)",
                        color: "var(--ink)",
                        fontWeight: 600,
                        width: "48px",
                      }}
                    >
                      #{ep.epochNumber}
                    </span>
                    <span style={{ color: "var(--ink-muted)", flex: 1 }}>{ep.eventCount} events</span>
                    <Badge tone={ep.committed ? "success" : "neutral"}>{ep.committed ? "committed" : "open"}</Badge>
                  </div>
                ))}
            </div>
          )}
        </Card>
      </div>
    </div>
  );
}

function isRunning(state: string): boolean {
  const s = state.toLowerCase();
  return s.includes("up") || s.includes("running");
}

function opTone(op: string): "success" | "warning" | "danger" | "info" {
  const o = op.toLowerCase();
  if (o.includes("insert")) return "success";
  if (o.includes("update")) return "warning";
  if (o.includes("delete")) return "danger";
  return "info";
}

function shortHash(h: string): string {
  if (!h) return "—";
  return h.length > 20 ? `${h.slice(0, 10)}…${h.slice(-8)}` : h;
}

function formatTs(unixSeconds: number): string {
  if (!unixSeconds) return "—";
  return new Date(unixSeconds * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}
