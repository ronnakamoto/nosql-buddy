import { useEffect, useMemo, useState } from "react";
import commands, { formatError, type SchemaField, type SchemaReport } from "../ipc/commands";
import { useToast } from "../context/ToastContext";
import {
  DateHistogramChart,
  DateStatLine,
  NumericHistogramChart,
  NumericStatLine,
  RatioBar,
  TopValuesChart,
} from "../components/SchemaCharts";
import { InfoPopover } from "../components/InfoPopover";

export interface SchemaTabProps {
  connectionId: string;
  database: string;
  collection: string;
}

export function SchemaTab({ connectionId, database, collection }: SchemaTabProps) {
  const [report, setReport] = useState<SchemaReport | null>(null);
  const toast = useToast();
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  useEffect(() => {
    setReport(null);
    setExpanded(new Set());
    commands
      .sampleSchema(connectionId, database, collection)
      .then(setReport)
      .catch((e) => toast.push(describeError(e), "error"));
  }, [connectionId, database, collection]);

  const fields = useMemo(
    () =>
      report
        ? [...report.fields].sort((a, b) => a.name.localeCompare(b.name))
        : [],
    [report],
  );

  function toggle(name: string) {
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
        <h2 className="pane__title">Schema — {database}.{collection}</h2>
        <div className="pane__sub">
          {report ? `${report.sampledDocuments} sampled · ${report.fields.length} fields` : "Sampling…"}
          {report && (
            <InfoPopover label="Sampled documents" title="Sampled documents">
              <p>Schema analysis is based on a random sample of documents, not the full collection. Large collections are sampled for performance.</p>
            </InfoPopover>
          )}
        </div>
      </div>
      <div className="pane__body" style={{ padding: 16 }}>
        {report && (
          <div className="schema-list">
            {fields.map((f) => (
              <SchemaFieldRow
                key={f.name}
                field={f}
                total={report.sampledDocuments}
                expanded={expanded.has(f.name)}
                onToggle={() => toggle(f.name)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function SchemaFieldRow({
  field,
  total,
  expanded,
  onToggle,
}: {
  field: SchemaField;
  total: number;
  expanded: boolean;
  onToggle: () => void;
}) {
  const nullCount = Math.round(field.nullRatio * total);
  const hasChart =
    field.topValues !== null ||
    field.numericStats !== null ||
    field.dateStats !== null;
  const expandable = hasChart;

  return (
    <div className={`schema-field${expanded ? " schema-field--open" : ""}`}>
      <div
        className="schema-field__head"
        onClick={expandable ? onToggle : undefined}
        role={expandable ? "button" : undefined}
        tabIndex={expandable ? 0 : undefined}
        onKeyDown={
          expandable
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onToggle();
                }
              }
            : undefined
        }
      >
        <span
          className={`schema-field__caret${expandable ? "" : " schema-field__caret--hidden"}${expanded ? " schema-field__caret--open" : ""}`}
        >
          ▸
        </span>
        <span className="schema-field__name" style={{ fontFamily: "var(--font-mono)" }}>
          {field.name}
        </span>
        <span className="schema-field__types">
          {Object.entries(field.types)
            .sort((a, b) => b[1] - a[1])
            .map(([t, c]) => (
              <span key={t} className="schema-type" title={`${c} occurrences`}>
                {t} · {c}
              </span>
            ))}
        </span>
        <span className="schema-field__ratio">
          <RatioBar
            total={total}
            nullCount={nullCount}
            missingCount={field.missingCount}
          />
          <span className="schema-field__ratio-label">
            {field.missingCount > 0 && (
              <span className="schema-field__missing" title={`${field.missingCount} docs missing this field`}>
                {field.missingCount} missing
                <InfoPopover label="Missing field" title="Missing field">
                  <p>Number of documents where this field does not exist at all. This is different from null, where the field exists but has no value.</p>
                </InfoPopover>
              </span>
            )}
            {(field.nullRatio > 0 || field.missingCount > 0) && " · "}
            <span title={`${nullCount} null values`}>
              {(field.nullRatio * 100).toFixed(1)}% null
              <InfoPopover label="Null ratio" title="Null ratio">
                <p>Percentage of sampled documents where this field exists but has a null value. Excludes documents where the field is missing entirely.</p>
              </InfoPopover>
            </span>
          </span>
        </span>
      </div>
      {expanded && hasChart && (
        <div className="schema-field__detail">
          {field.topValues && (
            <div className="schema-field__section">
              <h4 className="schema-field__section-title">Top values<InfoPopover label="Top values" title="Top values"><p>The most frequently occurring values for this field in the sample, with their counts. Useful for understanding data distribution and cardinality.</p></InfoPopover></h4>
              <TopValuesChart values={field.topValues} />
            </div>
          )}
          {field.numericStats && (
            <div className="schema-field__section">
              <h4 className="schema-field__section-title">
                Numeric distribution
                <InfoPopover label="Numeric distribution" title="Numeric distribution"><p>Statistical summary (min, mean, max) and histogram showing how numeric values are distributed across ranges.</p></InfoPopover>
              </h4>
              <NumericStatLine stats={field.numericStats} />
              <NumericHistogramChart stats={field.numericStats} />
            </div>
          )}
          {field.dateStats && (
            <div className="schema-field__section">
              <h4 className="schema-field__section-title">Date distribution<InfoPopover label="Date distribution" title="Date distribution"><p>Date range and histogram showing how dates are distributed over time. Useful for identifying time-based patterns.</p></InfoPopover></h4>
              <DateStatLine stats={field.dateStats} />
              <DateHistogramChart stats={field.dateStats} />
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function describeError(e: unknown): string {
  return formatError(e);
}
