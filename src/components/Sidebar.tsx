import { useMemo, useState } from "react";
import { errorMessage, type Kind } from "../lib/api";
import { applyDraftOverrides, filterTree } from "../lib/tree";
import { useCollection } from "../store/collection";
import { toast } from "../store/toast";
import { CollectionTree, type TreeActions } from "./CollectionTree";
import { ConfirmModal } from "./ConfirmModal";
import { Close, Collection, Search, Warn } from "./Icons";
import { NewMenu } from "./NewMenu";
import { PromptModal } from "./PromptModal";
import { TransferMenu } from "./TransferMenu";

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
  const createCollection = useCollection((s) => s.createCollection);
  const renameCollection = useCollection((s) => s.renameCollection);
  const deleteCollection = useCollection((s) => s.deleteCollection);
  const createFolder = useCollection((s) => s.createFolder);
  const renameFolder = useCollection((s) => s.renameFolder);
  const deleteFolder = useCollection((s) => s.deleteFolder);

  const [query, setQuery] = useState("");
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  const [confirm, setConfirm] = useState<Confirm | null>(null);

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
    onNewFolder: (collection, parent) =>
      setPrompt({ kind: "folder", collection, parent }),
    onRenameFolder: (collection, path, name) =>
      guard(() => renameFolder(collection, path, name)),
    onDeleteFolder: (collection, path) =>
      setConfirm({ kind: "folder", collection, path }),
  };

  const empty = tree.length === 0;
  const noMatches = !empty && visible.length === 0;

  return (
    <aside className="sidebar" style={{ width }}>
      <div className="sidebar-actions">
        <NewMenu
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
        <TransferMenu />
      </div>
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
          <span className="empty-title">Create your first collection</span>
          <p className="empty-line">
            Collections are plain folders of TOML files in your workspace —
            versionable with git, readable without Mándalo.
          </p>
          <button
            className="btn btn-primary"
            onClick={() => setPrompt({ kind: "collection" })}
          >
            New collection
          </button>
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
