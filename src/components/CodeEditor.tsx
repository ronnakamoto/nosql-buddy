import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView, keymap, placeholder as placeholderExt } from "@codemirror/view";
import { json } from "@codemirror/lang-json";
import { sql } from "@codemirror/lang-sql";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import {
  autocompletion,
  completionKeymap,
  startCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult as CMCompletionResult,
} from "@codemirror/autocomplete";
import { getSuggestions, type EditorContext, type Suggestion } from "../lib/autocomplete";

export interface CodeEditorSchema {
  topLevelFields: string[];
  allPaths: string[];
  childrenByPrefix: Map<string, string[]>;
}

export interface CodeEditorProps {
  value: string;
  onChange: (value: string) => void;
  /** Drives both syntax highlighting (sql vs json) and autocomplete content. */
  context: EditorContext;
  /** Schema for field-name suggestions. */
  schema?: CodeEditorSchema;
  className?: string;
  placeholder?: string;
  ariaLabel?: string;
  /** Fill the parent's height (grid cells). Defaults to true. */
  fillHeight?: boolean;
  /** Minimum editor height. Defaults to "140px" when filling. */
  minHeight?: string;
  /** Cap height when not filling (auto-growing editors). */
  maxHeight?: string;
  /** Tighter padding / smaller type for inline editors (e.g. agg stages). */
  compact?: boolean;
}

function cmType(kind: Suggestion["kind"]): Completion["type"] {
  switch (kind) {
    case "field":
      return "property";
    case "operator":
      return "keyword";
    case "keyword":
      return "keyword";
    case "value":
      return "text";
    case "snippet":
      return "text";
  }
}

/** Bridge the existing suggestion engine into a CodeMirror completion source. */
function makeCompletionSource(context: EditorContext, schema?: CodeEditorSchema) {
  return (cc: CompletionContext): CMCompletionResult | null => {
    const text = cc.state.doc.toString();
    const res = getSuggestions(text, cc.pos, context, schema);
    if (res.suggestions.length === 0) return null;
    return {
      from: res.replaceStart,
      to: res.replaceEnd,
      // The engine already filters by the typed prefix; don't let
      // CodeMirror re-filter against the (quoted) insert text.
      filter: false,
      options: res.suggestions.map((s) => ({
        label: s.label,
        detail: s.detail,
        type: cmType(s.kind),
        apply: s.insertText,
      })),
    };
  };
}

// Restrained theme mapped onto the app's OKLCH design tokens so the
// editor adapts to light/dark automatically (DESIGN.md).
const editorTheme = EditorView.theme({
  "&": { backgroundColor: "var(--bg)", color: "var(--ink)", fontSize: "var(--font-size-sm)" },
  "&.cm-focused": { outline: "none" },
  ".cm-content": {
    fontFamily: "var(--font-mono)",
    padding: "var(--space-3) var(--space-4)",
    caretColor: "var(--accent-600)",
  },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.5" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent-600)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--selection, var(--accent-100))",
  },
  ".cm-placeholder": { color: "var(--ink-faint)" },
  ".cm-tooltip": {
    backgroundColor: "var(--surface-2)",
    border: "1px solid var(--border)",
    borderRadius: "var(--radius-md)",
    boxShadow: "var(--shadow-md)",
    color: "var(--ink)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--font-mono)",
    fontSize: "var(--font-size-xs)",
    maxHeight: "16em",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    backgroundColor: "var(--accent-100)",
    color: "var(--accent-700)",
  },
  ".cm-completionDetail": { color: "var(--ink-muted)", fontStyle: "normal", marginLeft: "1em" },
  ".cm-completionIcon": { paddingRight: "0.6em", opacity: "0.7" },
});

const compactTheme = EditorView.theme({
  ".cm-content": { padding: "var(--space-2)", fontSize: "12px" },
  ".cm-scroller": { fontSize: "12px" },
});

const highlightStyle = HighlightStyle.define([
  { tag: t.keyword, color: "var(--accent-600)" },
  { tag: t.operator, color: "var(--accent-600)" },
  { tag: [t.string, t.special(t.string)], color: "var(--success-500)" },
  { tag: [t.number, t.bool, t.null], color: "var(--info-500)" },
  { tag: t.propertyName, color: "var(--ink)" },
  { tag: [t.punctuation, t.separator, t.bracket], color: "var(--ink-faint)" },
  { tag: t.comment, color: "var(--ink-muted)", fontStyle: "italic" },
  { tag: t.invalid, color: "var(--danger-500)" },
]);

// Open the completion popup as the user types (operators start with "$",
// keys open after "{" / "," / quote), deferred to avoid dispatching
// inside an update.
const autoTrigger = EditorView.updateListener.of((u) => {
  if (!u.docChanged || u.view.composing) return;
  const typed = u.transactions.some((tr) => tr.isUserEvent("input.type"));
  if (typed) queueMicrotask(() => startCompletion(u.view));
});

export function CodeEditor({
  value,
  onChange,
  context,
  schema,
  className,
  placeholder,
  ariaLabel,
  fillHeight = true,
  minHeight,
  maxHeight,
  compact = false,
}: CodeEditorProps) {
  const extensions = useMemo(() => {
    const exts = [
      context === "sql" ? sql() : json(),
      keymap.of(completionKeymap),
      autocompletion({
        override: [makeCompletionSource(context, schema)],
        activateOnTyping: false,
        defaultKeymap: false,
      }),
      autoTrigger,
      editorTheme,
      syntaxHighlighting(highlightStyle),
      EditorView.lineWrapping,
    ];
    if (compact) exts.push(compactTheme);
    if (placeholder) exts.push(placeholderExt(placeholder));
    if (ariaLabel) exts.push(EditorView.contentAttributes.of({ "aria-label": ariaLabel }));
    return exts;
  }, [context, schema, placeholder, ariaLabel, compact]);

  return (
    <CodeMirror
      value={value}
      onChange={onChange}
      extensions={extensions}
      theme={editorTheme}
      className={className}
      height={fillHeight ? "100%" : undefined}
      minHeight={minHeight}
      maxHeight={maxHeight}
      basicSetup={{
        lineNumbers: false,
        foldGutter: false,
        highlightActiveLine: false,
        highlightActiveLineGutter: false,
        autocompletion: false,
        bracketMatching: true,
        closeBrackets: true,
        indentOnInput: true,
      }}
    />
  );
}
