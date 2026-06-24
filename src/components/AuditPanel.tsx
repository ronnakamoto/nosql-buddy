import { useEffect, useState, useCallback, Fragment, type CSSProperties, type ReactNode, type InputHTMLAttributes } from "react";
import commands, {
  AuditStatus,
  AuditEvent,
  CommitResult,
  OnChainRoot,
  VerificationReport,
  IpfsPublishResult,
  Publisher,
  AttestationStatus,
  OplogIntegrityReport,
  ConnectionDescriptor,
  formatError,
} from "../ipc/commands";

/**
 * ZK Audit Log panel.
 *
 * Displays the current Merkle root, leaf count, and event list.
 * Allows generating inclusion proofs for individual events.
 * Supports committing roots to Stellar testnet and querying on-chain state.
 */

/** Format a unix-seconds timestamp as "relative · absolute local time". */
function formatOnchainTimestamp(unixSeconds: number): string {
  const ms = unixSeconds * 1000;
  const date = new Date(ms);
  const abs = date.toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
  const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  let rel: string;
  if (diffSec < 60) rel = "just now";
  else if (diffSec < 3600) rel = `${Math.floor(diffSec / 60)}m ago`;
  else if (diffSec < 86400) rel = `${Math.floor(diffSec / 3600)}h ago`;
  else rel = `${Math.floor(diffSec / 86400)}d ago`;
  return `${rel} · ${abs}`;
}

/** Format ms-since-epoch as "Ns ago" for the "last checked" freshness line. */
function formatRelativeMs(ms: number): string {
  const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (diffSec < 60) return `${diffSec}s ago`;
  if (diffSec < 3600) return `${Math.floor(diffSec / 60)}m ago`;
  return `${Math.floor(diffSec / 3600)}h ago`;
}
/** Compact stat chip for the status bar: label + value, divider-separated. */
function Stat({
  label,
  value,
  unit,
  last,
}: {
  label: string;
  value: string | number;
  unit?: string;
  last?: boolean;
}) {
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "baseline",
        gap: "4px",
        paddingRight: last ? 0 : "14px",
        marginRight: last ? 0 : "14px",
        borderRight: last ? "none" : "1px solid var(--border)",
      }}
    >
      <span style={{ color: "var(--ink-faint)" }}>{label}</span>
      <span style={{ color: "var(--ink)", fontWeight: 600 }}>{value}</span>
      {unit && <span style={{ color: "var(--ink-faint)", fontSize: "11px" }}>{unit}</span>}
    </span>
  );
}

/** Shared section card — the standard surface for every tab panel. */
function SectionCard({
  children,
  style,
}: {
  children: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <div
      style={{
        padding: "12px",
        background: "var(--surface-2)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
        ...style,
      }}
    >
      {children}
    </div>
  );
}

/** Section header: title on the left, actions on the right. */
function SectionHeader({
  title,
  actions,
}: {
  title: string;
  actions?: ReactNode;
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        marginBottom: "10px",
        gap: "8px",
        flexWrap: "wrap",
      }}
    >
      <strong
        style={{
          fontSize: "13px",
          fontFamily: "var(--font-sans)",
          fontWeight: 600,
        }}
      >
        {title}
      </strong>
      {actions && <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>{actions}</div>}
    </div>
  );
}

/** Primary action button — accent fill, white text. */
function BtnPrimary({
  children,
  disabled,
  loading,
  loadingLabel,
  onClick,
}: {
  children: ReactNode;
  disabled?: boolean;
  loading?: boolean;
  loadingLabel?: string;
  onClick?: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      style={{
        padding: "4px 12px",
        fontSize: "11px",
        fontFamily: "var(--font-sans)",
        cursor: disabled || loading ? "wait" : "pointer",
        background: "var(--accent-500)",
        color: "#fff",
        border: "none",
        borderRadius: "var(--radius-sm)",
        opacity: disabled || loading ? 0.55 : 1,
        whiteSpace: "nowrap",
      }}
    >
      {loading ? (loadingLabel ?? `${children}...`) : children}
    </button>
  );
}

/** Secondary action button — transparent with border. */
function BtnSecondary({
  children,
  disabled,
  loading,
  loadingLabel,
  onClick,
  title,
}: {
  children: ReactNode;
  disabled?: boolean;
  loading?: boolean;
  loadingLabel?: string;
  onClick?: () => void;
  title?: string;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled || loading}
      title={title}
      style={{
        padding: "4px 12px",
        fontSize: "11px",
        fontFamily: "var(--font-sans)",
        cursor: disabled || loading ? "wait" : "pointer",
        background: "transparent",
        color: "var(--ink)",
        border: "1px solid var(--border-strong)",
        borderRadius: "var(--radius-sm)",
        opacity: disabled || loading ? 0.55 : 1,
        whiteSpace: "nowrap",
      }}
    >
      {loading ? (loadingLabel ?? `${children}...`) : children}
    </button>
  );
}

/** Status pill — semantic-colored badge for state indicators. */
function StatusPill({
  tone,
  children,
}: {
  tone: "success" | "warning" | "danger" | "info";
  children: ReactNode;
}) {
  const colors: Record<string, [string, string, string]> = {
    success: ["var(--success-500)", "color-mix(in oklch, var(--success-500) 15%, var(--surface))", "color-mix(in oklch, var(--success-500) 45%, var(--border))"],
    warning: ["var(--warning-500)", "color-mix(in oklch, var(--warning-500) 15%, var(--surface))", "color-mix(in oklch, var(--warning-500) 45%, var(--border))"],
    danger: ["var(--danger-500)", "color-mix(in oklch, var(--danger-500) 15%, var(--surface))", "color-mix(in oklch, var(--danger-500) 45%, var(--border))"],
    info: ["var(--info-500)", "color-mix(in oklch, var(--info-500) 15%, var(--surface))", "color-mix(in oklch, var(--info-500) 45%, var(--border))"],
  };
  const [fg, bg, border] = colors[tone];
  return (
    <span
      style={{
        display: "inline-flex",
        alignItems: "center",
        gap: "6px",
        padding: "2px 8px",
        fontSize: "11px",
        fontWeight: 600,
        fontFamily: "var(--font-sans)",
        background: bg,
        color: fg,
        border: `1px solid ${border}`,
        borderRadius: "999px",
        lineHeight: "1.4",
      }}
    >
      {children}
    </span>
  );
}

/** Operation badge for the events table — semantic color per operation type. */
function OpBadge({ operation }: { operation: string }) {
  const toneMap: Record<string, "success" | "warning" | "danger" | "info"> = {
    insert: "success",
    update: "warning",
    delete: "danger",
    drop_collection: "danger",
    drop_database: "danger",
    create_index: "info",
    drop_index: "warning",
    rename: "info",
  };
  const tone = toneMap[operation] ?? "info";
  const colors: Record<string, [string, string, string]> = {
    success: ["var(--success-500)", "color-mix(in oklch, var(--success-500) 15%, var(--surface))", "color-mix(in oklch, var(--success-500) 35%, var(--border))"],
    warning: ["var(--warning-500)", "color-mix(in oklch, var(--warning-500) 15%, var(--surface))", "color-mix(in oklch, var(--warning-500) 35%, var(--border))"],
    danger: ["var(--danger-500)", "color-mix(in oklch, var(--danger-500) 15%, var(--surface))", "color-mix(in oklch, var(--danger-500) 35%, var(--border))"],
    info: ["var(--info-500)", "color-mix(in oklch, var(--info-500) 15%, var(--surface))", "color-mix(in oklch, var(--info-500) 35%, var(--border))"],
  };
  const [fg, bg, border] = colors[tone];
  return (
    <span
      style={{
        display: "inline-block",
        padding: "1px 7px",
        fontSize: "10px",
        fontWeight: 600,
        fontFamily: "var(--font-sans)",
        background: bg,
        color: fg,
        border: `1px solid ${border}`,
        borderRadius: "var(--radius-sm)",
        whiteSpace: "nowrap",
      }}
    >
      {operation}
    </span>
  );
}

