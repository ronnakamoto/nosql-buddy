import { useMemo, useState } from "react";
import type { CollectionShape, ShapeNode } from "../../ipc/commands";
import {
  DateHistogramChart,
  DateStatLine,
  NumericHistogramChart,
  NumericStatLine,
  TopValuesChart,
} from "../../components/SchemaCharts";
import { InfoPopover } from "../../components/InfoPopover";

export interface ShapeTreeViewProps {
  shape: CollectionShape;
}

export function ShapeTreeView({ shape }: ShapeTreeViewProps) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const toggle = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  return (
    <div className="shape-tree">
      <div className="shape-tree__header">
        <span className="shape-tree__title">{shape.database}.{shape.collection}</span>
        <span className="shape-tree__meta">
          {shape.sampledDocuments} sampled · depth {shape.maxDepth}
          {shape.documentCount !== null && ` · ${shape.documentCount.toLocaleString()} docs`}
        </span>
      </div>
      {shape.warnings.length > 0 && (
        <div className="shape-tree__warnings">
          {shape.warnings.map((w, i) => (
            <div key={i} className="shape-tree__warning">
              {w}
            </div>
          ))}
        </div>
      )}
      <div className="shape-tree__list">
        {shape.root.children.map((child) => (
          <ShapeNodeRow
            key={child.path}
            node={child}
            depth={0}
            expanded={expanded}
            onToggle={toggle}
          />
        ))}
      </div>
    </div>
  );
}

function ShapeNodeRow({
  node,
  depth,
  expanded,
  onToggle,
}: {
  node: ShapeNode;
  depth: number;
  expanded: Set<string>;
  onToggle: (path: string) => void;
}) {
  const isExpanded = expanded.has(node.path);
  const hasChildren = node.children.length > 0;
  const hasArrayItem = node.arrayItem != null;
  const hasCharts =
    node.topValues != null || node.numericStats != null || node.dateStats != null;
  const expandable = hasChildren || hasArrayItem || hasCharts;
  const isPlaceholder = node.name === "…";

  const typeEntries = useMemo(
    () => Object.entries(node.types).sort((a, b) => b[1] - a[1]),
    [node.types],
  );

  return (
    <div className={`shape-node${isExpanded ? " shape-node--open" : ""}`}>
      <div
        className="shape-node__head"
        style={{ paddingLeft: `${depth * 18}px` }}
        onClick={expandable ? () => onToggle(node.path) : undefined}
        role={expandable ? "button" : undefined}
        tabIndex={expandable ? 0 : undefined}
        onKeyDown={
          expandable
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onToggle(node.path);
                }
              }
            : undefined
        }
      >
        <span
          className={`shape-node__caret${expandable ? "" : " shape-node__caret--hidden"}${isExpanded ? " shape-node__caret--open" : ""}`}
        >
          ▸
        </span>
        <span className="shape-node__name" title={node.path}>
          {node.name}
        </span>
        <span className="shape-node__types">
          {typeEntries.map(([t, p]) => (
            <span key={t} className="schema-type" title={`${(p * 100).toFixed(1)}%`}>
              {t} · {(p * 100).toFixed(0)}%
            </span>
          ))}
        </span>
        <span className="shape-node__stats">
          {node.presence < 1 && (
            <span className="shape-node__presence" title="Fraction of documents where this path exists">
              {(node.presence * 100).toFixed(0)}% present
              <InfoPopover label="Field presence" title="Field presence"><p>Percentage of sampled documents that contain this field. Less than 100% indicates an optional field.</p></InfoPopover>
            </span>
          )}
          {node.nullRatio > 0 && (
            <span className="shape-node__null" title="Fraction of documents with an explicit null">
              · {(node.nullRatio * 100).toFixed(1)}% null
              <InfoPopover label="Null ratio" title="Null ratio"><p>Percentage of documents where this field explicitly has the value null, different from missing entirely.</p></InfoPopover>
            </span>
          )}
          {node.cardinality != null && (
            <span className="shape-node__cardinality">· {node.cardinality} distinct<InfoPopover label="Cardinality" title="Cardinality"><p>Number of unique values for this field. Low cardinality suggests an enum-like field. High cardinality suggests IDs or free-form text.</p></InfoPopover></span>
          )}
          {hasArrayItem && <span className="shape-node__array">· array</span>}
        </span>
      </div>
      {isExpanded && (
        <div className="shape-node__detail" style={{ paddingLeft: `${depth * 18}px` }}>
          {hasCharts && (
            <div className="shape-node__charts">
              {node.topValues && (
                <div className="shape-node__section">
                  <h4 className="shape-node__section-title">Top values</h4>
                  <TopValuesChart values={node.topValues} />
                </div>
              )}
              {node.numericStats && (
                <div className="shape-node__section">
                  <h4 className="shape-node__section-title">Numeric distribution</h4>
                  <NumericStatLine stats={node.numericStats} />
                  <NumericHistogramChart stats={node.numericStats} />
                </div>
              )}
              {node.dateStats && (
                <div className="shape-node__section">
                  <h4 className="shape-node__section-title">Date distribution</h4>
                  <DateStatLine stats={node.dateStats} />
                  <DateHistogramChart stats={node.dateStats} />
                </div>
              )}
            </div>
          )}
          {hasArrayItem && !isPlaceholder && (
            <div className="shape-node__array-section">
              <div className="shape-node__section-title">Array element</div>
              <ShapeNodeRow
                node={node.arrayItem!}
                depth={depth + 1}
                expanded={expanded}
                onToggle={onToggle}
              />
            </div>
          )}
          {hasChildren && (
            <div className="shape-node__children">
              {node.children.map((child) => (
                <ShapeNodeRow
                  key={child.path}
                  node={child}
                  depth={depth + 1}
                  expanded={expanded}
                  onToggle={onToggle}
                />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
