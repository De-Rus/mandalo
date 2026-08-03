import type { Kind } from "../lib/api";
import { Dropdown, MenuItem } from "./Dropdown";
import { ChevronDown, Collection, Doc, Folder, Plus } from "./Icons";

interface NewMenuProps {
  onNewRequest: (kind: Kind) => void;
  onNewFolder: (() => void) | null;
  onNewCollection: (() => void) | null;
}

export function NewMenu({
  onNewRequest,
  onNewFolder,
  onNewCollection,
}: NewMenuProps) {
  return (
    <Dropdown
      align="right"
      menuClassName="new-menu"
      trigger={({ open, toggle }) => (
        <button
          className={`btn btn-sm ${open ? "menu-item-active" : ""}`}
          onClick={toggle}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label="New"
          title="New (⌘N)"
        >
          <Plus size={12} />
          New
          <ChevronDown size={11} />
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="menu-head">Create</div>
          <MenuItem
            icon={<Doc size={13} />}
            hint="⌘N"
            onClick={() => {
              close();
              onNewRequest("http");
            }}
          >
            HTTP Request
          </MenuItem>
          <MenuItem
            icon={<Doc size={13} />}
            onClick={() => {
              close();
              onNewRequest("grpc");
            }}
          >
            gRPC Request
          </MenuItem>
          <MenuItem
            icon={<Doc size={13} />}
            onClick={() => {
              close();
              onNewRequest("graphql");
            }}
          >
            GraphQL Request
          </MenuItem>
          {(onNewFolder || onNewCollection) && <div className="menu-sep" />}
          {onNewFolder && (
            <MenuItem
              icon={<Folder size={13} />}
              onClick={() => {
                close();
                onNewFolder();
              }}
            >
              Folder
            </MenuItem>
          )}
          {onNewCollection && (
            <MenuItem
              icon={<Collection size={13} />}
              hint="⌘⇧N"
              onClick={() => {
                close();
                onNewCollection();
              }}
            >
              Collection
            </MenuItem>
          )}
        </>
      )}
    </Dropdown>
  );
}
