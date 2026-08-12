import { useEffect, useRef, useState, type ReactNode } from "react";
import type { CollectionNode, FolderNode, RequestSummary } from "../lib/api";
import { collectionRequestCount, folderRequestCount } from "../lib/tree";
import { ContextMenu } from "./ContextMenu";
import { Dropdown, MenuItem } from "./Dropdown";
import {
  ChevronRight,
  Collection,
  Copy,
  Doc,
  Dots,
  Export,
  Folder,
  FolderOpen,
  Pencil,
  Trash,
} from "./Icons";
import { MethodBadge } from "./MethodBadge";

const INDENT = 15;

/** How long a dragged request must hover a closed folder before it opens. */
const SPRING_MS = 600;

function guide(depth: number): React.CSSProperties {
  return { ["--guide" as string]: `${depth * INDENT + 15}px` };
}

export interface TreeActions {
  onOpen: (id: string) => void;
  onRenameRequest: (id: string, name: string) => void;
  onDeleteRequest: (id: string) => void;
  onDuplicateRequest: (id: string) => void;
  onMoveRequest: (id: string) => void;
  onNewRequestIn: (collection: string, folder: string) => void;
  onRenameCollection: (slug: string, name: string) => void;
  onDeleteCollection: (slug: string) => void;
  onExportCollection: (slug: string) => void;
  onNewFolder: (collection: string, parent: string) => void;
  onRenameFolder: (collection: string, path: string, name: string) => void;
  onDeleteFolder: (collection: string, path: string) => void;
  onDropRequestInto: (id: string, collection: string, folder: string) => void;
  /** Put `id` immediately before or after `anchor`, both in the same folder. */
  onDropBeside: (id: string, anchor: string, before: boolean) => void;
}

interface RowProps {
  depth: number;
  icon?: ReactNode;
  label: string;
  className?: string;
  active?: boolean;
  /**
   * Replaces the twisty and icon columns together. A request has nothing to
   * expand, so its method takes that whole span and the names still line up
   * with a sibling folder's.
   */
  leading?: ReactNode;
  count?: number;
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  onClick: () => void;
  onRename?: (name: string) => void;
  menu?: (close: () => void) => ReactNode;
  /** Set on a request row: the id this row hands over when dragged. */
  dragId?: string;
  /** Set on a row that accepts requests: a folder, or a collection root. */
  onDropRequest?: (id: string) => void;
  /**
   * Set on a request row: a drop on its top or bottom edge puts the dragged
   * request there instead of inside the folder. `before` says which edge.
   */
  onDropBeside?: (id: string, before: boolean) => void;
}

/**
 * A private type, so dragging a request over a Finder window or another app
 * offers it nothing, and a file dragged in from outside is never mistaken for a
 * request being moved.
 */
const DRAG_TYPE = "application/x-mandalo-request";