/** Key-value row grid — the standard data display for result blocks. */
function KVGrid({ rows }: { rows: { label: string; value: ReactNode }[] }) {
  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "auto 1fr",
        gap: "4px 12px",
        fontSize: "11px",
        padding: "8px 10px",
        background: "var(--surface)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-sm)",
      }}
    >
      {rows.map((row, i) => (
        <Fragment key={i}>
          <span style={{ color: "var(--ink-muted)" }}>{row.label}</span>
          <span style={{ color: "var(--ink)" }}>{row.value}</span>
        </Fragment>
      ))}
    </div>
  );
}

/** Teaching empty state — explains what the feature does and how to start. */
function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div style={{ fontSize: "11px", color: "var(--ink-muted)", fontFamily: "var(--font-sans)" }}>
      {children}
    </div>
  );
}

/** Skeleton row for loading states. */
function SkeletonRow({ width }: { width: string }) {
  return (
    <span
      style={{
        display: "inline-block",
        height: "10px",
        width,
        background: "var(--surface-3)",
        borderRadius: "var(--radius-sm)",
        animation: "onchain-pulse 1.2s ease-in-out infinite",
      }}
    />
  );
}

/** Standard text input styled with theme tokens. */
function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      {...props}
      style={{
        padding: "3px 8px",
        fontSize: "12px",
        fontFamily: "var(--font-mono)",
        background: "var(--surface)",
        border: "1px solid var(--border-strong)",
        borderRadius: "var(--radius-sm)",
        color: "var(--ink)",
        ...props.style,
      }}
    />
  );
}

/** Trust chain step state. */
type ChainStepState = "done" | "pending" | "unknown";

/** A single step in the trust chain stepper. */
function TrustChainStep({
  label,
  state,
  active,
  onClick,
  isLast,
}: {
  label: string;
  state: ChainStepState;
  active: boolean;
  onClick: () => void;
  isLast: boolean;
}) {
  const dotColor =
    state === "done"
      ? "var(--success-500)"
      : state === "pending"
        ? "var(--warning-500)"
        : "var(--ink-faint)";
  const dotContent = state === "done" ? "✓" : state === "pending" ? "!" : "";

  return (
    <>
      <button
        onClick={onClick}
        style={{
          display: "flex",
          alignItems: "center",
          gap: "6px",
          background: "none",
          border: "none",
          cursor: "pointer",
          padding: 0,
          fontFamily: "var(--font-sans)",
          fontSize: "11px",
          fontWeight: active ? 600 : 400,
          color: active ? "var(--ink)" : "var(--ink-muted)",
          whiteSpace: "nowrap",
        }}
      >
        <span
          style={{
            display: "inline-flex",
            alignItems: "center",
            justifyContent: "center",
            width: "16px",
            height: "16px",
            borderRadius: "50%",
            border: `1.5px solid ${dotColor}`,
            color: dotColor,
            fontSize: "9px",
            fontWeight: 700,
            background: state === "done" ? "color-mix(in oklch, var(--success-500) 15%, var(--surface))" : "transparent",
            flexShrink: 0,
          }}
        >
          {dotContent}
        </span>
        {label}
      </button>
      {!isLast && (
        <span
          style={{
            color: "var(--border-strong)",
            fontSize: "11px",
            flexShrink: 0,
          }}
        >
          →
        </span>
      )}
    </>
  );
}

/** The trust chain stepper — shows the 4-step workflow at a glance. */
function TrustChain({
  activeTab,
  onNavigate,
  steps,
}: {
  activeTab: string;
  onNavigate: (tab: "events" | "reader" | "ipfs" | "attestation") => void;
  steps: { tab: "events" | "reader" | "ipfs" | "attestation"; label: string; state: ChainStepState }[];
}) {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: "8px",
        padding: "8px 12px",
        background: "var(--surface)",
        border: "1px solid var(--border)",
        borderRadius: "var(--radius-md)",
        flexWrap: "wrap",
      }}
    >
      <span
        style={{
          fontFamily: "var(--font-sans)",
          fontSize: "10px",
          textTransform: "uppercase",
          letterSpacing: "0.06em",
          color: "var(--ink-faint)",
          fontWeight: 600,
          marginRight: "4px",
        }}
      >
        Trust chain
      </span>
      {steps.map((step, i) => (
        <TrustChainStep
          key={step.tab}
          label={step.label}
          state={step.state}
          active={activeTab === step.tab}
          onClick={() => onNavigate(step.tab)}
          isLast={i === steps.length - 1}
        />
      ))}
    </div>
  );
}

/** One-line tab description shown at the top of each tab panel. */
function TabDescription({ children }: { children: ReactNode }) {
  return (
    <p
      style={{
        margin: "0 0 10px 0",
        fontFamily: "var(--font-sans)",
        fontSize: "12px",
        color: "var(--ink-muted)",
        lineHeight: 1.5,
      }}
    >
      {children}
    </p>
  );
}

/** Inline link button that navigates to another tab. */
function TabLink({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        background: "none",
        border: "none",
        color: "var(--accent-600)",
        cursor: "pointer",
        padding: 0,
        fontSize: "inherit",
        fontFamily: "inherit",
        textDecoration: "underline",
        display: "inline",
      }}
    >
      {children}
    </button>
  );
}

