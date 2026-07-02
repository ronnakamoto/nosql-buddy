import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import Prism from "prismjs";
import {
  getSuggestions,
  type Suggestion,
  type EditorContext,
  type CompletionResult,
} from "../lib/autocomplete";
import { getCaretCoordinates } from "./AutocompleteTextarea";

export interface HighlightedTextareaProps {
  value: string;
  onChange: (value: string) => void;
  /** Prism grammar name, e.g. "sql", "json", "javascript". The
   *  corresponding prism-<lang> component must already be imported by
   *  the host. Falls back to plain escaped text when no grammar is
   *  registered. */
  language: string;
  className?: string;
  placeholder?: string;
  spellCheck?: boolean;
  ariaLabel?: string;
  /** When set, enables autocomplete with the given context. */
  autocompleteContext?: EditorContext;
  /** Schema for field-name suggestions. */
  schema?: {
    topLevelFields: string[];
    allPaths: string[];
    childrenByPrefix: Map<string, string[]>;
  };
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/**
 * Editable code editor built from a transparent `<textarea>` layered
 * over a Prism-highlighted `<pre>`. The two share identical typography,
 * padding, and wrapping so the highlighted backdrop aligns exactly with
 * the caret-driven textarea text. Scroll is mirrored from the textarea
 * onto the backdrop so long content stays in sync.
 *
 * Used by the SQL tab's input editor so hand-written SQL gets the same
 * restrained token coloring as the read-only driver-code panel.
 *
 * Optionally supports autocomplete when `autocompleteContext` is provided.
 */
export function HighlightedTextarea({
  value,
  onChange,
  language,
  className,
  placeholder,
  spellCheck,
  ariaLabel,
  autocompleteContext,
  schema,
}: HighlightedTextareaProps) {
  const taRef = useRef<HTMLTextAreaElement>(null);
  const preRef = useRef<HTMLPreElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [result, setResult] = useState<CompletionResult | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  const valueRef = useRef(value);
  valueRef.current = value;

  const compute = useCallback(() => {
    const ta = taRef.current;
    if (!ta || !autocompleteContext) return;
    const offset = ta.selectionStart ?? 0;
    const res = getSuggestions(valueRef.current, offset, autocompleteContext, schema);
    if (res.suggestions.length > 0) {
      setResult(res);
      setSelectedIdx(0);
      setPos(getCaretCoordinates(ta, offset));
    } else {
      setResult(null);
      setPos(null);
    }
  }, [autocompleteContext, schema]);

  useEffect(() => {
    compute();
  }, [value, compute]);

  const applySuggestion = useCallback(
    (suggestion: Suggestion) => {
      const ta = taRef.current;
      if (!ta || !result) return;
      const start = result.replaceStart;
      const end = result.replaceEnd;
      const before = value.slice(0, start);
      const after = value.slice(end);
      const next = before + suggestion.insertText + after;
      onChange(next);
      requestAnimationFrame(() => {
        if (!ta) return;
        const cursorPos = start + suggestion.insertText.length;
        ta.setSelectionRange(cursorPos, cursorPos);
        ta.focus();
      });
      setResult(null);
    },
    [value, onChange, result],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!result) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIdx((i) => (i + 1) % result.suggestions.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIdx((i) =>
          i === 0 ? result.suggestions.length - 1 : i - 1,
        );
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        applySuggestion(result.suggestions[selectedIdx]);
      } else if (e.key === "Escape") {
        setResult(null);
      }
    },
    [result, selectedIdx, applySuggestion],
  );

  const html = useMemo(() => {
    const grammar = Prism.languages[language];
    const body = grammar
      ? Prism.highlight(value, grammar, language)
      : escapeHtml(value);
    return body + "\n";
  }, [value, language]);

  useEffect(() => {
    const ta = taRef.current;
    const pre = preRef.current;
    if (!ta || !pre) return;
    const sync = () => {
      pre.scrollTop = ta.scrollTop;
      pre.scrollLeft = ta.scrollLeft;
    };
    sync();
    ta.addEventListener("scroll", sync, { passive: true });
    return () => ta.removeEventListener("scroll", sync);
  }, []);

  // Close popup on outside click
  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      const ta = taRef.current;
      const list = listRef.current;
      if (!ta || !list) return;
      if (e.target !== ta && !list.contains(e.target as Node)) {
        setResult(null);
      }
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  return (
    <div className={`hl-editor ${className ?? ""}`} style={{ position: "relative" }}>
      <pre ref={preRef} className="hl-editor__back" aria-hidden="true">
        <code
          className={`language-${language}`}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </pre>
      <textarea
        ref={taRef}
        className="hl-editor__front"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        onKeyUp={compute}
        onClick={compute}
        onSelect={compute}
        spellCheck={spellCheck}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
      {result && pos && (
        <div
          ref={listRef}
          className="ac-editor__popup"
          style={{
            position: "absolute",
            left: pos.left,
            top: pos.top + 20,
            zIndex: 1000,
            minWidth: 200,
            maxHeight: 78,
            overflowY: "auto",
            background: "var(--surface-2)",
            border: "1px solid var(--border)",
            borderRadius: 0,
            boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
            fontFamily: "var(--font-mono)",
            fontSize: 12,
          }}
        >
          {result.suggestions.map((s, i) => (
            <div
              key={s.label + i}
              className={`ac-editor__item ${i === selectedIdx ? "is-selected" : ""}`}
              onMouseEnter={() => setSelectedIdx(i)}
              onMouseDown={(e) => {
                e.preventDefault();
                applySuggestion(s);
              }}
              style={{
                padding: "4px 8px",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                gap: 6,
                background: i === selectedIdx ? "var(--accent-muted)" : "transparent",
                color: i === selectedIdx ? "var(--accent)" : "var(--ink)",
              }}
            >
              <span
                style={{
                  width: 14,
                  height: 14,
                  borderRadius: 0,
                  display: "inline-flex",
                  alignItems: "center",
                  justifyContent: "center",
                  fontSize: 9,
                  fontWeight: 600,
                  background: kindColor(s.kind),
                  color: "#fff",
                  flexShrink: 0,
                }}
              >
                {kindAbbrev(s.kind)}
              </span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
                {s.label}
              </span>
              {s.detail && (
                <span
                  style={{
                    color: "var(--ink-muted)",
                    fontSize: 10,
                    whiteSpace: "nowrap",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    maxWidth: 120,
                  }}
                >
                  {s.detail}
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function kindAbbrev(kind: Suggestion["kind"]): string {
  switch (kind) {
    case "field":
      return "F";
    case "operator":
      return "O";
    case "keyword":
      return "K";
    case "value":
      return "V";
    case "snippet":
      return "S";
  }
}

function kindColor(kind: Suggestion["kind"]): string {
  switch (kind) {
    case "field":
      return "#0ea5e9";
    case "operator":
      return "#a855f7";
    case "keyword":
      return "#f59e0b";
    case "value":
      return "#22c55e";
    case "snippet":
      return "#ec4899";
  }
}
