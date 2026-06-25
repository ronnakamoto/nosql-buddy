import { useState, useEffect, useCallback } from "react";
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
  Stat,
  ProgressBar,
  KeyValue,
  Alert,
  Spinner,
  EmptyState,
} from "./AuditUi";

/**
 * Dev Mode flow — the full audit system running locally via Docker.
 *
 * Pipeline:
 *  1. Check prerequisites (Docker, compose file, ports, daemon).
 *  2. Bring up the audit stack (publisher + attester + reader containers).
 *  3. Live view queries the docker publisher's HTTP API (via the backend
 *     proxy) for events / root / epochs / on-chain root.
 *  4. Commit flows through the docker publisher (close → publish-ipfs →
 *     commit on-chain).
 *  5. K-of-N attestation status + oplog completeness verification panels
 *     query the publisher + reader daemons.
 */

const PUBLISHER_PORT = 9173;
const READER_PORT = 9175;
const POLL_MS = 2500;

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
interface AttestationStatus {
  epochNumber: number;
  rootHex: string;
  threshold: number;
  totalPublishers: number;
  validAttestations: number;
  thresholdMet: boolean;
  attestedBy: string[];
  pending: string[];
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

export function AuditDevFlow({ onShowSettings }: { onShowSettings: () => void }) {
  const [prereqs, setPrereqs] = useState<DevPrerequisites | null>(null);
  const [stack, setStack] = useState<DevStackStatus | null>(null);
  const [busy, setBusy] = useState(false);
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

  const stackDown = async () => {
    setBusy(true);
    setError(null);
    try {
      await commands.auditDevStackDown();
      await refreshInfra();
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const showLogs = async () => {
    setBusy(true);
    try {
      const l = await commands.auditDevStackLogs(120);
      setLogs(l);
    } catch (e) {
      setError(formatError(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = prereqs?.auditStackRunning ?? false;
  const canStart =
    prereqs &&
    prereqs.dockerInstalled &&
    prereqs.dockerComposeAvailable &&
    prereqs.dockerDaemonRunning &&
    prereqs.composeFilePresent &&
    !ready;

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
          <Badge tone="accent" dot>Dev Mode</Badge>
          <Badge tone={ready ? "success" : "neutral"} dot={ready}>
            {ready ? "Stack running" : "Stack stopped"}
          </Badge>
          <div style={{ flex: 1 }} />
          <Button variant="ghost" onClick={showLogs}>Logs</Button>
          <Button variant="ghost" onClick={onShowSettings}>Settings</Button>
        </div>
      </Card>

      {error && <Alert tone="danger">{error}</Alert>}

      {/* ─── Infrastructure panel ───────────────────────────────────── */}
      <Card>
        <CardHeader
          title="Local Audit Stack"
          subtitle="Publisher · Independent Attester · Reader (Docker Compose)"
          actions={
            ready ? (
              <Button variant="danger" loading={busy} onClick={stackDown}>
                Stop Stack
              </Button>
            ) : (
              <Button variant="primary" loading={busy} disabled={!canStart} onClick={stackUp}>
                Start Stack
              </Button>
            )
          }
        />
        <PrereqGrid prereqs={prereqs} />
        {prereqs && !prereqs.dockerInstalled && (
          <Alert tone="warning">
            Docker is not installed. Install Docker Desktop, then start the 3-node replica set with
            <code style={{ margin: "0 4px" }}>docker compose up -d</code> before starting the audit stack.
          </Alert>
        )}
        {prereqs && prereqs.dockerInstalled && !prereqs.composeFilePresent && (
          <Alert tone="warning">docker-compose.audit.yml not found next to the app.</Alert>
        )}
        {prereqs && canStart && (
          <Alert tone="info">
            Tip: start the MongoDB replica set first (<code>docker compose up -d</code>), then click
            “Start Stack”. Credentials come from <code>.env.audit</code> (see audit-stack.env.example).
          </Alert>
        )}

        {stack && stack.services.length > 0 && (
          <div style={{ marginTop: "var(--space-3)" }}>
            <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)", marginBottom: "var(--space-2)", textTransform: "uppercase", letterSpacing: "0.04em" }}>
              Containers
            </div>
            {stack.services.map((s) => (
              <div
                key={s.name}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-2)",
                  padding: "var(--space-2) 0",
                  borderBottom: "1px solid var(--border)",
                  fontSize: "var(--font-size-sm)",
                }}
              >
                <Badge tone={s.state.toLowerCase().includes("up") || s.state.toLowerCase().includes("running") ? "success" : "neutral"} dot>
                  {s.name}
                </Badge>
                <span style={{ color: "var(--ink-muted)", fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)" }}>
                  {s.state}
                </span>
                <span style={{ flex: 1, color: "var(--ink-faint)", fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)" }}>
                  {s.ports}
                </span>
              </div>
            ))}
          </div>
        )}

        {logs !== null && (
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
              maxHeight: "240px",
              margin: 0,
              whiteSpace: "pre-wrap",
            }}
          >
            {logs || "(no logs)"}
          </pre>
        )}
      </Card>

      {/* ─── Live system view (only when stack is up) ───────────────── */}
      {ready ? (
        <DevLiveView />
      ) : (
        <Card>
          <EmptyState
            icon="🐳"
            title="Audit stack not running"
            body="Start the stack above to bring up the publisher, attester, and reader daemons. The live event feed, on-chain commitments, K-of-N attestation, and oplog verification will appear here."
            action={
              prereqs && canStart ? (
                <Button variant="primary" loading={busy} onClick={stackUp}>Start Stack</Button>
              ) : undefined
            }
          />
        </Card>
      )}
    </div>
  );
}

// ─── Prerequisite grid ──────────────────────────────────────────────────

function PrereqGrid({ prereqs }: { prereqs: DevPrerequisites | null }) {
  if (!prereqs) {
    return (
      <div style={{ display: "flex", gap: "var(--space-2)", color: "var(--ink-faint)" }}>
        <Spinner size={13} /> Checking prerequisites…
      </div>
    );
  }
  const items: [string, boolean, string][] = [
    ["Docker installed", prereqs.dockerInstalled, "Install Docker Desktop"],
    ["Docker Compose", prereqs.dockerComposeAvailable, "Enable the compose plugin"],
    ["Docker daemon", prereqs.dockerDaemonRunning, "Start Docker Desktop"],
    ["Compose file", prereqs.composeFilePresent, "docker-compose.audit.yml missing"],
    ["Ports 9173-9175", prereqs.portsFree, "A port is in use"],
  ];
  return (
    <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-2)" }}>
      {items.map(([label, ok, hint]) => (
        <div
          key={label}
          style={{
            display: "flex",
            alignItems: "center",
            gap: "var(--space-2)",
            padding: "var(--space-2) var(--space-3)",
            background: "var(--surface-2)",
            borderRadius: "var(--radius-md)",
            fontSize: "var(--font-size-sm)",
          }}
        >
          <span style={{ color: ok ? "var(--success-500)" : "var(--danger-500)", fontWeight: 700 }}>
            {ok ? "✓" : "✗"}
          </span>
          <span style={{ color: "var(--ink)" }}>{label}</span>
          {!ok && (
            <span style={{ flex: 1, textAlign: "right", fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {hint}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

// ─── Dev live view (queries the docker publisher via proxy) ─────────────

function DevLiveView() {
  const [status, setStatus] = useState<DaemonStatus | null>(null);
  const [events, setEvents] = useState<DevEvent[]>([]);
  const [epochs, setEpochs] = useState<DevEpoch[]>([]);
  const [current, setCurrent] = useState<DevEpoch | null>(null);
  const [onchain, setOnchain] = useState<DevOnChainRoot | null>(null);
  const [attStatus, setAttStatus] = useState<AttestationStatus | null>(null);
  const [oplog, setOplog] = useState<OplogReport | null>(null);
  const [commitBusy, setCommitBusy] = useState(false);
  const [commitStep, setCommitStep] = useState("");
  const [commitResult, setCommitResult] = useState<{ txHash: string; cid: string } | null>(null);
  const [error, setError] = useState<string | null>(null);

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
      setOnchain((r as DevOnChainRoot) ?? null);
    } catch {
      /* best-effort */
    }
  }, []);

  useEffect(() => {
    refreshOnchain();
  }, [refreshOnchain]);

  const refreshAttestation = useCallback(async () => {
    if (!current || current.endIndex === null) {
      setAttStatus(null);
      return;
    }
    try {
      const r = await commands.auditDevProxyGet(
        PUBLISHER_PORT,
        `attestations/${current.epochNumber}/status`,
      );
      setAttStatus(r as AttestationStatus);
    } catch {
      setAttStatus(null);
    }
  }, [current]);

  useEffect(() => {
    refreshAttestation();
  }, [refreshAttestation]);

  const handleCommit = async () => {
    if (!current) return;
    setCommitBusy(true);
    setError(null);
    setCommitResult(null);
    try {
      setCommitStep("Closing epoch…");
      const closed = (await commands.auditDevProxyPost(PUBLISHER_PORT, "epoch/close", {})) as DevEpoch;
      const num = closed.epochNumber;

      setCommitStep("Pinning batch to IPFS…");
      const pub = (await commands.auditDevProxyPost(
        PUBLISHER_PORT,
        `epoch/${num}/publish-ipfs`,
        {},
      )) as { cid?: string };

      setCommitStep("Committing root on-chain…");
      const res = (await commands.auditDevProxyPost(
        PUBLISHER_PORT,
        `epoch/${num}/commit`,
        {},
      )) as { txHash?: string };

      setCommitResult({ txHash: res?.txHash ?? "", cid: pub?.cid ?? "" });
      setCommitStep("Confirmed!");
      poll();
      refreshOnchain();
      refreshAttestation();
    } catch (e) {
      setError(formatError(e));
      setCommitStep("");
    } finally {
      setCommitBusy(false);
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

  const rootHex = status?.audit.rootHex ?? "";
  const leafCount = status?.audit.leafCount ?? 0;
  const epochEvents = current?.eventCount ?? 0;
  const epochThreshold = 100;
  const closed = current?.endIndex !== null && current?.endIndex !== undefined;

  return (
    <>
      {error && <Alert tone="danger">{error}</Alert>}

      {/* ─── Status + epoch + commit ──────────────────────────────── */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: "var(--space-3)" }}>
        <Card>
          <CardHeader
            title={`Epoch ${current?.epochNumber ?? 0}`}
            subtitle={`${epochEvents} / ${epochThreshold} events until auto-close`}
          />
          <ProgressBar current={epochEvents} max={epochThreshold} tone={closed ? "success" : "accent"} />
          <div style={{ display: "flex", justifyContent: "space-between", marginTop: "var(--space-2)" }}>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              {closed ? "Closed — ready to commit" : "Open — capturing"}
            </span>
            <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
              root {current?.rootHex ? shortHash(current.rootHex) : "—"}
            </span>
          </div>
        </Card>

        <Card>
          <CardHeader title="On-Chain Commitment" subtitle="Publisher → IPFS → Stellar" />
          {commitBusy && (
            <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: "var(--space-2)" }}>
              <Spinner size={13} />
              <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>{commitStep}</span>
            </div>
          )}
          <Button variant="primary" loading={commitBusy} onClick={handleCommit} style={{ width: "100%" }}>
            Commit Now
          </Button>
          {commitResult && (
            <div style={{ marginTop: "var(--space-2)" }}>
              <Badge tone="success" dot>Committed</Badge>
              <div style={{ marginTop: "var(--space-2)" }}>
                {commitResult.txHash && <KeyValue label="Tx hash" value={shortHash(commitResult.txHash)} />}
                {commitResult.cid && <KeyValue label="IPFS CID" value={commitResult.cid} />}
              </div>
            </div>
          )}
        </Card>
      </div>

      {/* ─── On-chain root ─────────────────────────────────────────── */}
      <Card>
        <CardHeader
          title="On-Chain Root"
          subtitle="Latest committed root from the Soroban contract"
          actions={<Button variant="ghost" onClick={refreshOnchain}>Refresh</Button>}
        />
        {onchain ? (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "var(--space-3)" }}>
            <Stat label="Sequence" value={onchain.sequence} mono />
            <Stat label="Root" value={shortHash(onchain.rootHex)} mono />
            <Stat label="Committed" value={formatTs(onchain.timestamp)} />
          </div>
        ) : (
          <EmptyState title="No on-chain commitment yet" body="Commit an epoch to anchor the audit log on Stellar testnet." />
        )}
      </Card>

      {/* ─── K-of-N attestation ────────────────────────────────────── */}
      <Card>
        <CardHeader
          title="K-of-N Attestation"
          subtitle="Independent attester daemons sign epoch roots"
          actions={<Button variant="ghost" onClick={refreshAttestation}>Refresh</Button>}
        />
        {attStatus ? (
          <>
            <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: "var(--space-3)", marginBottom: "var(--space-3)" }}>
              <Stat label="Threshold (K)" value={attStatus.threshold} />
              <Stat label="Valid" value={`${attStatus.validAttestations} / ${attStatus.totalPublishers}`} />
              <Stat label="Status" value={attStatus.thresholdMet ? "Met" : "Pending"} />
            </div>
            <Badge tone={attStatus.thresholdMet ? "success" : "warning"} dot>
              {attStatus.thresholdMet ? "Threshold met — epoch attested" : "Awaiting attestations"}
            </Badge>
            {attStatus.attestedBy.length > 0 && (
              <div style={{ marginTop: "var(--space-2)", fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
                Attested by: {attStatus.attestedBy.join(", ")}
              </div>
            )}
            {attStatus.pending.length > 0 && (
              <div style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                Pending: {attStatus.pending.join(", ") || "none"}
              </div>
            )}
          </>
        ) : (
          <EmptyState
            title="No attestation data"
            body="Close and commit an epoch, then run the independent attester to submit attestations."
          />
        )}
      </Card>

      {/* ─── Oplog completeness verification ───────────────────────── */}
      <Card>
        <CardHeader
          title="Oplog Completeness"
          subtitle="Three-way compare: on-chain vs independent auditor"
          actions={<Button variant="secondary" onClick={handleOplogVerify}>Verify Oplog</Button>}
        />
        {oplog ? (
          <>
            <Alert tone={oplog.allMatch ? "success" : oplog.verdict === "no_commitment" ? "info" : "danger"}>
              {oplog.allMatch
                ? `✓ Complete — oplog root matches across all parties (${oplog.oplogEntryCount ?? 0} entries).`
                : `✗ ${oplog.verdict}: ${oplog.explanation}`}
            </Alert>
            <div style={{ marginTop: "var(--space-2)" }}>
              <KeyValue label="On-chain oplog root" value={shortHash(oplog.onChainOplogRoot)} />
              <KeyValue label="Auditor oplog root" value={oplog.auditorOplogRoot ? shortHash(oplog.auditorOplogRoot) : "—"} />
              <KeyValue label="Oplog entries" value={oplog.oplogEntryCount ?? "—"} />
            </div>
            {oplog.alerts.length > 0 && (
              <div style={{ marginTop: "var(--space-2)", fontSize: "var(--font-size-xs)", color: "var(--danger-500)" }}>
                {oplog.alerts.map((a) => `• ${a}`).join("\n")}
              </div>
            )}
          </>
        ) : (
          <EmptyState
            title="Not verified yet"
            body="Click Verify Oplog to compare the on-chain oplog commitment against the independent auditor's computed oplog root."
          />
        )}
      </Card>

      {/* ─── Event feed ────────────────────────────────────────────── */}
      <Card>
        <CardHeader title="Event Feed" subtitle={`${events.length} event${events.length === 1 ? "" : "s"} · ${leafCount} leaves · root ${shortHash(rootHex)}`} />
        {events.length === 0 ? (
          <EmptyState
            icon="○"
            title="No events yet"
            body="The publisher is watching the replica set's change stream. Write data to MongoDB to populate the audit log."
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
            {events.slice().reverse().map((ev) => (
              <div
                key={ev.index}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: "var(--space-3)",
                  padding: "var(--space-2) var(--space-3)",
                  borderBottom: "1px solid var(--border)",
                  fontSize: "var(--font-size-sm)",
                }}
              >
                <Badge tone={opTone(ev.operation)}>{ev.operation}</Badge>
                <span style={{ fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-muted)" }}>
                  {ev.database}.{ev.collection}
                </span>
                <span style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                  leaf {ev.leafHex.slice(0, 10)}…
                </span>
                <span style={{ fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                  {new Date(ev.timestamp).toLocaleTimeString()}
                </span>
              </div>
            ))}
          </div>
        )}
      </Card>

      {/* ─── Epoch history ─────────────────────────────────────────── */}
      {epochs.length > 0 && (
        <Card>
          <CardHeader title="Epoch History" subtitle={`${epochs.length} epoch${epochs.length === 1 ? "" : "s"}`} />
          <div style={{ display: "flex", flexDirection: "column" }}>
            {epochs
              .slice()
              .reverse()
              .map((ep) => (
                <div
                  key={ep.epochNumber}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "var(--space-3)",
                    padding: "var(--space-2) 0",
                    borderBottom: "1px solid var(--border)",
                    fontSize: "var(--font-size-sm)",
                  }}
                >
                  <span style={{ fontFamily: "var(--font-mono)", color: "var(--ink)", fontWeight: 600 }}>
                    #{ep.epochNumber}
                  </span>
                  <span style={{ color: "var(--ink-muted)" }}>{ep.eventCount} events</span>
                  <span style={{ flex: 1, fontFamily: "var(--font-mono)", fontSize: "var(--font-size-xs)", color: "var(--ink-faint)" }}>
                    {ep.rootHex ? shortHash(ep.rootHex) : "open"}
                  </span>
                  <Badge tone={ep.committed ? "success" : "neutral"}>{ep.committed ? "committed" : "open"}</Badge>
                </div>
              ))}
          </div>
        </Card>
      )}
    </>
  );
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
