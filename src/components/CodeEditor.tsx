import { useMemo } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { EditorView, keymap, placeholder as placeholderExt, tooltips, type KeyBinding } from "@codemirror/view";
import { Prec, type Extension } from "@codemirror/state";
import { json } from "@codemirror/lang-json";
import { sql } from "@codemirror/lang-sql";
import { javascript } from "@codemirror/lang-javascript";
import { linter, lintGutter, type Diagnostic } from "@codemirror/lint";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import {
  autocompletion,
  completionKeymap,
  startCompletion,
  type Completion,
  type CompletionContext,
  type CompletionResult as CMCompletionResult,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { getSuggestions, type EditorContext, type Suggestion } from "../lib/autocomplete";

import { lintJsonText, autoFixJson } from "../lib/jsonLint";

const isMac = navigator.platform.startsWith("Mac") || navigator.platform === "iPhone";
const FIX_SHORTCUT = isMac ? "\u2318\u21E7F" : "Ctrl+Shift+F";

/** Apply autoFixJson to the current editor content. */
function applyAutoFix(view: EditorView): boolean {
  const text = view.state.doc.toString();
  const fixed = autoFixJson(text);
  if (fixed === null) return false;
  view.dispatch({
    changes: { from: 0, to: text.length, insert: fixed },
  });
  return true;
}

/** Build a custom DOM node for a lint action button with an inset
 *  keyboard-shortcut badge, similar to Zed's button style. */
function renderFixAction(view: EditorView): Node {
  const btn = document.createElement("button");
  btn.className = "cm-fix-btn";
  btn.type = "button";

  const label = document.createElement("span");
  label.textContent = "Fix all";

  const kbd = document.createElement("kbd");
  kbd.className = "cm-fix-btn__kbd";
  kbd.textContent = FIX_SHORTCUT;

  btn.append(label, kbd);
  btn.addEventListener("click", () => applyAutoFix(view));
  return btn;
}

/** Adapter: CodeMirror linter callback → pure lintJsonText function.
 *  Attaches a custom-rendered "Fix all" button with kbd badge to the
 *  first diagnostic when autofix is available. */
function mongoJsonLinter(view: EditorView): Diagnostic[] {
  const text = view.state.doc.toString();
  const raw = lintJsonText(text);
  const fixable = autoFixJson(text) !== null;
  return raw.map((d, i): Diagnostic => {
    const diag: Diagnostic = {
      from: d.from,
      to: d.to,
      severity: d.severity,
      message: d.message,
    };
    if (fixable && i === 0) {
      diag.renderMessage = (v: EditorView) => {
        const wrapper = document.createElement("span");
        wrapper.className = "cm-fix-wrapper";
        const msg = document.createElement("span");
        msg.textContent = d.message;
        wrapper.append(msg, renderFixAction(v));
        return wrapper;
      };
    }
    return diag;
  });
}

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
  /** Optional completion source for non-JSON/SQL modes such as Shell. */
  completionSource?: CompletionSource;
  /** Optional high-priority keymap layered before the default editor bindings. */
  extraKeymap?: KeyBinding[];
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
  ".cm-lint-marker-error": { content: "'!'", color: "var(--danger-500)" },
  ".cm-lintRange-error": { backgroundImage: "none", textDecoration: "wavy underline var(--danger-500)" },
  ".cm-gutter-lint": { width: "1.2em" },
});

const compactTheme = EditorView.theme({
  ".cm-content": { padding: "var(--space-2)", fontSize: "12px" },
  ".cm-scroller": { fontSize: "12px" },
});

const highlightStyle = HighlightStyle.define([
  // Shared across JSON, SQL, and JS
  { tag: t.keyword, color: "var(--accent-600)" },
  { tag: t.operator, color: "var(--accent-600)" },
  { tag: [t.string, t.special(t.string)], color: "var(--success-500)" },
  { tag: [t.number, t.bool, t.null], color: "var(--info-500)" },
  { tag: [t.propertyName], color: "var(--ink)" },
  { tag: [t.punctuation, t.separator, t.bracket], color: "var(--ink-faint)" },
  { tag: t.comment, color: "var(--ink-muted)", fontStyle: "italic" },
  { tag: t.invalid, color: "var(--danger-500)" },
  // JavaScript-specific (lang-javascript)
  { tag: [t.variableName, t.definition(t.variableName)], color: "var(--ink)" },
  { tag: [t.function(t.variableName), t.function(t.propertyName)], color: "var(--accent-600)" },
  { tag: [t.typeName, t.definition(t.typeName)], color: "var(--info-500)" },
  { tag: t.tagName, color: "var(--accent-600)" },
  { tag: t.regexp, color: "var(--success-500)" },
  { tag: [t.annotation], color: "var(--ink-muted)" },
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
  completionSource,
  extraKeymap,
}: CodeEditorProps) {
  const extensions = useMemo(() => {
    const language =
      context === "sql" ? sql() : context === "shell" ? javascript() : json();
    const exts: Extension[] = [
      language,
      keymap.of(completionKeymap),
      autocompletion({
        override: [completionSource ?? makeCompletionSource(context, schema)],
        activateOnTyping: false,
        defaultKeymap: false,
      }),
      autoTrigger,
      editorTheme,
      syntaxHighlighting(highlightStyle),
      EditorView.lineWrapping,
      tooltips({ parent: document.body }),
    ];
    if (context !== "sql" && context !== "shell") {
      exts.push(
        linter(mongoJsonLinter, { delay: 300 }),
        lintGutter(),
        keymap.of([{ key: "Mod-Shift-f", run: applyAutoFix }]),
      );
    }
    if (compact) exts.push(compactTheme);
    if (placeholder) exts.push(placeholderExt(placeholder));
    if (ariaLabel) exts.push(EditorView.contentAttributes.of({ "aria-label": ariaLabel }));
    if (extraKeymap && extraKeymap.length > 0) {
      exts.push(Prec.highest(keymap.of(extraKeymap)));
    }
    return exts;
  }, [context, schema, placeholder, ariaLabel, compact, completionSource, extraKeymap]);

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
