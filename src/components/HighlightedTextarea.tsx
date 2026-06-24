import { useEffect, useMemo, useRef } from "react";
import Prism from "prismjs";

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
 */
export function HighlightedTextarea({
  value,
  onChange,
  language,
  className,
  placeholder,
  spellCheck,
  ariaLabel,
}: HighlightedTextareaProps) {
  const taRef = useRef<HTMLTextAreaElement>(null);
  const preRef = useRef<HTMLPreElement>(null);

  const html = useMemo(() => {
    const grammar = Prism.languages[language];
    // Trailing newline keeps the final line visible inside the scroll
    // container when the user's input ends without one.
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

  return (
    <div className={`hl-editor ${className ?? ""}`}>
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
        spellCheck={spellCheck}
        placeholder={placeholder}
        aria-label={ariaLabel}
      />
    </div>
  );
}
