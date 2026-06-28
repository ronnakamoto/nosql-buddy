import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Clock,
  Search,
  Trash2,
  StickyNote,
  ChevronDown,
  ChevronRight,
  Filter,
  History,
  AlertTriangle,
  CheckCircle,
  XCircle,
  FileText,
  Database,
  Table,
} from "lucide-react";
import type { TimelineEntry, OperationKind } from "../../ipc/timeline";
import {
  listTimeline,
  deleteTimelineEntry,
  addTimelineNote,
  operationKindLabel,
} from "../../ipc/timeline";
import { Alert } from "../../components/Alert";
import { ConfirmDialog } from "../../components/ConfirmDialog";

interface TimelinePanelProps {
  profileId?: string | null;
  database?: string | null;
  collection?: string | null;
}

const KIND_OPTIONS: { value: OperationKind; label: string }[] = [
  { value: "find", label: "Find" },
  { value: "aggregate", label: "Aggregate" },
  { value: "insertOne", label: "Insert One" },
  { value: "insertMany", label: "Insert Many" },
  { value: "updateOne", label: "Update One" },
  { value: "updateMany", label: "Update Many" },
  { value: "deleteOne", label: "Delete One" },
  { value: "deleteMany", label: "Delete Many" },
  { value: "indexCreate", label: "Create Index" },
  { value: "indexDrop", label: "Drop Index" },
  { value: "import", label: "Import" },
  { value: "export", label: "Export" },
  { value: "dump", label: "Dump" },
  { value: "restore", label: "Restore" },
];

function kindBadgeClass(kind: OperationKind): string {
  switch (kind) {
    case "find":
    case "aggregate":
    case "sql":
    case "explain":
      return "timeline-badge--read";
    case "insertOne":
    case "insertMany":
      return "timeline-badge--insert";
    case "updateOne":
    case "updateMany":
    case "replaceOne":
      return "timeline-badge--update";
    case "deleteOne":
    case "deleteMany":
      return "timeline-badge--delete";
    case "aggregationWrite":
      return "timeline-badge--danger";
    case "indexCreate":
    case "indexDrop":
    case "collectionCreate":
    case "collectionDrop":
    case "collectionRename":
      return "timeline-badge--schema";
    default:
      return "timeline-badge--default";
  }
}

function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString();
}

