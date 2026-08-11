import { useMemo, useState } from "react";
import {
  errorMessage,
  type CollectionNode,
  type FolderNode,
  type Kind,
} from "../lib/api";
import { applyDraftOverrides, filterTree, folderOf } from "../lib/tree";
import { locationOf, useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { useEnv } from "../store/env";
import { useLayout } from "../store/layout";
import { useModalGuard } from "../store/ui";
import { CollectionTree, type TreeActions } from "./CollectionTree";
import { ConfirmModal } from "./ConfirmModal";
import { EnvList } from "./EnvList";
import { ExportDialog } from "./ExportDialog";
import { Close, Collection, Search, Warn } from "./Icons";
import { NewMenu } from "./NewMenu";
import { PromptModal } from "./PromptModal";
import { SidebarSection } from "./SidebarSection";

function slugify(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "folder"
  );
}

type Prompt =
  | { kind: "collection" }
  | { kind: "folder"; collection: string; parent: string };

type Confirm =
  | { kind: "collection"; slug: string; name: string }
  | { kind: "folder"; collection: string; path: string };

interface Move {
  id: string;
  collection: string;
  current: string;
}

interface Destination {
  path: string;
  label: string;
}

function destinations(collection: CollectionNode): Destination[] {
  const out: Destination[] = [{ path: "", label: collection.name }];
  const walk = (folders: FolderNode[], prefix: string) => {
    for (const folder of folders) {
      const label = `${prefix} / ${folder.name}`;
      out.push({ path: folder.path, label });
      walk(folder.folders, label);
    }
  };
  walk(collection.folders, collection.name);
  return out;
}

