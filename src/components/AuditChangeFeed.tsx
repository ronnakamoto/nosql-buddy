import { useMemo, useState, useRef, useCallback } from "react";
import type { AuditEvent } from "../ipc/commands";
import { Badge, Button, EmptyState, Spinner } from "./AuditUi";
import { CircleDashed } from "lucide-react";

/**
 * AuditChangeFeed — real-time stream of audit events with filter chips + search.
 *
 * Features:
 *   - Filter chips: [All] [insert] [update] [delete]
 *   - Search: by database, collection, or leaf hash prefix
 *   - Click row → inline proof drawer (not modal)
 *   - Virtualized scrolling: renders only visible rows for smooth 1000+ event lists
 */

// ─── Constants ───────────────────────────────────────────────────────────────

const ROW_HEIGHT = 36;      // px — must match .audit-event-row height in CSS
const VISIBLE_ROWS = 10;    // max rows shown before virtualizing
const OVERSCAN = 3;         // extra rows rendered above/below viewport

// ─── Types ───────────────────────────────────────────────────────────────────

type OpFilter = "all" | "insert" | "update" | "delete";

// ─── Helpers ─────────────────────────────────────────────────────────────────

function opTone(op: string): "success" | "warning" | "danger" | "info" {
  const lower = op.toLowerCase();
  if (lower.includes("insert")) return "success";
  if (lower.includes("update")) return "warning";
  if (lower.includes("delete")) return "danger";
  return "info";
}

