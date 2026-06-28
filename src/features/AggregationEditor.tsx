import { useCallback, useEffect, useMemo, useState } from "react";
import commands, { type DocumentPage, type ExplainResult } from "../ipc/commands";
import { ResultsTable } from "../components/ResultsTable";
import { ExplainTree } from "../components/ExplainTree";
import { InfoPopover } from "../components/InfoPopover";
import { DriverCodePanel } from "../components/DriverCodePanel";
import type { Language } from "../components/driverCodeTypes";
import { useToast } from "../context/ToastContext";

export interface AggregationEditorProps {
  connectionId: string;
  database: string;
  collection: string;
  /** Profile metadata for the active connection. Used by the
   *  driver-code popover to embed the user's real URI + a
   *  profile / auth comment. May be null when no connection
   *  is open (e.g. tests). */
  profile?: { id: string; name: string; authMechanism: string } | null;
  /** Called when a Run or Explain completes successfully. */
  onResult?: (page: DocumentPage | null) => void;
  /** Called when the pipeline changes (so parents can persist it). */
  onPipelineChange?: (pipeline: unknown[]) => void;
  /** Initial pipeline to populate the editor. */
  initialPipeline?: unknown[];
}

interface Stage {
  /** Stable id for React keys + drag tracking. */
  key: string;
  /** Raw JSON the user edited for this stage. */
  body: string;
}

const STAGE_TEMPLATES: Array<{ name: string; body: string }> = [
  { name: "$match", body: '{ "field": "value" }' },
  { name: "$project", body: '{ "field": 1 }' },
  { name: "$group", body: '{ "_id": "$field", "count": { "$sum": 1 } }' },
  { name: "$sort", body: '{ "field": 1 }' },
  { name: "$limit", body: "50" },
  { name: "$skip", body: "0" },
  { name: "$lookup", body: '{\n  "from": "other",\n  "localField": "id",\n  "foreignField": "_id",\n  "as": "joined"\n}' },
  { name: "$unwind", body: '"$arrayField"' },
  { name: "$addFields", body: '{ "newField": "value" }' },
  { name: "$count", body: '"total"' },
];

let stageCounter = 0;
function makeKey(): string {
  stageCounter += 1;
  return `stage-${Date.now().toString(36)}-${stageCounter}`;
}

function defaultStages(): Stage[] {
  return [
    {
      key: makeKey(),
      body: '{ "active": true }',
    },
    {
      key: makeKey(),
      body: "50",
    },
  ];
}

function parseStageBody(body: string): { value: unknown; error: string | null } {
  const trimmed = body.trim();
  if (trimmed === "") {
    return { value: null, error: "Stage body cannot be empty." };
  }
  try {
    return { value: JSON.parse(trimmed), error: null };
  } catch (e) {
    return { value: null, error: `Invalid JSON: ${describeError(e)}` };
  }
}

function describeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  return "Unexpected error";
}

function inferStageOperator(stageValue: unknown): string | null {
  if (stageValue === null || typeof stageValue !== "object" || Array.isArray(stageValue)) {
    return null;
  }
  const keys = Object.keys(stageValue as Record<string, unknown>);
  if (keys.length !== 1) return null;
  const k = keys[0];
  return k.startsWith("$") ? k : null;
}

