import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { X } from "lucide-react";
import { formatShortcut, isMac, parseShortcut } from "../lib/shortcutUtils";
import { LogViewer, type LogViewerStats } from "./LogViewer";
import "./audit.css";

/**
 * Shared UI primitives for the redesigned audit system.
 *
 * One cohesive visual language built on the app's theme tokens
 * (theme.css). Used by the mode chooser, dev flow, production flow,
 * and settings so every audit surface looks and behaves the same.
 */

const STELLAR_EXPLORER_BASE = "https://stellar.expert/explorer";

function explorerNetworkPath(network: "testnet" | "mainnet"): string {
  return network === "mainnet" ? "public" : network;
}

function getExplorerUrl(network: "testnet" | "mainnet", txHash: string): string {
  return `${STELLAR_EXPLORER_BASE}/${explorerNetworkPath(network)}/tx/${txHash}`;
}

function getContractExplorerUrl(
  network: "testnet" | "mainnet",
  contractId: string,
): string {
  return `${STELLAR_EXPLORER_BASE}/${explorerNetworkPath(network)}/contract/${contractId}`;
}

/**
 * Renders a transaction hash as a clickable link to the Stellar explorer.
 * Shows a truncated hash with hover underline; opens in the system browser.
 */
export function TxHashLink({
  txHash,
  network = "testnet",
  showExternalIcon = true,
}: {
  txHash: string;
  network?: "testnet" | "mainnet";
  showExternalIcon?: boolean;
}) {
  if (!txHash) return <span style={{ color: "var(--ink-faint)" }}>—</span>;
  const href = getExplorerUrl(network, txHash);
  const display = txHash.length > 20 ? `${txHash.slice(0, 10)}…${txHash.slice(-8)}` : txHash;
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      title={`View on Stellar Explorer: ${txHash}`}
      style={{
        color: "var(--link)",
        textDecoration: "none",
        fontFamily: "var(--font-mono)",
        letterSpacing: "var(--letter-mono)",
        fontSize: "var(--font-size-sm)",
        display: "inline-flex",
        alignItems: "center",
        gap: "4px",
        cursor: "pointer",
        transition: "color 0.12s ease",
      }}
    >
      {display}
      {showExternalIcon && (
        <svg
          width="11"
          height="11"
          viewBox="0 0 16 16"
          fill="none"
          style={{ opacity: 0.6, flexShrink: 0 }}
        >
          <path
            d="M6 3h7v7M13 3L6 10M11 9v4a1 1 0 01-1 1H4a1 1 0 01-1-1V7a1 1 0 011-1h4"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      )}
    </a>
  );
}

/**
 * Renders a Soroban contract ID as a clickable link to the Stellar explorer.
 * Opens the contract page so anyone can inspect the on-chain root/commitment.
 */
export function ContractLink({
  contractId,
  network = "testnet",
  showExternalIcon = true,
}: {
  contractId: string;
  network?: "testnet" | "mainnet";
  showExternalIcon?: boolean;
}) {
  if (!contractId) return <span style={{ color: "var(--ink-faint)" }}>—</span>;
  const href = getContractExplorerUrl(network, contractId);
  const display = contractId.length > 20 ? `${contractId.slice(0, 10)}…${contractId.slice(-8)}` : contractId;
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      title={`View contract on Stellar Explorer: ${contractId}`}
      style={{
        color: "var(--link)",
        textDecoration: "none",
        fontFamily: "var(--font-mono)",
        letterSpacing: "var(--letter-mono)",
        fontSize: "var(--font-size-sm)",
        display: "inline-flex",
        alignItems: "center",
        gap: "4px",
        cursor: "pointer",
        transition: "color 0.12s ease",
      }}
    >
      {display}
      {showExternalIcon && (
        <svg
          width="11"
          height="11"
          viewBox="0 0 16 16"
          fill="none"
          style={{ opacity: 0.6, flexShrink: 0 }}
        >
          <path
            d="M6 3h7v7M13 3L6 10M11 9v4a1 1 0 01-1 1H4a1 1 0 01-1-1V7a1 1 0 011-1h4"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      )}
    </a>
  );
}

