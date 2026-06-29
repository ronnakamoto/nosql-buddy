import { useEffect, useState } from "react";
import { 
  shortcuts, 
  shortcutsByCategory, 
  type ShortcutCategory,
  type Shortcut 
} from "../lib/shortcuts";
import { Search, X } from "lucide-react";

interface ShortcutsMapProps {
  open: boolean;
  onClose: () => void;
}

const categoryLabels: Record<ShortcutCategory, string> = {
  global: "Global Shortcuts",
  navigation: "Navigation",
  editing: "Text Editing",
  modal: "Modal Controls",
  query: "Query Editor",
  shell: "MongoDB Shell",
  "data-model": "Data Model",
};

const categoryDescriptions: Record<ShortcutCategory, string> = {
  global: "Shortcuts that work anywhere in the app",
  navigation: "Navigate through lists, menus, and UI elements",
  editing: "Text editing and autocomplete controls",
  modal: "Control dialogs and popups",
  query: "Query tab specific shortcuts",
  shell: "MongoDB shell interaction",
  "data-model": "Data model diagram controls",
};

export function ShortcutsMap({ open, onClose }: ShortcutsMapProps) {
  const [search, setSearch] = useState("");
  const [selectedCategory, setSelectedCategory] = useState<ShortcutCategory | "all">("all");

  // Handle Escape key to close
  useEffect(() => {
    if (!open) return;
    
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [open, onClose]);

  const filteredShortcuts = shortcuts.filter(shortcut => {
    const matchesSearch = search.trim() === "" || 
      shortcut.description.toLowerCase().includes(search.toLowerCase()) ||
      shortcut.keys.some(key => key.toLowerCase().includes(search.toLowerCase())) ||
      (shortcut.context && shortcut.context.toLowerCase().includes(search.toLowerCase()));
    
    const matchesCategory = selectedCategory === "all" || shortcut.category === selectedCategory;
    
    return matchesSearch && matchesCategory;
  });

  if (!open) return null;

  return (
    <div className="shortcuts-map" role="dialog" aria-label="Keyboard shortcuts">
      <div className="shortcuts-map__backdrop" onClick={onClose} />
      <div className="shortcuts-map__content">
        <div className="shortcuts-map__header">
          <h2 className="shortcuts-map__title">Keyboard Shortcuts</h2>
          <button
            className="shortcuts-map__close"
            onClick={onClose}
            aria-label="Close shortcuts"
          >
            <X size={16} />
          </button>
        </div>

        <div className="shortcuts-map__search">
          <Search size={14} className="shortcuts-map__search-icon" />
          <input
            type="text"
            placeholder="Search shortcuts..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="shortcuts-map__search-input"
            autoFocus
          />
        </div>

        <div className="shortcuts-map__categories">
          <button
            className={`shortcuts-map__category ${selectedCategory === "all" ? "is-active" : ""}`}
            onClick={() => setSelectedCategory("all")}
          >
            All ({shortcuts.length})
          </button>
          {Object.entries(categoryLabels).map(([category, label]) => {
            const count = shortcutsByCategory[category as ShortcutCategory]?.length || 0;
            return (
              <button
                key={category}
                className={`shortcuts-map__category ${selectedCategory === category ? "is-active" : ""}`}
                onClick={() => setSelectedCategory(category as ShortcutCategory)}
              >
                {label} ({count})
              </button>
            );
          })}
        </div>

        <div className="shortcuts-map__list">
          {filteredShortcuts.length === 0 ? (
            <div className="shortcuts-map__empty">
              No shortcuts found matching "{search}"
            </div>
          ) : (
            (selectedCategory === "all" 
              ? Object.entries(categoryLabels) as [ShortcutCategory, string][]
              : [[selectedCategory, categoryLabels[selectedCategory]]] as [ShortcutCategory, string][]
            ).map(([category, label]) => {
              const categoryShortcuts = filteredShortcuts.filter(s => s.category === category);
              if (categoryShortcuts.length === 0) return null;

              return (
                <div key={category} className="shortcuts-map__section">
                  <div className="shortcuts-map__section-header">
                    <h3 className="shortcuts-map__section-title">{label}</h3>
                    {categoryDescriptions[category] && (
                      <p className="shortcuts-map__section-description">
                        {categoryDescriptions[category]}
                      </p>
                    )}
                  </div>
                  <div className="shortcuts-map__items">
                    {categoryShortcuts.map(shortcut => (
                      <ShortcutItem key={shortcut.id} shortcut={shortcut} />
                    ))}
                  </div>
                </div>
              );
            })
          )}
        </div>
      </div>
    </div>
  );
}

function ShortcutItem({ shortcut }: { shortcut: Shortcut }) {
  return (
    <div className="shortcuts-map__item">
      <div className="shortcuts-map__item-main">
        <div className="shortcuts-map__item-description">{shortcut.description}</div>
        {shortcut.context && (
          <div className="shortcuts-map__item-context">{shortcut.context}</div>
        )}
      </div>
      <div className="shortcuts-map__item-keys">
        {shortcut.keys.map((key, index) => (
          <span key={key} className="kbd">
            {key}
            {index < shortcut.keys.length - 1 && (
              <span className="shortcuts-map__key-or"> or </span>
            )}
          </span>
        ))}
      </div>
    </div>
  );
}