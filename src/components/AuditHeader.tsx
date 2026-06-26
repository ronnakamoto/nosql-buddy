import type { ReactNode } from "react";
import type { AuditStatus, AuditModeConfig, Epoch } from "../ipc/commands";
import { Badge } from "./AuditUi";

/**
 * AuditHeader — the always-visible compact status bar at the top of the audit surface.
 *
 * Shows: health dot · mode/network badge · event count · batch status · settings gear
 * A DBA can read health at a glance without scrolling.
 */

export type HealthState = "healthy" | "unverified" | "tamper" | "idle";

function healthDot(state: HealthState) {
  const colors: Record<HealthState, string> = {
    healthy: "var(--success-500)",
    unverified: "var(--warning-500)",
    tamper: "var(--danger-500)",
    idle: "var(--ink-faint)",
  };
  const labels: Record<HealthState, string> = {
    healthy: "Healthy",
    unverified: "Unverified",
    tamper: "Tamper detected",
    idle: "Not started",
  };
  const color = colors[state];
  return (
    <span
      className="audit-header__dot"
      style={{
        background: color,
        boxShadow: state !== "idle" ? `0 0 0 2px color-mix(in oklch, ${color} 25%, transparent)` : undefined,
      }}
      title={labels[state]}
    />
  );
}

function modeBadgeLabel(config: AuditModeConfig | null): string {
  if (!config) return "Not configured";
  const mode = config.mode === "dev" ? "Dev" : "Production";
  const net = config.network === "mainnet" ? "Mainnet" : "Testnet";
  return `${mode} · ${net}`;
}

function modeBadgeTone(config: AuditModeConfig | null): "neutral" | "accent" | "success" {
  if (!config) return "neutral";
  if (config.mode === "dev") return "accent";
  return config.network === "mainnet" ? "success" : "accent";
}

function batchLabel(currentEpoch: Epoch | null): string {
  if (!currentEpoch) return "";
  const n = currentEpoch.epochNumber;
  if (currentEpoch.committed) return `Batch #${n} committed`;
  if (currentEpoch.endIndex !== null && currentEpoch.endIndex !== undefined) return `Batch #${n} sealed`;
  const pct = Math.round(Math.min(100, (currentEpoch.eventCount / 100) * 100));
  return `Batch #${n} ${pct}%`;
}

export interface AuditHeaderProps {
  health: HealthState;
  config: AuditModeConfig | null;
  status: AuditStatus | null;
  currentEpoch: Epoch | null;
  onSettings: () => void;
  /** optional slot for extra right-side controls */
  extra?: ReactNode;
}

export function AuditHeader({
  health,
  config,
  status,
  currentEpoch,
  onSettings,
  extra,
}: AuditHeaderProps) {
  const eventCount = status?.eventCount ?? 0;
  const batch = batchLabel(currentEpoch);

  return (
    <div className="audit-header">
      {/* Health dot + state label */}
      <div className="audit-header__status">
        {healthDot(health)}
        <span className="audit-header__health-label">
          {health === "healthy" ? "Healthy" : health === "tamper" ? "Tamper detected" : health === "unverified" ? "Unverified" : "Not started"}
        </span>
      </div>

      <span className="audit-header__sep" />

      {/* Mode/network badge */}
      <Badge tone={modeBadgeTone(config)}>
        {modeBadgeLabel(config)}
      </Badge>

      {eventCount > 0 && (
        <>
          <span className="audit-header__sep" />
          <span className="audit-header__meta">{eventCount.toLocaleString()} events</span>
        </>
      )}

      {batch && (
        <>
          <span className="audit-header__sep" />
          <span className="audit-header__meta">{batch}</span>
        </>
      )}

      {/* Push remaining items to the right */}
      <div style={{ flex: 1 }} />

      {extra}

      {/* Settings gear */}
      <button
        className="audit-header__gear"
        onClick={onSettings}
        title="Audit settings"
        aria-label="Open audit settings"
      >
        <svg width="15" height="15" viewBox="0 0 16 16" fill="none" aria-hidden>
          <path
            d="M6.5 1.5h3l.5 1.5 1.5.9 1.5-.5 2.1 2.1-.5 1.5.9 1.5 1.5.5v3l-1.5.5-.9 1.5.5 1.5-2.1 2.1-1.5-.5-1.5.9-.5 1.5h-3l-.5-1.5-1.5-.9-1.5.5L1 11.6l.5-1.5L.6 8.6 -1 8.1v-3l1.5-.5.9-1.5L1 1.6 3.1-.5l1.5.5 1.5-.9.4-1.1z"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <circle cx="8" cy="8" r="2.5" stroke="currentColor" strokeWidth="1.4" />
        </svg>
      </button>
    </div>
  );
}
