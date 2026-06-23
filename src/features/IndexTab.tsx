import { useEffect, useState } from "react";
import commands, { type IndexInfo, type CreateIndexRequest } from "../ipc/commands";

export interface IndexTabProps {
  connectionId: string;
  database: string;
  collection: string;
}

export function IndexTab({ connectionId, database, collection }: IndexTabProps) {
  const [indexes, setIndexes] = useState<IndexInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [draftName, setDraftName] = useState("");
  const [draftKey, setDraftKey] = useState("{ \"name\": 1 }");
  const [draftUnique, setDraftUnique] = useState(false);
  const [draftSparse, setDraftSparse] = useState(false);
  const [draftTtl, setDraftTtl] = useState("");

  async function refresh() {
    setError(null);
    try {
      setIndexes(
        await commands.listIndexes(connectionId, database, collection),
      );
    } catch (e) {
      setError(describeError(e));
    }
  }

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [connectionId, database, collection]);

  async function handleCreate() {
    setError(null);
    let key: unknown;
    try {
      key = JSON.parse(draftKey);
    } catch (e) {
      setError("Index key must be valid JSON.");
      return;
    }
    if (!draftName.trim()) {
      setError("Give the index a name.");
      return;
    }
    if (!key || typeof key !== "object") {
      setError("Index key must be a JSON object.");
      return;
    }
    const request: CreateIndexRequest = {
      connectionId,
      database,
      collection,
      name: draftName.trim(),
      keyJson: JSON.stringify(key),
      unique: draftUnique,
      sparse: draftSparse,
      ttlSeconds: draftTtl ? Number.parseInt(draftTtl, 10) : null,
    };
    try {
      await commands.createIndex(request);
      setDraftName("");
      setDraftTtl("");
      await refresh();
    } catch (e) {
      setError(describeError(e));
    }
  }

  async function handleDrop(name: string) {
    if (!window.confirm(`Drop index ${name}?`)) return;
    setError(null);
    try {
      await commands.dropIndex(connectionId, database, collection, name);
      await refresh();
    } catch (e) {
      setError(describeError(e));
    }
  }

  return (
    <div className="pane">
      <div className="pane__header">
        <h2 className="pane__title">Indexes — {database}.{collection}</h2>
        <div className="pane__sub">{indexes ? `${indexes.length} index(es)` : "Loading…"}</div>
      </div>
      <div className="pane__body" style={{ padding: 16, display: "grid", gap: 16 }}>
        {error && <div className="toast toast--error" style={{ position: "static" }}>{error}</div>}
        <section
          style={{
            background: "var(--surface)",
            border: "1px solid var(--border)",
            borderRadius: 8,
            padding: 12,
            display: "grid",
            gap: 8,
          }}
        >
          <div className="row">
            <input
              className="field__input"
              style={{ flex: "1 1 160px" }}
              placeholder="index name"
              value={draftName}
              onChange={(e) => setDraftName(e.target.value)}
            />
            <input
              className="field__input"
              style={{ flex: "2 1 200px", fontFamily: "var(--font-mono)" }}
              placeholder='{"name": 1}'
              value={draftKey}
              onChange={(e) => setDraftKey(e.target.value)}
              spellCheck={false}
            />
            <input
              className="field__input"
              style={{ flex: "0 0 100px" }}
              type="number"
              placeholder="TTL s"
              value={draftTtl}
              onChange={(e) => setDraftTtl(e.target.value)}
            />
            <label className="row" style={{ gap: 6 }}>
              <input
                type="checkbox"
                checked={draftUnique}
                onChange={(e) => setDraftUnique(e.target.checked)}
              />
              unique
            </label>
            <label className="row" style={{ gap: 6 }}>
              <input
                type="checkbox"
                checked={draftSparse}
                onChange={(e) => setDraftSparse(e.target.checked)}
              />
              sparse
            </label>
            <button className="btn btn--primary" onClick={handleCreate}>
              Create
            </button>
          </div>
        </section>
        {indexes && indexes.length > 0 && (
          <div style={{ border: "1px solid var(--border)", borderRadius: 8, overflow: "hidden" }}>
            <div className="index-row index-row--header">
              <span>Name</span>
              <span>Key</span>
              <span>Unique</span>
              <span>Sparse</span>
              <span></span>
            </div>
            {indexes.map((idx) => (
              <div key={idx.name} className="index-row">
                <span style={{ fontFamily: "var(--font-mono)" }}>
                  {idx.name}
                  {idx.isText && <span className="kind-badge" style={{ marginLeft: 6 }}>text</span>}
                  {idx.isGeo && <span className="kind-badge" style={{ marginLeft: 6 }}>geo</span>}
                  {idx.isId && <span className="kind-badge" style={{ marginLeft: 6 }}>_id</span>}
                </span>
                <span style={{ fontFamily: "var(--font-mono)", color: "var(--ink-muted)" }}>
                  {JSON.stringify(idx.key)}
                  {idx.ttlSeconds != null && (
                    <span style={{ color: "var(--ink-faint)" }}>
                      {"  TTL "} {idx.ttlSeconds}s
                    </span>
                  )}
                </span>
                <span>{idx.unique ? "yes" : "no"}</span>
                <span>{idx.sparse ? "yes" : "no"}</span>
                <span>
                  {!idx.isId && (
                    <button className="btn btn--danger btn--sm" onClick={() => handleDrop(idx.name)}>
                      Drop
                    </button>
                  )}
                </span>
              </div>
            ))}
          </div>
        )}
      </div>
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
