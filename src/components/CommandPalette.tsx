import { useEffect, useMemo, useState } from "react";
import { fuzzyFilter } from "../lib/fuzzy";
import { flattenTree, type FlatRequest } from "../lib/tree";
import { useCollection } from "../store/collection";
import { useModalGuard } from "../store/ui";
import { MethodBadge } from "./MethodBadge";

function location(item: FlatRequest): string {
  return item.folder ? `${item.collection} / ${item.folder}` : item.collection;
}

export function CommandPalette({ onClose }: { onClose: () => void }) {
  useModalGuard();
  const collections = useCollection((s) => s.tree.collections);
  const openRequest = useCollection((s) => s.openRequest);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);

  const all = useMemo(() => flattenTree(collections), [collections]);
  const results = useMemo(
    () => fuzzyFilter(all, query, (r) => `${r.name} ${location(r)}`).slice(0, 40),
    [all, query],
  );

  useEffect(() => setIndex(0), [query]);

  const choose = (item: FlatRequest | undefined) => {
    if (!item) return;
    openRequest(item.id);
    onClose();
  };

  return (
    <div className="palette-backdrop" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="input palette-input"
          autoFocus
          placeholder="Jump to a request…"
          aria-label="Jump to a request"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Escape") onClose();
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setIndex((i) => Math.min(i + 1, results.length - 1));
            }
            if (e.key === "ArrowUp") {
              e.preventDefault();
              setIndex((i) => Math.max(i - 1, 0));
            }
            if (e.key === "Enter") choose(results[index]);
          }}
        />
        <div className="palette-list" role="listbox">
          {results.length === 0 ? (
            <p className="palette-empty">No request matches “{query}”.</p>
          ) : (
            results.map((item, i) => (
              <button
                key={item.id}
                role="option"
                aria-selected={i === index}
                className={`palette-item ${i === index ? "palette-item-active" : ""}`}
                onMouseEnter={() => setIndex(i)}
                onClick={() => choose(item)}
              >
                <MethodBadge item={item} />
                <span className="palette-item-label">{item.name}</span>
                <span className="palette-item-hint">{location(item)}</span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