/**
 * Renders an IPFS CID as a clickable link to the gateway that served it.
 * Prefers the `gatewayUrl` the publish call actually returned (Pinata's
 * configured gateway, or the local Kubo daemon's), and falls back to a
 * public gateway so the link still works if one wasn't supplied.
 */
export function IpfsCidLink({
  cid,
  gatewayUrl,
  showExternalIcon = true,
  encrypted = false,
}: {
  cid: string;
  gatewayUrl?: string;
  showExternalIcon?: boolean;
  encrypted?: boolean;
}) {
  if (!cid) return <span style={{ color: "var(--ink-faint)" }}>—</span>;
  const href = gatewayUrl || `https://ipfs.io/ipfs/${cid}`;
  const display = cid.length > 20 ? `${cid.slice(0, 10)}…${cid.slice(-8)}` : cid;
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "6px" }}>
      <a
        href={href}
        target="_blank"
        rel="noopener noreferrer"
        title={`View on IPFS: ${cid}`}
        style={{
          color: "var(--link)",
          textDecoration: "none",
          fontFamily: "var(--font-mono)",
          letterSpacing: "var(--letter-mono)",
          fontSize: "var(--font-size-sm)",
          display: "inline-flex",
          alignItems: "center",
          gap: "4px",
          cursor: "pointer",
          transition: "color 0.12s ease",
        }}
      >
        {display}
        {showExternalIcon && (
          <svg
            width="11"
            height="11"
            viewBox="0 0 16 16"
            fill="none"
            style={{ opacity: 0.6, flexShrink: 0 }}
          >
            <path
              d="M6 3h7v7M13 3L6 10M11 9v4a1 1 0 01-1 1H4a1 1 0 01-1-1V7a1 1 0 011-1h4"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        )}
      </a>
      {encrypted && (
        <span
          title="Encrypted with age — only authorized auditors can decrypt"
          style={{
            fontSize: "10px",
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: "0.04em",
            padding: "1px 6px",
            borderRadius: "4px",
            background: "var(--success-100, #dcfce7)",
            color: "var(--success-700, #15803d)",
            border: "1px solid var(--success-200, #bbf7d0)",
            whiteSpace: "nowrap",
          }}
        >
          Encrypted
        </span>
      )}
    </span>
  );
}

// ─── Card ───────────────────────────────────────────────────────────────

export function Card({
  children,
  style,
  padded = true,
  compact = false,
}: {
  children: ReactNode;
  style?: CSSProperties;
  padded?: boolean;
  compact?: boolean;
}) {
  const className = compact ? "audit-card audit-card--padded-sm" : padded ? "audit-card" : "audit-card audit-card--flush";
  return (
    <div className={className} style={style}>
      {children}
    </div>
  );
}

export function CardHeader({
  title,
  subtitle,
  icon,
  actions,
  compact = false,
}: {
  title: ReactNode;
  subtitle?: string;
  icon?: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div className={compact ? "audit-card-header audit-card-header--compact" : "audit-card-header"}>
      {icon && <span style={{ fontSize: compact ? "var(--font-size-md)" : "var(--font-size-lg)", lineHeight: 1, marginTop: "1px" }}>{icon}</span>}
      <div className="audit-card-header__text">
        <div className={compact ? "audit-card-header__title audit-card-header__title--compact" : "audit-card-header__title"}>
          {title}
        </div>
        {subtitle && (
          <div className={compact ? "audit-card-header__subtitle audit-card-header__subtitle--compact" : "audit-card-header__subtitle"}>
            {subtitle}
          </div>
        )}
      </div>
      {actions && <div className="audit-card-header__actions">{actions}</div>}
    </div>
  );
}

// ─── Badge ──────────────────────────────────────────────────────────────

type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

export function Badge({
  children,
  tone = "neutral",
  dot = false,
}: {
  children: ReactNode;
  tone?: BadgeTone;
  dot?: boolean;
}) {
  return (
    <span className={`audit-badge audit-badge--${tone}`}>
      {dot && <span className="audit-badge__dot" />}
      {children}
    </span>
  );
}

// ─── Button ─────────────────────────────────────────────────────────────

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
type ButtonSize = "sm" | "md";