function Row({
  depth,
  icon,
  label,
  className = "",
  active = false,
  leading,
  count,
  expandable = false,
  expanded = false,
  onToggle,
  onClick,
  onRename,
  menu,
  dragId,
  onDropRequest,
  onDropBeside,
}: RowProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(label);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number } | null>(null);
  const [over, setOver] = useState(false);
  const [edge, setEdge] = useState<"before" | "after" | null>(null);
  // dragenter/dragleave fire for every child element the pointer crosses — the
  // icon, the label, the ⋮ — so a plain boolean flickers. Count instead.
  const depthRef = useRef(0);
  const springRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelSpring = () => {
    if (springRef.current !== null) {
      clearTimeout(springRef.current);
      springRef.current = null;
    }
  };

  const leave = () => {
    depthRef.current = Math.max(0, depthRef.current - 1);
    if (depthRef.current === 0) {
      setOver(false);
      setEdge(null);
      cancelSpring();
    }
  };

  /// The middle of a row means "into"; the top and bottom quarters mean "here",
  /// which is the only way to say where among siblings something goes.
  const edgeAt = (e: React.DragEvent<HTMLDivElement>): "before" | "after" | null => {
    if (!onDropBeside) return null;
    const box = e.currentTarget.getBoundingClientRect();
    const offset = e.clientY - box.top;
    if (offset < box.height * 0.25) return "before";
    if (offset > box.height * 0.75) return "after";
    return null;
  };

  useEffect(() => cancelSpring, []);

  const startRename = () => {
    setDraft(label);
    setEditing(true);
  };

  /// The same entries behind the ⋮ button and behind a right-click, written once
  /// so the two can never drift apart.
  const entries = (close: () => void) => (
    <>
      {onRename && (
        <MenuItem
          icon={<Pencil size={12} />}
          onClick={() => {
            close();
            startRename();
          }}
        >
          Rename
        </MenuItem>
      )}
      {menu?.(close)}
    </>
  );

  const commit = () => {
    setEditing(false);
    if (onRename && draft.trim() !== "" && draft.trim() !== label)
      onRename(draft.trim());
  };

  return (
    <div
      className={`tree-row ${className} ${active ? "tree-row-active" : ""} ${
        over && edge === null ? "tree-row-drop" : ""
      } ${edge === "before" ? "tree-row-before" : ""} ${
        edge === "after" ? "tree-row-after" : ""
      }`}
      style={{ paddingLeft: 4 + depth * INDENT }}
      draggable={dragId !== undefined && !editing}
      onDragStart={(e) => {
        if (dragId === undefined) return;
        e.stopPropagation();
        e.dataTransfer.setData(DRAG_TYPE, dragId);
        e.dataTransfer.effectAllowed = "move";
      }}
      onDragEnter={(e) => {
        if (
          (!onDropRequest && !onDropBeside) ||
          !e.dataTransfer.types.includes(DRAG_TYPE)
        )
          return;
        depthRef.current += 1;
        setOver(true);
        // Spring-loading: a folder inside a collapsed one cannot be dropped on
        // because it is not rendered. Hovering opens the way through, the same
        // as Finder.
        if (expandable && !expanded && onToggle && springRef.current === null) {
          springRef.current = setTimeout(() => {
            springRef.current = null;
            onToggle();
          }, SPRING_MS);
        }
      }}
      onDragOver={(e) => {
        if (
          (!onDropRequest && !onDropBeside) ||
          !e.dataTransfer.types.includes(DRAG_TYPE)
        )
          return;
        // Without preventDefault the browser refuses the drop outright.
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setEdge(edgeAt(e));
      }}
      onDragLeave={leave}
      onDrop={(e) => {
        if (
          (!onDropRequest && !onDropBeside) ||
          !e.dataTransfer.types.includes(DRAG_TYPE)
        )
          return;
        e.preventDefault();
        e.stopPropagation();
        const where = edgeAt(e);
        depthRef.current = 0;
        setOver(false);
        setEdge(null);
        cancelSpring();
        const id = e.dataTransfer.getData(DRAG_TYPE);
        if (id === "") return;
        if (where !== null && onDropBeside) onDropBeside(id, where === "before");
        else onDropRequest?.(id);
      }}
      onClick={onClick}
      onDoubleClick={() => onRename && startRename()}
      onContextMenu={(e) => {
        if (editing || !menu) return;
        e.preventDefault();
        setMenuAt({ x: e.clientX, y: e.clientY });
      }}
      role="treeitem"
      aria-selected={active}
      aria-level={depth + 1}
      aria-expanded={expandable ? expanded : undefined}
    >
      {leading ?? (
        <>
          {expandable ? (
            <button
              className={`tree-twisty ${expanded ? "tree-twisty-open" : ""}`}
              aria-label={expanded ? `Collapse ${label}` : `Expand ${label}`}
              onClick={(e) => {
                e.stopPropagation();
                onToggle?.();
              }}
            >
              <ChevronRight size={12} />
            </button>
          ) : (
            <span className="tree-twisty-spacer" />
          )}
          <span className="tree-icon">{icon}</span>
        </>
      )}
      {editing ? (
        <input
          className="tree-rename"
          autoFocus
          value={draft}
          aria-label="Rename"
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") commit();
            if (e.key === "Escape") setEditing(false);
          }}
        />
      ) : (
        <span className="tree-label" title={label}>
          {label}
        </span>
      )}
      {!editing && count !== undefined && (
        <span className="tree-count">{count}</span>
      )}
      {!editing && menu && (
        <Dropdown
          align="right"
          trigger={({ open, toggle }) => (
            <button
              className={`btn-ghost btn-icon btn-icon-sm tree-more ${
                open ? "tree-more-open" : ""
              }`}
              aria-label={`Actions for ${label}`}
              onClick={(e) => {
                e.stopPropagation();
                toggle();
              }}
            >
              <Dots size={13} />
            </button>
          )}
        >
          {(close) => entries(close)}
        </Dropdown>
      )}
      {menuAt && (
        <ContextMenu
          x={menuAt.x}
          y={menuAt.y}
          label={`Actions for ${label}`}
          onClose={() => setMenuAt(null)}
        >
          {(close) => entries(close)}
        </ContextMenu>
      )}
    </div>
  );
}

