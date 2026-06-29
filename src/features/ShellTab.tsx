import { acceptCompletion, completionStatus, moveCompletionSelection } from "@codemirror/autocomplete";
import type { EditorView, KeyBinding } from "@codemirror/view";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import commands, {
  type ProfileSummary,
  type ShellOutput,
  type ShellResponse,
  type ShellTable,
} from "../ipc/commands";
import { CodeEditor } from "../components/CodeEditor";
import { ResultsTable } from "../components/ResultsTable";
import { ExplainTree } from "../components/ExplainTree";
import { DriverCodePanel } from "../components/DriverCodePanel";
import { useToast } from "../context/ToastContext";
import { makeShellCompletionSource } from "../lib/shellCompletion";
import { HISTORY_CLEAR, nextHistoryIndex, prevHistoryIndex } from "../lib/shellHistory";

export interface ShellTabProps {
  connectionId: string;
  database: string;
  profile: ProfileSummary | null;
}

interface HistoryEntry {
  script: string;
  outputs: ShellOutput[];
  activeDatabase: string;
  executionMs: number;
  timestamp: number;
}

/**
 * Shell tab — production-grade mongo shell REPL.
 *
 * Architecture:
 *  - CodeMirror editor for input. Enter runs the script;
 *    Shift+Enter inserts a newline.
 *  - Output area renders one card per output line (text / json /
 *    error / table) with colour coding.
 *  - History is held in component state for the lifetime of the
 *    tab. Up arrow recalls the most recent entry.
 *  - The last aggregate call exposes an "Explain" button (opens
 *    the visual explain diagram) and a "Code" button (opens the
 *    driver-code popover). These reuse the same components the
 *    Aggregation Editor uses, so the UI is consistent.
 */
