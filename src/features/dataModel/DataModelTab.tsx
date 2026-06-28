import { useEffect, useMemo, useState } from "react";
import { Network, LayoutGrid, List } from "lucide-react";
import commands, {
  type CollectionSummary,
  type DataModelGraph,
  type RelationshipEdge,
} from "../../ipc/commands";
import { useToast } from "../../context/ToastContext";
import { ShapeTreeView } from "./ShapeTreeView";
import { DiagramCanvas } from "./DiagramCanvas";
import { InfoPopover } from "../../components/InfoPopover";

export interface DataModelTabProps {
  connectionId: string;
  database: string;
}

type ViewMode = "diagram" | "relationships" | "shape";

export function DataModelTab({ connectionId, database }: DataModelTabProps) {
  const [collections, setCollections] = useState<CollectionSummary[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [graph, setGraph] = useState<DataModelGraph | null>(null);
  const [scanning, setScanning] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("diagram");
  const [selectedNode, setSelectedNode] = useState<string | null>(null);
  const [confidenceThreshold, setConfidenceThreshold] = useState(0.25);
  const toast = useToast();

  useEffect(() => {
    setCollections(null);
    setSelected(new Set());
    setGraph(null);
    setSelectedNode(null);
    commands
      .listCollections(connectionId, database)
      .then((cols) => {
        setCollections(cols);
        // Default selection: first 20 regular collections, sorted by doc count.
        const sorted = [...cols].sort((a, b) =>
          (b.documentCount ?? 0) > (a.documentCount ?? 0) ? 1 : -1,
        );
        const defaults = new Set(
          sorted
            .filter((c) => c.type === "collection")
            .slice(0, 20)
            .map((c) => c.name),
        );
        setSelected(defaults);
      })
      .catch((e) => toast.push(formatError(e), "error"));
  }, [connectionId, database]);

  const runScan = async () => {
    if (selected.size === 0) {
      toast.push("Select at least one collection", "warning");
      return;
    }
    setScanning(true);
    setGraph(null);
    try {
      const g = await commands.scanDataModel({
        connectionId,
        database,
        collections: Array.from(selected),
        sampleSize: 200,
        signals: {
          objectIdMatch: true,
          naming: true,
          lookup: false,
          index: true,
          appSchema: false,
        },
        confidenceThreshold,
        appSchemaPath: null,
      });
      setGraph(g);
      if (g.nodes.length > 0) {
        setSelectedNode(g.nodes[0].collection);
      }
    } catch (e) {
      toast.push(formatError(e), "error");
    } finally {
      setScanning(false);
    }
  };

  const toggleCollection = (name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const selectedShape = useMemo(
    () => graph?.nodes.find((n) => n.collection === selectedNode),
    [graph, selectedNode],
  );

  const visibleEdges = useMemo(
    () =>
      graph?.edges.filter((e) => !e.hidden && e.confidence >= confidenceThreshold) ?? [],
    [graph, confidenceThreshold],
  );

  const diagramGraph = useMemo(() => {
    if (!graph) return null;
    return { ...graph, edges: visibleEdges };
  }, [graph, visibleEdges]);

  return (
    <div className="pane">
      <div className="pane__header">
        <h2 className="pane__title">Data Model — {database}</h2>
        <div className="pane__sub">
          {scanning ? "Scanning collections…" : graph ? `${graph.nodes.length} collections · ${visibleEdges.length} relationships` : "Select collections and scan"}
        </div>
      </div>
      <div className="pane__body" style={{ display: "flex", flexDirection: "column" }}>
        <div className="shape-toolbar">
          <button className="btn btn--primary" onClick={runScan} disabled={scanning || selected.size === 0}>
            {scanning ? "Scanning…" : "Scan selected"}
          </button>
          <label className="data-model__threshold">
            <span className="data-model__threshold-label">
              Confidence
              <InfoPopover
                label="What is confidence?"
                title="Relationship confidence"
              >
                <p>
                  Confidence is how sure NoSQLBuddy is that a detected
                  relationship is real. It is the combined weight of the
                  signals that suggested the link:
                </p>
                <ul>
                  <li><strong>ObjectId match</strong>: sampled ref values exist in the target collection&apos;s <code>_id</code>s.</li>
                  <li><strong>App schema</strong>: a Mongoose or Prisma model declares the reference.</li>
                  <li><strong>$lookup</strong>: a saved aggregation or view joins these collections.</li>
                  <li><strong>Naming</strong>: the field name matches a collection name (e.g. <code>userId</code> → <code>users</code>).</li>
                  <li><strong>Index</strong>: the ref field is indexed, so it is likely queried.</li>
                </ul>
                <p>
                  Strong evidence (a high ObjectId match ratio, or an app
                  schema ref) boosts confidence to 90% or higher. Lower the
                  threshold to reveal weaker, naming-only guesses; raise it to
                  keep only well-supported relationships on the canvas.
                </p>
              </InfoPopover>
            </span>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={confidenceThreshold}
              onChange={(e) => setConfidenceThreshold(parseFloat(e.target.value))}
              aria-label="Confidence threshold"
            />
            <span className="data-model__threshold-value">{(confidenceThreshold * 100).toFixed(0)}%</span>
          </label>
          <div className="data-model__view-switch">
            <button
              className={`data-model__view-btn${viewMode === "diagram" ? " is-active" : ""}`}
              onClick={() => setViewMode("diagram")}
              title="Diagram"
            >
              <Network size={14} /> Diagram
            </button>
            <button
              className={`data-model__view-btn${viewMode === "relationships" ? " is-active" : ""}`}
              onClick={() => setViewMode("relationships")}
              title="Relationships"
            >
              <List size={14} /> Relationships
            </button>
            <button
              className={`data-model__view-btn${viewMode === "shape" ? " is-active" : ""}`}
              onClick={() => setViewMode("shape")}
              title="Shape"
            >
              <LayoutGrid size={14} /> Shape
            </button>
          </div>
          <InfoPopover label="View modes" title="View modes">
            <p><strong>Diagram</strong>: visual node graph of collections and relationships.</p>
            <p><strong>Relationships</strong>: tabular list of all detected links with confidence scores.</p>
            <p><strong>Shape</strong>: detailed field-by-field schema for a single collection.</p>
          </InfoPopover>
          <span className="shape-toolbar__hint">
            {selected.size} selected · {collections?.length ?? 0} total collections
          </span>
        </div>
        <div className="data-model__workspace">
          <div className="data-model__selector">
            <div className="data-model__selector-header">
              <span>Collections</span>
              <button
                className="btn btn--ghost btn--sm"
                onClick={() => setSelected(new Set(collections?.map((c) => c.name) ?? []))}
              >
                All
              </button>
            </div>
            <div className="data-model__selector-list">
              {collections?.map((c) => (
                <label key={c.name} className="data-model__selector-row">
                  <input
                    type="checkbox"
                    checked={selected.has(c.name)}
                    onChange={() => toggleCollection(c.name)}
                  />
                  <span className="data-model__selector-name" title={c.name}>
                    {c.name}
                  </span>
                  <span className="data-model__selector-count">
                    {c.documentCount != null ? c.documentCount.toLocaleString() : "?"}
                  </span>
                </label>
              ))}
            </div>
          </div>
          <div className="data-model__main">
            {viewMode === "diagram" && diagramGraph && (
              <DiagramCanvas
                graph={diagramGraph}
                onNodeClick={(name) => {
                  setSelectedNode(name);
                  setViewMode("shape");
                }}
              />
            )}
            {viewMode === "relationships" && graph && (
              <div className="data-model__relationships">
                <h3 className="data-model__section-title">Relationships</h3>
                {visibleEdges.length > 0 ? (
                  <RelationshipsTable edges={visibleEdges} />
                ) : (
                  <div className="shape-empty">No relationships detected above the confidence threshold.</div>
                )}
              </div>
            )}
            {viewMode === "shape" && graph && selectedShape && (
              <div className="data-model__shape">
                <div className="data-model__shape-header">
                  <h3 className="data-model__section-title">Shape: {selectedShape.collection}</h3>
                  <select
                    className="field__input"
                    value={selectedNode ?? ""}
                    onChange={(e) => setSelectedNode(e.target.value)}
                  >
                    {graph.nodes.map((n) => (
                      <option key={n.collection} value={n.collection}>
                        {n.collection}
                      </option>
                    ))}
                  </select>
                </div>
                <ShapeTreeView shape={selectedShape} />
              </div>
            )}
            {viewMode === "shape" && graph && !selectedShape && (
              <div className="shape-empty">Select a collection from the dropdown above.</div>
            )}
            {!graph && !scanning && (
              <div className="shape-empty">
                Select collections on the left and click “Scan selected” to infer the data model.
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

function RelationshipsTable({ edges }: { edges: RelationshipEdge[] }) {
  return (
    <div className="data-model__relationships-table">
      {edges.map((e) => (
        <div key={e.id} className="data-model__relationship-row">
          <span className="data-model__relationship-from">{e.fromCollection}</span>
          <span className="data-model__relationship-field" title={e.fromField}>
            {e.fromField}
          </span>
          <span className="data-model__relationship-kind">{kindLabel(e.kind)}</span>
          <span className="data-model__relationship-to">{e.toCollection}</span>
          <span className="data-model__relationship-confidence">
            <span
              className="data-model__confidence-bar"
              style={{ width: `${e.confidence * 100}%` }}
            />
            <span className="data-model__confidence-label">{(e.confidence * 100).toFixed(0)}%</span>
          </span>
        </div>
      ))}
    </div>
  );
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

function formatError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}
