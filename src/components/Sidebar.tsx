import { useState } from "react";
import { useCollection } from "../store/collection";
import { MethodBadge } from "./MethodBadge";
import { TransferMenu } from "./TransferMenu";

function RequestItem({ id }: { id: string }) {
  const request = useCollection((s) => s.requests.find((r) => r.id === id));
  const activeId = useCollection((s) => s.activeId);
  const openRequest = useCollection((s) => s.openRequest);
  const renameRequest = useCollection((s) => s.renameRequest);
  const deleteRequest = useCollection((s) => s.deleteRequest);
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");

  if (!request) return null;

  const commit = () => {
    renameRequest(id, name);
    setEditing(false);
  };

  return (
    <div
      className={`request-item ${id === activeId ? "request-item-active" : ""}`}
      onClick={() => openRequest(id)}
      onDoubleClick={() => {
        setName(request.name);
        setEditing(true);
      }}
    >
      <MethodBadge draft={request} />
      {editing ? (
        <input
          className="request-rename"
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit();
            if (e.key === "Escape") setEditing(false);
            e.stopPropagation();
          }}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span className="request-name" title={request.name}>
          {request.name}
        </span>
      )}
      <button
        className="request-delete"
        aria-label="Delete request"
        title="Delete request"
        onClick={(e) => {
          e.stopPropagation();
          deleteRequest(id);
        }}
      >
        ×
      </button>
    </div>
  );
}

export function Sidebar() {
  const ids = useCollection((s) => s.requests.map((r) => r.id).join(","));
  const addRequest = useCollection((s) => s.addRequest);
  const error = useCollection((s) => s.error);
  const requestIds = ids === "" ? [] : ids.split(",");

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <span className="sidebar-title">Collection</span>
        <div className="sidebar-actions">
          <button className="btn-ghost" onClick={addRequest} title="New request (⌘N)">
            + New
          </button>
          <TransferMenu />
        </div>
      </div>
      {error && <p className="inline-error sidebar-error">{error}</p>}
      <div className="sidebar-list">
        {requestIds.length === 0 ? (
          <p className="empty-line">No requests yet. Create one to get started.</p>
        ) : (
          requestIds.map((id) => <RequestItem key={id} id={id} />)
        )}
      </div>
    </aside>
  );
}
