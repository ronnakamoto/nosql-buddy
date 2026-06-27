import { useCallback, useEffect, useMemo, useState } from "react";
import commands, {
  type CollationDto,
  type CreateIndexRequest,
  type IndexInfo,
  type IndexStats,
} from "../ipc/commands";
import { Modal } from "../components/Modal";
import { Alert } from "../components/Alert";

export interface IndexTabProps {
  connectionId: string;
  database: string;
  collection: string;
}

/** A single field in an index key spec. `type` is the per-field value:
 *  numeric directions are stored as their integer string ("1"/"-1") and
 *  special types ("text", "2dsphere", "2d", "hashed", "geoHaystack") are
 *  stored as their string name. The builder converts to the final
 *  `{ field: <int|str> }` document on submit. */
interface KeyField {
  key: string;
  field: string;
  type: string;
}

const KEY_TYPES = ["1", "-1", "text", "2dsphere", "2d", "hashed", "geoHaystack"];
const KEY_TYPE_LABELS: Record<string, string> = {
  "1": "asc",
  "-1": "desc",
  text: "text",
  "2dsphere": "2dsphere",
  "2d": "2d",
  hashed: "hashed",
  geoHaystack: "geoHaystack",
};

const COLLATION_STRENGTHS = [
  { value: "", label: "(default)" },
  { value: "1", label: "1 — Primary" },
  { value: "2", label: "2 — Secondary" },
  { value: "3", label: "3 — Tertiary" },
  { value: "4", label: "4 — Quaternary" },
  { value: "5", label: "5 — Identical" },
];

const COLLATION_ALTERNATES = [
  { value: "", label: "(default)" },
  { value: "non-ignorable", label: "non-ignorable" },
  { value: "shifted", label: "shifted" },
];

const COLLATION_MAX_VARIABLE = [
  { value: "", label: "(default)" },
  { value: "punct", label: "punct" },
  { value: "space", label: "space" },
];

const COLLATION_CASE_FIRST = [
  { value: "", label: "(default)" },
  { value: "upper", label: "upper" },
  { value: "lower", label: "lower" },
  { value: "off", label: "off" },
];

let keyFieldCounter = 0;
function newKeyField(field = "", type = "1"): KeyField {
  keyFieldCounter += 1;
  return { key: `kf-${keyFieldCounter}`, field, type };
}

/** Convert a list of key fields into the MongoDB key document JSON. */
function keyFieldsToJson(fields: KeyField[]): string {
  const obj: Record<string, unknown> = {};
  for (const f of fields) {
    const name = f.field.trim();
    if (!name) continue;
    if (f.type === "1" || f.type === "-1") {
      obj[name] = Number.parseInt(f.type, 10);
    } else {
      obj[name] = f.type;
    }
  }
  return JSON.stringify(obj);
}

/** Parse a key document (from an existing index) back into editable fields. */
function keyFieldsFromJson(key: Record<string, unknown>): KeyField[] {
  const out: KeyField[] = [];
  for (const [field, value] of Object.entries(key)) {
    let type = "1";
    if (typeof value === "number") {
      type = value >= 0 ? "1" : "-1";
    } else if (typeof value === "string") {
      type = KEY_TYPES.includes(value) ? value : "1";
    }
    out.push(newKeyField(field, type));
  }
  return out;
}

interface DraftState {
  name: string;
  fields: KeyField[];
  unique: boolean;
  sparse: boolean;
  hidden: boolean;
  ttlSeconds: string;
  partialFilterExpression: string;
  collationLocale: string;
  collationStrength: string;
  collationCaseLevel: boolean;
  collationCaseFirst: string;
  collationNumericOrdering: boolean;
  collationAlternate: string;
  collationMaxVariable: string;
  collationNormalization: boolean;
  collationBackwards: boolean;
  wildcardProjection: string;
}