export default function AuditPanel() {
  const [status, setStatus] = useState<AuditStatus | null>(null);
  const [events, setEvents] = useState<AuditEvent[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [proofIndex, setProofIndex] = useState<number | null>(null);
  const [proofResult, setProofResult] = useState<string | null>(null);
  const [proofLoading, setProofLoading] = useState(false);
  const [commitLoading, setCommitLoading] = useState(false);
  const [commitResult, setCommitResult] = useState<CommitResult | null>(null);
  const [onchainRoot, setOnchainRoot] = useState<OnChainRoot | null>(null);
  const [onchainLoading, setOnchainLoading] = useState(false);
  // Epoch ms of the last completed check. Distinguishes "never queried"
  // from "queried and the contract returned no commitment".
  const [onchainCheckedAt, setOnchainCheckedAt] = useState<number | null>(null);
  const [onchainCopied, setOnchainCopied] = useState(false);
  const [activeTab, setActiveTab] = useState<
    "events" | "reader" | "ipfs" | "attestation"
  >("events");

  // Reader mode state
  const [verificationReport, setVerificationReport] =
    useState<VerificationReport | null>(null);
  const [verifyLoading, setVerifyLoading] = useState(false);

  // Oplog integrity verification state
  const [oplogReport, setOplogReport] =
    useState<OplogIntegrityReport | null>(null);
  const [oplogLoading, setOplogLoading] = useState(false);
  const [oplogConnectionId, setOplogConnectionId] = useState<string>("");
  const [connections, setConnections] = useState<ConnectionDescriptor[]>([]);

  // IPFS state
  const [ipfsDaemonOnline, setIpfsDaemonOnline] = useState<boolean | null>(
    null,
  );
  const [ipfsPublishResult, setIpfsPublishResult] =
    useState<IpfsPublishResult | null>(null);
  const [ipfsLoading, setIpfsLoading] = useState(false);
  const [ipfsEpochNumber, setIpfsEpochNumber] = useState<number>(0);

  // Attestation state
  const [publishers, setPublishers] = useState<Publisher[]>([]);
  const [attestationStatus, setAttestationStatus] =
    useState<AttestationStatus | null>(null);
  const [attestationThreshold, setAttestationThreshold] = useState<number>(2);
  const [newPublisherKey, setNewPublisherKey] = useState("");
  const [newPublisherName, setNewPublisherName] = useState("");
  const [attestationLoading, setAttestationLoading] = useState(false);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    setError(null);
    try {
      const [s, e] = await Promise.all([
        commands.auditGetStatus(),
        commands.auditListEvents(),
      ]);
      setStatus(s);
      setEvents(e);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleGenerateProof = async (index: number) => {
    setProofIndex(index);
    setProofLoading(true);
    setProofResult(null);
    setError(null);
    try {
      // Use bundled circuit resources (no explicit paths needed).
      const result = await commands.auditGenerateProof(index);
      setProofResult(
        `Proof generated for leaf ${index}.\nRoot: ${result.rootHex}\n` +
          `Public signal: ${result.pubSignals[0]}\n` +
          `Proof A: ${result.proof.a.slice(0, 32)}...`,
      );
    } catch (err) {
      setError(formatError(err));
    } finally {
      setProofLoading(false);
    }
  };

  const handleCommitRoot = async () => {
    setCommitLoading(true);
    setError(null);
    setCommitResult(null);
    try {
      const result = await commands.auditCommitRoot();
      setCommitResult(result);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setCommitLoading(false);
    }
  };

  const handleCheckOnchain = async () => {
    setOnchainLoading(true);
    setError(null);
    try {
      const result = await commands.auditGetOnchainRoot();
      setOnchainRoot(result);
      setOnchainCheckedAt(Date.now());
    } catch (err) {
      setError(formatError(err));
      // A failed check should not leave the user staring at the idle
      // placeholder; record that an attempt was made.
      setOnchainCheckedAt(Date.now());
    } finally {
      setOnchainLoading(false);
    }
  };

  const handleCopyOnchainRoot = async () => {
    if (!onchainRoot) return;
    try {
      await navigator.clipboard.writeText(onchainRoot.rootHex);
      setOnchainCopied(true);
      setTimeout(() => setOnchainCopied(false), 1500);
    } catch {
      // Clipboard may be unavailable in some contexts; fail silently.
    }
  };

  // ─── Reader mode ───────────────────────────────────────────────────
  const handleVerifyReaderMode = async () => {
    setVerifyLoading(true);
    setError(null);
    try {
      const report = await commands.auditVerifyReaderMode();
      setVerificationReport(report);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setVerifyLoading(false);
    }
  };

  // ─── Oplog integrity verification ──────────────────────────────────
  const refreshConnections = useCallback(async () => {
    try {
      const list = await commands.listActiveConnections();
      setConnections(list);
    } catch {
      // Ignore — connections may not be available
    }
  }, []);

  // Refresh connections when the reader tab is opened (for oplog verification).
  useEffect(() => {
    if (activeTab === "reader") {
      refreshConnections();
    }
  }, [activeTab, refreshConnections]);

  const handleVerifyOplogIntegrity = async () => {
    if (!oplogConnectionId) {
      setError("Select a connection to the independent replica member first.");
      return;
    }
    setOplogLoading(true);
    setError(null);
    setOplogReport(null);
    try {
      const report = await commands.auditVerifyOplogIntegrity(oplogConnectionId);
      setOplogReport(report);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setOplogLoading(false);
    }
  };

  // ─── IPFS ──────────────────────────────────────────────────────────
  const handleCheckIpfsDaemon = async () => {
    setError(null);
    try {
      const online = await commands.auditCheckIpfsDaemon();
      setIpfsDaemonOnline(online);
    } catch (err) {
      setError(formatError(err));
    }
  };

  const handlePublishToIpfs = async () => {
    setIpfsLoading(true);
    setError(null);
    setIpfsPublishResult(null);
    try {
      const result = await commands.auditPublishEpochToIpfs(ipfsEpochNumber);
      setIpfsPublishResult(result);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setIpfsLoading(false);
    }
  };

  // ─── Attestation ───────────────────────────────────────────────────
  const refreshPublishers = useCallback(async () => {
    try {
      const list = await commands.auditListPublishers();
      setPublishers(list);
      const threshold = await commands.auditGetAttestationThreshold();
      setAttestationThreshold(threshold);
    } catch (err) {
      // Ignore — attestation may not be initialized
    }
  }, []);

  const handleAddPublisher = async () => {
    if (!newPublisherKey.trim() || !newPublisherName.trim()) return;
    setAttestationLoading(true);
    setError(null);
    try {
      await commands.auditAddPublisher(
        newPublisherKey.trim(),
        newPublisherName.trim(),
      );
      setNewPublisherKey("");
      setNewPublisherName("");
      await refreshPublishers();
    } catch (err) {
      setError(formatError(err));
    } finally {
      setAttestationLoading(false);
    }
  };

  const handleRemovePublisher = async (publicKey: string) => {
    setError(null);
    try {
      await commands.auditRemovePublisher(publicKey);
      await refreshPublishers();
    } catch (err) {
      setError(formatError(err));
    }
  };

  const handleSetThreshold = async () => {
    setError(null);
    try {
      await commands.auditSetAttestationThreshold(attestationThreshold);
    } catch (err) {
      setError(formatError(err));
    }
  };

  const handleCheckAttestationStatus = async () => {
    if (!status) return;
    setAttestationLoading(true);
    setError(null);
    try {
      const result = await commands.auditGetAttestationStatus(
        0,
        status.rootHex,
      );
      setAttestationStatus(result);
    } catch (err) {
      setError(formatError(err));
    } finally {
      setAttestationLoading(false);
    }
  };

  useEffect(() => {
    if (activeTab === "attestation") {
      refreshPublishers();
    }
  }, [activeTab, refreshPublishers]);

  // Derive trust chain step states for the stepper.
  const chainSteps = [
    {
      tab: "events" as const,
      label: "Recorded",
      state: (status && status.eventCount > 0 ? "done" : "unknown") as ChainStepState,
    },
    {
      tab: "events" as const,
      label: "Anchored",
      state: (onchainRoot
        ? (status && onchainRoot.rootHex.toLowerCase() === status.rootHex.toLowerCase() ? "done" : "pending")
        : onchainCheckedAt ? "unknown" : "unknown") as ChainStepState,
    },
    {
      tab: "ipfs" as const,
      label: "Published",
      state: (ipfsPublishResult ? "done" : "unknown") as ChainStepState,
    },
    {
      tab: "attestation" as const,
      label: "Witnessed",
      state: (attestationStatus
        ? (attestationStatus.thresholdMet ? "done" : "pending")
        : "unknown") as ChainStepState,
    },
  ];

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        padding: "16px",
        gap: "12px",
        overflow: "auto",
        fontFamily: "var(--font-mono)",
        fontSize: "13px",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
        }}
      >
        <h2 style={{ margin: 0, fontSize: "16px", fontFamily: "var(--font-sans)", fontWeight: 600 }}>
          ZK Audit Log
        </h2>
        <button
          onClick={refresh}
          disabled={refreshing}
          style={{
            padding: "4px 12px",
            fontSize: "12px",
            fontFamily: "var(--font-sans)",
            cursor: refreshing ? "wait" : "pointer",
            background: "transparent",
            color: "var(--ink-muted)",
            border: "1px solid var(--border-strong)",
            borderRadius: "var(--radius-sm)",
          }}
        >
          {refreshing ? "Refreshing..." : "Refresh"}
        </button>
      </div>

      {error && (
        <div
          style={{
            padding: "8px 12px",
            background: "color-mix(in oklch, var(--danger-500) 12%, var(--surface))",
            border: "1px solid color-mix(in oklch, var(--danger-500) 40%, var(--border))",
            borderRadius: "var(--radius-sm)",
            color: "var(--danger-500)",
            fontFamily: "var(--font-sans)",
            fontSize: "12px",
          }}
        >
          {error}
        </div>
      )}

      {/* Tab navigation */}
      <div style={{ display: "flex", gap: "2px", borderBottom: "1px solid var(--border)" }}>
        {(["events", "reader", "ipfs", "attestation"] as const).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              padding: "6px 14px",
              fontSize: "12px",
              fontFamily: "var(--font-sans)",
              cursor: "pointer",
              background: activeTab === tab ? "var(--surface-2)" : "transparent",
              border: "none",
              borderBottom: activeTab === tab ? "2px solid var(--accent-500)" : "2px solid transparent",
              color: activeTab === tab ? "var(--ink)" : "var(--ink-muted)",
              fontWeight: activeTab === tab ? 600 : 400,
              borderRadius: 0,
            }}
          >
            {tab === "events" ? "Events" : tab === "reader" ? "Verify" : tab === "ipfs" ? "Publish" : "Witnesses"}
          </button>
        ))}
      </div>

      {/* ─── Merkle tree status bar ─────────────────────────────────── */}
      {status && (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "8px",
            padding: "10px 12px",
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-md)",
          }}
        >
          {/* Root hash — the primary artifact, gets its own line */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "8px",
              flexWrap: "wrap",
            }}
          >
            <span
              style={{
                fontFamily: "var(--font-sans)",
                fontSize: "11px",
                color: "var(--ink-muted)",
                flexShrink: 0,
                textTransform: "uppercase",
                letterSpacing: "0.04em",
                fontWeight: 600,
              }}
            >
              Root
            </span>
            <code
              style={{
                fontSize: "12px",
                color: "var(--accent-600)",
                wordBreak: "break-all",
                lineHeight: 1.4,
              }}
            >
              0x{status.rootHex}
            </code>
          </div>

          {/* Stats row — compact, divider-separated */}
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "0",
              fontFamily: "var(--font-sans)",
              fontSize: "12px",
              color: "var(--ink-muted)",
              flexWrap: "wrap",
            }}
          >
            <Stat label="Leaves" value={status.leafCount} />
            <Stat label="Events" value={status.eventCount} />
            <Stat label="Height" value={`${status.treeHeight}`} unit="levels" last />
          </div>
        </div>
      )}

      {/* ─── Trust chain stepper ────────────────────────────────────── */}
      <TrustChain
        activeTab={activeTab}
        onNavigate={setActiveTab}
        steps={chainSteps}
      />

      {/* On-chain commitment section — shown on Events and Reader tabs */}
      {(activeTab === "events" || activeTab === "reader") && (() => {
        // Derive the sync state between the local root and the on-chain root.
        const localRoot = status?.rootHex?.toLowerCase();
        const onchainRootHex = onchainRoot?.rootHex?.toLowerCase();
        type SyncState = "idle" | "loading" | "empty" | "synced" | "diverged";
        let syncState: SyncState;
        if (onchainLoading) syncState = "loading";
        else if (!onchainCheckedAt) syncState = "idle";
        else if (!onchainRoot) syncState = "empty";
        else if (localRoot && onchainRootHex === localRoot) syncState = "synced";
        else syncState = "diverged";

        const badgeStyle = (bg: string, fg: string, border: string): CSSProperties => ({
          display: "inline-flex",
          alignItems: "center",
          gap: "6px",
          padding: "2px 8px",
          fontSize: "11px",
          fontWeight: 600,
          background: bg,
          color: fg,
          border: `1px solid ${border}`,
          borderRadius: "999px",
          lineHeight: "1.4",
        });

        return (
        <div
          style={{
            padding: "12px",
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            borderRadius: "var(--radius-md)",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              marginBottom: "10px",
              gap: "8px",
              flexWrap: "wrap",
            }}
          >
            <strong style={{ fontSize: "13px" }}>
              Stellar On-Chain Commitment
            </strong>
            <div style={{ display: "flex", gap: "8px" }}>
              <button
                onClick={handleCommitRoot}
                disabled={commitLoading || !status}
                style={{
                  padding: "4px 12px",
                  fontSize: "11px",
                  cursor: commitLoading || !status ? "wait" : "pointer",
                  background: "var(--accent-500)",
                  color: "#fff",
                  border: "none",
                  borderRadius: "var(--radius-sm)",
                  opacity: commitLoading || !status ? 0.6 : 1,
                }}
              >
                {commitLoading ? "Committing..." : "Commit Root"}
              </button>
              <button
                onClick={handleCheckOnchain}
                disabled={onchainLoading}
                style={{
                  padding: "4px 12px",
                  fontSize: "11px",
                  cursor: onchainLoading ? "wait" : "pointer",
                  background: "transparent",
                  color: "var(--ink)",
                  border: "1px solid var(--border-strong)",
                  borderRadius: "var(--radius-sm)",
                }}
              >
                {onchainLoading ? "Checking..." : "Check On-Chain"}
              </button>
            </div>
          </div>

          {/* Last commit result (transient confirmation after Commit Root) */}
          {commitResult && (
            <div
              style={{
                fontSize: "11px",
                padding: "6px 10px",
                marginBottom: "8px",
                background: "var(--accent-100)",
                border: "1px solid var(--accent-500)",
                borderRadius: "var(--radius-sm)",
                color: "var(--ink)",
              }}
            >
              <span style={{ opacity: 0.7 }}>Committed:</span> seq #
              {commitResult.sequence} · root{" "}
              <code style={{ fontSize: "11px" }}>
                0x{commitResult.rootHex.slice(0, 12)}…
              </code>
              {commitResult.txHash && (
                <>
                  {" · "}
                  <a
                    href={`https://stellar.expert/explorer/testnet/tx/${commitResult.txHash}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    style={{ color: "var(--accent-600)" }}
                  >
                    view tx ↗
                  </a>
                </>
              )}
            </div>
          )}

          {/* ─── State-aware on-chain result ─── */}

          {syncState === "idle" && (
            <div style={{ fontSize: "11px", color: "var(--ink-muted)", fontFamily: "var(--font-sans)" }}>
              No on-chain root queried yet. Click{" "}
              <strong>Check On-Chain</strong> to fetch the latest commitment
              from Stellar testnet, or{" "}
              <strong>Commit Root</strong> to anchor your current Merkle root.
            </div>
          )}

          {syncState === "loading" && (
            <div
              style={{
                display: "grid",
                gridTemplateColumns: "auto 1fr",
                gap: "6px 12px",
                fontSize: "11px",
              }}
              aria-busy="true"
              aria-live="polite"
            >
              <span style={{ color: "var(--ink-muted)" }}>Status</span>
              <span
                style={{
                  height: "10px",
                  width: "120px",
                  background: "var(--surface-3)",
                  borderRadius: "var(--radius-sm)",
                  animation: "onchain-pulse 1.2s ease-in-out infinite",
                }}
              />
              <span style={{ color: "var(--ink-muted)" }}>Root</span>
              <span
                style={{
                  height: "10px",
                  width: "220px",
                  background: "var(--surface-3)",
                  borderRadius: "var(--radius-sm)",
                  animation: "onchain-pulse 1.2s ease-in-out infinite",
                }}
              />
              <span style={{ color: "var(--ink-muted)" }}>Committed</span>
              <span
                style={{
                  height: "10px",
                  width: "160px",
                  background: "var(--surface-3)",
                  borderRadius: "var(--radius-sm)",
                  animation: "onchain-pulse 1.2s ease-in-out infinite",
                }}
              />
            </div>
          )}

          {syncState === "empty" && (
            <div
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "6px",
                padding: "10px 12px",
                background: "color-mix(in oklch, var(--warning-500) 12%, var(--surface))",
                border: "1px solid color-mix(in oklch, var(--warning-500) 40%, var(--border))",
                borderRadius: "var(--radius-sm)",
                fontSize: "12px",
              }}
            >
              <div style={{ fontWeight: 600, color: "var(--ink)" }}>
                No root committed on-chain yet
              </div>
              <div style={{ color: "var(--ink-muted)" }}>
                The contract returned no commitment. Your local audit log is
                not yet anchored to Stellar testnet. Click{" "}
                <strong>Commit Root</strong> to anchor the current Merkle root.
              </div>
              {onchainCheckedAt && (
                <div style={{ fontSize: "11px", color: "var(--ink-faint)" }}>
                  Checked {formatRelativeMs(onchainCheckedAt)}.
                </div>
              )}
            </div>
          )}

          {(syncState === "synced" || syncState === "diverged") && onchainRoot && (
            <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
              {/* Sync badge — the at-a-glance answer */}
              <div style={{ display: "flex", alignItems: "center", gap: "8px", flexWrap: "wrap" }}>
                {syncState === "synced" ? (
                  <span
                    style={badgeStyle(
                      "color-mix(in oklch, var(--success-500) 15%, var(--surface))",
                      "var(--success-500)",
                      "color-mix(in oklch, var(--success-500) 45%, var(--border))",
                    )}
                  >
                    ● In sync
                  </span>
                ) : (
                  <span
                    style={badgeStyle(
                      "color-mix(in oklch, var(--info-500) 15%, var(--surface))",
                      "var(--info-500)",
                      "color-mix(in oklch, var(--info-500) 45%, var(--border))",
                    )}
                  >
                    ● Local root ahead
                  </span>
                )}
                <span style={{ fontSize: "11px", color: "var(--ink-muted)" }}>
                  seq #{onchainRoot.sequence}
                </span>
                {onchainCheckedAt && (
                  <span style={{ fontSize: "11px", color: "var(--ink-faint)" }}>
                    · checked {formatRelativeMs(onchainCheckedAt)}
                  </span>
                )}
              </div>

              {syncState === "diverged" && (
                <div style={{ fontSize: "11px", color: "var(--ink-muted)" }}>
                  The on-chain root differs from your local root. This is
                  expected after new audit events. Commit the current root to
                  re-anchor, or run{" "}
                  <button
                    onClick={() => setActiveTab("reader")}
                    style={{
                      background: "none",
                      border: "none",
                      color: "var(--accent-600)",
                      cursor: "pointer",
                      padding: 0,
                      fontSize: "11px",
                      textDecoration: "underline",
                    }}
                  >
                    Reader Mode verification
                  </button>{" "}
                  to detect tampering.
                </div>
              )}

              {/* Structured result rows */}
              <div
                style={{
                  display: "grid",
                  gridTemplateColumns: "auto 1fr",
                  gap: "4px 12px",
                  fontSize: "11px",
                  padding: "8px 10px",
                  background: "var(--surface)",
                  border: "1px solid var(--border)",
                  borderRadius: "var(--radius-sm)",
                }}
              >
                <span style={{ color: "var(--ink-muted)" }}>On-chain root</span>
                <span style={{ display: "flex", alignItems: "center", gap: "6px" }}>
                  <code
                    style={{
                      wordBreak: "break-all",
                      color: "var(--ink)",
                      fontSize: "11px",
                    }}
                  >
                    0x{onchainRoot.rootHex}
                  </code>
                  <button
                    onClick={handleCopyOnchainRoot}
                    title="Copy root hash"
                    style={{
                      background: "none",
                      border: "1px solid var(--border-strong)",
                      borderRadius: "var(--radius-sm)",
                      padding: "1px 6px",
                      fontSize: "10px",
                      cursor: "pointer",
                      color: "var(--ink-muted)",
                      flexShrink: 0,
                    }}
                  >
                    {onchainCopied ? "Copied" : "Copy"}
                  </button>
                </span>

                <span style={{ color: "var(--ink-muted)" }}>Committed</span>
                <span style={{ color: "var(--ink)" }}>
                  {formatOnchainTimestamp(onchainRoot.timestamp)}
                </span>

                {onchainRoot.metadata && (
                  <>
                    <span style={{ color: "var(--ink-muted)" }}>Metadata</span>
                    <span
                      style={{
                        wordBreak: "break-all",
                        color: "var(--ink)",
                      }}
                    >
                      {onchainRoot.metadata}
                    </span>
                  </>
                )}

                <span style={{ color: "var(--ink-muted)" }}>Explorer</span>
                <span>
                  <a
                    href={`https://stellar.expert/explorer/testnet/contract/${"CCUCFDRF6IMY3STBIFRBBGFFPETBSAPPTDACNOBYWKNPG5QUCAMGGUQ5"}`}
                    target="_blank"
                    rel="noopener noreferrer"
                    style={{ color: "var(--accent-600)" }}
                  >
                    view contract on testnet ↗
                  </a>
                </span>
              </div>
            </div>
          )}
        </div>
        );
      })()}

      <style>{`
        @keyframes onchain-pulse {
          0%, 100% { opacity: 0.45; }
          50% { opacity: 0.9; }
        }
        @media (prefers-reduced-motion: reduce) {
          @keyframes onchain-pulse {
            0%, 100% { opacity: 0.7; }
          }
        }
      `}</style>

      {/* ─── Reader Mode tab ─────────────────────────────────────────── */}
      {activeTab === "reader" && (
        <>
        <SectionCard>
          <SectionHeader
            title="Verify Integrity"
            actions={
              <BtnPrimary onClick={handleVerifyReaderMode} loading={verifyLoading} loadingLabel="Verifying...">
                Verify
              </BtnPrimary>
            }
          />
          <TabDescription>
            Check your local audit log against the on-chain anchor to detect
            tampering. This reads the latest committed root from Stellar and
            verifies every event up to the commitment point.
          </TabDescription>

          {/* Loading skeleton */}
          {verifyLoading && !verificationReport && (
            <div
              style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 12px", fontSize: "11px" }}
              aria-busy="true"
              aria-live="polite"
            >
              <span style={{ color: "var(--ink-muted)" }}>Status</span>
              <SkeletonRow width="180px" />
              <span style={{ color: "var(--ink-muted)" }}>Root</span>
              <SkeletonRow width="220px" />
              <span style={{ color: "var(--ink-muted)" }}>Events</span>
              <SkeletonRow width="100px" />
            </div>
          )}

          {verificationReport && (() => {
            const r = verificationReport;
            const tone = r.tamperDetected ? "danger" : r.onchainRootFound ? "success" : "warning";
            const icon = r.tamperDetected ? "●" : r.onchainRootFound ? "✓" : "○";
            const label = r.tamperDetected
              ? "Tamper detected"
              : r.onchainRootFound
                ? "Log verified"
                : "No on-chain anchor";
            return (
              <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
                {/* Summary banner */}
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "8px",
                    padding: "8px 12px",
                    background: "color-mix(in oklch, var(--" + tone + "-500) 12%, var(--surface))",
                    border: "1px solid color-mix(in oklch, var(--" + tone + "-500) 40%, var(--border))",
                    borderRadius: "var(--radius-sm)",
                    fontSize: "12px",
                    fontFamily: "var(--font-sans)",
                    color: "var(--ink)",
                  }}
                >
                  <StatusPill tone={tone}>{icon} {label}</StatusPill>
                  <span style={{ color: "var(--ink-muted)" }}>{r.summary}</span>
                </div>

                {/* Verification details */}
                <KVGrid
                  rows={[
                    { label: "On-chain root found", value: r.onchainRootFound ? "yes" : "no" },
                    { label: "Commitment event", value: r.commitmentEventIndex ?? "—" },
                    { label: "Total events", value: r.totalEvents },
                    { label: "Verified events", value: r.verifiedEvents },
                    { label: "Events after commitment", value: r.eventsAfterCommitment },
                    {
                      label: "Chain intact",
                      value: (
                        <span style={{ color: r.chainIntact ? "var(--success-500)" : "var(--danger-500)", fontWeight: 600 }}>
                          {r.chainIntact ? "yes" : "no"}
                        </span>
                      ),
                    },
                    {
                      label: "Tamper detected",
                      value: (
                        <span style={{ color: r.tamperDetected ? "var(--danger-500)" : "var(--success-500)", fontWeight: 600 }}>
                          {r.tamperDetected ? "YES" : "no"}
                        </span>
                      ),
                    },
                  ]}
                />
              </div>
            );
          })()}

          {!verificationReport && !verifyLoading && (
            <EmptyState>
              Click <strong>Verify</strong> to check your local audit log
              against the latest on-chain root. If you haven't anchored a root
              yet,{" "}
              <TabLink onClick={() => setActiveTab("events")}>
                commit one first
              </TabLink>{" "}
              on the Events tab.
            </EmptyState>
          )}
        </SectionCard>

        {/* ─── Oplog integrity verification ─────────────────────────── */}
        <SectionCard>
          <SectionHeader
            title="Verify Oplog Integrity"
            actions={
              <BtnPrimary
                onClick={handleVerifyOplogIntegrity}
                loading={oplogLoading}
                loadingLabel="Verifying..."
                disabled={!oplogConnectionId}
              >
                Verify Oplog
              </BtnPrimary>
            }
          />
          <TabDescription>
            Three-way compare: checks the on-chain oplog root against an
            independent computation from your own replica member. This detects
            omitted writes — the operator's replication betrays them. Connect
            to the <strong>independent replica member</strong> (not the
            operator's server).
          </TabDescription>

          {/* Connection selector */}
          <div style={{ display: "flex", gap: "8px", alignItems: "center", marginBottom: "12px" }}>
            <label
              htmlFor="oplog-connection-select"
              style={{ fontSize: "11px", color: "var(--ink-muted)", fontFamily: "var(--font-sans)" }}
            >
              Independent member:
            </label>
            <select
              id="oplog-connection-select"
              value={oplogConnectionId}
              onChange={(e) => setOplogConnectionId(e.target.value)}
              style={{
                fontSize: "11px",
                fontFamily: "var(--font-sans)",
                padding: "4px 8px",
                borderRadius: "var(--radius-sm)",
                border: "1px solid var(--border)",
                background: "var(--surface)",
                color: "var(--ink)",
                minWidth: "200px",
              }}
            >
              <option value="">Select a connection…</option>
              {connections.map((c) => (
                <option key={c.connectionId} value={c.connectionId}>
                  {c.name} ({c.connectionId.slice(0, 12)}…)
                </option>
              ))}
            </select>
            {connections.length === 0 && (
              <span style={{ fontSize: "10px", color: "var(--ink-muted)" }}>
                No active connections — open one to the independent replica first.
              </span>
            )}
          </div>

          {/* Loading skeleton */}
          {oplogLoading && !oplogReport && (
            <div
              style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 12px", fontSize: "11px" }}
              aria-busy="true"
              aria-live="polite"
            >
              <span style={{ color: "var(--ink-muted)" }}>Verifying</span>
              <SkeletonRow width="180px" />
            </div>
          )}

          {/* Oplog verification result */}
          {oplogReport && (() => {
            const r = oplogReport;
            const tone =
              r.verdict === "complete" ? "success" :
              r.verdict === "mismatch" ? "danger" :
              r.verdict === "stale" ? "warning" : "warning";
            const icon =
              r.verdict === "complete" ? "✓" :
              r.verdict === "mismatch" ? "✗" :
              r.verdict === "stale" ? "⚠" : "○";
            const label =
              r.verdict === "complete" ? "Oplog verified" :
              r.verdict === "mismatch" ? "Omission detected" :
              r.verdict === "stale" ? "Stale — oplog rolled over" :
              r.verdict === "no_commitment" ? "No on-chain commitment" :
              r.verdict === "no_oplog_commitment" ? "No oplog commitment" :
              "Verification error";
            return (
              <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
                {/* Summary banner */}
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "8px",
                    padding: "8px 12px",
                    background: "color-mix(in oklch, var(--" + tone + "-500) 12%, var(--surface))",
                    border: "1px solid color-mix(in oklch, var(--" + tone + "-500) 40%, var(--border))",
                    borderRadius: "var(--radius-sm)",
                    fontSize: "12px",
                    fontFamily: "var(--font-sans)",
                    color: "var(--ink)",
                  }}
                >
                  <StatusPill tone={tone}>{icon} {label}</StatusPill>
                  <span style={{ color: "var(--ink-muted)" }}>Epoch #{r.sequence}</span>
                </div>

                {/* Details */}
                <KVGrid
                  rows={[
                    { label: "On-chain oplog root", value: r.onChainOplogRoot === "none" ? "—" : `${r.onChainOplogRoot.slice(0, 24)}…` },
                    {
                      label: "Auditor's computed root",
                      value: r.auditorOplogRoot ? `${r.auditorOplogRoot.slice(0, 24)}…` : "—",
                    },
                    { label: "Oplog entries", value: r.oplogEntryCount ?? "—" },
                    {
                      label: "On-chain matches auditor",
                      value: (
                        <span style={{ color: r.onChainMatchesAuditor ? "var(--success-500)" : "var(--danger-500)", fontWeight: 600 }}>
                          {r.onChainMatchesAuditor ? "yes" : "no"}
                        </span>
                      ),
                    },
                    {
                      label: "Verdict",
                      value: (
                        <span style={{ fontWeight: 600 }}>{r.verdict}</span>
                      ),
                    },
                  ]}
                />

                {/* Explanation */}
                <div style={{ fontSize: "11px", color: "var(--ink-muted)", fontFamily: "var(--font-sans)", lineHeight: 1.5 }}>
                  {r.explanation}
                </div>

                {/* Alerts */}
                {r.alerts.length > 0 && (
                  <div style={{ display: "flex", flexDirection: "column", gap: "4px" }}>
                    {r.alerts.map((alert, i) => (
                      <div
                        key={i}
                        style={{
                          fontSize: "10px",
                          fontFamily: "var(--font-mono)",
                          color: "var(--danger-500)",
                          padding: "4px 8px",
                          background: "color-mix(in oklch, var(--danger-500) 8%, var(--surface))",
                          borderRadius: "var(--radius-sm)",
                        }}
                      >
                        {alert}
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })()}

          {!oplogReport && !oplogLoading && (
            <EmptyState>
              Click <strong>Verify Oplog</strong> to check the on-chain oplog
              root against your independent computation. This detects omitted
              writes — if the operator skipped a write, the oplog hashes won't
              match.
            </EmptyState>
          )}
        </SectionCard>
        </>
      )}

      {/* ─── IPFS tab ────────────────────────────────────────────────── */}
      {activeTab === "ipfs" && (
        <SectionCard>
          <SectionHeader
            title="Publish to IPFS"
            actions={
              <BtnSecondary onClick={handleCheckIpfsDaemon}>
                Check Daemon
              </BtnSecondary>
            }
          />
          <TabDescription>
            Share epoch batches to IPFS so others can independently verify your
            audit log without trusting your server. Requires a running IPFS
            daemon (Kubo) at localhost:5001.
          </TabDescription>

          {/* Daemon status */}
          {ipfsDaemonOnline !== null && (
            <div style={{ marginBottom: "10px" }}>
              <StatusPill tone={ipfsDaemonOnline ? "success" : "danger"}>
                {ipfsDaemonOnline ? "● Daemon online" : "● Daemon offline"}
              </StatusPill>
            </div>
          )}

          {/* Publish controls */}
          <div style={{ display: "flex", gap: "8px", alignItems: "center", marginBottom: "10px" }}>
            <label
              style={{
                fontSize: "11px",
                fontFamily: "var(--font-sans)",
                color: "var(--ink-muted)",
              }}
            >
              Epoch
            </label>
            <TextInput
              type="number"
              min={0}
              value={ipfsEpochNumber}
              onChange={(e) => setIpfsEpochNumber(Number(e.target.value))}
              style={{ width: "60px" }}
            />
            <BtnPrimary
              onClick={handlePublishToIpfs}
              loading={ipfsLoading}
              loadingLabel="Publishing..."
            >
              Publish to IPFS
            </BtnPrimary>
          </div>

          {/* Loading skeleton */}
          {ipfsLoading && !ipfsPublishResult && (
            <div
              style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 12px", fontSize: "11px" }}
              aria-busy="true"
              aria-live="polite"
            >
              <span style={{ color: "var(--ink-muted)" }}>CID</span>
              <SkeletonRow width="240px" />
              <span style={{ color: "var(--ink-muted)" }}>Size</span>
              <SkeletonRow width="80px" />
            </div>
          )}

          {/* Result */}
          {ipfsPublishResult && (
            <KVGrid
              rows={[
                {
                  label: "CID",
                  value: (
                    <code style={{ wordBreak: "break-all", color: "var(--accent-600)", fontSize: "11px" }}>
                      {ipfsPublishResult.cid}
                    </code>
                  ),
                },
                { label: "Epoch", value: ipfsPublishResult.epochNumber },
                { label: "Events", value: ipfsPublishResult.eventCount },
                { label: "Size", value: `${ipfsPublishResult.batchSizeBytes} bytes` },
                {
                  label: "Gateway",
                  value: (
                    <a
                      href={ipfsPublishResult.gatewayUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      style={{ color: "var(--accent-600)" }}
                    >
                      {ipfsPublishResult.gatewayUrl} ↗
                    </a>
                  ),
                },
              ]}
            />
          )}

          {/* Empty state */}
          {!ipfsPublishResult && !ipfsLoading && (
            <EmptyState>
              Select an epoch number and click <strong>Publish to IPFS</strong>
              {" "}to share its event batch. The CID can then be committed as
              on-chain metadata. Make sure a Kubo daemon is running at
              localhost:5001. Need to anchor first?{" "}
              <TabLink onClick={() => setActiveTab("events")}>
                Commit a root on the Events tab
              </TabLink>
              .
            </EmptyState>
          )}
        </SectionCard>
      )}

      {/* ─── Attestation tab ─────────────────────────────────────────── */}
      {activeTab === "attestation" && (
        <div style={{ display: "flex", flexDirection: "column", gap: "12px" }}>
          {/* Threshold config + status */}
          <SectionCard>
            <SectionHeader
              title="Witnesses (K-of-N)"
              actions={
                <>
                  <label
                    style={{
                      fontSize: "11px",
                      fontFamily: "var(--font-sans)",
                      color: "var(--ink-muted)",
                    }}
                  >
                    K
                  </label>
                  <TextInput
                    type="number"
                    min={1}
                    value={attestationThreshold}
                    onChange={(e) => setAttestationThreshold(Number(e.target.value))}
                    style={{ width: "40px" }}
                  />
                  <BtnSecondary onClick={handleSetThreshold}>Set</BtnSecondary>
                  <BtnSecondary
                    onClick={handleCheckAttestationStatus}
                    disabled={attestationLoading || !status}
                    loading={attestationLoading}
                    loadingLabel="Checking..."
                  >
                    Check Status
                  </BtnSecondary>
                </>
              }
            />
            <TabDescription>
              Register publishers who sign epoch roots with their ed25519 keys.
              A threshold of K-of-N signatures is required to trust a
              commitment, preventing any single party from forging the log.
            </TabDescription>

            {/* Loading skeleton */}
            {attestationLoading && !attestationStatus && (
              <div
                style={{ display: "grid", gridTemplateColumns: "auto 1fr", gap: "6px 12px", fontSize: "11px" }}
                aria-busy="true"
                aria-live="polite"
              >
                <span style={{ color: "var(--ink-muted)" }}>Threshold</span>
                <SkeletonRow width="120px" />
                <span style={{ color: "var(--ink-muted)" }}>Attested by</span>
                <SkeletonRow width="180px" />
              </div>
            )}

            {attestationStatus && (() => {
              const s = attestationStatus;
              return (
                <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
                  {/* Threshold status banner */}
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      gap: "8px",
                      flexWrap: "wrap",
                    }}
                  >
                    <StatusPill tone={s.thresholdMet ? "success" : "warning"}>
                      {s.thresholdMet ? "✓ Threshold met" : "○ Threshold pending"}
                    </StatusPill>
                    <span style={{ fontSize: "12px", fontFamily: "var(--font-sans)", color: "var(--ink)" }}>
                      {s.validAttestations}/{s.threshold}
                    </span>
                    <span style={{ fontSize: "11px", color: "var(--ink-muted)" }}>
                      of {s.totalPublishers} publishers
                    </span>
                  </div>

                  {/* Attested by */}
                  {s.attestedBy.length > 0 && (
                    <div style={{ fontSize: "11px", fontFamily: "var(--font-sans)" }}>
                      <span style={{ color: "var(--ink-muted)", fontWeight: 600 }}>Attested by: </span>
                      <span style={{ color: "var(--ink)", fontFamily: "var(--font-mono)" }}>
                        {s.attestedBy.map((pk) => pk.slice(0, 12) + "...").join(", ")}
                      </span>
                    </div>
                  )}

                  {/* Pending */}
                  {s.pending.length > 0 && (
                    <div style={{ fontSize: "11px", fontFamily: "var(--font-sans)" }}>
                      <span style={{ color: "var(--ink-faint)", fontWeight: 600 }}>Pending: </span>
                      <span style={{ color: "var(--ink-faint)", fontFamily: "var(--font-mono)" }}>
                        {s.pending.map((pk) => pk.slice(0, 12) + "...").join(", ")}
                      </span>
                    </div>
                  )}
                </div>
              );
            })()}

            {!attestationStatus && !attestationLoading && (
              <EmptyState>
                Click <strong>Check Status</strong> to see how many witnesses
                have signed the current root. You need at least one{" "}
                <TabLink onClick={() => setActiveTab("ipfs")}>
                  published epoch
                </TabLink>{" "}
                and registered publishers below before attestation can begin.
              </EmptyState>
            )}
          </SectionCard>

          {/* Publisher management */}
          <SectionCard>
            <SectionHeader
              title={`Registered Witnesses (${publishers.length})`}
              actions={
                <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
                  <TextInput
                    type="text"
                    placeholder="Public key (hex, 32 bytes)"
                    value={newPublisherKey}
                    onChange={(e) => setNewPublisherKey(e.target.value)}
                    style={{ flex: 1, minWidth: "180px" }}
                  />
                  <TextInput
                    type="text"
                    placeholder="Name"
                    value={newPublisherName}
                    onChange={(e) => setNewPublisherName(e.target.value)}
                    style={{ width: "100px" }}
                  />
                  <BtnPrimary
                    onClick={handleAddPublisher}
                    disabled={attestationLoading || !newPublisherKey.trim() || !newPublisherName.trim()}
                  >
                    Add
                  </BtnPrimary>
                </div>
              }
            />

            {publishers.length === 0 ? (
              <EmptyState>
                No witnesses registered yet. Add a publisher above with their
                ed25519 public key and a name. Once you have at least K
                publishers, they can sign epoch roots to attest the audit log.
              </EmptyState>
            ) : (
              <table
                style={{
                  width: "100%",
                  borderCollapse: "collapse",
                  fontSize: "11px",
                  fontFamily: "var(--font-sans)",
                }}
              >
                <thead>
                  <tr style={{ textAlign: "left", borderBottom: "1px solid var(--border)" }}>
                    <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Name</th>
                    <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Public Key</th>
                    <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Registered</th>
                    <th style={{ padding: "6px 8px" }}></th>
                  </tr>
                </thead>
                <tbody>
                  {publishers.map((p) => (
                    <tr key={p.publicKey} style={{ borderBottom: "1px solid var(--border)" }}>
                      <td style={{ padding: "6px 8px", color: "var(--ink)" }}>{p.name}</td>
                      <td style={{ padding: "6px 8px", fontFamily: "var(--font-mono)", color: "var(--ink-muted)" }}>
                        {p.publicKey.slice(0, 24)}...
                      </td>
                      <td style={{ padding: "6px 8px", color: "var(--ink-faint)" }}>
                        {p.registeredAt.slice(0, 10)}
                      </td>
                      <td style={{ padding: "6px 8px" }}>
                        <BtnSecondary
                          onClick={() => handleRemovePublisher(p.publicKey)}
                        >
                          Remove
                        </BtnSecondary>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </SectionCard>
        </div>
      )}

      {proofResult && activeTab === "events" && (
        <div
          style={{
            padding: "10px 12px",
            background: "color-mix(in oklch, var(--accent-500) 10%, var(--surface))",
            border: "1px solid var(--accent-500)",
            borderRadius: "var(--radius-sm)",
            whiteSpace: "pre-wrap",
            fontSize: "11px",
            fontFamily: "var(--font-mono)",
            color: "var(--ink)",
            position: "relative",
          }}
        >
          <button
            onClick={() => setProofResult(null)}
            style={{
              position: "absolute",
              top: "8px",
              right: "8px",
              background: "none",
              border: "1px solid var(--border-strong)",
              borderRadius: "var(--radius-sm)",
              padding: "1px 6px",
              fontSize: "10px",
              cursor: "pointer",
              color: "var(--ink-muted)",
              fontFamily: "var(--font-sans)",
            }}
          >
            Close
          </button>
          <div style={{ fontWeight: 600, fontFamily: "var(--font-sans)", fontSize: "11px", marginBottom: "4px" }}>
            Proof Result
          </div>
          {proofResult}
        </div>
      )}

      {activeTab === "events" && (
      <div>
        <h3
          style={{
            fontSize: "13px",
            margin: "0 0 8px 0",
            fontFamily: "var(--font-sans)",
            fontWeight: 600,
            color: "var(--ink)",
          }}
        >
          Audit Events ({events.length})
        </h3>
        <TabDescription>
          Every write operation is automatically hashed into a tamper-evident
          Merkle tree. Generate a ZK inclusion proof for any event to prove it
          was recorded without revealing the others.
        </TabDescription>
        {events.length === 0 ? (
          <EmptyState>
            No audit events recorded yet. Once you perform a write operation
            (insert, update, or delete) in any collection, it will be
            automatically captured here. Then{" "}
            <TabLink onClick={() => setActiveTab("reader")}>
              verify the log
            </TabLink>{" "}
            and{" "}
            <TabLink onClick={() => setActiveTab("ipfs")}>
              publish it to IPFS
            </TabLink>{" "}
            to complete the trust chain.
          </EmptyState>
        ) : (
          <table
            style={{
              width: "100%",
              borderCollapse: "collapse",
              fontSize: "12px",
              fontFamily: "var(--font-sans)",
            }}
          >
            <thead>
              <tr
                style={{
                  borderBottom: "1px solid var(--border)",
                  textAlign: "left",
                }}
              >
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>#</th>
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Operation</th>
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Database</th>
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Collection</th>
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Leaf Hash</th>
                <th style={{ padding: "6px 8px", color: "var(--ink-muted)", fontWeight: 600 }}>Time</th>
                <th style={{ padding: "6px 8px" }}></th>
              </tr>
            </thead>
            <tbody>
              {events.map((event) => (
                <tr
                  key={event.index}
                  style={{ borderBottom: "1px solid var(--border)" }}
                >
                  <td style={{ padding: "6px 8px", color: "var(--ink-faint)" }}>{event.index}</td>
                  <td style={{ padding: "6px 8px" }}>
                    <OpBadge operation={event.operation} />
                  </td>
                  <td style={{ padding: "6px 8px", color: "var(--ink)" }}>{event.database}</td>
                  <td style={{ padding: "6px 8px", color: "var(--ink)" }}>{event.collection}</td>
                  <td
                    style={{
                      padding: "6px 8px",
                      color: "var(--ink-muted)",
                      fontSize: "11px",
                      fontFamily: "var(--font-mono)",
                      maxWidth: "120px",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                    title={event.leafHex}
                  >
                    {event.leafHex.slice(0, 16)}...
                  </td>
                  <td
                    style={{
                      padding: "6px 8px",
                      color: "var(--ink-faint)",
                      fontSize: "11px",
                      fontFamily: "var(--font-mono)",
                    }}
                  >
                    {event.timestamp.slice(11, 19)}
                  </td>
                  <td style={{ padding: "6px 8px" }}>
                    <BtnSecondary
                      onClick={() => handleGenerateProof(event.index)}
                      loading={proofLoading && proofIndex === event.index}
                      loadingLabel="Proving..."
                    >
                      Prove
                    </BtnSecondary>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
      )}
    </div>
  );
}
