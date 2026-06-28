import { useState } from "react";
import { Check, Eye, EyeOff } from "lucide-react";
import type {
  CollectionShape,
  RelationshipEdge,
  RelationshipSignal,
} from "../../ipc/commands";
import { InfoPopover } from "../../components/InfoPopover";

export interface DataModelInspectorProps {
  /** Edges already filtered by confidence threshold (but including hidden ones
   * so the inspector can list and un-hide them). */
  edges: RelationshipEdge[];
  /** The currently selected collection name, if any. */
  selectedCollection: string | null;
  /** All collection shapes, for the detail card. */
  nodes: CollectionShape[];
  /** Apply a confirm/hide override to one edge. */
  onOverrideEdge: (edgeId: string, overrides: { confirmed?: boolean; hidden?: boolean }) => void;
  /** Select a collection (e.g. when clicking an edge endpoint). */
  onSelectCollection: (name: string) => void;
}

/**
 * Right-hand inspector panel for the data-model tab. Combines:
 *  - the relationships list (with per-edge confirm/hide controls + signal
 *    breakdown on selection), and
 *  - a detail card for the selected collection (doc count, field count,
 *    indexes, links into Schema/Query/Index tabs via the host).
 *
 * The panel is self-contained: it owns the "selected edge" state and only
 * calls back to the host for overrides and collection selection.
 */
export function DataModelInspector({
  edges,
  selectedCollection,
  nodes,
  onOverrideEdge,
  onSelectCollection,
}: DataModelInspectorProps) {
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | null>(null);
  const selectedEdge = edges.find((e) => e.id === selectedEdgeId) ?? null;
  const selectedShape = nodes.find((n) => n.collection === selectedCollection) ?? null;

  return (
    <div className="data-model__inspector">
      <div className="data-model__inspector-section">
        <div className="data-model__inspector-heading">
          <h3 className="data-model__section-title">Relationships ({edges.length})</h3>
          <InfoPopover label="What am I looking at?" title="Inferred relationships">
            <p>
              Each row is a relationship NoSQLBuddy inferred from your data.
              Click a row to see the evidence signals and decide whether to
              keep or hide it.
            </p>
            <p>
              <strong>Confirm</strong> locks an edge as real (treats it as
              100% confident). <strong>Hide</strong> removes it from the canvas
              without deleting it — useful for noisy guesses.
            </p>
          </InfoPopover>
        </div>

        {edges.length === 0 ? (
          <div className="shape-empty data-model__inspector-empty">
            No relationships detected at the current confidence threshold.
          </div>
        ) : (
          <div className="data-model__inspector-edges">
            {edges.map((e) => (
              <button
                key={e.id}
                className={`data-model__edge-row${e.id === selectedEdgeId ? " is-selected" : ""}${e.confirmed ? " is-confirmed" : ""}${e.hidden ? " is-hidden" : ""}`}
                onClick={() => setSelectedEdgeId(e.id)}
              >
                <span className="data-model__edge-from" title={e.fromCollection}>
                  {e.fromCollection}
                </span>
                <span className="data-model__edge-field" title={e.fromField}>
                  {e.fromField}
                </span>
                <span className="data-model__edge-kind">{kindLabel(e.kind)}</span>
                <span className="data-model__edge-to" title={e.toCollection}>
                  {e.toCollection}
                </span>
                <span className="data-model__edge-confidence">
                  <span
                    className="data-model__confidence-bar"
                    style={{ width: `${(e.confirmed ? 1 : e.confidence) * 100}%` }}
                  />
                  <span className="data-model__confidence-label">
                    {e.confirmed ? "✓" : `${(e.confidence * 100).toFixed(0)}%`}
                  </span>
                </span>
              </button>
            ))}
          </div>
        )}
      </div>

      {selectedEdge && (
        <EdgeDetail
          edge={selectedEdge}
          onConfirm={() => onOverrideEdge(selectedEdge.id, { confirmed: !selectedEdge.confirmed })}
          onHide={() => onOverrideEdge(selectedEdge.id, { hidden: !selectedEdge.hidden })}
          onSelectCollection={onSelectCollection}
        />
      )}

      {selectedShape && (
        <CollectionDetail shape={selectedShape} />
      )}
    </div>
  );
}