function emptyDraft(): DraftState {
  return {
    name: "",
    fields: [newKeyField()],
    unique: false,
    sparse: false,
    hidden: false,
    ttlSeconds: "",
    partialFilterExpression: "",
    collationLocale: "",
    collationStrength: "",
    collationCaseLevel: false,
    collationCaseFirst: "",
    collationNumericOrdering: false,
    collationAlternate: "",
    collationMaxVariable: "",
    collationNormalization: false,
    collationBackwards: false,
    wildcardProjection: "",
  };
}

function draftFromIndex(idx: IndexInfo): DraftState {
  const d = emptyDraft();
  d.name = idx.name;
  const parsedFields = keyFieldsFromJson(idx.key);
  d.fields = parsedFields.length > 0 ? parsedFields : [newKeyField()];
  d.unique = idx.unique;
  d.sparse = idx.sparse;
  d.hidden = idx.hidden;
  d.ttlSeconds = idx.ttlSeconds != null ? String(idx.ttlSeconds) : "";
  d.partialFilterExpression = idx.partialFilterExpression
    ? JSON.stringify(idx.partialFilterExpression, null, 2)
    : "";
  if (idx.collation) {
    d.collationLocale = idx.collation.locale;
    d.collationStrength = idx.collation.strength != null ? String(idx.collation.strength) : "";
    d.collationCaseLevel = idx.collation.caseLevel ?? false;
    d.collationCaseFirst = idx.collation.caseFirst ?? "";
    d.collationNumericOrdering = idx.collation.numericOrdering ?? false;
    d.collationAlternate = idx.collation.alternate ?? "";
    d.collationMaxVariable = idx.collation.maxVariable ?? "";
    d.collationNormalization = idx.collation.normalization ?? false;
    d.collationBackwards = idx.collation.backwards ?? false;
  }
  d.wildcardProjection = idx.wildcardProjection
    ? JSON.stringify(idx.wildcardProjection, null, 2)
    : "";
  return d;
}

function draftToRequest(
  d: DraftState,
  connectionId: string,
  database: string,
  collection: string,
): CreateIndexRequest | { error: string } {
  const name = d.name.trim();
  if (!name) return { error: "Give the index a name." };
  const fields = d.fields.filter((f) => f.field.trim() !== "");
  if (fields.length === 0) return { error: "Add at least one key field." };
  const keyJson = keyFieldsToJson(fields);
  if (keyJson === "{}") return { error: "Index key must have at least one field." };

  let partialFilterExpressionJson: string | null = null;
  if (d.partialFilterExpression.trim()) {
    try {
      JSON.parse(d.partialFilterExpression);
      partialFilterExpressionJson = d.partialFilterExpression.trim();
    } catch {
      return { error: "Partial filter expression must be valid JSON." };
    }
  }

  let wildcardProjectionJson: string | null = null;
  if (d.wildcardProjection.trim()) {
    try {
      JSON.parse(d.wildcardProjection);
      wildcardProjectionJson = d.wildcardProjection.trim();
    } catch {
      return { error: "Wildcard projection must be valid JSON." };
    }
  }

  let collation: CollationDto | null = null;
  if (d.collationLocale.trim()) {
    collation = {
      locale: d.collationLocale.trim(),
      strength: d.collationStrength ? Number.parseInt(d.collationStrength, 10) : null,
      caseLevel: d.collationCaseLevel || null,
      caseFirst: d.collationCaseFirst || null,
      numericOrdering: d.collationNumericOrdering || null,
      alternate: d.collationAlternate || null,
      maxVariable: d.collationMaxVariable || null,
      normalization: d.collationNormalization || null,
      backwards: d.collationBackwards || null,
    };
  }

  const ttlSeconds = d.ttlSeconds.trim()
    ? Number.parseInt(d.ttlSeconds, 10)
    : null;

  return {
    connectionId,
    database,
    collection,
    name,
    keyJson,
    unique: d.unique,
    sparse: d.sparse,
    hidden: d.hidden,
    ttlSeconds: ttlSeconds != null && Number.isFinite(ttlSeconds) ? ttlSeconds : null,
    partialFilterExpressionJson,
    collation,
    wildcardProjectionJson,
  };
}

