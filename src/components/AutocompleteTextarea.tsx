import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getSuggestions,
  type Suggestion,
  type EditorContext,
  type CompletionResult,
} from "../lib/autocomplete";

export interface AutocompleteTextareaProps {
  value: string;
  onChange: (value: string) => void;
  context: EditorContext;
  /** Schema fields for field-name suggestions. */
  schema?: {
    topLevelFields: string[];
    allPaths: string[];
    childrenByPrefix: Map<string, string[]>;
  };
  className?: string;
  placeholder?: string;
  spellCheck?: boolean;
  ariaLabel?: string;
  rows?: number;
}

/** Small inline suggestion popup for textarea-based editors. */
export function AutocompleteTextarea({
  value,
  onChange,
  context,
  schema,
  className,
  placeholder,
  spellCheck,
  ariaLabel,
  rows,
}: AutocompleteTextareaProps) {
  const taRef = useRef<HTMLTextAreaElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [result, setResult] = useState<CompletionResult | null>(null);
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);

  // Keep a ref to the latest value so compute can use it without
  // depending on the render cycle – this avoids stale-closure issues
  // when onSelect fires before React has re-rendered with the new prop.
  const valueRef = useRef(value);
  valueRef.current = value;

  const compute = useCallback(() => {
    const ta = taRef.current;
    if (!ta) return;
    const offset = ta.selectionStart ?? 0;
    const res = getSuggestions(valueRef.current, offset, context, schema);
    if (res.suggestions.length > 0) {
      setResult(res);
      setSelectedIdx(0);
      setPos(getCaretCoordinates(ta, offset));
    } else {
      setResult(null);
      setPos(null);
    }
  }, [context, schema]);

  // Recompute whenever the prop value changes (i.e. parent updated state)
  useEffect(() => {
    compute();
  }, [value, compute]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (!result) return;
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIdx((i) => (i + 1) % result.suggestions.length);
        scrollSelectedIntoView();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIdx((i) =>
          i === 0 ? result.suggestions.length - 1 : i - 1,
        );
        scrollSelectedIntoView();
      } else if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        applySuggestion(result.suggestions[selectedIdx]);
      } else if (e.key === "Escape") {
        setResult(null);
        setPos(null);
      }
    },
    [result, selectedIdx],
  );

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
      // Move cursor to end of inserted text
      requestAnimationFrame(() => {
        if (!ta) return;
        const cursorPos = start + suggestion.insertText.length;
        ta.setSelectionRange(cursorPos, cursorPos);
        ta.focus();
      });
      setResult(null);
      setPos(null);
    },
    [value, onChange, result],
  );

  const scrollSelectedIntoView = useCallback(() => {
    const list = listRef.current;
    if (!list) return;
    const item = list.children[selectedIdx] as HTMLElement | undefined;
    if (item) {
      item.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIdx]);

  // Close popup on outside click
  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      const ta = taRef.current;
      const list = listRef.current;
      if (!ta || !list) return;
      if (
        e.target !== ta &&
        !list.contains(e.target as Node)
      ) {
        setResult(null);
        setPos(null);
      }
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, []);

  // When result changes, ensure selectedIdx is in bounds
  useEffect(() => {
    if (result && selectedIdx >= result.suggestions.length) {
      setSelectedIdx(0);
    }
  }, [result, selectedIdx]);

  const suggestions = useMemo(() => result?.suggestions ?? [], [result]);

  return (
    <div className="ac-editor" style={{ position: "relative" }}>
      <textarea
        ref={taRef}
        className={className}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        onKeyUp={compute}
        onClick={compute}
        onSelect={compute}
        spellCheck={spellCheck}
        placeholder={placeholder}
        aria-label={ariaLabel}
        rows={rows}
        style={{
          fontFamily: "var(--font-mono)",
          fontSize: "var(--font-size-sm)",
          width: "100%",
          height: "100%",
          minHeight: 140,
          resize: "none",
          border: "none",
          outline: "none",
          background: "var(--bg)",
          color: "var(--ink)",
          padding: "var(--space-3) var(--space-4)",
          lineHeight: 1.5,
          tabSize: 2,
        }}
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
          {suggestions.map((s, i) => (
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

// ─── Caret coordinates ─────────────────────────────────────────────────

/**
 * Returns the visual {left, top} coordinates of the caret inside a
 * textarea. Uses a mirror element technique for accuracy.
 */
export function getCaretCoordinates(
  textarea: HTMLTextAreaElement,
  offset: number,
): { left: number; top: number } {
  const style = window.getComputedStyle(textarea);

  // Create a mirror div
  const div = document.createElement("div");
  document.body.appendChild(div);

  const copyStyle = (
    "fontFamily fontSize fontWeight fontStyle lineHeight letterSpacing textTransform " +
    "wordSpacing textIndent paddingTop paddingRight paddingBottom paddingLeft " +
    "borderWidth boxSizing width height whiteSpace overflowWrap"
  ).split(" ");

  copyStyle.forEach((prop) => {
    // @ts-expect-error style indexing
    div.style[prop] = style[prop];
  });

  div.style.position = "absolute";
  div.style.visibility = "hidden";
  div.style.whiteSpace = "pre-wrap";
  div.style.overflow = "hidden";
  div.style.wordWrap = "break-word";

  const textBefore = textarea.value.slice(0, offset);
  const textAfter = textarea.value.slice(offset);

  const spanBefore = document.createElement("span");
  spanBefore.textContent = textBefore;

  const cursorSpan = document.createElement("span");
  cursorSpan.textContent = "|";

  const spanAfter = document.createElement("span");
  spanAfter.textContent = textAfter;

  div.appendChild(spanBefore);
  div.appendChild(cursorSpan);
  div.appendChild(spanAfter);

  const rect = cursorSpan.getBoundingClientRect();
  const taRect = textarea.getBoundingClientRect();

  document.body.removeChild(div);

  return {
    left: rect.left - taRect.left + textarea.scrollLeft,
    top: rect.top - taRect.top + textarea.scrollTop,
  };
}
