interface TabsProps {
  tabs: string[];
  active: string;
  onSelect: (tab: string) => void;
}

export function Tabs({ tabs, active, onSelect }: TabsProps) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button
          key={t}
          role="tab"
          aria-selected={t === active}
          className={`tab ${t === active ? "tab-active" : ""}`}
          onClick={() => onSelect(t)}
        >
          {t}
        </button>
      ))}
    </div>
  );
}