function RequestRow({
  request,
  depth,
  activeId,
  actions,
  slug,
  folder,
}: {
  request: RequestSummary;
  depth: number;
  activeId: string | null;
  actions: TreeActions;
  slug: string;
  /** The folder holding this request; "" at the collection root. */
  folder: string;
}) {
  return (
    <Row
      depth={depth}
      dragId={request.id}
      // Most of a folder's visible area is its requests. Dropping on the middle
      // means "put it in this folder"; on an edge it means "put it here".
      onDropRequest={(id) =>
        id === request.id ? undefined : actions.onDropRequestInto(id, slug, folder)
      }
      onDropBeside={(id, before) =>
        id === request.id ? undefined : actions.onDropBeside(id, request.id, before)
      }
      leading={<MethodBadge item={request} tree />}
      label={request.name}
      active={request.id === activeId}
      onClick={() => actions.onOpen(request.id)}
      onRename={(name) => actions.onRenameRequest(request.id, name)}
      menu={(close) => (
        <>
          <MenuItem
            icon={<Copy size={12} />}
            onClick={() => {
              close();
              actions.onDuplicateRequest(request.id);
            }}
          >
            Duplicate
          </MenuItem>
          <MenuItem
            icon={<Folder size={12} />}
            onClick={() => {
              close();
              actions.onMoveRequest(request.id);
            }}
          >
            Move to…
          </MenuItem>
          <div className="menu-sep" />
          <MenuItem
            danger
            icon={<Trash size={12} />}
            onClick={() => {
              close();
              actions.onDeleteRequest(request.id);
            }}
          >
            Delete
          </MenuItem>
        </>
      )}
    />
  );
}

function FolderRows({
  folder,
  slug,
  depth,
  activeId,
  actions,
  expanded,
  toggle,
}: {
  folder: FolderNode;
  slug: string;
  depth: number;
  activeId: string | null;
  actions: TreeActions;
  expanded: (key: string) => boolean;
  toggle: (key: string) => void;
}) {
  const key = `${slug}/${folder.path}`;
  const open = expanded(key);
  return (
    <>
      <Row
        depth={depth}
        icon={open ? <FolderOpen size={13} /> : <Folder size={13} />}
        label={folder.name}
        count={folderRequestCount(folder)}
        expandable
        expanded={open}
        onToggle={() => toggle(key)}
        onClick={() => toggle(key)}
        onRename={(name) => actions.onRenameFolder(slug, folder.path, name)}
        onDropRequest={(id) =>
          actions.onDropRequestInto(id, slug, folder.path)
        }
        menu={(close) => (
          <>
            <MenuItem
              icon={<Doc size={12} />}
              onClick={() => {
                close();
                actions.onNewRequestIn(slug, folder.path);
              }}
            >
              New request
            </MenuItem>
            <MenuItem
              icon={<Folder size={12} />}
              onClick={() => {
                close();
                actions.onNewFolder(slug, folder.path);
              }}
            >
              New folder
            </MenuItem>
            <div className="menu-sep" />
            <MenuItem
              danger
              icon={<Trash size={12} />}
              onClick={() => {
                close();
                actions.onDeleteFolder(slug, folder.path);
              }}
            >
              Delete folder
            </MenuItem>
          </>
        )}
      />
      {open && (
        <div className="tree-children" style={guide(depth)}>
          {folder.folders.map((child) => (
            <FolderRows
              key={child.path}
              folder={child}
              slug={slug}
              depth={depth + 1}
              activeId={activeId}
              actions={actions}
              expanded={expanded}
              toggle={toggle}
            />
          ))}
          {folder.requests.map((request) => (
            <RequestRow
              key={request.id}
              request={request}
              depth={depth + 1}
              activeId={activeId}
              actions={actions}
              slug={slug}
              folder={folder.path}
            />
          ))}
          {folder.folders.length === 0 && folder.requests.length === 0 && (
            // An expanded folder with nothing in it collapses to zero height, so
            // the rows that follow — its siblings — read as its contents. Saying
            // it is empty is what makes the tree honest about where things are.
            <div
              className="tree-empty"
              style={{ paddingLeft: 4 + (depth + 1) * INDENT }}
            >
              Empty
            </div>
          )}
        </div>
      )}
    </>
  );
}

