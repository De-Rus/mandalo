import { useEffect, useMemo, useState } from "react";
import { applyDraftOverrides, flattenTree } from "../lib/tree";
import { flushPendingSaves, useCollection } from "../store/collection";
import { useTabs } from "../store/tabs";
import { anyModalOpen } from "../store/ui";
import { ConfirmModal } from "./ConfirmModal";
import { ContextMenu } from "./ContextMenu";
import { MenuItem } from "./Dropdown";
import { Close, Dots, Plus } from "./Icons";
import { MethodBadge } from "./MethodBadge";

type TabAction = "close" | "others" | "right" | "all";

interface Pending {
  ids: string[];
  dirty: number;
}

export function TabStrip() {
  const openIds = useTabs((s) => s.openIds);
  const dirtyIds = useTabs((s) => s.dirtyIds);
  const closeOne = useTabs((s) => s.close);
  const closeMany = useTabs((s) => s.closeMany);
  const collections = useCollection((s) => s.tree.collections);
  const drafts = useCollection((s) => s.drafts);
  const activeId = useCollection((s) => s.activeId);
  const openRequest = useCollection((s) => s.openRequest);
  const addRequest = useCollection((s) => s.addRequest);

  const [menuAt, setMenuAt] = useState<{
    id: string;
    x: number;
    y: number;
    fromButton?: boolean;
  } | null>(null);
  const [pending, setPending] = useState<Pending | null>(null);

  const summaries = useMemo(
    () => flattenTree(applyDraftOverrides(collections, Object.values(drafts))),
    [collections, drafts],
  );

  const settle = (next: string | null) => {
    if (next && next !== activeId) openRequest(next);
    else if (!next) useCollection.setState({ activeId: null });
  };

  const closeTab = (id: string) => settle(closeOne(id, activeId));

  const idsFor = (action: TabAction, id: string): string[] => {
    if (action === "close") return [id];
    if (action === "others") return openIds.filter((v) => v !== id);
    if (action === "right") return openIds.slice(openIds.indexOf(id) + 1);
    return [...openIds];
  };

  const apply = (ids: string[]) => {
    if (ids.length === 0) return;
    void flushPendingSaves();
    settle(closeMany(ids, activeId));
  };

  const ask = (action: TabAction, id: string) => {
    const ids = idsFor(action, id);
    if (ids.length === 0) return;
    const dirty = ids.filter((v) => dirtyIds.includes(v)).length;
    if (ids.length > 1 && dirty > 0) setPending({ ids, dirty });
    else apply(ids);
  };

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || !e.shiftKey) return;
      if (e.key.toLowerCase() !== "w") return;
      if (anyModalOpen()) return;
      e.preventDefault();
      const anchor = activeId ?? openIds[0];
      if (anchor) ask("all", anchor);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const menuItems = (id: string, close: () => void) => {
    const fire = (action: TabAction) => {
      close();
      ask(action, id);
    };
    const last = openIds.indexOf(id) === openIds.length - 1;
    return (
      <>
        <MenuItem hint="⌘W" onClick={() => fire("close")}>
          Close
        </MenuItem>
        {openIds.length > 1 && (
          <MenuItem onClick={() => fire("others")}>Close Others</MenuItem>
        )}
        {!last && (
          <MenuItem onClick={() => fire("right")}>Close to the Right</MenuItem>
        )}
        <div className="menu-sep" />
        <MenuItem hint="⌘⇧W" onClick={() => fire("all")}>
          Close All
        </MenuItem>
      </>
    );
  };

  return (
    <div className="tabstrip">
      <div className="tabstrip-scroll" role="tablist" aria-label="Open requests">
        {openIds.map((id) => {
          const summary = summaries.find((r) => r.id === id) ?? drafts[id] ?? null;
          if (!summary) return null;
          const dirty = dirtyIds.includes(id);
          return (
            <div
              key={id}
              role="tab"
              aria-selected={id === activeId}
              className={`rtab ${id === activeId ? "rtab-active" : ""}`}
              onClick={() => openRequest(id)}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenuAt({ id, x: e.clientX, y: e.clientY });
              }}
              onAuxClick={(e) => {
                if (e.button === 1) closeTab(id);
              }}
            >
              <MethodBadge item={summary} />
              <span className="rtab-label" title={summary.name}>
                {summary.name}
              </span>
              <span className="rtab-dirty-slot">
                {dirty && <span className="rtab-dirty" title="Unsaved changes" />}
                <button
                  className="rtab-close"
                  aria-label={`Close ${summary.name}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(id);
                  }}
                >
                  <Close size={11} />
                </button>
              </span>
            </div>
          );
        })}
        <button
          className="btn-ghost btn-icon tabstrip-add"
          aria-label="New request"
          title="New request (⌘N)"
          onClick={() => addRequest("http")}
        >
          <Plus size={13} />
        </button>
      </div>
      {openIds.length > 0 && (
        <button
          className="btn-ghost btn-icon tabstrip-more"
          aria-label="Tab actions"
          title="Tab actions"
          onMouseDown={(e) => {
            e.stopPropagation();
            const box = e.currentTarget.getBoundingClientRect();
            setMenuAt((open) =>
              open?.fromButton
                ? null
                : {
                    id: activeId ?? openIds[0],
                    x: box.right,
                    y: box.bottom + 4,
                    fromButton: true,
                  },
            );
          }}
        >
          <Dots size={14} />
        </button>
      )}
      {menuAt && (
        <ContextMenu
          x={menuAt.x}
          y={menuAt.y}
          label="Tab actions"
          onClose={() => setMenuAt(null)}
        >
          {(close) => menuItems(menuAt.id, close)}
        </ContextMenu>
      )}
      {pending && (
        <ConfirmModal
          title="Unsaved changes"
          message={
            pending.dirty === 1
              ? `One of the ${pending.ids.length} tabs you are closing has edits that are not on disk yet. Mándalo saves it before closing.`
              : `${pending.dirty} of the ${pending.ids.length} tabs you are closing have edits that are not on disk yet. Mándalo saves them before closing.`
          }
          confirmLabel="Save and close"
          tone="primary"
          onConfirm={() => apply(pending.ids)}
          onClose={() => setPending(null)}
        />
      )}
    </div>
  );
}