export function AggregationEditor({
  connectionId,
  database,
  collection,
  profile,
  onResult,
  onPipelineChange,
  initialPipeline,
}: AggregationEditorProps) {
  const [stages, setStages] = useState<Stage[]>(() => stagesFromInitial(initialPipeline));
  const [page, setPage] = useState<DocumentPage | null>(null);
  const [explainResult, setExplainResult] = useState<ExplainResult | null>(null);
  const [running, setRunning] = useState(false);
  const toast = useToast();
  const [showCodeModal, setShowCodeModal] = useState(false);
  const [showExplainModal, setShowExplainModal] = useState(false);
  const [resolvedUri, setResolvedUri] = useState<string>("");
  const [draggingKey, setDraggingKey] = useState<string | null>(null);
  const [showTemplates, setShowTemplates] = useState(false);

  // Re-parse every stage on every change so we can flag invalid bodies.
  const parsedStages = useMemo(
    () =>
      stages.map((s) => {
        const parsed = parseStageBody(s.body);
        return { key: s.key, value: parsed.value, error: parsed.error };
      }),
    [stages],
  );

  const allValid = parsedStages.every((p) => p.error === null);
  const pipeline = useMemo(
    () => parsedStages.map((p) => p.value),
    [parsedStages],
  );

  useEffect(() => {
    if (onPipelineChange) onPipelineChange(pipeline);
  }, [pipeline, onPipelineChange]);

  // Close modals on Escape.
  useEffect(() => {
    if (!showCodeModal && !showExplainModal) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setShowCodeModal(false);
        setShowExplainModal(false);
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [showCodeModal, showExplainModal]);

  // Track whether the URI is still resolving so the code-fetch
  // effect can wait for it. Without this gate, opening the panel
  // immediately would fire the IPC with `uri=""` and we'd fall
  // back to the placeholder URI permanently.
  const [uriReady, setUriReady] = useState(false);

  // Fetch the full (unmasked) URI for the active profile so the
  // driver-code panel can embed it in the generated snippet. We
  // only do this once per profile id; switching tabs or running
  // the pipeline does not require another fetch.
  useEffect(() => {
    let cancelled = false;
    if (!profile) {
      setResolvedUri("");
      setUriReady(true);
      return;
    }
    setUriReady(false);
    commands
      .resolveProfileUri(profile.id)
      .then((uri) => {
        if (cancelled) return;
        setResolvedUri(uri);
        setUriReady(true);
      })
      .catch(() => {
        if (cancelled) return;
        setResolvedUri("");
        setUriReady(true);
      });
    return () => {
      cancelled = true;
    };
  }, [profile]);

  // Code map for the DriverCodePanel. The editor owns the IPC;
  // the panel itself is purely presentational. We fetch all six
  // languages up-front when the user opens the panel so the
  // language dropdown is instant.
  const [codeByLanguage, setCodeByLanguage] = useState<
    Partial<Record<Language, string>>
  >({});
  useEffect(() => {
    if (!showCodeModal) return;
    if (!profile) {
      setCodeByLanguage({});
      return;
    }
    if (!uriReady) return;
    let cancelled = false;
    // Debounce: don't fire 6 IPC calls on every keystroke. The
    // user is editing stage JSON which is captured by `pipeline`,
    // but they probably don't care about the generated code until
    // they pause for half a second.
    const handle = window.setTimeout(() => {
      const all: Language[] = [
        "node-js",
        "python",
        "java",
        "c-sharp",
        "ruby",
        "shell",
      ];
      Promise.all(
        all.map(async (lang) => {
          try {
            const code = await commands.generatePipelineCode({
              database,
              collection,
              pipeline,
              language: lang,
              profileName: profile.name,
              authMechanism: profile.authMechanism,
              uri: resolvedUri,
            });
            return [lang, code] as const;
          } catch {
            return [lang, ""] as const;
          }
        }),
      ).then((entries) => {
        if (cancelled) return;
        const next: Partial<Record<Language, string>> = {};
        for (const [lang, code] of entries) next[lang] = code;
        setCodeByLanguage(next);
      });
    }, 400);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [showCodeModal, profile, resolvedUri, uriReady, database, collection, pipeline]);

  async function run() {
    if (!allValid) {
      toast.push("Fix invalid stage JSON before running.", "error");
      return;
    }
    setExplainResult(null);
    setRunning(true);
    try {
      const result = await commands.aggregateDocuments({
        connectionId,
        database,
        collection,
        pipelineJson: JSON.stringify(pipeline),
        limit: 50,
      });
      setPage(result);
      toast.push(
        `${result.documents.length} returned · ${result.executionMs ?? 0} ms`,
        "success",
      );
      if (onResult) onResult(result);
    } catch (e) {
      toast.push(describeError(e), "error");
    } finally {
      setRunning(false);
    }
  }

  async function runExplain() {
    if (!allValid) {
      toast.push("Fix invalid stage JSON before explaining.", "error");
      return;
    }
    setExplainResult(null);
    setRunning(true);
    try {
      const result = await commands.explainAggregate(
        connectionId,
        database,
        collection,
        JSON.stringify(pipeline),
      );
      setExplainResult(result);
      setShowExplainModal(true);
      toast.push("Explain completed.", "success");
    } catch (e) {
      toast.push(`Explain failed: ${describeError(e)}`, "error");
    } finally {
      setRunning(false);
    }
  }

  function addStage(template?: { name: string; body: string }) {
    setStages((prev) => [
      ...prev,
      { key: makeKey(), body: template ? template.body : '{ }' },
    ]);
    setShowTemplates(false);
  }

  function updateStageBody(key: string, body: string) {
    setStages((prev) =>
      prev.map((s) => (s.key === key ? { ...s, body } : s)),
    );
  }

  function removeStage(key: string) {
    setStages((prev) => prev.filter((s) => s.key !== key));
  }

  const moveStage = useCallback((fromKey: string, toKey: string) => {
    setStages((prev) => {
      if (fromKey === toKey) return prev;
      const fromIdx = prev.findIndex((s) => s.key === fromKey);
      const toIdx = prev.findIndex((s) => s.key === toKey);
      if (fromIdx === -1 || toIdx === -1) return prev;
      const next = prev.slice();
      const [moved] = next.splice(fromIdx, 1);
      next.splice(toIdx, 0, moved);
      return next;
    });
  }, []);

  function clearAll() {
    setStages([]);
    setPage(null);
    setExplainResult(null);
    setShowCodeModal(false);
    setShowExplainModal(false);
  }

  return (
    <div className="agg-editor">
      <div className="agg-editor__toolbar">
        <button
          className="btn btn--primary btn--sm"
          onClick={() => void run()}
          disabled={running || stages.length === 0 || !allValid}
        >
          {running ? "Running…" : "Run"}
        </button>
        <button
          className="btn btn--sm"
          onClick={() => void runExplain()}
          disabled={running || stages.length === 0 || !allValid}
          title="Run server-side explain on the pipeline"
        >
          Explain
        </button>
        <InfoPopover label="Explain plan help" title="Explain plan"><p>Shows how MongoDB will execute your pipeline without running it. Reveals which indexes are used, execution stages, and performance characteristics. Use to optimize slow queries.</p></InfoPopover>
        <button
          className="btn btn--sm"
          onClick={clearAll}
          disabled={stages.length === 0 && !page && !explainResult}
        >
          Clear
        </button>
        <button
          className="btn btn--sm"
          onClick={() => setShowCodeModal(true)}
          disabled={pipeline.length === 0}
          title="Generate driver code for the current pipeline"
        >
          Code
        </button>
        <span className="agg-editor__sub">
          {stages.length === 0
            ? "No stages."
            : `${stages.length} stage${stages.length === 1 ? "" : "s"}`}
          {page ? ` · ${page.documents.length} returned · ${page.executionMs ?? 0} ms` : ""}
        </span>
        <div className="agg-editor__add">
          <button
            className="btn btn--sm"
            onClick={() => setShowTemplates((v) => !v)}
            disabled={stages.length === 0 && false /* always allow add */}
          >
            + Add stage
          </button>
          <InfoPopover label="Aggregation stages help" title="Aggregation stages">
          <ul>
            <li><strong>$match</strong>: filter documents.</li>
            <li><strong>$project</strong>: reshape output fields.</li>
            <li><strong>$group</strong>: aggregate values.</li>
            <li><strong>$sort</strong>: order results.</li>
            <li><strong>$limit / $skip</strong>: paginate results.</li>
            <li><strong>$lookup</strong>: join another collection.</li>
            <li><strong>$unwind</strong>: flatten arrays into rows.</li>
            <li><strong>$addFields</strong>: compute new fields.</li>
          </ul>
        </InfoPopover>
          {showTemplates && (
            <div className="agg-editor__templates">
              {STAGE_TEMPLATES.map((t) => (
                <button
                  key={t.name}
                  className="agg-editor__template"
                  onClick={() => addStage(t)}
                  title={`Insert a ${t.name} stage`}
                >
                  <span className="agg-editor__template-name">{t.name}</span>
                </button>
              ))}
            </div>
          )}
        </div>
      </div>
      <div className="agg-editor__stages">
        {stages.length === 0 ? (
          <div className="empty-state">
            <h2>No stages yet</h2>
            <p>Click "+ Add stage" to start building a pipeline.</p>
          </div>
        ) : (
          stages.map((stage, idx) => {
            const parsed = parsedStages[idx];
            const error = parsed?.error ?? null;
            const op = parsed && error === null ? inferStageOperator(parsed.value) : null;
            return (
              <div
                key={stage.key}
                className={`agg-stage ${draggingKey === stage.key ? "agg-stage--dragging" : ""}`}
                draggable
                onDragStart={() => setDraggingKey(stage.key)}
                onDragOver={(e) => {
                  // Allow the drop on this target; do not reorder on
                  // every hover event — reorder only on drop.
                  e.preventDefault();
                }}
                onDragEnd={() => setDraggingKey(null)}
                onDrop={(e) => {
                  e.preventDefault();
                  if (draggingKey && draggingKey !== stage.key) {
                    moveStage(draggingKey, stage.key);
                  }
                  setDraggingKey(null);
                }}
              >
                <div className="agg-stage__head">
                  <span
                    className="agg-stage__handle"
                    title="Drag to reorder"
                    aria-label="Drag handle"
                  >
                    ⋮⋮
                  </span>
                  <span className="agg-stage__index">{idx + 1}.</span>
                  <span className="agg-stage__operator">{op ?? "(invalid)"}</span>
                  <button
                    className="btn btn--sm agg-stage__delete"
                    onClick={() => removeStage(stage.key)}
                    title="Remove this stage"
                  >
                    Delete
                  </button>
                </div>
                <textarea
                  className="agg-stage__editor"
                  value={stage.body}
                  onChange={(e) => updateStageBody(stage.key, e.target.value)}
                  spellCheck={false}
                  rows={Math.min(10, Math.max(2, stage.body.split("\n").length))}
                  aria-label={`Stage ${idx + 1} JSON`}
                />
                {error && (
                  <div className="agg-stage__error">{error}</div>
                )}
              </div>
            );
          })
        )}
      </div>
      {page && (
        <div className="agg-editor__results">
          <ResultsTable
            documents={page.documents as Array<Record<string, unknown>>}
            connectionId={connectionId}
            database={database}
            collection={collection}
            editable={false}
          />
        </div>
      )}
      {showCodeModal && (
        <div
          className="modal-backdrop"
          role="dialog"
          aria-modal="true"
          aria-label="Driver code"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowCodeModal(false);
          }}
        >
          <div className="modal modal--code" style={{ width: "min(760px, 92vw)" }}>
            <div className="modal__header">
              <div className="modal__heading">
                <h2 className="modal__title">Driver code</h2>
                <span className="modal__subtitle">
                  {stages.length} stage{stages.length === 1 ? "" : "s"} · {database}.{collection}
                </span>
              </div>
              <button
                className="modal__close"
                onClick={() => setShowCodeModal(false)}
                aria-label="Close"
              >
                ×
              </button>
            </div>
            <div className="modal__body modal__body--flush">
              {!profile && (
                <div className="modal__notice">
                  Connect to a profile to embed your real Mongo URI in the generated code.
                </div>
              )}
              <DriverCodePanel
                pipeline={pipeline}
                codeByLanguage={codeByLanguage}
                initialLanguage="node-js"
              />
            </div>
            <div className="modal__footer">
              <span className="modal__footer-hint">Esc to close</span>
              <button className="btn btn--sm" onClick={() => setShowCodeModal(false)}>
                Close
              </button>
            </div>
          </div>
        </div>
      )}
      {showExplainModal && (
        <div
          className="modal-backdrop"
          role="dialog"
          aria-modal="true"
          aria-label="Explain plan"
          onClick={(e) => {
            if (e.target === e.currentTarget) setShowExplainModal(false);
          }}
        >
          <div className="modal modal--explain" style={{ width: "min(760px, 92vw)" }}>
            <div className="modal__header">
              <div className="modal__heading">
                <h2 className="modal__title">Explain plan</h2>
                <span className="modal__subtitle">
                  {database}.{collection}
                </span>
              </div>
              <button
                className="modal__close"
                onClick={() => setShowExplainModal(false)}
                aria-label="Close"
              >
                ×
              </button>
            </div>
            <div className="modal__body modal__body--flush">
              {explainResult && <ExplainTree raw={explainResult} />}
            </div>
            <div className="modal__footer">
              <span className="modal__footer-hint">Esc to close</span>
              <button className="btn btn--sm" onClick={() => setShowExplainModal(false)}>
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function stagesFromInitial(initial: unknown[] | undefined): Stage[] {
  if (!initial || initial.length === 0) return defaultStages();
  return initial.map((stage) => {
    const body = JSON.stringify(stage, null, 2);
    return {
      key: makeKey(),
      body,
    };
  });
}