function MoveModal({
  move,
  onSubmit,
  onClose,
}: {
  move: Move;
  onSubmit: (target: string) => void;
  onClose: () => void;
}) {
  useModalGuard();
  const collections = useCollection((s) => s.tree.collections);
  const [target, setTarget] = useState(move.current);
  const collection = collections.find((c) => c.slug === move.collection);
  const options = collection ? destinations(collection) : [];

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        style={{ width: 420 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>Move request</h2>
          <button className="btn-ghost btn-icon" aria-label="Close" onClick={onClose}>
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {collection ? (
            <>
              <label className="field">
                <span className="field-label">Destination</span>
                <select
                  className="select"
                  autoFocus
                  aria-label="Destination"
                  value={target}
                  onChange={(e) => setTarget(e.target.value)}
                >
                  {options.map((option) => (
                    <option key={option.path} value={option.path}>
                      {option.label}
                      {option.path === move.current ? " (current)" : ""}
                    </option>
                  ))}
                </select>
              </label>
              <p className="empty-line">
                Requests move within “{collection.name}”. Moving one to another
                collection is not supported yet.
              </p>
            </>
          ) : (
            <p className="inline-error">
              “{move.collection}” is no longer in this workspace.
            </p>
          )}
          <div className="modal-actions">
            <button className="btn" onClick={onClose}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={!collection || target === move.current}
              onClick={() => {
                onSubmit(target);
                onClose();
              }}
            >
              Move
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export function Sidebar({ width }: { width: number }) {
  const collections = useCollection((s) => s.tree.collections);
  const drafts = useCollection((s) => s.drafts);
  const activeId = useCollection((s) => s.activeId);
  const error = useCollection((s) => s.error);
  const warning = useCollection((s) => s.warning);
  const openRequest = useCollection((s) => s.openRequest);
  const addRequest = useCollection((s) => s.addRequest);
  const renameRequest = useCollection((s) => s.renameRequest);
  const deleteRequest = useCollection((s) => s.deleteRequest);
  const duplicateRequest = useCollection((s) => s.duplicateRequest);
  const moveRequest = useCollection((s) => s.moveRequest);
  const createCollection = useCollection((s) => s.createCollection);
  const renameCollection = useCollection((s) => s.renameCollection);
  const deleteCollection = useCollection((s) => s.deleteCollection);
  const createFolder = useCollection((s) => s.createFolder);
  const renameFolder = useCollection((s) => s.renameFolder);
  const deleteFolder = useCollection((s) => s.deleteFolder);

  const collectionsOpen = useLayout((s) => s.collectionsOpen);
  const environmentsOpen = useLayout((s) => s.environmentsOpen);
  const setCollectionsOpen = useLayout((s) => s.setCollectionsOpen);
  const setEnvironmentsOpen = useLayout((s) => s.setEnvironmentsOpen);
  const envError = useEnv((s) => s.error);
  const envCount = useEnv((s) => s.envs.length);

  const [query, setQuery] = useState("");
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);
  const [move, setMove] = useState<Move | null>(null);
  const [exporting, setExporting] = useState<string | null>(null);

  const tree = useMemo(
    () => applyDraftOverrides(collections, Object.values(drafts)),
    [collections, drafts],
  );
  const visible = useMemo(() => filterTree(tree, query), [tree, query]);

  const guard = (run: () => Promise<void>) => {
    void run().catch((e) => toast("error", errorMessage(e)));
  };

  const actions: TreeActions = {
    onOpen: openRequest,
    onRenameRequest: renameRequest,
    onDeleteRequest: deleteRequest,
    onDuplicateRequest: (id) => guard(() => duplicateRequest(id)),
    onMoveRequest: (id) => {
      const location = locationOf(id);
      if (!location) {
        toast("error", "This request has no saved file yet, so it cannot be moved");
        return;
      }
      setMove({
        id,
        collection: location.collection,
        current: folderOf(location.path),
      });
    },
    onNewRequestIn: (collection, folder) =>
      addRequest("http", collection, folder),
    onRenameCollection: (slug, name) =>
      guard(() => renameCollection(slug, name)),
    onDeleteCollection: (slug) =>
      setConfirm({
        kind: "collection",
        slug,
        name: collections.find((c) => c.slug === slug)?.name ?? slug,
      }),
    onExportCollection: (slug) => setExporting(slug),
    onNewFolder: (collection, parent) =>
      setPrompt({ kind: "folder", collection, parent }),
    onRenameFolder: (collection, path, name) =>
      guard(() => renameFolder(collection, path, name)),
    onDeleteFolder: (collection, path) =>
      setConfirm({ kind: "folder", collection, path }),
    onDropRequestInto: (id, collection, folder) => {
      const location = locationOf(id);
      if (!location) {
        toast("error", "This request has no saved file yet, so it cannot be moved");
        return;
      }
      // moveRequest works inside one collection; crossing them is not supported
      // by the backend yet, so say so rather than silently doing nothing.
      if (location.collection !== collection) {
        toast(
          "error",
          "Moving a request to another collection is not supported yet",
        );
        return;
      }
      if (folderOf(location.path) === folder) return;
      guard(() => moveRequest(id, folder));
    },
  };

  const empty = tree.length === 0;
  const noMatches = !empty && visible.length === 0;

  return (
    <aside className="sidebar" style={{ width }}>
      <div className="sidebar-actions">
        <NewMenu
          scratch={collections.length === 0}
          onNewRequest={(kind: Kind) => addRequest(kind)}
          onNewFolder={
            collections.length > 0
              ? () =>
                  setPrompt({
                    kind: "folder",
                    collection: collections[0].slug,
                    parent: "",
                  })
              : null
          }
          onNewCollection={() => setPrompt({ kind: "collection" })}
        />
      </div>
      {envError && !environmentsOpen && (
        <div className="notice notice-error sidebar-warning" title={envError}>
          <Warn size={13} />
          <span className="notice-text">{envError}</span>
        </div>
      )}
      <SidebarSection
        id="collections"
        title="Collections"
        open={collectionsOpen}
        onToggle={setCollectionsOpen}
        count={collections.length}
        grow
      >
        <>
            <div className="sidebar-top">
              <div className="search-wrap">
                <span className="search-icon">
                  <Search size={13} />
                </span>
                <input
                  className="input search-input"
                  placeholder="Search requests"
                  aria-label="Search requests"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                />
                {query !== "" && (
                  <button
                    className="btn-ghost btn-icon search-clear"
                    aria-label="Clear search"
                    onClick={() => setQuery("")}
                  >
                    <Close size={11} />
                  </button>
                )}
              </div>
            </div>
            {error && (
              <div className="notice notice-error sidebar-warning" title={error}>
                <Warn size={13} />
                <span className="notice-text">{error}</span>
              </div>
            )}
            {warning && (
              <div className="notice sidebar-warning" title={warning}>
                <Warn size={13} />
                <span className="notice-text">{warning}</span>
              </div>
            )}
            {empty ? (
              <div className="empty tree-empty">
                <span className="empty-icon">
                  <Collection size={30} />
                </span>
                <span className="empty-title">Nothing here yet</span>
                <p className="empty-line">
                  Collections are plain folders of TOML files in your workspace —
                  versionable with git, readable without Mándalo.
                </p>
                <button
                  className="btn btn-primary"
                  onClick={() => addRequest("http")}
                >
                  New request
                </button>
                <p className="empty-line">
                  It lands in a collection called “Scratch”.{" "}
                  <button
                    className="btn-link"
                    onClick={() => setPrompt({ kind: "collection" })}
                  >
                    Name it yourself instead
                  </button>
                  .
                </p>
                <div className="empty-kbd">
                  <kbd>⌘N</kbd> new request
                </div>
              </div>
            ) : noMatches ? (
              <div className="empty tree-empty">
                <span className="empty-icon">
                  <Search size={26} />
                </span>
                <p className="empty-line">
                  Nothing matches “{query}”. Try a shorter term.
                </p>
              </div>
            ) : (
              <CollectionTree
                collections={visible}
                activeId={activeId}
                actions={actions}
              />
            )}
        </>
      </SidebarSection>
      <SidebarSection
        id="environments"
        title="Environments"
        open={environmentsOpen}
        onToggle={setEnvironmentsOpen}
        count={envCount}
      >
        <EnvList />
      </SidebarSection>
      {prompt?.kind === "collection" && (
        <PromptModal
          title="New collection"
          label="Name"
          placeholder="Acme API"
          onSubmit={(name) => guard(() => createCollection(name))}
          onClose={() => setPrompt(null)}
        />
      )}
      {prompt?.kind === "folder" && (
        <PromptModal
          title="New folder"
          label="Name"
          placeholder="Users"
          onSubmit={(name) =>
            guard(() =>
              createFolder(
                prompt.collection,
                prompt.parent === ""
                  ? slugify(name)
                  : `${prompt.parent}/${slugify(name)}`,
              ),
            )
          }
          onClose={() => setPrompt(null)}
        />
      )}
      {move && (
        <MoveModal
          move={move}
          onSubmit={(target) => guard(() => moveRequest(move.id, target))}
          onClose={() => setMove(null)}
        />
      )}
      {exporting !== null && (
        <ExportDialog
          collection={exporting}
          onClose={() => setExporting(null)}
        />
      )}
      {confirm?.kind === "collection" && (
        <ConfirmModal
          title="Delete collection"
          message={`“${confirm.name}” and every request inside it will be removed from disk. This cannot be undone.`}
          onConfirm={() => guard(() => deleteCollection(confirm.slug))}
          onClose={() => setConfirm(null)}
        />
      )}
      {confirm?.kind === "folder" && (
        <ConfirmModal
          title="Delete folder"
          message={`“${confirm.path}” and every request inside it will be removed from disk. This cannot be undone.`}
          onConfirm={() =>
            guard(() => deleteFolder(confirm.collection, confirm.path))
          }
          onClose={() => setConfirm(null)}
        />
      )}
    </aside>
  );
}