export function Button({
  children,
  variant = "secondary",
  size = "md",
  shortcut,
  loading = false,
  disabled = false,
  onClick,
  style,
  title,
}: {
  children: ReactNode;
  variant?: ButtonVariant;
  size?: ButtonSize;
  shortcut?: string;
  loading?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  style?: CSSProperties;
  title?: string;
}) {
  const isInactive = disabled || loading;
  const displayShortcut = shortcut ? formatAuditShortcut(shortcut) : undefined;
  return (
    <button
      title={title}
      onClick={onClick}
      disabled={isInactive}
      className={size === "sm" ? `audit-btn audit-btn--${variant} audit-btn--sm` : `audit-btn audit-btn--${variant}`}
      style={style}
    >
      {loading && <Spinner size={13} />}
      {children}
      {displayShortcut && <kbd className="kbd">{displayShortcut}</kbd>}
    </button>
  );
}

function formatAuditShortcut(shortcut: string): string {
  if (shortcut.startsWith("CmdOrCtrl+")) {
    const key = shortcut.slice("CmdOrCtrl+".length);
    return `${isMac ? "⌘" : "Ctrl+"}${formatShortcut(parseShortcut(key))}`;
  }
  return formatShortcut(parseShortcut(shortcut));
}

// ─── Spinner ────────────────────────────────────────────────────────────

export function Spinner({ size = 16 }: { size?: number }) {
  return (
    <span
      className="audit-spinner"
      style={{ width: size, height: size }}
    />
  );
}

// ─── Stat ───────────────────────────────────────────────────────────────

export function Stat({
  label,
  value,
  mono = false,
  compact = false,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
  compact?: boolean;
}) {
  return (
    <div className="audit-stat" style={{ gap: compact ? "1px" : "2px" }}>
      <span className="audit-stat__label">{label}</span>
      <span
        className={mono ? "audit-stat__value audit-stat__value--mono" : "audit-stat__value"}
        style={compact ? { fontSize: "var(--font-size-sm)" } : undefined}
      >
        {value}
      </span>
    </div>
  );
}

// ─── StatusCard ─────────────────────────────────────────────────────────

export function StatusCard({
  title,
  status,
  value,
  detail,
  action,
}: {
  title: ReactNode;
  status: "good" | "warning" | "danger" | "neutral";
  value: ReactNode;
  detail?: ReactNode;
  action?: ReactNode;
}) {
  const tone: BadgeTone = status === "good" ? "success" : status === "warning" ? "warning" : status === "danger" ? "danger" : "neutral";
  const dotColor = `var(--${tone === "neutral" ? "ink-faint" : tone + "-500"})`;
  return (
    <div className="audit-status-card">
      <div className="audit-status-card__head">
        <div className="audit-status-card__label">
          <span
            className="audit-status-card__dot"
            style={{
              background: dotColor,
              boxShadow: `0 0 0 2px color-mix(in oklch, ${dotColor} 25%, transparent)`,
            }}
          />
          {title}
        </div>
        {action}
      </div>
      <div className="audit-status-card__value">{value}</div>
      {detail && <div className="audit-status-card__detail">{detail}</div>}
    </div>
  );
}

// ─── ProgressBar ────────────────────────────────────────────────────────

export function ProgressBar({
  current,
  max,
  tone = "accent",
}: {
  current: number;
  max: number;
  tone?: "accent" | "success";
}) {
  const pct = max > 0 ? Math.min(100, (current / max) * 100) : 0;
  const active = pct > 0 && pct < 100;
  return (
    <div className="audit-progress">
      <div
        className={`audit-progress__fill audit-progress__fill--${tone}`}
        style={{
          transform: `scaleX(${Math.max(pct, 0.001) / 100})`,
          animation: active ? "audit-progress-pulse 2s ease-in-out infinite" : undefined,
        }}
      />
    </div>
  );
}

// ─── EmptyState ─────────────────────────────────────────────────────────

export function EmptyState({
  icon,
  title,
  body,
  action,
}: {
  icon?: ReactNode;
  title: string;
  body?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="audit-empty">
      {icon && <div style={{ fontSize: "2rem", opacity: 0.5, lineHeight: 1 }}>{icon}</div>}
      <div className="audit-empty__title">{title}</div>
      {body && <div className="audit-empty__body">{body}</div>}
      {action}
    </div>
  );
}

// ─── InlineEmpty ────────────────────────────────────────────────────────

