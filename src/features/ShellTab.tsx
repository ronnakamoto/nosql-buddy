import { useCallback, useEffect, useRef, useState } from "react";
import commands, {
  type AutocompleteResponse,
  type CompletionItem,
  type ProfileSummary,
  type ShellOutput,
  type ShellResponse,
  type ShellTable,
} from "../ipc/commands";
import { ResultsTable } from "../components/ResultsTable";
import { ExplainTree } from "../components/ExplainTree";
import { DriverCodePanel } from "../components/DriverCodePanel";

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
 * IntelliShell tab — production-grade mongo shell REPL.
 *
 * Architecture:
 *  - Multi-line textarea for input. Enter runs the script;
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
  const [error, setError] = useState<string | null>(null);
  const [lastResponse, setLastResponse] = useState<ShellResponse | null>(null);
  const [resolvedUri, setResolvedUri] = useState<string>("");
  const [codeOpen, setCodeOpen] = useState(false);
  const [explainOpen, setExplainOpen] = useState(false);
  const [explainRaw, setExplainRaw] = useState<unknown | null>(null);
  const [historyCursor, setHistoryCursor] = useState<number>(-1);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // --- Autocomplete state ---
  const [autocompleteItems, setAutocompleteItems] = useState<CompletionItem[]>([]);
  const [autocompleteOpen, setAutocompleteOpen] = useState(false);
  const [autocompleteIdx, setAutocompleteIdx] = useState(0);
  const autocompleteRef = useRef<AutocompleteResponse | null>(null);
  const autocompleteTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // The partial token being typed (e.g. "findO" in "db.users.findO").
  // We store it so we know what to replace when the user accepts
  // a suggestion.
  const autocompletePartial = useRef<string>("");

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
      setError(null);
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
        setError(String(e));
      } finally {
        setRunning(false);
      }
    },
    [script, running, connectionId, activeDb, database],
  );

  // --- Autocomplete: debounced fetch on text/cursor change ---
  const fetchAutocomplete = useCallback(
    (text: string, cursorPos: number) => {
      const textBeforeCursor = text.slice(0, cursorPos);
      // Extract the partial token being typed (identifier chars
      // at the end of textBeforeCursor).
      const partialMatch = textBeforeCursor.match(/[A-Za-z0-9_]*$/);
      const partial = partialMatch ? partialMatch[0] : "";
      autocompletePartial.current = partial;

      if (autocompleteTimer.current) {
        clearTimeout(autocompleteTimer.current);
      }
      autocompleteTimer.current = setTimeout(async () => {
        try {
          const resp = await commands.shellAutocomplete({
            connectionId,
            textBeforeCursor,
            activeDatabase: activeDb,
            fallbackDatabase: database,
          });
          autocompleteRef.current = resp;
          if (resp.items.length > 0 && resp.kind.kind !== "none") {
            setAutocompleteItems(resp.items);
            setAutocompleteIdx(0);
            setAutocompleteOpen(true);
          } else {
            setAutocompleteOpen(false);
          }
        } catch {
          setAutocompleteOpen(false);
        }
      }, 150);
    },
    [connectionId, activeDb, database],
  );

  // Accept a completion: replace the partial token with the
  // chosen label and close the dropdown.
  const acceptCompletion = useCallback(
    (item: CompletionItem) => {
      const ta = textareaRef.current;
      if (!ta) return;
      const pos = ta.selectionStart ?? script.length;
      const textBefore = script.slice(0, pos);
      const textAfter = script.slice(pos);
      // Remove the partial token from the end of textBefore.
      const partial = autocompletePartial.current;
      const cutPos = textBefore.length - partial.length;
      const next = textBefore.slice(0, cutPos) + item.label + textAfter;
      setScript(next);
      setAutocompleteOpen(false);
      // Move cursor to just after the inserted label.
      const newPos = cutPos + item.label.length;
      requestAnimationFrame(() => {
        ta.focus();
        ta.setSelectionRange(newPos, newPos);
      });
    },
    [script],
  );

  const closeAutocomplete = useCallback(() => {
    setAutocompleteOpen(false);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      // --- Autocomplete navigation (takes priority when open) ---
      if (autocompleteOpen && autocompleteItems.length > 0) {
        if (e.key === "Tab" || (e.key === "Enter" && !e.shiftKey)) {
          e.preventDefault();
          const item = autocompleteItems[autocompleteIdx];
          if (item) acceptCompletion(item);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          closeAutocomplete();
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setAutocompleteIdx((i) =>
            i <= 0 ? autocompleteItems.length - 1 : i - 1,
          );
          return;
        }
        if (e.key === "ArrowDown") {
          e.preventDefault();
          setAutocompleteIdx((i) =>
            i >= autocompleteItems.length - 1 ? 0 : i + 1,
          );
          return;
        }
      }

      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        void run();
        return;
      }
      if (e.key === "ArrowUp" && history.length > 0 && !autocompleteOpen) {
        // Only navigate history when the cursor is on the first
        // line — otherwise the user is just moving within the
        // textarea.
        const ta = e.currentTarget;
        if (ta.selectionStart === 0 || ta.value.indexOf("\n", ta.selectionStart) === -1) {
          e.preventDefault();
          const next = historyCursor < 0
            ? history.length - 1
            : Math.max(0, historyCursor - 1);
          const entry = history[next];
          if (entry) {
            setScript(entry.script);
            setHistoryCursor(next);
          }
        }
      }
      if (e.key === "ArrowDown" && historyCursor >= 0 && !autocompleteOpen) {
        e.preventDefault();
        const next = historyCursor + 1;
        if (next >= history.length) {
          setScript("");
          setHistoryCursor(-1);
        } else {
          const entry = history[next];
          if (entry) {
            setScript(entry.script);
            setHistoryCursor(next);
          }
        }
      }
    },
    [
      history,
      historyCursor,
      run,
      autocompleteOpen,
      autocompleteItems,
      autocompleteIdx,
      acceptCompletion,
      closeAutocomplete,
    ],
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
      setError(String(e));
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

      {error && (
        <div className="toast toast--error" style={{ margin: "0 var(--space-3) var(--space-2)" }}>
          {error}
        </div>
      )}

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
          <textarea
            ref={textareaRef}
            className="shell-tab__input"
            value={script}
            onChange={(e) => {
              setScript(e.target.value);
              fetchAutocomplete(e.target.value, e.target.selectionStart ?? e.target.value.length);
            }}
            onKeyUp={(e) => {
              // Re-fetch on cursor movement keys that don't
              // change the text (Left/Right/Home/End). Skip
              // navigation keys when the dropdown is open —
              // otherwise the async response resets the
              // selected index and fights the user's
              // ArrowUp/Down navigation.
              if (autocompleteOpen && ["ArrowUp", "ArrowDown", "Tab", "Enter", "Escape"].includes(e.key)) {
                return;
              }
              const ta = e.currentTarget;
              fetchAutocomplete(ta.value, ta.selectionStart ?? ta.value.length);
            }}
            onKeyDown={handleKeyDown}
            onBlur={() => {
              // Delay close so a click on a suggestion item
              // registers first.
              setTimeout(() => closeAutocomplete(), 150);
            }}
            spellCheck={false}
            placeholder={`// Try one of these (one per line — Enter to run):\n// use sample_mflix;\n// db.movies.find({year: 2010}, null, null, 3);\n// var recent = db.movies.countDocuments({year: {"$gte": 2015}});\n// printjson(recent);\n// db.runCommand({ping: 1});`}
            rows={6}
          />
          {autocompleteOpen && autocompleteItems.length > 0 && (
            <AutocompleteDropdown
              items={autocompleteItems}
              selectedIdx={autocompleteIdx}
              onSelect={acceptCompletion}
            />
          )}
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

/**
 * Autocomplete dropdown overlay. Renders below the textarea
 * input area. Keyboard navigation (Up/Down/Tab/Enter/Esc) is
 * handled by the parent's `handleKeyDown`; this component only
 * handles mouse clicks.
 */
function AutocompleteDropdown({
  items,
  selectedIdx,
  onSelect,
}: {
  items: CompletionItem[];
  selectedIdx: number;
  onSelect: (item: CompletionItem) => void;
}) {
  return (
    <div className="shell-autocomplete" data-testid="shell-autocomplete">
      {items.map((item, i) => (
        <button
          key={`${item.label}-${i}`}
          className={
            "shell-autocomplete__item" +
            (i === selectedIdx ? " shell-autocomplete__item--selected" : "")
          }
          onMouseDown={(e) => {
            // mousedown fires before the textarea's onBlur,
            // so the click registers before the dropdown closes.
            e.preventDefault();
            onSelect(item);
          }}
          type="button"
        >
          <span className="shell-autocomplete__label">{item.label}</span>
          {item.detail && (
            <span className="shell-autocomplete__detail">{item.detail}</span>
          )}
        </button>
      ))}
    </div>
  );
}