function EdgeDetail({
  edge,
  onConfirm,
  onHide,
  onSelectCollection,
}: {
  edge: RelationshipEdge;
  onConfirm: () => void;
  onHide: () => void;
  onSelectCollection: (name: string) => void;
}) {
  return (
    <div className="data-model__inspector-section data-model__edge-detail">
      <h3 className="data-model__section-title">Selected relationship</h3>
      <div className="data-model__edge-detail-link">
        <button className="data-model__link-btn" onClick={() => onSelectCollection(edge.fromCollection)}>
          {edge.fromCollection}
        </button>
        <span className="data-model__edge-detail-field">{edge.fromField}</span>
        <span className="data-model__edge-kind">{kindLabel(edge.kind)}</span>
        <span className="data-model__edge-detail-field">{edge.toField}</span>
        <button className="data-model__link-btn" onClick={() => onSelectCollection(edge.toCollection)}>
          {edge.toCollection}
        </button>
      </div>
      <div className="data-model__edge-detail-confidence">
        Confidence:{" "}
        <strong>{edge.confirmed ? "100% (confirmed)" : `${(edge.confidence * 100).toFixed(0)}%`}</strong>
      </div>

      <div className="data-model__edge-actions">
        <button
          className={`btn btn--sm${edge.confirmed ? " btn--primary" : " btn--ghost"}`}
          onClick={onConfirm}
          title={edge.confirmed ? "Unconfirm this relationship" : "Mark as confirmed (100% confidence)"}
        >
          <Check size={12} /> {edge.confirmed ? "Confirmed" : "Confirm"}
        </button>
        <button
          className={`btn btn--sm${edge.hidden ? " btn--primary" : " btn--ghost"}`}
          onClick={onHide}
          title={edge.hidden ? "Show this relationship on the canvas" : "Hide this relationship from the canvas"}
        >
          {edge.hidden ? <Eye size={12} /> : <EyeOff size={12} />}
          {edge.hidden ? "Show" : "Hide"}
        </button>
      </div>

      <div className="data-model__signals">
        <div className="data-model__signals-title">Evidence ({edge.signals.length})</div>
        {edge.signals.length === 0 ? (
          <div className="data-model__signals-empty">No signals recorded.</div>
        ) : (
          <ul className="data-model__signals-list">
            {edge.signals.map((s, i) => (
              <SignalRow key={i} signal={s} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function SignalRow({ signal }: { signal: RelationshipSignal }) {
  return (
    <li className="data-model__signal-row">
      <span className={`data-model__signal-kind kind-badge kind-badge--${signalKindClass(signal.kind)}`}>
        {signalKindLabel(signal.kind)}
      </span>
      <span className="data-model__signal-detail">{signal.detail}</span>
      <span className="data-model__signal-weight">{(signal.weight * 100).toFixed(0)}%</span>
    </li>
  );
}

function CollectionDetail({ shape }: { shape: CollectionShape }) {
  const fieldCount = countFields(shape);
  return (
    <div className="data-model__inspector-section data-model__collection-detail">
      <h3 className="data-model__section-title">{shape.collection}</h3>
      <dl className="data-model__stats">
        <div className="data-model__stat">
          <dt>Documents</dt>
          <dd>{shape.documentCount != null ? shape.documentCount.toLocaleString() : "—"}</dd>
        </div>
        <div className="data-model__stat">
          <dt>Fields</dt>
          <dd>{fieldCount}</dd>
        </div>
        <div className="data-model__stat">
          <dt>Indexes</dt>
          <dd>{shape.indexes.length}</dd>
        </div>
        <div className="data-model__stat">
          <dt>Sampled</dt>
          <dd>{shape.sampledDocuments}</dd>
        </div>
      </dl>
      {shape.warnings.length > 0 && (
        <div className="data-model__collection-warnings">
          {shape.warnings.map((w, i) => (
            <div key={i} className="data-model__collection-warning">{w}</div>
          ))}
        </div>
      )}
    </div>
  );
}

function countFields(shape: CollectionShape): number {
  const walk = (n: { children?: unknown[] }): number => {
    const kids = Array.isArray(n.children) ? n.children : [];
    return kids.reduce<number>((acc, c) => acc + 1 + walk(c as { children?: unknown[] }), 0);
  };
  return walk(shape.root);
}

function kindLabel(kind: string): string {
  switch (kind) {
    case "one-to-one":
      return "1:1";
    case "one-to-many":
      return "1:N";
    case "many-to-one":
      return "N:1";
    case "many-to-many":
      return "N:N";
    default:
      return kind;
  }
}

function signalKindLabel(kind: string): string {
  switch (kind) {
    case "objectIdMatch":
      return "ObjectId";
    case "namingConvention":
      return "Naming";
    case "lookup":
      return "$lookup";
    case "index":
      return "Index";
    case "appSchema":
      return "App schema";
    default:
      return kind;
  }
}

function signalKindClass(kind: string): string {
  switch (kind) {
    case "objectIdMatch":
      return "objectid";
    case "namingConvention":
      return "naming";
    case "lookup":
      return "lookup";
    case "index":
      return "index";
    case "appSchema":
      return "appschema";
    default:
      return "default";
  }
}
