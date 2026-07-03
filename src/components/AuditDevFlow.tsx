import { useMemo, useState, useEffect, useCallback, Fragment } from "react";
import commands, {
  type DevPrerequisites,
  type DevStackStatus,
  type DevStackIdentities,
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
  Stat,
  InlineEmpty,
  EmptyState,
  TxHashLink,
  ContractLink,
  IpfsCidLink,
  LogsModal,
  Modal,
} from "./AuditUi";
import { LogViewer } from "./LogViewer";
import type { ProofResult, DevSetupParams } from "../ipc/commands";
import { InfoPopover } from "./InfoPopover";
import { KeyRound, ShieldCheck, Users } from "lucide-react";
import { onAuditSetupProgress, onAuditStackProgress } from "../ipc/events";
import { useToast } from "../context/ToastContext";
import { FlaskConical, CircleDashed, X, CheckCircle, ExternalLink, ChevronDown, Copy, Check } from "lucide-react";

/**
 * Dev Mode — a guided, step-based audit control surface.
 *
 * The job is simple: see the stack is healthy, watch the epoch fill,
 * and commit it to Stellar. Everything else is secondary detail.
 */

const PUBLISHER_PORT = 9173;
const ATTESTER_PORT = 9174;
const READER_PORT = 9175;
// The dev-mode replica set now runs with auth enabled (see
// docker-compose.audit-db.yml + scripts/rs-init-audit.js) — these are the
// fixed, non-secret dev-only root credentials created by that bootstrap.
const AUDITED_MONGO_URI =
  "mongodb://root:nosqlbuddy-dev-root-pw@127.0.0.1:27020/?directConnection=true&authSource=admin";
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
  const toast = useToast();
  const [setupModalOpen, setSetupModalOpen] = useState(false);
  const [setupBusy, setSetupBusy] = useState(false);
  const [setupResultLog, setSetupResultLog] = useState<string | null>(null);
  const [setupProgress, setSetupProgress] = useState<string[]>([]);
  const [stackProgress, setStackProgress] = useState<string[]>([]);
  // `busy` is shared with stop/reset (drives the button spinner for both);
  // this tracks specifically whether a start is in flight, so the live
  // progress panel below only appears while starting, never while stopping.
  const [startingUp, setStartingUp] = useState(false);

  const refreshInfra = useCallback(async () => {
    try {
      const [p, s] = await Promise.all([
        commands.auditCheckDevPrerequisites(),
        commands.auditDevStackStatus(),
      ]);
      setPrereqs(p);
      setStack(s);
    } catch (e) {
      toast.push(formatError(e), "error");
    }
  }, []);

  useEffect(() => {
    refreshInfra();
  }, [refreshInfra]);

  const stackUp = async () => {
    setBusy(true);
    setStartingUp(true);

    setLogs(null);
    setStackProgress([]);
    const unlisten = await onAuditStackProgress((line) =>
      setStackProgress((prev) => [...prev, line]),
    );
    try {
      await commands.auditDevStackUp();
      await refreshInfra();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      unlisten();
      setBusy(false);
      setStartingUp(false);
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

    try {
      await commands.auditDevStackDown();
      await pollUntilDown();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const stackResetData = async () => {
    setResetBusy(true);
    setConfirmReset(false);

    try {
      await commands.auditDevStackResetData();
      await pollUntilDown();
    } catch (e) {
      toast.push(formatError(e), "error");
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
      toast.push(formatError(e), "error");
    } finally {
      setLogsBusy(false);
    }
  };

  const handleSetup = async (params: DevSetupParams) => {
    setSetupBusy(true);

    setSetupProgress([]);
    const unlisten = await onAuditSetupProgress((line) =>
      setSetupProgress((prev) => [...prev, line]),
    );
    try {
      const res = await commands.auditDevStackSetup(params);
      setSetupResultLog(res.log);
      await refreshInfra();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      unlisten();
      setSetupBusy(false);
    }
  };

  const ready = prereqs?.auditStackRunning ?? false;

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, overflow: "auto" }}>
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          gap: "var(--space-3)",
          padding: "var(--space-3)",
          flex: 1,
        }}
      >
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
          onOpenSetup={() => { setSetupResultLog(null); setSetupModalOpen(true); }}
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

        <SetupWizardModal
          open={setupModalOpen}
          onClose={() => setSetupModalOpen(false)}
          onSetup={handleSetup}
          busy={setupBusy}
          resultLog={setupResultLog}
          progress={setupProgress}
        />

        {/* ─── Live view, starting progress, or empty state ─────────────── */}
        {ready ? (
          <DevLiveView auditedMongoUri={stack?.publisherMongoUri || AUDITED_MONGO_URI} />
        ) : startingUp ? (
          <Card>
            <CardHeader
              title="Starting the audit stack…"
              subtitle="First run builds the containers locally and can take a few minutes. Subsequent starts are fast."
            />
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-2)" }}>
              <Spinner size={16} />
              <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
                Starting MongoDB, then the publisher, attester, and reader containers.
              </span>
            </div>
            <LogViewer
              lines={stackProgress}
              loading={stackProgress.length === 0}
              loadingLabel="Waiting for Docker to start…"
              live
              showLineNumbers={false}
              minHeight={160}
              maxHeight={320}
            />
          </Card>
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
  onOpenSetup,
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
  onOpenSetup: () => void;
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

  const configured = prereqs.auditConfigured;
  const canStart = !missingPrereq && !ready && configured;
  const auditedMongoUri = stack?.publisherMongoUri || AUDITED_MONGO_URI;

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
              {configured ? "Start the MongoDB replica set first" : "Run Set up to generate audit credentials"}
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
              {configured ? (
                <Button variant="primary" loading={busy} disabled={!canStart} onClick={onStart}>
                  Start Stack
                </Button>
              ) : (
                <Button
                  variant="primary"
                  disabled={!!missingPrereq}
                  onClick={onOpenSetup}
                >
                  Set up
                </Button>
              )}
            </div>
          )}
        </div>
      </div>
      {ready && (
        <div
          style={{
            marginTop: "var(--space-2)",
            paddingTop: "var(--space-2)",
            borderTop: "1px solid var(--border)",
            fontSize: "var(--font-size-xs)",
            color: "var(--ink-muted)",
          }}
        >
          Use <code>{auditedMongoUri}</code> for collection changes you want captured. If this
          points at the bundled demo DB, port 27017 is the separate single-node dev DB.
        </div>
      )}
    </Card>
  );
}