export function IndexTab({ connectionId, database, collection }: IndexTabProps) {
  const [indexes, setIndexes] = useState<IndexInfo[] | null>(null);
  const [stats, setStats] = useState<IndexStats[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [modalOpen, setModalOpen] = useState(false);
  const [draft, setDraft] = useState<DraftState>(emptyDraft());
  const [editingName, setEditingName] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [sortByUsage, setSortByUsage] = useState(false);
  const [statsUnavailable, setStatsUnavailable] = useState(false);

  async function refresh() {
    setError(null);
    setStatsUnavailable(false);
    try {
      const [idx, st] = await Promise.all([
        commands.listIndexes(connectionId, database, collection),
        commands.indexStats(connectionId, database, collection).catch((e) => {
          // $indexStats is unsupported on some backends (e.g. Serverless
          // preview, or the user lacks the clusterMonitor role). Treat
          // any failure as "stats unavailable" and keep the index list
          // usable rather than failing the whole tab.
          const msg = describeError(e);
          setStatsUnavailable(true);
          setError(`Index usage stats unavailable: ${msg}`);
          return null;
        }),
      ]);
      setIndexes(idx);
      setStats(st);
    } catch (e) {
      setError(describeError(e));
    }
  }

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId, database, collection]);

  const statsByName = useMemo(() => {
    const m = new Map<string, IndexStats>();
    if (stats) for (const s of stats) m.set(s.name, s);
    return m;
  }, [stats]);

  const maxOps = useMemo(() => {
    if (!stats || stats.length === 0) return 0;
    return Math.max(1, ...stats.map((s) => s.ops));
  }, [stats]);

  const sortedIndexes = useMemo(() => {
    if (!indexes) return null;
    if (!sortByUsage) return indexes;
    return [...indexes].sort((a, b) => {
      const ao = statsByName.get(a.name)?.ops ?? 0;
      const bo = statsByName.get(b.name)?.ops ?? 0;
      return bo - ao;
    });
  }, [indexes, sortByUsage, statsByName]);

  function openCreate() {
    setDraft(emptyDraft());
    setEditingName(null);
    setModalOpen(true);
  }

  function openEdit(idx: IndexInfo) {
    setDraft(draftFromIndex(idx));
    setEditingName(idx.isId ? null : idx.name);
    setModalOpen(true);
  }

  async function handleSubmit() {
    setError(null);
    const result = draftToRequest(draft, connectionId, database, collection);
    if ("error" in result) {
      setError(result.error);
      return;
    }
    // Editing an existing index means MongoDB cannot apply option changes
    // in place, so we drop then recreate. The modal note informed the
    // user of this; confirm once more because dropping is destructive.
    if (editingName) {
      if (
        !window.confirm(
          `Drop and recreate index "${editingName}"? The existing index will be removed first.`,
        )
      ) {
        return;
      }
    }
    try {
      if (editingName) {
        await commands.dropIndex(connectionId, database, collection, editingName);
      }
      await commands.createIndex(result);
      setModalOpen(false);
      setNotice(`Index "${result.name}" saved.`);
      window.setTimeout(() => setNotice(null), 2500);
      await refresh();
    } catch (e) {
      setError(describeError(e));
      // If the drop succeeded but create failed, refresh so the user
      // sees the index is gone and can retry without a stale list.
      if (editingName) await refresh();
    }
  }

  async function handleDrop(name: string) {
    if (!window.confirm(`Drop index ${name}? This cannot be undone.`)) return;
    setError(null);
    try {
      await commands.dropIndex(connectionId, database, collection, name);
      setNotice(`Dropped index "${name}".`);
      window.setTimeout(() => setNotice(null), 2500);
      await refresh();
    } catch (e) {
      setError(describeError(e));
    }
  }

  function toggleExpanded(name: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  return (
    <div className="pane">
      <div className="pane__header">
        <h2 className="pane__title">Indexes — {database}.{collection}</h2>
        <div className="pane__sub">
          {indexes ? `${indexes.length} index(es)` : "Loading…"}
        </div>
      </div>
      <div className="pane__body" style={{ padding: 16, display: "grid", gap: 16 }}>
        {error && (
          <Alert tone="danger">{error}</Alert>
        )}
        {notice && (
          <Alert tone="success">{notice}</Alert>
        )}

        <div className="row" style={{ justifyContent: "space-between" }}>
          <button className="btn btn--primary" onClick={openCreate}>
            + New index
          </button>
          <label className="row" style={{ gap: 6, fontSize: "var(--font-size-sm)" }}>
            <input
              type="checkbox"
              checked={sortByUsage}
              onChange={(e) => setSortByUsage(e.target.checked)}
              disabled={!stats || stats.length === 0}
            />
            Sort by usage
          </label>
        </div>

        {sortedIndexes && sortedIndexes.length > 0 && (
          <div className="index-table">
            <div className="index-row index-row--header">
              <span></span>
              <span>Name</span>
              <span>Key</span>
              <span>Options</span>
              <span>Usage</span>
              <span>Size</span>
              <span></span>
            </div>
            {sortedIndexes.map((idx) => {
              const st = statsByName.get(idx.name);
              const isOpen = expanded.has(idx.name);
              const opsPct = st ? Math.round((st.ops / maxOps) * 100) : 0;
              return (
                <div key={idx.name} className="index-row-group">
                  <div className="index-row">
                    <span className="index-row__expand">
                      <button
                        className="btn btn--sm btn--ghost"
                        onClick={() => toggleExpanded(idx.name)}
                        aria-label={isOpen ? "Collapse" : "Expand"}
                        title={isOpen ? "Collapse" : "Expand"}
                      >
                        {isOpen ? "▾" : "▸"}
                      </button>
                    </span>
                    <span style={{ fontFamily: "var(--font-mono)" }}>
                      {idx.name}
                      {idx.isId && <span className="kind-badge" style={{ marginLeft: 6 }}>_id</span>}
                      {idx.isText && <span className="kind-badge" style={{ marginLeft: 6 }}>text</span>}
                      {idx.isGeo && <span className="kind-badge" style={{ marginLeft: 6 }}>geo</span>}
                      {idx.hidden && (
                        <span className="kind-badge kind-badge--muted" style={{ marginLeft: 6 }}>
                          hidden
                        </span>
                      )}
                    </span>
                    <span style={{ fontFamily: "var(--font-mono)", color: "var(--ink-muted)" }}>
                      {formatKey(idx.key)}
                    </span>
                    <span className="index-row__options">
                      {idx.unique && <span className="kind-badge">unique</span>}
                      {idx.sparse && <span className="kind-badge">sparse</span>}
                      {idx.ttlSeconds != null && (
                        <span className="kind-badge">TTL {idx.ttlSeconds}s</span>
                      )}
                      {idx.partialFilterExpression && (
                        <span className="kind-badge">partial</span>
                      )}
                      {idx.collation && (
                        <span className="kind-badge">collation:{idx.collation.locale}</span>
                      )}
                      {idx.wildcardProjection && (
                        <span className="kind-badge">wildcard</span>
                      )}
                    </span>
                    <span className="index-row__usage">
                      {statsUnavailable ? (
                        <span className="index-row__usage-na">n/a</span>
                      ) : st ? (
                        <div className="usage-bar">
                          <div
                            className="usage-bar__fill"
                            style={{ transform: `scaleX(${Math.max(opsPct, 0.001) / 100})` }}
                            title={`${st.ops} ops`}
                          />
                          <span className="usage-bar__label">{st.ops.toLocaleString()}</span>
                        </div>
                      ) : (
                        <span className="index-row__usage-na">—</span>
                      )}
                    </span>
                    <span style={{ fontFamily: "var(--font-mono)", color: "var(--ink-muted)" }}>
                      {st?.sizeBytes != null ? formatBytes(st.sizeBytes) : "—"}
                    </span>
                    <span className="index-row__actions">
                      <button
                        className="btn btn--sm btn--ghost"
                        onClick={() => openEdit(idx)}
                        title="View / edit details"
                      >
                        Edit
                      </button>
                      {!idx.isId && (
                        <button
                          className="btn btn--sm btn--danger"
                          onClick={() => handleDrop(idx.name)}
                        >
                          Drop
                        </button>
                      )}
                    </span>
                  </div>
                  {isOpen && (
                    <div className="index-row-detail">
                      <DetailRow label="Key spec">
                        <code>{JSON.stringify(idx.key)}</code>
                      </DetailRow>
                      {idx.ttlSeconds != null && (
                        <DetailRow label="TTL">
                          {idx.ttlSeconds} seconds
                        </DetailRow>
                      )}
                      {idx.partialFilterExpression && (
                        <DetailRow label="Partial filter">
                          <code>{JSON.stringify(idx.partialFilterExpression, null, 2)}</code>
                        </DetailRow>
                      )}
                      {idx.collation && (
                        <DetailRow label="Collation">
                          <code>{JSON.stringify(idx.collation, null, 2)}</code>
                        </DetailRow>
                      )}
                      {idx.wildcardProjection && (
                        <DetailRow label="Wildcard projection">
                          <code>{JSON.stringify(idx.wildcardProjection, null, 2)}</code>
                        </DetailRow>
                      )}
                      {st && (
                        <>
                          <DetailRow label="Operations">{st.ops.toLocaleString()}</DetailRow>
                          {st.accesses != null && (
                            <DetailRow label="Accesses">{st.accesses.toLocaleString()}</DetailRow>
                          )}
                          {st.sinceMs != null && (
                            <DetailRow label="Since">
                              {new Date(st.sinceMs).toLocaleString()}
                            </DetailRow>
                          )}
                          {st.building && <DetailRow label="Building">yes</DetailRow>}
                        </>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
        {sortedIndexes && sortedIndexes.length === 0 && (
          <div className="empty-state">
            <h2>No indexes</h2>
            <p>This collection has no indexes (other than the default _id_).</p>
          </div>
        )}
      </div>

      <Modal
        open={modalOpen}
        title={editingName ? `Index — ${editingName}` : "New index"}
        onClose={() => setModalOpen(false)}
        width={640}
        footer={
          <>
            <button className="btn" onClick={() => setModalOpen(false)}>
              Cancel
            </button>
            <button className="btn btn--primary" onClick={handleSubmit}>
              {editingName ? "Recreate" : "Create"}
            </button>
          </>
        }
      >
        <IndexForm draft={draft} setDraft={setDraft} editing={editingName != null} />
        {editingName && (
          <p className="index-form__note">
            MongoDB does not support in-place index option changes. Saving will
            drop the existing index and recreate it with these options.
          </p>
        )}
      </Modal>
    </div>
  );
}

function DetailRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="index-row-detail__row">
      <span className="index-row-detail__label">{label}</span>
      <span className="index-row-detail__value">{children}</span>
    </div>
  );
}

function formatKey(key: Record<string, unknown>): string {
  const entries = Object.entries(key);
  return entries
    .map(([f, v]) => {
      if (typeof v === "number") return `${f}:${v >= 0 ? "1" : "-1"}`;
      return `${f}:${v}`;
    })
    .join(", ");
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

interface IndexFormProps {
  draft: DraftState;
  setDraft: (updater: (prev: DraftState) => DraftState) => void;
  editing: boolean;
}

function IndexForm({ draft, setDraft, editing }: IndexFormProps) {
  const [draggingKey, setDraggingKey] = useState<string | null>(null);

  const moveField = useCallback((fromKey: string, toKey: string) => {
    setDraft((prev) => {
      if (fromKey === toKey) return prev;
      const fromIdx = prev.fields.findIndex((f) => f.key === fromKey);
      const toIdx = prev.fields.findIndex((f) => f.key === toKey);
      if (fromIdx === -1 || toIdx === -1) return prev;
      const next = prev.fields.slice();
      const [moved] = next.splice(fromIdx, 1);
      next.splice(toIdx, 0, moved);
      return { ...prev, fields: next };
    });
  }, [setDraft]);

  function updateField(key: string, patch: Partial<KeyField>) {
    setDraft((prev) => ({
      ...prev,
      fields: prev.fields.map((f) => (f.key === key ? { ...f, ...patch } : f)),
    }));
  }

  function addField() {
    setDraft((prev) => ({ ...prev, fields: [...prev.fields, newKeyField()] }));
  }

  function removeField(key: string) {
    setDraft((prev) => ({
      ...prev,
      fields: prev.fields.filter((f) => f.key !== key),
    }));
  }

  return (
    <div className="index-form">
      <label className="field">
        <span className="field__label">Index name</span>
        <input
          className="field__input"
          value={draft.name}
          onChange={(e) =>
            setDraft((prev) => ({ ...prev, name: e.target.value }))
          }
          placeholder="e.g. idx_user_email"
          disabled={editing}
        />
      </label>

      <div className="field">
        <span className="field__label">Key fields (drag to reorder)</span>
        <div className="key-builder">
          {draft.fields.map((f) => (
            <div
              key={f.key}
              className={`key-builder__row ${draggingKey === f.key ? "key-builder__row--dragging" : ""}`}
              draggable
              onDragStart={() => setDraggingKey(f.key)}
              onDragOver={(e) => e.preventDefault()}
              onDragEnd={() => setDraggingKey(null)}
              onDrop={(e) => {
                e.preventDefault();
                if (draggingKey && draggingKey !== f.key) {
                  moveField(draggingKey, f.key);
                }
                setDraggingKey(null);
              }}
            >
              <span className="key-builder__handle" title="Drag to reorder">⋮⋮</span>
              <input
                className="field__input key-builder__field"
                value={f.field}
                onChange={(e) => updateField(f.key, { field: e.target.value })}
                placeholder="field name"
                spellCheck={false}
              />
              <select
                className="field__input key-builder__type"
                value={f.type}
                onChange={(e) => updateField(f.key, { type: e.target.value })}
              >
                {KEY_TYPES.map((t) => (
                  <option key={t} value={t}>
                    {KEY_TYPE_LABELS[t]}
                  </option>
                ))}
              </select>
              <button
                className="btn btn--sm btn--ghost key-builder__remove"
                onClick={() => removeField(f.key)}
                title="Remove field"
                aria-label="Remove field"
              >
                ×
              </button>
            </div>
          ))}
          <button className="btn btn--sm key-builder__add" onClick={addField}>
            + Add field
          </button>
        </div>
      </div>

      <div className="index-form__row">
        <label className="field__checkbox">
          <input
            type="checkbox"
            checked={draft.unique}
            onChange={(e) =>
              setDraft((prev) => ({ ...prev, unique: e.target.checked }))
            }
          />
          unique
        </label>
        <label className="field__checkbox">
          <input
            type="checkbox"
            checked={draft.sparse}
            onChange={(e) =>
              setDraft((prev) => ({ ...prev, sparse: e.target.checked }))
            }
          />
          sparse
        </label>
        <label className="field__checkbox">
          <input
            type="checkbox"
            checked={draft.hidden}
            onChange={(e) =>
              setDraft((prev) => ({ ...prev, hidden: e.target.checked }))
            }
          />
          hidden
        </label>
        <label className="field">
          <span className="field__label">TTL (seconds)</span>
          <input
            className="field__input"
            type="number"
            value={draft.ttlSeconds}
            onChange={(e) =>
              setDraft((prev) => ({ ...prev, ttlSeconds: e.target.value }))
            }
            placeholder="off"
            style={{ width: 120 }}
          />
        </label>
      </div>

      <label className="field">
        <span className="field__label">Partial filter expression (JSON, optional)</span>
        <textarea
          className="field__input index-form__textarea"
          value={draft.partialFilterExpression}
          onChange={(e) =>
            setDraft((prev) => ({ ...prev, partialFilterExpression: e.target.value }))
          }
          placeholder='{ "status": "active" }'
          spellCheck={false}
          rows={3}
        />
      </label>

      <label className="field">
        <span className="field__label">Wildcard projection (JSON, optional)</span>
        <textarea
          className="field__input index-form__textarea"
          value={draft.wildcardProjection}
          onChange={(e) =>
            setDraft((prev) => ({ ...prev, wildcardProjection: e.target.value }))
          }
          placeholder='{ "field": 1 }'
          spellCheck={false}
          rows={2}
        />
      </label>

      <fieldset className="index-form__collation">
        <legend>Collation (optional — leave locale empty to omit)</legend>
        <div className="index-form__row">
          <label className="field">
            <span className="field__label">Locale</span>
            <input
              className="field__input"
              value={draft.collationLocale}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationLocale: e.target.value }))
              }
              placeholder="e.g. en_US"
              spellCheck={false}
            />
          </label>
          <label className="field">
            <span className="field__label">Strength</span>
            <select
              className="field__input"
              value={draft.collationStrength}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationStrength: e.target.value }))
              }
            >
              {COLLATION_STRENGTHS.map((s) => (
                <option key={s.value} value={s.value}>{s.label}</option>
              ))}
            </select>
          </label>
          <label className="field">
            <span className="field__label">Case first</span>
            <select
              className="field__input"
              value={draft.collationCaseFirst}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationCaseFirst: e.target.value }))
              }
            >
              {COLLATION_CASE_FIRST.map((s) => (
                <option key={s.value} value={s.value}>{s.label}</option>
              ))}
            </select>
          </label>
        </div>
        <div className="index-form__row">
          <label className="field">
            <span className="field__label">Alternate</span>
            <select
              className="field__input"
              value={draft.collationAlternate}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationAlternate: e.target.value }))
              }
            >
              {COLLATION_ALTERNATES.map((s) => (
                <option key={s.value} value={s.value}>{s.label}</option>
              ))}
            </select>
          </label>
          <label className="field">
            <span className="field__label">Max variable</span>
            <select
              className="field__input"
              value={draft.collationMaxVariable}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationMaxVariable: e.target.value }))
              }
            >
              {COLLATION_MAX_VARIABLE.map((s) => (
                <option key={s.value} value={s.value}>{s.label}</option>
              ))}
            </select>
          </label>
        </div>
        <div className="index-form__row">
          <label className="field__checkbox">
            <input
              type="checkbox"
              checked={draft.collationCaseLevel}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationCaseLevel: e.target.checked }))
              }
            />
            case level
          </label>
          <label className="field__checkbox">
            <input
              type="checkbox"
              checked={draft.collationNumericOrdering}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationNumericOrdering: e.target.checked }))
              }
            />
            numeric ordering
          </label>
          <label className="field__checkbox">
            <input
              type="checkbox"
              checked={draft.collationNormalization}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationNormalization: e.target.checked }))
              }
            />
            normalization
          </label>
          <label className="field__checkbox">
            <input
              type="checkbox"
              checked={draft.collationBackwards}
              onChange={(e) =>
                setDraft((prev) => ({ ...prev, collationBackwards: e.target.checked }))
              }
            />
            backwards
          </label>
        </div>
      </fieldset>
    </div>
  );
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}