export function InlineEmpty({
  icon,
  title,
  body,
}: {
  icon?: ReactNode;
  title: string;
  body?: ReactNode;
}) {
  return (
    <div className="audit-inline-empty">
      {icon && <div style={{ fontSize: "1.5rem", opacity: 0.5, lineHeight: 1, marginBottom: "var(--space-1)" }}>{icon}</div>}
      <div className="audit-inline-empty__title">{title}</div>
      {body && <div className="audit-inline-empty__body">{body}</div>}
    </div>
  );
}

// ─── KeyValue ───────────────────────────────────────────────────────────

export function KeyValue({
  label,
  value,
  mono = true,
}: {
  label: string;
  value: ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="audit-kv">
      <span className="audit-kv__label">{label}</span>
      <span className={mono ? "audit-kv__value audit-kv__value--mono" : "audit-kv__value"}>
        {value}
      </span>
    </div>
  );
}

// ─── Alert ──────────────────────────────────────────────────────────────
// Re-exported from the app-wide Alert component so every surface (audit
// and main app) shares one notification vocabulary: lucide icons, a tinted
// surface, and a tone accent stripe. See components/Alert.tsx.

export { Alert } from "./Alert";

// ─── Modal ───────────────────────────────────────────────────────────────
//
// Reuses the app-wide .modal / .modal-backdrop classes from styles.css,
// so audit modals share the same backdrop blur, scale-in animation, and
// z-index scale as the rest of the app.

export function Modal({
  open,
  onClose,
  title,
  subtitle,
  children,
  footer,
  maxWidth = 640,
  onSubmit,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  subtitle?: string;
  children: ReactNode;
  footer?: ReactNode;
  maxWidth?: number;
  onSubmit?: () => void;
}) {
  // Escape closes; Cmd/Ctrl+Enter submits when a primary action is wired.
  // Mirrors the keyboard vocabulary the rest of the app uses for modals.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (onSubmit && (e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        onSubmit();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose, onSubmit]);

  if (!open) return null;

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) { e.preventDefault(); onClose(); } }}>
      <div
        className="modal"
        style={{ width: `min(${maxWidth}px, 92vw)` }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal__header">
          <div className="modal__heading">
            <h2 className="modal__title">{title}</h2>
            {subtitle && <div className="modal__subtitle">{subtitle}</div>}
          </div>
          <button className="modal__close" onClick={onClose} aria-label="Close">
            <X />
          </button>
        </div>
        <div className="modal__body">{children}</div>
        {footer && <div className="modal__footer">{footer}</div>}
      </div>
    </div>
  );
}

// ─── LogsModal ───────────────────────────────────────────────────────────
//
// A purpose-built modal for viewing Docker / service logs.
// Parses each line for common log-level keywords and color-codes them,
// provides a search filter, and shows a live count of matching lines.

export function LogsModal({
  open,
  onClose,
  logs,
  loading = false,
  title = "Stack Logs",
}: {
  open: boolean;
  onClose: () => void;
  logs: string;
  loading?: boolean;
  title?: string;
}) {
  const [stats, setStats] = useState<LogViewerStats>({ total: 0, visible: 0, errors: 0, warnings: 0 });

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={title}
      subtitle={
        loading
          ? "Loading…"
          : `${stats.total} lines${stats.errors > 0 ? ` · ${stats.errors} errors` : ""}${stats.warnings > 0 ? ` · ${stats.warnings} warnings` : ""}`
      }
      maxWidth={780}
      footer={
        <>
          <div className="modal__footer-hint">
            {stats.visible !== stats.total
              ? `${stats.visible} of ${stats.total} lines match`
              : "Most recent 120 lines"}
          </div>
          <Button variant="secondary" shortcut="Escape" onClick={onClose}>Close</Button>
        </>
      }
    >
      <LogViewer
        lines={logs}
        loading={loading}
        loadingLabel="Fetching logs…"
        searchable
        copyable
        maxHeight="48vh"
        minHeight={200}
        onStats={setStats}
      />
    </Modal>
  );
}

/**
 * Keyframes are now defined in audit.css. This function is kept as a no-op
 * for backward compatibility with components that call it during init.
 */
export function injectAuditKeyframes() {
  // no-op: keyframes are in audit.css
}