const COLLAPSED_KEY = "mandalo.tree.collapsed.v1";

function readCollapsed(): string[] {
  const raw = localStorage.getItem(COLLAPSED_KEY);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as unknown;
    return Array.isArray(parsed) ? (parsed as string[]) : [];
  } catch {
    return [];
  }
}

interface CollectionTreeProps {
  collections: CollectionNode[];
  activeId: string | null;
  actions: TreeActions;
}

export function CollectionTree({
  collections,
  activeId,
  actions,
}: CollectionTreeProps) {
  const [collapsed, setCollapsed] = useState<string[]>(readCollapsed);

  const expanded = (key: string) => !collapsed.includes(key);
  const toggle = (key: string) => {
    const next = collapsed.includes(key)
      ? collapsed.filter((k) => k !== key)
      : [...collapsed, key];
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify(next));
    setCollapsed(next);
  };

  return (
    <div className="tree" role="tree" aria-label="Collections">
      {collections.map((collection) => {
        const key = `c:${collection.slug}`;
        const open = expanded(key);
        return (
          <div key={collection.id}>
            <Row
              depth={0}
              className="tree-row-collection"
              icon={<Collection size={13} />}
              label={collection.name}
              count={collectionRequestCount(collection)}
              expandable
              expanded={open}
              onToggle={() => toggle(key)}
              onClick={() => toggle(key)}
              onRename={(name) => actions.onRenameCollection(collection.slug, name)}
              onDropRequest={(id) =>
                actions.onDropRequestInto(id, collection.slug, "")
              }
              menu={(close) => (
                <>
                  <MenuItem
                    icon={<Doc size={12} />}
                    onClick={() => {
                      close();
                      actions.onNewRequestIn(collection.slug, "");
                    }}
                  >
                    New request
                  </MenuItem>
                  <MenuItem
                    icon={<Folder size={12} />}
                    onClick={() => {
                      close();
                      actions.onNewFolder(collection.slug, "");
                    }}
                  >
                    New folder
                  </MenuItem>
                  <div className="menu-sep" />
                  <MenuItem
                    icon={<Export size={12} />}
                    onClick={() => {
                      close();
                      actions.onExportCollection(collection.slug);
                    }}
                  >
                    Export…
                  </MenuItem>
                  <div className="menu-sep" />
                  <MenuItem
                    danger
                    icon={<Trash size={12} />}
                    onClick={() => {
                      close();
                      actions.onDeleteCollection(collection.slug);
                    }}
                  >
                    Delete collection
                  </MenuItem>
                </>
              )}
            />
            {open && (
              <div className="tree-children" style={guide(0)}>
                {collection.folders.map((folder) => (
                  <FolderRows
                    key={folder.path}
                    folder={folder}
                    slug={collection.slug}
                    depth={1}
                    activeId={activeId}
                    actions={actions}
                    expanded={expanded}
                    toggle={toggle}
                  />
                ))}
                {collection.requests.map((request) => (
                  <RequestRow
                    key={request.id}
                    request={request}
                    depth={1}
                    activeId={activeId}
                    actions={actions}
                    slug={collection.slug}
                    folder=""
                  />
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
