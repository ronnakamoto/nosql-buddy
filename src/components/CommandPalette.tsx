import { useEffect, useMemo, useRef, useState } from "react";

export interface CommandPaletteItem {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string;
  run: () => void;
}

export function CommandPalette({
  open,
  onClose,
  items,
}: {
  open: boolean;
  onClose: () => void;
  items: CommandPaletteItem[];
}) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter(
      (it) =>
        it.label.toLowerCase().includes(q) ||
        (it.hint ?? "").toLowerCase().includes(q),
    );
  }, [query, items]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setActive((i) => Math.min(i + 1, filtered.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActive((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = filtered[active];
        if (item) {
          item.run();
          onClose();
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, filtered, active, onClose]);

  if (!open) return null;

  return (
    <div className="command-palette" role="listbox" aria-label="Command palette">
      <input
        ref={inputRef}
        className="command-palette__input"
        type="text"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
        }}
        placeholder="Search commands, collections, saved queries…"
      />
      <div className="command-palette__list">
        {filtered.length === 0 && (
          <div className="command-palette__item" style={{ color: "var(--ink-faint)" }}>
            No matches
          </div>
        )}
        {filtered.map((it, i) => (
          <div
            key={it.id}
            className={`command-palette__item ${i === active ? "is-active" : ""}`}
            onMouseEnter={() => setActive(i)}
            onClick={() => {
              it.run();
              onClose();
            }}
            role="option"
            aria-selected={i === active}
          >
            <span>{it.label}</span>
            {it.hint && (
              <span style={{ color: "var(--ink-faint)", fontSize: 12 }}>{it.hint}</span>
            )}
            {it.shortcut && <span className="kbd">{it.shortcut}</span>}
          </div>
        ))}
      </div>
    </div>
  );
}