export function ShellTab({
  connectionId,
  database,
  profile,
}: ShellTabProps) {
  const [script, setScript] = useState<string>("");
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const [activeDb, setActiveDb] = useState<string>(database);
  const [running, setRunning] = useState(false);
  const toast = useToast();
  const [lastResponse, setLastResponse] = useState<ShellResponse | null>(null);
  const [resolvedUri, setResolvedUri] = useState<string>("");
  const [codeOpen, setCodeOpen] = useState(false);
  const [explainOpen, setExplainOpen] = useState(false);
  const [explainRaw, setExplainRaw] = useState<unknown | null>(null);
  const [historyCursor, setHistoryCursor] = useState<number>(-1);

  // Live refs so the CodeMirror completion source and keymap can be built
  // once (stable identity) yet always read the latest props/state. Rebuilding
  // these on every keystroke would force CodeMirror to reconfigure the editor.
  const runRef = useRef<(text?: string) => void>(() => {});
  const historyRef = useRef<HistoryEntry[]>(history);
  const historyCursorRef = useRef<number>(historyCursor);
  const completionCtxRef = useRef({ connectionId, activeDb, database });
  historyRef.current = history;
  historyCursorRef.current = historyCursor;
  completionCtxRef.current = { connectionId, activeDb, database };

  // Fetch the full URI for driver-code generation. Same pattern
  // as AggregationEditor.
  useEffect(() => {
    if (!profile) {
      setResolvedUri("");
      return;
    }
    let cancelled = false;
    commands
      .resolveProfileUri(profile.id)
      .then((uri) => {
        if (!cancelled) setResolvedUri(uri);
      })
      .catch(() => {
        if (!cancelled) setResolvedUri("");
      });
    return () => {
      cancelled = true;
    };
  }, [profile?.id]);

  const run = useCallback(
    async (text?: string) => {
      const source = (text ?? script).trim();
      if (!source || running) return;
      setRunning(true);
      try {
        const resp = await commands.evalShell({
          connectionId,
          script: source,
          activeDatabase: activeDb,
          fallbackDatabase: database,
        });
        setLastResponse(resp);
        setActiveDb(resp.activeDatabase);
        setHistory((h) => [
          ...h,
          {
            script: source,
            outputs: resp.outputs,
            activeDatabase: resp.activeDatabase,
            executionMs: resp.executionMs,
            timestamp: Date.now(),
          },
        ]);
        setHistoryCursor(-1);
        setScript("");
      } catch (e) {
        toast.push(String(e), "error");
      } finally {
        setRunning(false);
      }
    },
    [script, running, connectionId, activeDb, database],
  );
  runRef.current = run;

  /** Replace the entire editor document and place the caret at its end.
   *  Mirrors the new text into React state so the controlled value stays
   *  in sync with the view. */
  const replaceDoc = useCallback((view: EditorView, text: string) => {
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      selection: { anchor: text.length },
    });
    setScript(text);
  }, []);

  const shellCompletionSource = useMemo(
    () =>
      makeShellCompletionSource({
        request: ({ textBeforeCursor }) => {
          const ctx = completionCtxRef.current;
          return commands.shellAutocomplete({
            connectionId: ctx.connectionId,
            textBeforeCursor,
            activeDatabase: ctx.activeDb,
            fallbackDatabase: ctx.database,
          });
        },
      }),
    [],
  );

  const shellKeymap = useMemo<KeyBinding[]>(
    () => [
      {
        key: "Tab",
        run(view) {
          if (completionStatus(view.state) === "active") {
            acceptCompletion(view);
            return true;
          }
          return false;
        },
      },
      {
        key: "Enter",
        run(view) {
          if (completionStatus(view.state) === "active") {
            acceptCompletion(view);
            return true;
          }
          runRef.current(view.state.doc.toString());
          return true;
        },
      },
      {
        key: "Shift-Enter",
        run(view) {
          view.dispatch(view.state.replaceSelection("\n"));
          return true;
        },
      },
      {
        key: "ArrowUp",
        run(view) {
          if (completionStatus(view.state) === "active") {
            moveCompletionSelection(false)(view);
            return true;
          }
          const head = view.state.selection.main.head;
          if (view.state.doc.lineAt(head).number !== 1) return false;
          const entries = historyRef.current;
          const next = prevHistoryIndex(entries.length, historyCursorRef.current);
          if (next == null) return false;
          const entry = entries[next];
          if (!entry) return false;
          replaceDoc(view, entry.script);
          setHistoryCursor(next);
          return true;
        },
      },
      {
        key: "ArrowDown",
        run(view) {
          if (completionStatus(view.state) === "active") {
            moveCompletionSelection(true)(view);
            return true;
          }
          const head = view.state.selection.main.head;
          if (view.state.doc.lineAt(head).number !== view.state.doc.lines) return false;
          const entries = historyRef.current;
          const next = nextHistoryIndex(entries.length, historyCursorRef.current);
          if (next == null) return false;
          if (next === HISTORY_CLEAR) {
            replaceDoc(view, "");
            setHistoryCursor(-1);
            return true;
          }
          const entry = entries[next];
          if (!entry) return false;
          replaceDoc(view, entry.script);
          setHistoryCursor(next);
          return true;
        },
      },
    ],
    [replaceDoc],
  );

  // The pipeline used for the explain / code handoff. The shell
  // returns the pipeline that produced the most recent aggregate
  // call. For older entries, we recompute it from the script by
  // looking up the entry's outputs. The current run is the
  // authoritative source.
  const lastPipeline = lastResponse?.lastPipeline ?? null;
  const lastCollection = lastResponse?.lastCollection ?? null;

  // Build the code map for the driver-code panel. We only fetch
  // the six snippets when the panel is opened.
  const [codeByLanguage, setCodeByLanguage] = useState<Record<string, string>>({});
  useEffect(() => {
    if (!codeOpen || !lastPipeline || !lastCollection) return;
    if (!profile) return;
    let cancelled = false;
    const all = ["node-js", "python", "java", "c-sharp", "ruby", "shell"];
    Promise.all(
      all.map(async (lang) => {
        try {
          const code = await commands.generatePipelineCode({
            database: activeDb,
            collection: lastCollection,
            pipeline: lastPipeline,
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
      const next: Record<string, string> = {};
      for (const [lang, code] of entries) next[lang] = code;
      setCodeByLanguage(next);
    });
    return () => {
      cancelled = true;
    };
  }, [codeOpen, profile, resolvedUri, lastPipeline, lastCollection, activeDb]);

  // Re-fetch the explain result on demand. The shell returns the
  // pipeline but not the raw explain JSON; we ask the existing
  // `explain_aggregate` IPC to fill in the missing piece.
  async function openExplain() {
    if (!lastPipeline || !lastCollection) return;
    setExplainOpen(true);
    setExplainRaw(null);
    try {
      const result = await commands.explainAggregate(
        connectionId,
        activeDb,
        lastCollection,
        JSON.stringify(lastPipeline),
      );
      setExplainRaw(result);
    } catch (e) {
      toast.push(String(e), "error");
    }
  }

  return (
    <div className="shell-tab">
      <div className="shell-tab__toolbar">
        <span className="shell-tab__db-pill" title="Active database">
          db: <strong>{activeDb}</strong>
        </span>
        <button
          className="btn btn--primary btn--sm"
          onClick={() => void run()}
          disabled={running || script.trim().length === 0}
        >
          {running ? "Running…" : "Run (Enter)"}
        </button>
        {lastPipeline && lastCollection && (
          <>
            <button
              className="btn btn--sm"
              onClick={() => void openExplain()}
              title="Open the visual explain diagram for the last aggregate"
            >
              Explain
            </button>
            <button
              className="btn btn--sm"
              onClick={() => setCodeOpen((v) => !v)}
              title="Open the driver-code popover for the last aggregate"
            >
              {codeOpen ? "Hide code" : "Code"}
            </button>
          </>
        )}
        <span className="shell-tab__hint">
          Enter to run · Shift+Enter for newline · Up arrow for history
        </span>
      </div>

      {codeOpen && lastPipeline && lastCollection && (
        <div className="shell-tab__code">
          <DriverCodePanel
            pipeline={lastPipeline}
            codeByLanguage={codeByLanguage}
          />
        </div>
      )}

      {explainOpen && (
        <div className="shell-tab__explain">
          <div className="shell-tab__explain-head">
            <h3>Explain plan</h3>
            <button
              className="btn btn--sm"
              onClick={() => setExplainOpen(false)}
            >
              Close
            </button>
          </div>
          {explainRaw ? (
            <ExplainTree raw={explainRaw as never} />
          ) : (
            <p className="shell-tab__explain-loading">Loading explain…</p>
          )}
        </div>
      )}

      <div className="shell-tab__body">
        <div className="shell-tab__input-wrap">
          <CodeEditor
            className="shell-tab__editor"
            value={script}
            onChange={setScript}
            context="shell"
            ariaLabel="Mongo shell script"
            completionSource={shellCompletionSource}
            extraKeymap={shellKeymap}
            placeholder={`// Try one of these (one per line — Enter to run):\n// use sample_mflix;\n// db.movies.find({year: 2010}, null, null, 3);\n// var recent = db.movies.countDocuments({year: {"$gte": 2015}});\n// printjson(recent);\n// db.runCommand({ping: 1});`}
            minHeight="120px"
          />
        </div>
        <div className="shell-tab__output" data-testid="shell-output">
          {history.length === 0 ? (
            <div className="empty-state">
              <h2>No commands yet</h2>
              <p>Type a shell command above and press Enter to run it.</p>
            </div>
          ) : (
            history.map((entry, idx) => (
              <HistoryEntryView
                key={`${entry.timestamp}-${idx}`}
                entry={entry}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}

function HistoryEntryView({ entry }: { entry: HistoryEntry }) {
  return (
    <div className="shell-tab__entry">
      <div className="shell-tab__entry-head">
        <span className="shell-tab__entry-script">{entry.script}</span>
        <span className="shell-tab__entry-meta">
          {entry.activeDatabase} · {entry.executionMs} ms
        </span>
      </div>
      {entry.outputs.map((o, i) => (
        <OutputLine key={i} output={o} />
      ))}
    </div>
  );
}

function OutputLine({ output }: { output: ShellOutput }) {
  switch (output.kind) {
    case "text":
      return <div className="shell-tab__line shell-tab__line--text">{output.value}</div>;
    case "error":
      return (
        <div className="shell-tab__line shell-tab__line--error">
          {output.value}
        </div>
      );
    case "json":
      return (
        <pre className="shell-tab__line shell-tab__line--json">
          <code>{JSON.stringify(output.value, null, 2)}</code>
        </pre>
      );
    case "table":
      return <TableView table={output.value} />;
  }
}

function TableView({ table }: { table: ShellTable }) {
  // Reuse ResultsTable in read-only mode for consistency with
  // the rest of the app. Build a list of plain records from the
  // table; ResultsTable derives columns and rows from the union
  // of keys.
  const documents: Array<Record<string, unknown>> = table.rows.map((row) => {
    const obj: Record<string, unknown> = {};
    table.columns.forEach((col, i) => {
      obj[col] = row[i];
    });
    return obj;
  });
  return (
    <div className="shell-tab__table">
      <div className="shell-tab__table-meta">
        {table.rows.length} row{table.rows.length === 1 ? "" : "s"} ·{" "}
        {table.executionMs} ms
      </div>
      <ResultsTable
        documents={documents}
        connectionId=""
        database=""
        collection=""
        editable={false}
      />
    </div>
  );
}