function DevLiveView({ auditedMongoUri }: { auditedMongoUri: string }) {
  return <DevLiveViewInner auditedMongoUri={auditedMongoUri} />;
}

function SetupWizardModal({
  open,
  onClose,
  onSetup,
  busy,
  resultLog,
  progress,
}: {
  open: boolean;
  onClose: () => void;
  onSetup: (params: DevSetupParams) => void;
  busy: boolean;
  resultLog: string | null;
  progress: string[];
}) {
  const [pinataApiKey, setPinataApiKey] = useState("");
  const [pinataApiSecret, setPinataApiSecret] = useState("");
  const [pinataGatewayUrl, setPinataGatewayUrl] = useState("");
  const [publisherMongoUri, setPublisherMongoUri] = useState("");
  const [attesterMongoUri, setAttesterMongoUri] = useState("");

  const submit = () => {
    onSetup({
      network: "testnet",
      pinataApiKey: pinataApiKey.trim() || undefined,
      pinataApiSecret: pinataApiSecret.trim() || undefined,
      pinataGatewayUrl: pinataGatewayUrl.trim() || undefined,
      publisherMongoUri: publisherMongoUri.trim() || undefined,
      attesterMongoUri: attesterMongoUri.trim() || undefined,
    });
  };

  return (
    <Modal
      open={open}
      onClose={busy ? () => {} : onClose}
      title="Set up the audit stack"
      subtitle="Generates Stellar keys, funds them on testnet, and writes credentials"
      maxWidth={560}
      onSubmit={resultLog || busy ? undefined : submit}
      footer={
        resultLog ? (
          <Button variant="primary" shortcut="Escape" onClick={onClose}>Done</Button>
        ) : (
          <div style={{ display: "flex", gap: "var(--space-2)" }}>
            <Button variant="ghost" shortcut="Escape" onClick={onClose} disabled={busy}>Cancel</Button>
            <Button variant="primary" shortcut="CmdOrCtrl+Enter" loading={busy} disabled={busy} onClick={submit}>
              Run Setup
            </Button>
          </div>
        )
      }
    >
      {resultLog ? (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <Alert tone="success">
            Setup complete. Credentials were written locally. You can now start the stack.
          </Alert>
          <LogViewer
            lines={resultLog}
            copyable
            showLineNumbers={false}
            maxHeight={320}
          />
        </div>
      ) : busy ? (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)" }}>
            <Spinner size={16} />
            <span style={{ fontSize: "var(--font-size-sm)", color: "var(--ink-muted)" }}>
              Generating keys, funding accounts, deploying the contract, and authorizing the
              attester… this can take a minute or two.
            </span>
          </div>
          <LogViewer
            lines={progress}
            loading={progress.length === 0}
            loadingLabel="Waiting for the setup wizard to start…"
            live
            showLineNumbers={false}
            minHeight={120}
            maxHeight={240}
          />
        </div>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)" }}>
          <Alert tone="info">
            This uses the Stellar <strong>testnet</strong>. Fresh publisher and attester keypairs
            are generated and funded, and a new audit contract is deployed automatically (your
            publisher becomes its admin) — no terminal needed. The secret keys are stored locally
            and never shown here.
          </Alert>
          <div>
            <div style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, marginBottom: "var(--space-2)" }}>
              MongoDB URI to audit
            </div>
            <p style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: 0, marginBottom: "var(--space-2)" }}>
              Point this at the DBA/operator-maintained replica set you want the publisher to
              watch. Leave blank to use the bundled demo replica set. If MongoDB runs on your
              host, use <code>host.docker.internal</code> because the publisher runs in Docker.
            </p>
            <label className="field__label">Publisher MongoDB URI</label>
            <input
              className="field__input"
              type="text"
              value={publisherMongoUri}
              onChange={(e) => setPublisherMongoUri(e.target.value)}
              placeholder="mongodb://root:nosqlbuddy-dev-root-pw@host.docker.internal:27020/?directConnection=true&authSource=admin"
            />
            <label className="field__label" style={{ marginTop: "var(--space-2)" }}>
              Attester MongoDB URI (independent member)
            </label>
            <input
              className="field__input"
              type="text"
              value={attesterMongoUri}
              onChange={(e) => setAttesterMongoUri(e.target.value)}
              placeholder="mongodb://auditor:nosqlbuddy-dev-auditor-pw@host.docker.internal:27019/?directConnection=true&authSource=admin"
            />
            <p style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: "var(--space-2)", marginBottom: 0 }}>
              Use a replica member controlled by the audit team for a real trust anchor. Leave
              blank to reuse the publisher URI, which captures events but does not prove
              independent oplog completeness.
            </p>
          </div>
          <div>
            <div style={{ fontSize: "var(--font-size-sm)", fontWeight: 600, marginBottom: "var(--space-2)" }}>
              Pinata IPFS credentials (optional)
            </div>
            <p style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginTop: 0, marginBottom: "var(--space-2)" }}>
              Provide these to pin audit batches to IPFS. Leave blank to skip; you can still anchor
              roots on-chain.
            </p>
            <label className="field__label">Pinata API key</label>
            <input
              className="field__input"
              type="text"
              value={pinataApiKey}
              onChange={(e) => setPinataApiKey(e.target.value)}
              placeholder="optional"
            />
            <label className="field__label" style={{ marginTop: "var(--space-2)" }}>Pinata API secret</label>
            <input
              className="field__input"
              type="password"
              value={pinataApiSecret}
              onChange={(e) => setPinataApiSecret(e.target.value)}
              placeholder="optional"
            />
            <label className="field__label" style={{ marginTop: "var(--space-2)" }}>Pinata gateway URL</label>
            <input
              className="field__input"
              type="text"
              value={pinataGatewayUrl}
              onChange={(e) => setPinataGatewayUrl(e.target.value)}
              placeholder="https://gateway.pinata.cloud"
            />
          </div>
        </div>
      )}
    </Modal>
  );
}

