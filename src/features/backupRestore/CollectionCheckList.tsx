import { useMemo } from "react";

export interface CollectionItem {
  name: string;
  documentCount: number | null;
  sizeBytes: number | null;
}

interface CollectionCheckListProps {
  items: CollectionItem[];
  selected: string[];
  onChange: (selected: string[]) => void;
}

export function CollectionCheckList({ items, selected, onChange }: CollectionCheckListProps) {
  const allSelected = items.length > 0 && selected.length === items.length;

  const toggle = (name: string) => {
    if (selected.includes(name)) {
      onChange(selected.filter((n) => n !== name));
    } else {
      onChange([...selected, name]);
    }
  };

  const toggleAll = () => {
    if (allSelected) {
      onChange([]);
    } else {
      onChange(items.map((i) => i.name));
    }
  };

  const sorted = useMemo(() => {
    return [...items].sort((a, b) => a.name.localeCompare(b.name));
  }, [items]);

  return (
    <div className="collection-check-list">
      <div className="collection-check-list__toolbar">
        <label className="collection-check-list__all">
          <input
            type="checkbox"
            checked={allSelected}
            onChange={toggleAll}
            aria-label="Select all collections"
          />
          <span>{allSelected ? "Deselect all" : "Select all"}</span>
        </label>
        <span className="collection-check-list__count">
          {selected.length} of {items.length} selected
        </span>
      </div>
      <div className="collection-check-list__body">
        {sorted.map((item) => (
          <label key={item.name} className="collection-check-list__row">
            <input
              type="checkbox"
              checked={selected.includes(item.name)}
              onChange={() => toggle(item.name)}
            />
            <span className="collection-check-list__name">{item.name}</span>
            <span className="collection-check-list__meta">
              {item.documentCount != null && `${item.documentCount.toLocaleString()} docs`}
            </span>
          </label>
        ))}
      </div>
    </div>
  );
}
