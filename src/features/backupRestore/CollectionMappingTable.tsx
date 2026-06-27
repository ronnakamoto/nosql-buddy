import { useMemo } from "react";
import type { CollectionMapping } from "../../ipc/commands";

interface CollectionMappingTableProps {
  mappings: CollectionMapping[];
  onChange: (mappings: CollectionMapping[]) => void;
}

export function CollectionMappingTable({ mappings, onChange }: CollectionMappingTableProps) {
  const sorted = useMemo(() => {
    return [...mappings].sort((a, b) => a.source.localeCompare(b.source));
  }, [mappings]);

  const update = (source: string, patch: Partial<CollectionMapping>) => {
    const next = mappings.map((m) => (m.source === source ? { ...m, ...patch } : m));
    onChange(next);
  };

  const allEnabled = mappings.length > 0 && mappings.every((m) => m.enabled);

  const toggleAll = () => {
    const target = !allEnabled;
    onChange(mappings.map((m) => ({ ...m, enabled: target })));
  };

  return (
    <div className="collection-mapping-table">
      <div className="collection-mapping-table__header">
        <label className="collection-mapping-table__all">
          <input type="checkbox" checked={allEnabled} onChange={toggleAll} />
          <span>{allEnabled ? "Deselect all" : "Select all"}</span>
        </label>
      </div>
      <div className="collection-mapping-table__body">
        {sorted.map((m) => (
          <label
            key={m.source}
            className={`collection-mapping-table__row ${!m.enabled ? "collection-mapping-table__row--disabled" : ""}`}
            htmlFor={`restore-map-${m.source}`}
          >
            <input
              id={`restore-map-${m.source}`}
              type="checkbox"
              checked={m.enabled}
              onChange={(e) => update(m.source, { enabled: e.target.checked })}
              aria-label={`Include ${m.source}`}
            />
            <span className="collection-mapping-table__source">{m.source}</span>
            <span className="collection-mapping-table__arrow">→</span>
            <input
              type="text"
              className="field__input field__input--sm"
              value={m.target}
              onChange={(e) => update(m.source, { target: e.target.value })}
              disabled={!m.enabled}
              aria-label={`Target name for ${m.source}`}
              onClick={(e) => e.stopPropagation()}
            />
          </label>
        ))}
      </div>
    </div>
  );
}
