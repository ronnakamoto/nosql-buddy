import { useEffect, useMemo, useState } from "react";
import { type Language, type SqlLanguage, languageLabel } from "./driverCodeTypes";

export interface DriverCodePanelProps {
  /** Current pipeline (post-Run). The panel renders nothing while null. */
  pipeline: unknown[] | null;
  /** Pre-computed code per language. The host owns the IPC call so
   *  the panel itself stays synchronous and easy to test. Missing
   *  entries fall back to a small built-in JS snippet. */
  codeByLanguage?: Partial<Record<Language, string>>;
  /** Title shown at the top of the panel. */
  title?: string;
  /** Initial language selection. */
  initialLanguage?: Language;
}

const LANGUAGES: Language[] = [
  "node-js",
  "python",
  "java",
  "c-sharp",
  "ruby",
  "shell",
];

/**
 * Language dropdown + generated driver code + Copy-to-clipboard.
 * Used by the AggregationEditor's toolbar and Explain panel.
 *
 * The host owns the IPC call: it passes a `codeByLanguage` map that
 * already contains the generated snippet for each language (the
 * editor fetches all six at once when the user opens the panel).
 * The panel itself is purely presentational.
 */
export function DriverCodePanel({
  pipeline,
  codeByLanguage,
  title = "Driver code",
  initialLanguage = "node-js",
}: DriverCodePanelProps) {
  const [language, setLanguage] = useState<Language>(initialLanguage);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  const code = useMemo(() => {
    if (!pipeline || pipeline.length === 0) return "";
    const fromHost = codeByLanguage?.[language];
    if (fromHost) return fromHost;
    return fallbackJs(pipeline);
  }, [codeByLanguage, language, pipeline]);

  useEffect(() => {
    if (copyState === "idle") return;
    const t = window.setTimeout(() => setCopyState("idle"), 1500);
    return () => window.clearTimeout(t);
  }, [copyState]);

  async function handleCopy() {
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  if (!pipeline || pipeline.length === 0) {
    return (
      <div className="driver-code-panel driver-code-panel--empty">
        <div className="driver-code-panel__title">{title}</div>
        <p className="driver-code-panel__empty-msg">
          Run the pipeline first to generate driver code.
        </p>
      </div>
    );
  }

  return (
    <div className="driver-code-panel">
      <div className="driver-code-panel__head">
        <span className="driver-code-panel__title">{title}</span>
        <select
          className="input input--sm"
          value={language}
          onChange={(e) => setLanguage(e.target.value as Language)}
          aria-label="Driver language"
        >
          {LANGUAGES.map((l) => (
            <option key={l} value={l}>
              {languageLabel(l)}
            </option>
          ))}
        </select>
        <button
          className="btn btn--sm driver-code-panel__copy"
          onClick={() => void handleCopy()}
          disabled={!code}
          title="Copy code to clipboard"
        >
          {copyState === "copied"
            ? "Copied!"
            : copyState === "failed"
              ? "Copy failed"
              : "Copy"}
        </button>
      </div>
      <pre className="driver-code-panel__code">
        <code>{code}</code>
      </pre>
    </div>
  );
}

/**
 * Minimal fallback that emits JS code using the localhost URI.
 * Used only when the host does not provide a `generate` prop
 * (e.g. the explain panel before the IPC contract for the new
 * `generate_pipeline_code` command is wired). The full generator
 * lives in `features/driverCodeIpc.ts`.
 */
function fallbackJs(pipeline: unknown[]): string {
  const pipelineLit = JSON.stringify(pipeline, null, 2);
  return [
    `import { MongoClient } from "mongodb";`,
    ``,
    `const client = new MongoClient("mongodb://127.0.0.1:27017");`,
    `await client.connect();`,
    `const cursor = client`,
    `  .db(/* database */)`,
    `  .collection(/* collection */)`,
    `  .aggregate(${pipelineLit});`,
    `const docs = await cursor.toArray();`,
    `console.log(docs);`,
  ].join("\n");
}

// Re-export so consumers don't have to import from the types file.
export type { Language, SqlLanguage };