function DevLiveViewInner({ auditedMongoUri }: { auditedMongoUri: string }) {
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [events, setEvents] = useState<DevEvent[]>([]);
  const [epochs, setEpochs] = useState<DevEpoch[]>([]);
  const [current, setCurrent] = useState<DevEpoch | null>(null);
  const [onchain, setOnchain] = useState<DevOnChainRoot | null>(null);
  const [onChainAttesters, setOnChainAttesters] = useState<string[]>([]);
  const [oplog, setOplog] = useState<OplogReport | null>(null);
  const [proofBusy, setProofBusy] = useState<number | null>(null);
  const [proofResult, setProofResult] = useState<ProofResult | null>(null);
  const [provenIndex, setProvenIndex] = useState<number | null>(null);
  const [showProofDetails, setShowProofDetails] = useState(false);
  const [copyProofHint, setCopyProofHint] = useState(false);
  const [verifyTxHash, setVerifyTxHash] = useState<string | null>(null);
  const [verifyBusy, setVerifyBusy] = useState(false);
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const [closeBusy, setCloseBusy] = useState(false);
  const [commitBusy, setCommitBusy] = useState(false);
  const [commitStep, setCommitStep] = useState("");
  const [commitResult, setCommitResult] = useState<{ txHash: string; cid: string; gatewayUrl?: string; encrypted?: boolean } | null>(null);
  const toast = useToast();
  // Track the most recently committed epoch so attestation queries the
  // right epoch number (not `current`, which becomes the new open epoch).
  const [committedEpochNum, setCommittedEpochNum] = useState<number | null>(null);
  // Track whether we should poll on-chain root until it appears.
  const [pollingOnchain, setPollingOnchain] = useState(false);
  // Dev-stack public identities for the key-separation panel.
  const [identities, setIdentities] = useState<DevStackIdentities | null>(null);

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

  // Load dev-stack identities (publisher + attester public addresses) for the
  // key-separation proof panel. Loaded whenever the stack is reachable.
  useEffect(() => {
    if (!status) {
      setIdentities(null);
      return;
    }
    commands
      .auditDevStackIdentities()
      .then((id) => setIdentities(id))
      .catch(() => setIdentities(null));
  }, [status]);



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

    try {
      await commands.auditDevProxyPost(PUBLISHER_PORT, "epoch/close", {});
      await poll();
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setCloseBusy(false);
    }
  };

  const handleCommit = async () => {
    if (!lastClosedEpoch) return;
    setCommitBusy(true);

    setCommitResult(null);
    let cid = "";
    let gatewayUrl: string | undefined;
    let encrypted = false;
    try {
      const num = lastClosedEpoch.epochNumber;

      setCommitStep("Pinning to IPFS…");
      try {
        const pub = (await commands.auditDevProxyPost(
          PUBLISHER_PORT,
          `epoch/${num}/publish-ipfs`,
          {},
        )) as { cid?: string; gatewayUrl?: string; encrypted?: boolean };
        cid = pub?.cid ?? "";
        gatewayUrl = pub?.gatewayUrl;
        encrypted = pub?.encrypted ?? false;
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

      setCommitResult({ txHash: res?.txHash ?? "", cid, gatewayUrl, encrypted });
      setCommitStep("Confirmed");
      setCommittedEpochNum(num);
      setPollingOnchain(true);
      poll();
      refreshOnchain();
    } catch (e) {
      toast.push(formatError(e), "error");
      setCommitStep("");
    } finally {
      setCommitBusy(false);
    }
  };

  const handleGenerateProof = async (index: number) => {
    setProofBusy(index);
    setProofResult(null);

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
      toast.push(formatError(e), "error");
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
        leafHex: proofResult.leafHex,
        proofA: proofResult.proof.a,
        proofB: proofResult.proof.b,
        proofC: proofResult.proof.c,
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
      toast.push(formatError(e), "error");
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

  const displayFingerprint = useMemo(() => {
    if (committedEpochNum !== null) {
      const ep = epochs.find((e) => e.epochNumber === committedEpochNum);
      if (ep?.rootHex) return shortHash(ep.rootHex);
    }
    if (lastClosedEpoch?.rootHex) return shortHash(lastClosedEpoch.rootHex);
    if (status?.audit.rootHex) return shortHash(status.audit.rootHex);
    return "—";
  }, [committedEpochNum, epochs, lastClosedEpoch, status]);

  const committed = !!commitResult;
  const hasRecordedChanges = epochEvents > 0 || closed || lastClosedEpoch !== null;
  const hasSealedBatch = closed || lastClosedEpoch !== null;
  const activeStep = committed ? 0 : hasSealedBatch ? 3 : hasRecordedChanges ? 2 : 1;
  const stepDefs = [
    { n: 1, label: "Write Data", desc: "Capture MongoDB changes" },
    { n: 2, label: "Seal Batch", desc: "Lock the recorded events" },
    { n: 3, label: "Commit to Chain", desc: "Anchor the root on Stellar" },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-4)" }}>
      <Card padded={false} style={{ overflow: "hidden" }}>
        <div className="audit-stage">
          <div className="audit-stepper" role="list" aria-label="Audit commit workflow">
            {stepDefs.map((step, index) => {
              const done = step.n === 1
                ? hasRecordedChanges
                : step.n === 2
                  ? hasSealedBatch
                  : committed;
              const active = activeStep === step.n;
              return (
                <Fragment key={step.n}>
                  <div
                    className={[
                      "audit-stepper__item",
                      active ? "is-active" : "",
                      done ? "is-done" : "",
                    ].filter(Boolean).join(" ")}
                    role="listitem"
                    aria-current={active ? "step" : undefined}
                  >
                    <span className="audit-stepper__marker">{done ? "✓" : step.n}</span>
                    <span className="audit-stepper__copy">
                      <span className="audit-stepper__label">{step.label}</span>
                      <span className="audit-stepper__desc">{step.desc}</span>
                    </span>
                  </div>
                  {index < stepDefs.length - 1 && (
                    <span
                      className={[
                        "audit-stepper__connector",
                        (step.n === 1 && hasRecordedChanges) || (step.n === 2 && hasSealedBatch)
                          ? "is-done"
                          : "",
                      ].filter(Boolean).join(" ")}
                      aria-hidden="true"
                    />
                  )}
                </Fragment>
              );
            })}
          </div>

          <div className="audit-stage__body">
            <div className="audit-stage__main">
              <span className="audit-stage__step">
                {committed ? "Workflow complete" : `Step ${activeStep} of 3`}
              </span>
              <CardHeader
                title={
                  committed
                    ? "Batch committed to Stellar"
                    : activeStep === 1
                      ? "Write to the audited MongoDB endpoint"
                      : activeStep === 2
                        ? "Seal this batch"
                        : "Commit the sealed batch"
                }
                subtitle={
                  committed
                    ? "The committed root can now be verified independently."
                    : activeStep === 1
                      ? "Make one or more writes, then the batch can be sealed."
                      : activeStep === 2
                        ? "Lock the captured events before anchoring them on-chain."
                        : lastClosedEpoch
                          ? `Batch #${lastClosedEpoch.epochNumber} is ready to anchor on-chain.`
                          : "Anchor the sealed batch on-chain."
                }
              />

              <div className="audit-stage__stats">
                <Stat label="Batch" value={current?.epochNumber ?? 0} />
                <Stat label="Events captured" value={`${epochEvents} / ${EPOCH_THRESHOLD}`} />
                <Stat label="Fingerprint" value={displayFingerprint} mono />
              </div>

              <ProgressBar current={epochEvents} max={EPOCH_THRESHOLD} tone={hasSealedBatch ? "success" : "accent"} />

              <div className="audit-stage__meta">
                <span>{hasSealedBatch ? "Sealed and ready" : "Recording MongoDB changes"}</span>
              </div>

              {epochEvents === 0 && !closed && activeStep === 1 && (
                <Alert tone="info">
                  Insert, update, or delete a document through <code>{auditedMongoUri}</code>. The audit stack watches that deployment's change stream.
                </Alert>
              )}

              {activeStep === 2 && !closed && (
                <div className="audit-stage__action">
                  <Button
                    variant="primary"
                    loading={closeBusy}
                    disabled={!canClose}
                    onClick={handleCloseEpoch}
                    style={{ width: "100%" }}
                    title={closeDisabledReason ?? "Seal the current batch so it can be committed"}
                  >
                    Seal Batch
                  </Button>
                  {closeDisabledReason && <span>{closeDisabledReason}</span>}
                </div>
              )}

              {(activeStep === 3 || committed) && (
                <div className="audit-stage__action">
                  {commitBusy && (
                    <div className="audit-stage__busy">
                      <Spinner size={13} />
                      <span>{commitStep}</span>
                    </div>
                  )}
                  {commitResult && !commitBusy && (
                    <div className="audit-stage__result">
                      <Badge tone="success" dot>Committed</Badge>
                      {commitResult.txHash && <KeyValue label="Tx hash" value={<TxHashLink txHash={commitResult.txHash} network="testnet" />} />}
                      {commitResult.cid && (
                        <KeyValue
                          label="IPFS CID"
                          value={<IpfsCidLink cid={commitResult.cid} gatewayUrl={commitResult.gatewayUrl} encrypted={commitResult.encrypted} />}
                        />
                      )}
                    </div>
                  )}
                  {!committed && (
                    <Button
                      variant="primary"
                      loading={commitBusy}
                      disabled={!canCommit}
                      onClick={handleCommit}
                      style={{ width: "100%" }}
                      title={commitDisabledReason ?? "Commit the closed epoch to Stellar"}
                    >
                      Commit Batch
                    </Button>
                  )}
                  {commitDisabledReason && !committed && <span>{commitDisabledReason}</span>}
                </div>
              )}
            </div>

            <aside className="audit-stage__aside" aria-label="Current workflow guidance">
              <span className="audit-stage__aside-label">Next available action</span>
              <strong>
                {committed
                  ? "Verify the committed root below"
                  : activeStep === 1
                    ? "Write to MongoDB through the audited URI"
                    : activeStep === 2
                      ? "Seal the captured batch"
                      : "Commit the sealed root to Stellar"}
              </strong>
              <p>
                {committed
                  ? "Use the integrity checks below to refresh chain state, collect attester sign-off, or verify oplog completeness."
                  : activeStep === 1
                    ? "The stepper advances as soon as a write is captured, so there is no disabled action to hunt for."
                    : activeStep === 2
                      ? "Sealing freezes this batch and makes the commitment action available."
                      : "Committing stores the batch root on-chain and returns the transaction details."}
              </p>
            </aside>
          </div>
        </div>
      </Card>

      {/* ─── Key separation proof ────────────────────────────────────── */}
      {status && identities && (
        <Card>
          <CardHeader
            title={<>Key separation<InfoPopover label="Help: Key separation" title="Key separation"><p>The operator (publisher) and the auditor (attester) run as separate processes with distinct Stellar keys. The attester independently recomputes the oplog root and signs on-chain only on an exact match. The operator cannot produce a valid attestation alone.</p></InfoPopover></>}
            subtitle="Publisher and attester are distinct accounts on distinct infrastructure"
            actions={
              identities.publisherAddress && identities.attesterAddress ? (
                <Badge tone="success" dot>Separated</Badge>
              ) : (
                <Badge tone="warning" dot>Same key</Badge>
              )
            }
          />
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-4)" }}>
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
              <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", color: "var(--ink-muted)" }}>
                <KeyRound size={14} />
                <span style={{ fontSize: "var(--font-size-xs)", fontWeight: 600 }}>Publisher (operator)</span>
              </div>
              <code style={{ fontSize: "var(--font-size-xs)", color: "var(--ink)", fontFamily: "var(--font-mono)", wordBreak: "break-all" }}>
                {identities.publisherAddress || "—"}
              </code>
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                Commits batch roots on-chain. Contract admin.
              </span>
            </div>
            <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-1)" }}>
              <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", color: "var(--ink-muted)" }}>
                <ShieldCheck size={14} />
                <span style={{ fontSize: "var(--font-size-xs)", fontWeight: 600 }}>Attester (auditor)</span>
              </div>
              <code style={{ fontSize: "var(--font-size-xs)", color: "var(--ink)", fontFamily: "var(--font-mono)", wordBreak: "break-all" }}>
                {identities.attesterAddress || "—"}
              </code>
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                Independent process. Signs only on root match.
              </span>
            </div>
          </div>
          {identities.publisherAddress && identities.attesterAddress && (
            identities.publisherAddress !== identities.attesterAddress ? (
              <div style={{ marginTop: "var(--space-3)", display: "flex", alignItems: "center", gap: "var(--space-2)", fontSize: "var(--font-size-xs)", color: "var(--success-500)" }}>
                <Users size={13} />
                <span>Two distinct Stellar accounts. The operator cannot self-attest.</span>
              </div>
            ) : (
              <div style={{ marginTop: "var(--space-3)", display: "flex", alignItems: "center", gap: "var(--space-2)", fontSize: "var(--font-size-xs)", color: "var(--danger-500)" }}>
                <ShieldCheck size={13} />
                <span>Warning: publisher and attester share the same key. Re-run setup to regenerate.</span>
              </div>
            )
          )}
        </Card>
      )}

      {/* ─── Integrity row ──────────────────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", gap: "var(--space-4)" }}>
        <StatusCard
          title={<>On-chain root<InfoPopover label="Help: On-chain root" title="On-chain root"><p>The Merkle root previously committed to the Stellar blockchain. Used as the trusted reference to detect tampering.</p></InfoPopover></>}
          status={onChainStatus}
          value={onchain ? shortHash(onchain.rootHex) : pollingOnchain ? "…" : "—"}
          detail={onchain ? `Batch ${onchain.sequence} · ${formatTs(onchain.timestamp)}` : pollingOnchain ? "Waiting for Stellar confirmation…" : "No batch committed yet"}
          action={<Button variant="ghost" onClick={refreshOnchain} loading={pollingOnchain}>Refresh</Button>}
        />

        <StatusCard
          title={<>Multi-party sign-off<InfoPopover label="Help: Multi-party sign-off" title="Multi-party sign-off"><p>Attester nodes cryptographically sign the committed batch root. More attesters increase trust and decentralization.</p></InfoPopover></>}
          status={attestationStatus}
          value={onchain ? `${onChainAttesters.length} attester${onChainAttesters.length !== 1 ? "s" : ""}` : "—"}
          detail={onChainAttesters.length > 0 ? `Signed batch ${onchain?.sequence}` : onchain ? "Awaiting attestation" : "Seal a batch to begin"}
          action={<Button variant="ghost" onClick={() => onchain && refreshOnChainAttesters(onchain.sequence)}>Refresh</Button>}
        />

        <StatusCard
          title={<>Oplog verification<InfoPopover label="Help: Oplog verification" title="Oplog verification"><p>Compares the MongoDB oplog against the audit commitment to verify every database operation is accounted for.</p></InfoPopover></>}
          status={oplogStatus}
          value={oplog ? (oplog.onChainMatchesAuditor ? "Match" : oplog.verdict === "incomplete" ? "Incomplete" : "Mismatch") : "—"}
          detail={oplog ? (oplog.verdict === "incomplete" ? (oplog.explanation ?? "Cannot verify") : `${oplog.oplogEntryCount ?? 0} entries checked`) : "Not verified yet"}
          action={<Button variant="ghost" onClick={handleOplogVerify}>Verify</Button>}
        />
      </div>

      {/* ─── Proof result ─────────────────────────────────────────────── */}
      {proofResult && (
        <Card>
          <CardHeader
            title={<>Inclusion Proof<InfoPopover label="Help: Inclusion Proof" title="Inclusion Proof"><p>A cryptographic Merkle proof showing that a specific event is included in the batch. Can be independently verified against the batch root.</p></InfoPopover></>}
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
                <CheckCircle size={22} />
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
                  className="audit-btn audit-btn--primary"
                  href={`https://stellar.expert/explorer/${proofResult.network === "mainnet" ? "public" : proofResult.network}/tx/${verifyTxHash}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  title="View the on-chain verification transaction in Stellar Expert"
                >
                  <ExternalLink size={14} />
                  View verification tx
                </a>
              ) : (
                <Button
                  variant="primary"
                  loading={verifyBusy}
                  disabled={verifyBusy || !proofResult.txHash}
                  onClick={handleVerifyOnchain}
                  title={
                    proofResult.txHash
                      ? "Submit the proof to the Soroban contract for on-chain verification"
                      : "Batch root must be committed on-chain first"
                  }
                >
                  {verifyBusy ? (
                    "Verifying on-chain…"
                  ) : (
                    <>
                      <ExternalLink size={14} />
                      Verify on-chain
                    </>
                  )}
                </Button>
              )}
              {verifyError && (
                <span className="audit-proof-card__error">{verifyError}</span>
              )}
              <Button
                variant="secondary"
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
                title="Copy the full proof payload as JSON"
              >
                {copyProofHint ? (
                  <>
                    <Check size={14} />
                    Copied
                  </>
                ) : (
                  <>
                    <Copy size={14} />
                    Copy proof
                  </>
                )}
              </Button>
              <button
                className="audit-btn audit-btn--ghost"
                onClick={() => setShowProofDetails((s) => !s)}
                aria-expanded={showProofDetails}
                aria-controls="audit-proof-advanced"
              >
                <ChevronDown
                  size={14}
                  className={`audit-proof-chevron${showProofDetails ? " audit-proof-chevron--open" : ""}`}
                />
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
                    <span className="audit-proof-field__label">Public signals (root, leaf)</span>
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
            title={<>Change Feed<InfoPopover label="Help: Change Feed" title="Change Feed"><p>Real-time stream of audited MongoDB operations. Click Prove to generate a cryptographic Merkle proof for any event.</p></InfoPopover></>}
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
                    size="sm"
                    loading={proofBusy === ev.index}
                    disabled={proofBusy !== null}
                    onClick={() => handleGenerateProof(ev.index)}
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
            title={<>Batches<InfoPopover label="Help: Batches" title="Batches"><p>History of sealed and committed audit batches. Each batch contains a group of events with a cryptographic fingerprint (Merkle root).</p></InfoPopover></>}
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