function formatDuration(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export function TimelinePanel({ profileId, database, collection }: TimelinePanelProps) {
  const [entries, setEntries] = useState<TimelineEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [filterKind, setFilterKind] = useState<OperationKind | "">("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [noteDraft, setNoteDraft] = useState<Record<string, string>>({});
  const [savingNote, setSavingNote] = useState<string | null>(null);
  const [pendingDeleteId, setPendingDeleteId] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!profileId) {
      setEntries([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const results = await listTimeline({
        profileId,
        database: database ?? undefined,
        collection: collection ?? undefined,
        kind: filterKind || undefined,
        limit: 200,
      });
      setEntries(results);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [profileId, database, collection, filterKind]);

  useEffect(() => {
    load();
  }, [load]);

  const filtered = useMemo(() => {
    if (!search) return entries;
    const q = search.toLowerCase();
    return entries.filter(
      (e) =>
        e.database.toLowerCase().includes(q) ||
        e.collection.toLowerCase().includes(q) ||
        operationKindLabel(e.kind).toLowerCase().includes(q) ||
        (e.queryJson && e.queryJson.toLowerCase().includes(q)),
    );
  }, [entries, search]);

  const handleDelete = (id: string) => {
    setPendingDeleteId(id);
  };

  const confirmDelete = async () => {
    if (!pendingDeleteId) return;
    const id = pendingDeleteId;
    setPendingDeleteId(null);
    try {
      const ok = await deleteTimelineEntry(id);
      if (ok) {
        setEntries((prev) => prev.filter((e) => e.id !== id));
        if (expandedId === id) setExpandedId(null);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const handleSaveNote = async (id: string) => {
    const text = noteDraft[id]?.trim();
    if (!text) return;
    setSavingNote(id);
    try {
      const ok = await addTimelineNote(id, text);
      if (ok) {
        setEntries((prev) =>
          prev.map((e) => (e.id === id ? { ...e, notes: text } : e)),
        );
        setNoteDraft((prev) => {
          const next = { ...prev };
          delete next[id];
          return next;
        });
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSavingNote(null);
    }
  };

  if (!profileId) {
    return (
      <div className="timeline-panel timeline-panel--empty">
        <History size={48} className="timeline-panel__icon" />
        <p className="timeline-panel__hint">Connect to a database to view the timeline.</p>
      </div>
    );
  }

  return (
    <div className="timeline-panel">
      <div className="timeline-panel__header">
        <div className="timeline-panel__title-row">
          <History size={18} />
          <h2 className="timeline-panel__title">Data Timeline</h2>
          <span className="timeline-panel__count">{filtered.length} entries</span>
        </div>

        <div className="timeline-panel__toolbar">
          <div className="timeline-panel__search">
            <Search size={14} className="timeline-panel__search-icon" />
            <input
              type="text"
              placeholder="Search operations..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="timeline-panel__search-input"
            />
          </div>

          <div className="timeline-panel__filter">
            <Filter size={14} />
            <select
              value={filterKind}
              onChange={(e) => setFilterKind(e.target.value as OperationKind | "")}
              className="timeline-panel__filter-select"
            >
              <option value="">All kinds</option>
              {KIND_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </div>

          <button
            className="timeline-panel__refresh"
            onClick={load}
            disabled={loading}
            title="Refresh"
          >
            <Clock size={14} />
          </button>
        </div>
      </div>

      {error && (
        <Alert tone="danger" onDismiss={() => setError(null)} compact>
          {error}
        </Alert>
      )}

      {loading && entries.length === 0 && (
        <div className="timeline-panel__loading">Loading timeline...</div>
      )}

      {!loading && filtered.length === 0 && (
        <div className="timeline-panel__empty">
          <FileText size={32} />
          <p>No timeline entries yet.</p>
          <p className="timeline-panel__empty-hint">
            Run a query or perform a write operation to see it here.
          </p>
        </div>
      )}

      <div className="timeline-panel__list">
        {filtered.map((entry) => {
          const isExpanded = expandedId === entry.id;
          const hasNote = !!entry.notes;

          return (
            <div
              key={entry.id}
              className={[
                "timeline-entry",
                isExpanded ? "timeline-entry--expanded" : "",
                entry.errored ? "timeline-entry--errored" : "",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              <button
                className="timeline-entry__summary"
                onClick={() => setExpandedId(isExpanded ? null : entry.id)}
              >
                <span className="timeline-entry__chevron">
                  {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                </span>

                <span className={["timeline-badge", kindBadgeClass(entry.kind)].join(" ")}>
                  {operationKindLabel(entry.kind)}
                </span>

                <span className="timeline-entry__target">
                  <Database size={12} />
                  {entry.database}
                  {entry.collection && (
                    <>
                      <span className="timeline-entry__dot">.</span>
                      <Table size={12} />
                      {entry.collection}
                    </>
                  )}
                </span>

                {entry.matchedCount != null && (
                  <span className="timeline-entry__count" title="Matched">
                    {entry.matchedCount.toLocaleString()} matched
                  </span>
                )}
                {entry.modifiedCount != null && (
                  <span className="timeline-entry__count" title="Modified">
                    {entry.modifiedCount.toLocaleString()} modified
                  </span>
                )}
                {entry.deletedCount != null && (
                  <span className="timeline-entry__count timeline-entry__count--danger" title="Deleted">
                    {entry.deletedCount.toLocaleString()} deleted
                  </span>
                )}
                {entry.insertedCount != null && (
                  <span className="timeline-entry__count timeline-entry__count--insert" title="Inserted">
                    {entry.insertedCount.toLocaleString()} inserted
                  </span>
                )}
                {entry.returnedCount != null && (
                  <span className="timeline-entry__count" title="Returned">
                    {entry.returnedCount.toLocaleString()} returned
                  </span>
                )}

                <span className="timeline-entry__time">
                  {formatTimestamp(entry.createdAt)}
                </span>

                {entry.errored && <XCircle size={14} className="timeline-entry__error-icon" />}
                {hasNote && <StickyNote size={14} className="timeline-entry__note-icon" />}
              </button>

              {isExpanded && (
                <div className="timeline-entry__detail">
                  <div className="timeline-entry__detail-grid">
                    <div className="timeline-entry__detail-item">
                      <span className="timeline-entry__detail-label">ID</span>
                      <code className="timeline-entry__detail-value timeline-entry__detail-value--code">
                        {entry.id}
                      </code>
                    </div>
                    <div className="timeline-entry__detail-item">
                      <span className="timeline-entry__detail-label">Actor</span>
                      <span className="timeline-entry__detail-value">{entry.actor}</span>
                    </div>
                    {entry.executionMs != null && (
                      <div className="timeline-entry__detail-item">
                        <span className="timeline-entry__detail-label">Duration</span>
                        <span className="timeline-entry__detail-value">
                          {formatDuration(entry.executionMs)}
                        </span>
                      </div>
                    )}
                    {entry.environmentTag && (
                      <div className="timeline-entry__detail-item">
                        <span className="timeline-entry__detail-label">Environment</span>
                        <span className="timeline-entry__detail-value">{entry.environmentTag}</span>
                      </div>
                    )}
                  </div>

                  {entry.queryJson && (
                    <div className="timeline-entry__code-block">
                      <span className="timeline-entry__code-label">Filter / Query</span>
                      <pre className="timeline-entry__code">
                        {entry.queryJson}
                      </pre>
                    </div>
                  )}

                  {entry.updateJson && (
                    <div className="timeline-entry__code-block">
                      <span className="timeline-entry__code-label">Update</span>
                      <pre className="timeline-entry__code">
                        {entry.updateJson}
                      </pre>
                    </div>
                  )}

                  {entry.errorMessage && (
                    <Alert tone="danger" compact>
                      {entry.errorMessage}
                    </Alert>
                  )}

                  {entry.riskScore != null && (
                    <div className="timeline-entry__risk">
                      <AlertTriangle size={14} />
                      <span>Risk score: {entry.riskScore} / 100</span>
                      {entry.riskReasons && entry.riskReasons.length > 0 && (
                        <ul className="timeline-entry__risk-reasons">
                          {entry.riskReasons.map((r, i) => (
                            <li key={i}>{r}</li>
                          ))}
                        </ul>
                      )}
                    </div>
                  )}

                  {entry.rollbackLevel !== "none" && (
                    <div className="timeline-entry__rollback">
                      <CheckCircle size={14} />
                      <span>Rollback: {entry.rollbackLevel}</span>
                    </div>
                  )}

                  <div className="timeline-entry__notes">
                    {entry.notes ? (
                      <div className="timeline-entry__note">
                        <StickyNote size={14} />
                        <span>{entry.notes}</span>
                      </div>
                    ) : null}
                    <div className="timeline-entry__note-input-row">
                      <input
                        type="text"
                        placeholder="Add a note..."
                        value={noteDraft[entry.id] ?? ""}
                        onChange={(e) =>
                          setNoteDraft((prev) => ({ ...prev, [entry.id]: e.target.value }))
                        }
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSaveNote(entry.id);
                        }}
                        className="timeline-entry__note-input"
                      />
                      <button
                        className="timeline-entry__note-save"
                        onClick={() => handleSaveNote(entry.id)}
                        disabled={savingNote === entry.id || !noteDraft[entry.id]?.trim()}
                      >
                        Save
                      </button>
                    </div>
                  </div>

                  <div className="timeline-entry__actions">
                    <button
                      className="timeline-entry__action timeline-entry__action--danger"
                      onClick={() => handleDelete(entry.id)}
                      title="Delete entry"
                    >
                      <Trash2 size={14} />
                      Delete
                    </button>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>

      <ConfirmDialog
        open={pendingDeleteId !== null}
        title="Delete timeline entry?"
        description="This entry will be permanently removed from the operation timeline. This cannot be undone."
        confirmLabel="Delete entry"
        onConfirm={() => void confirmDelete()}
        onCancel={() => setPendingDeleteId(null)}
      />
    </div>
  );
}
