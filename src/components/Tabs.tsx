export interface TabItem {
  id: string;
  label: string;
  count?: number;
  dot?: boolean;
}

interface TabsProps {
  items: TabItem[];
  active: string;
  onSelect: (id: string) => void;
}

export function Tabs({ items, active, onSelect }: TabsProps) {
  return (
    <div className="tabs" role="tablist">
      {items.map((t) => (
        <button
          key={t.id}
          role="tab"
          aria-selected={t.id === active}
          className={`tab ${t.id === active ? "tab-active" : ""}`}
          onClick={() => onSelect(t.id)}
        >
          {t.label}
          {t.count !== undefined && t.count > 0 && (
            <span className="count">{t.count}</span>
          )}
          {t.dot && <span className="tab-dot" />}
        </button>
      ))}
    </div>
  );
}
