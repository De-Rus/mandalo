import type { Kind } from "../lib/api";
import { Dropdown, MenuItem } from "./Dropdown";
import {
  Broadcast,
  ChevronDown,
  Collection,
  Doc,
  Folder,
  Plus,
} from "./Icons";

interface NewMenuProps {
  onNewRequest: (kind: Kind) => void;
  onNewFolder: (() => void) | null;
  onNewCollection: (() => void) | null;
  scratch?: boolean;
}

export function NewMenu({
  onNewRequest,
  onNewFolder,
  onNewCollection,
  scratch = false,
}: NewMenuProps) {
  return (
    <Dropdown
      align="left"
      menuClassName="new-menu"
      trigger={({ open, toggle }) => (
        <button
          className={`btn btn-new ${open ? "menu-item-active" : ""}`}
          onClick={toggle}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label="New"
          title="New (⌘N)"
        >
          <Plus size={12} />
          New
          <ChevronDown size={11} className="btn-new-caret" />
        </button>
      )}
    >
      {(close) => (
        <>
          <div className="menu-head">
            {scratch ? "Create in “Scratch”" : "Create"}
          </div>
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
          <div className="menu-sep" />
          <div className="menu-head">Realtime</div>
          <MenuItem
            icon={<Broadcast size={13} />}
            onClick={() => {
              close();
              onNewRequest("websocket");
            }}
          >
            WebSocket Connection
          </MenuItem>
          <MenuItem
            icon={<Broadcast size={13} />}
            onClick={() => {
              close();
              onNewRequest("sse");
            }}
          >
            Server-Sent Events
          </MenuItem>
          <MenuItem
            icon={<Broadcast size={13} />}
            onClick={() => {
              close();
              onNewRequest("mqtt");
            }}
          >
            MQTT Connection
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
