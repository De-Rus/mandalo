import { useState, type ReactNode } from "react";
import type { CollectionNode, FolderNode, RequestSummary } from "../lib/api";
import { collectionRequestCount, folderRequestCount } from "../lib/tree";
import { Dropdown, MenuItem } from "./Dropdown";
import {
  ChevronRight,
  Collection,
  Doc,
  Dots,
  Folder,
  FolderOpen,
  Pencil,
  Trash,
} from "./Icons";
import { MethodBadge } from "./MethodBadge";

const INDENT = 15;

function guide(depth: number): React.CSSProperties {
  return { ["--guide" as string]: `${depth * INDENT + 15}px` };
}

export interface TreeActions {
  onOpen: (id: string) => void;
  onRenameRequest: (id: string, name: string) => void;
  onDeleteRequest: (id: string) => void;
  onNewRequestIn: (collection: string, folder: string) => void;
  onRenameCollection: (slug: string, name: string) => void;
  onDeleteCollection: (slug: string) => void;
  onNewFolder: (collection: string, parent: string) => void;
  onRenameFolder: (collection: string, path: string, name: string) => void;
  onDeleteFolder: (collection: string, path: string) => void;
}

interface RowProps {
  depth: number;
  icon?: ReactNode;
  label: string;
  className?: string;
  active?: boolean;
  badge?: ReactNode;
  count?: number;
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  onClick: () => void;
  onRename?: (name: string) => void;
  menu?: (close: () => void) => ReactNode;
}

function Row({
  depth,
  icon,
  label,
  className = "",
  active = false,
  badge,
  count,
  expandable = false,
  expanded = false,
  onToggle,
  onClick,
  onRename,
  menu,
}: RowProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(label);

  const startRename = () => {
    setDraft(label);
    setEditing(true);
  };

  const commit = () => {
    setEditing(false);
    if (onRename && draft.trim() !== "" && draft.trim() !== label)
      onRename(draft.trim());
  };

  return (
    <div
      className={`tree-row ${className} ${active ? "tree-row-active" : ""}`}
      style={{ paddingLeft: 4 + depth * INDENT }}
      onClick={onClick}
      onDoubleClick={() => onRename && startRename()}
      role="treeitem"
      aria-selected={active}
      aria-level={depth + 1}
      aria-expanded={expandable ? expanded : undefined}
    >
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
      {badge ?? <span className="tree-icon">{icon}</span>}
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
          {(close) => (
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
              {menu(close)}
            </>
          )}
        </Dropdown>
      )}
    </div>
  );
}

function RequestRow({
  request,
  depth,
  activeId,
  actions,
}: {
  request: RequestSummary;
  depth: number;
  activeId: string | null;
  actions: TreeActions;
}) {
  return (
    <Row
      depth={depth}
      badge={<MethodBadge item={request} />}
      label={request.name}
      active={request.id === activeId}
      onClick={() => actions.onOpen(request.id)}
      onRename={(name) => actions.onRenameRequest(request.id, name)}
      menu={(close) => (
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
            />
          ))}
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
