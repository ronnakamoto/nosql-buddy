import { useState } from "react";
import type { Epoch } from "../ipc/commands";
import { Badge, Button, KeyValue, TxHashLink } from "./AuditUi";

/**
 * AuditBatchHistory — compact epoch table with expandable rows and inline actions.
 *
 * | # | Status   | Events  | On-chain   | Verified     |
 * |---|----------|---------|------------|--------------|
 * | 12| filling  | 73/100  | —          | —            |
 * | 11| committed| 100/100 | tx 0xa3…   | ✓ 2 min ago  |
 *
 * Click a row → expands to show: root hash, IPFS CID, tx hash link, details.
 * Sealed rows show a "Commit" button. Committed rows show "Verify". Filling rows show "Seal Now".
 */

const EPOCH_THRESHOLD = 100;

function epochStatus(epoch: Epoch): "filling" | "sealed" | "committed" {
  if (epoch.committed) return "committed";
  if (epoch.endIndex !== null && epoch.endIndex !== undefined) return "sealed";
  return "filling";
}

function statusBadgeTone(status: "filling" | "sealed" | "committed") {
  if (status === "committed") return "success" as const;
  if (status === "sealed") return "warning" as const;
  return "neutral" as const;
}

function formatTs(ts: string | null): string {
  if (!ts) return "";
  const d = new Date(ts);
  if (isNaN(d.getTime())) return "";
  const diff = Math.floor((Date.now() - d.getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  return d.toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
}

function shortHash(h: string | null): string {
  if (!h) return "—";
  return h.length > 16 ? `${h.slice(0, 8)}…${h.slice(-6)}` : h;
}

interface EpochRowProps {
  epoch: Epoch;
  network: "testnet" | "mainnet";
  expanded: boolean;
  onToggle: () => void;
  onSeal?: () => void;
  onCommit?: () => void;
  onVerify?: () => void;
  sealLoading?: boolean;
  commitLoading?: boolean;
  verifyLoading?: boolean;
}

function EpochRow({
  epoch,
  network,
  expanded,
  onToggle,
  onSeal,
  onCommit,
  onVerify,
  sealLoading,
  commitLoading,
  verifyLoading,
}: EpochRowProps) {
  const status = epochStatus(epoch);

  return (
    <div className={`audit-epoch-row ${expanded ? "audit-epoch-row--expanded" : ""}`}>
      {/* Summary row */}
      <div className="audit-epoch-summary" onClick={onToggle}>
        <span className="audit-epoch-summary__num">#{epoch.epochNumber}</span>
        <Badge tone={statusBadgeTone(status)}>{status}</Badge>
        <span className="audit-epoch-summary__events">
          {epoch.eventCount}{status !== "filling" ? `/${EPOCH_THRESHOLD}` : ""}
        </span>
        <span className="audit-epoch-summary__onchain">
          {epoch.txHash ? (
            <TxHashLink txHash={epoch.txHash} network={network} />
          ) : (
            <span style={{ color: "var(--ink-faint)" }}>—</span>
          )}
        </span>
        <span className="audit-epoch-summary__verified">
          {epoch.committed && epoch.committedAt ? (
            <span style={{ color: "var(--success-500)" }}>✓ {formatTs(epoch.committedAt)}</span>
          ) : (
            <span style={{ color: "var(--ink-faint)" }}>—</span>
          )}
        </span>

        {/* Inline action */}
        <span className="audit-epoch-summary__action" onClick={(e) => e.stopPropagation()}>
          {status === "filling" && onSeal && (
            <Button
              variant="ghost"
              loading={sealLoading}
              onClick={onSeal}
              style={{ padding: "2px 8px", fontSize: "var(--font-size-xs)" }}
            >
              Seal Now
            </Button>
          )}
          {status === "sealed" && onCommit && (
            <Button
              variant="secondary"
              loading={commitLoading}
              onClick={onCommit}
              style={{ padding: "2px 8px", fontSize: "var(--font-size-xs)" }}
            >
              Commit
            </Button>
          )}
          {status === "committed" && onVerify && (
            <Button
              variant="ghost"
              loading={verifyLoading}
              onClick={onVerify}
              style={{ padding: "2px 8px", fontSize: "var(--font-size-xs)" }}
            >
              Verify
            </Button>
          )}
        </span>

        <span className={`audit-epoch-summary__chevron ${expanded ? "audit-epoch-summary__chevron--open" : ""}`}>▶</span>
      </div>

      {/* Expanded detail */}
      {expanded && (
        <div className="audit-epoch-detail">
          <KeyValue label="Root hash" value={shortHash(epoch.rootHex)} mono />
          {epoch.txHash && (
            <KeyValue
              label="Tx hash"
              value={<TxHashLink txHash={epoch.txHash} network={network} />}
              mono={false}
            />
          )}
          <KeyValue label="Start index" value={epoch.startIndex} mono />
          {epoch.endIndex !== null && epoch.endIndex !== undefined && (
            <KeyValue label="End index" value={epoch.endIndex} mono />
          )}
          {epoch.committedAt && (
            <KeyValue label="Committed" value={formatTs(epoch.committedAt)} mono={false} />
          )}
        </div>
      )}
    </div>
  );
}

export interface AuditBatchHistoryProps {
  epochs: Epoch[];
  network: "testnet" | "mainnet";
  collapsed: boolean;
  onToggle: () => void;

  // Actions (forwarded from parent which owns the loading/IPC state)
  onSealEpoch?: (epochNumber: number) => void;
  onCommitEpoch?: (epochNumber: number) => void;
  onVerifyEpoch?: (epochNumber: number) => void;
  sealLoadingEpoch?: number | null;
  commitLoadingEpoch?: number | null;
  verifyLoadingEpoch?: number | null;
}

export function AuditBatchHistory({
  epochs,
  network,
  collapsed,
  onToggle,
  onSealEpoch,
  onCommitEpoch,
  onVerifyEpoch,
  sealLoadingEpoch,
  commitLoadingEpoch,
  verifyLoadingEpoch,
}: AuditBatchHistoryProps) {
  const [expandedEpoch, setExpandedEpoch] = useState<number | null>(null);

  const sorted = [...epochs].sort((a, b) => b.epochNumber - a.epochNumber);

  return (
    <div className="audit-section">
      <div className="audit-section-header" onClick={onToggle} style={{ cursor: "pointer" }}>
        <span className="audit-section-header__title">
          Batch History
          <span className="audit-section-header__count">
            · {epochs.length} batch{epochs.length === 1 ? "" : "es"}
          </span>
        </span>
        <span className={`audit-section-header__chevron ${collapsed ? "" : "audit-section-header__chevron--open"}`}>
          ▶
        </span>
      </div>

      <div className={`audit-section-body ${collapsed ? "" : "audit-section-body--open"}`}>
        <div className="audit-section-body__inner">
        <div className="audit-batch-table">
          {/* Column headers */}
          <div className="audit-epoch-header">
            <span className="audit-epoch-header__num">#</span>
            <span className="audit-epoch-header__status">Status</span>
            <span className="audit-epoch-header__events">Events</span>
            <span className="audit-epoch-header__onchain">On-chain</span>
            <span className="audit-epoch-header__verified">Verified</span>
            <span className="audit-epoch-header__action" />
            <span className="audit-epoch-header__chevron" />
          </div>

          {sorted.length === 0 ? (
            <div className="audit-batch-empty">No batches yet — seal and commit your first batch to see history here.</div>
          ) : (
            sorted.map((epoch) => (
              <EpochRow
                key={epoch.epochNumber}
                epoch={epoch}
                network={network}
                expanded={expandedEpoch === epoch.epochNumber}
                onToggle={() =>
                  setExpandedEpoch((prev) => (prev === epoch.epochNumber ? null : epoch.epochNumber))
                }
                onSeal={onSealEpoch ? () => onSealEpoch(epoch.epochNumber) : undefined}
                onCommit={onCommitEpoch ? () => onCommitEpoch(epoch.epochNumber) : undefined}
                onVerify={onVerifyEpoch ? () => onVerifyEpoch(epoch.epochNumber) : undefined}
                sealLoading={sealLoadingEpoch === epoch.epochNumber}
                commitLoading={commitLoadingEpoch === epoch.epochNumber}
                verifyLoading={verifyLoadingEpoch === epoch.epochNumber}
              />
            ))
          )}
        </div>
        </div>
      </div>
    </div>
  );
}