function relativeTime(ts: string): string {
  const d = new Date(ts);
  if (isNaN(d.getTime())) return "";
  const diff = Math.floor((Date.now() - d.getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  return d.toLocaleTimeString();
}

// ─── ProofDrawer ─────────────────────────────────────────────────────────────

interface ProofDrawerProps {
  event: AuditEvent;
  loading: boolean;
  result: string | null;
  onClose: () => void;
}

function ProofDrawer({ event, loading, result, onClose }: ProofDrawerProps) {
  return (
    <div className="audit-proof-drawer">
      <div className="audit-proof-drawer__header">
        <span className="audit-proof-drawer__title">
          Proof for event #{event.index}
        </span>
        <button className="audit-proof-drawer__close" onClick={onClose} aria-label="Close proof">
          ×
        </button>
      </div>
      <div className="audit-proof-drawer__body">
        <div className="audit-proof-drawer__meta">
          <span>
            <strong>{event.operation}</strong> · {event.database}.{event.collection}
          </span>
          <span className="audit-proof-drawer__leaf">leaf {event.leafHex.slice(0, 16)}…</span>
        </div>
        {loading ? (
          <div className="audit-proof-drawer__loading">
            <Spinner size={14} />
            <span>Generating proof…</span>
          </div>
        ) : result ? (
          <pre className="audit-proof-drawer__pre">{result}</pre>
        ) : null}
      </div>
    </div>
  );
}

// ─── EventRow ────────────────────────────────────────────────────────────────

interface EventRowProps {
  event: AuditEvent;
  selected: boolean;
  proofLoading: boolean;
  onSelect: () => void;
  onProof: () => void;
  style?: React.CSSProperties;
}

function EventRow({ event, selected, proofLoading, onSelect, onProof, style }: EventRowProps) {
  return (
    <div
      className={`audit-event-row ${selected ? "audit-event-row--selected" : ""}`}
      style={style}
      onClick={onSelect}
    >
      <Badge tone={opTone(event.operation)}>{event.operation}</Badge>
      <span className="audit-event-row__ns">
        {event.database}.{event.collection}
      </span>
      <span className="audit-event-row__time">{relativeTime(event.timestamp)}</span>
      <span className="audit-event-row__leaf">leaf {event.leafHex.slice(0, 10)}…</span>
      <Button
        variant="ghost"
        loading={proofLoading}
        onClick={onProof}
        style={{ padding: "2px 8px", fontSize: "var(--font-size-xs)", marginLeft: "auto" }}
      >
        Proof
      </Button>
    </div>
  );
}

// ─── VirtualList ─────────────────────────────────────────────────────────────
// Simple fixed-height virtualizer — no dependencies.

interface VirtualListProps {
  items: AuditEvent[];
  selectedIndex: number | null;
  proofIndex: number | null;
  proofLoading: boolean;
  onSelect: (index: number) => void;
  onProof: (index: number) => void;
}

function VirtualList({
  items,
  selectedIndex,
  proofIndex,
  proofLoading,
  onSelect,
  onProof,
}: VirtualListProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);

  const handleScroll = useCallback(() => {
    setScrollTop(containerRef.current?.scrollTop ?? 0);
  }, []);

  const containerHeight = ROW_HEIGHT * VISIBLE_ROWS;
  const totalHeight = items.length * ROW_HEIGHT;

  // Window: which rows are visible + overscan
  const firstVisible = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const lastVisible = Math.min(
    items.length - 1,
    Math.ceil((scrollTop + containerHeight) / ROW_HEIGHT) + OVERSCAN,
  );

  const visibleItems = items.slice(firstVisible, lastVisible + 1);

  return (
    <div
      ref={containerRef}
      className="audit-event-list audit-event-list--virtual"
      style={{ height: containerHeight, overflowY: "auto", position: "relative" }}
      onScroll={handleScroll}
    >
      {/* Spacer that gives the scrollbar the correct total height */}
      <div style={{ height: totalHeight, pointerEvents: "none" }} />

      {/* Absolutely-positioned rendered rows */}
      {visibleItems.map((ev, i) => (
        <EventRow
          key={ev.index}
          event={ev}
          selected={selectedIndex === ev.index}
          proofLoading={proofLoading && proofIndex === ev.index}
          onSelect={() => onSelect(ev.index)}
          onProof={() => onProof(ev.index)}
          style={{
            position: "absolute",
            top: (firstVisible + i) * ROW_HEIGHT,
            left: 0,
            right: 0,
            height: ROW_HEIGHT,
          }}
        />
      ))}
    </div>
  );
}

// ─── AuditChangeFeed ─────────────────────────────────────────────────────────

export interface AuditChangeFeedProps {
  events: AuditEvent[];
  collapsed: boolean;
  onToggle: () => void;
  proofIndex: number | null;
  proofLoading: boolean;
  proofResult: string | null;
  onProof: (index: number) => void;
}

export function AuditChangeFeed({
  events,
  collapsed,
  onToggle,
  proofIndex,
  proofLoading,
  proofResult,
  onProof,
}: AuditChangeFeedProps) {
  const [opFilter, setOpFilter] = useState<OpFilter>("all");
  const [search, setSearch] = useState("");
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);

  const filtered = useMemo(() => {
    let list = events.slice().reverse(); // newest first
    if (opFilter !== "all") {
      list = list.filter((e) => e.operation.toLowerCase().includes(opFilter));
    }
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      list = list.filter(
        (e) =>
          e.database.toLowerCase().includes(q) ||
          e.collection.toLowerCase().includes(q) ||
          e.leafHex.toLowerCase().startsWith(q),
      );
    }
    return list;
  }, [events, opFilter, search]);

  const selectedEvent = selectedIndex !== null
    ? events.find((e) => e.index === selectedIndex) ?? null
    : null;

  const handleSelect = useCallback((index: number) => {
    setSelectedIndex((prev) => (prev === index ? null : index));
  }, []);

  const handleProof = useCallback((index: number) => {
    setSelectedIndex(index);
    onProof(index);
  }, [onProof]);

  // Use virtualization only when there are many rows
  const useVirtual = filtered.length > VISIBLE_ROWS;

  return (
    <div className="audit-section audit-section--feed">
      {/* Section header */}
      <div className="audit-section-header" onClick={onToggle} style={{ cursor: "pointer" }}>
        <span className="audit-section-header__title">
          Change Feed
          <span className="audit-section-header__count">
            · {events.length} event{events.length === 1 ? "" : "s"}
          </span>
        </span>
        <span className={`audit-section-header__chevron ${collapsed ? "" : "audit-section-header__chevron--open"}`}>
          ▶
        </span>
      </div>

      <div className={`audit-section-body ${collapsed ? "" : "audit-section-body--open"}`}>
        <div className="audit-section-body__inner">
        <div className="audit-feed-body">
          {/* Filter chips + search */}
          <div className="audit-feed-controls">
            <div className="audit-filter-chips">
              {(["all", "insert", "update", "delete"] as OpFilter[]).map((op) => (
                <button
                  key={op}
                  className={`audit-filter-chip ${opFilter === op ? "audit-filter-chip--active" : ""}`}
                  onClick={() => setOpFilter(op)}
                >
                  {op}
                </button>
              ))}
            </div>
            <input
              className="audit-feed-search"
              placeholder="Search db, collection, or leaf…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>

          {/* Event list */}
          {events.length === 0 ? (
            <EmptyState
              icon={<CircleDashed size={26} />}
              title="No events yet"
              body="Write data to MongoDB (insert, update, delete) to populate the audit log."
            />
          ) : filtered.length === 0 ? (
            <div className="audit-feed-empty-filter">No events match the filter</div>
          ) : useVirtual ? (
            <VirtualList
              items={filtered}
              selectedIndex={selectedIndex}
              proofIndex={proofIndex}
              proofLoading={proofLoading}
              onSelect={handleSelect}
              onProof={handleProof}
            />
          ) : (
            <div className="audit-event-list">
              {filtered.map((ev) => (
                <EventRow
                  key={ev.index}
                  event={ev}
                  selected={selectedIndex === ev.index}
                  proofLoading={proofLoading && proofIndex === ev.index}
                  onSelect={() => handleSelect(ev.index)}
                  onProof={() => handleProof(ev.index)}
                />
              ))}
            </div>
          )}

          {/* Inline proof drawer — shown below the list when a proof is requested */}
          {selectedEvent && (proofLoading || proofResult) && (
            <ProofDrawer
              event={selectedEvent}
              loading={proofLoading && proofIndex === selectedEvent.index}
              result={proofIndex === selectedEvent.index ? proofResult : null}
              onClose={() => setSelectedIndex(null)}
            />
          )}
        </div>
        </div>
      </div>
    </div>
  );
}
