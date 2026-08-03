import { useEffect } from "react";
import { EnvBar } from "./components/EnvBar";
import { ResponsePane } from "./components/ResponsePane";
import { Sidebar } from "./components/Sidebar";
import { Workbench } from "./components/Workbench";
import { useActiveRequest, useCollection } from "./store/collection";
import { useActiveVars } from "./store/env";
import { useResponse, useSession } from "./store/session";
import { anyModalOpen } from "./store/ui";

export default function App() {
  const active = useActiveRequest();
  const addRequest = useCollection((s) => s.addRequest);
  const initCollection = useCollection((s) => s.init);
  const sidebarCollapsed = useSession((s) => s.sidebarCollapsed);
  const toggleSidebar = useSession((s) => s.toggleSidebar);
  const send = useSession((s) => s.send);
  const vars = useActiveVars();
  const response = useResponse(active?.id ?? null);

  useEffect(() => {
    void initCollection();
  }, [initCollection]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (anyModalOpen()) return;
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key === "Enter") {
        e.preventDefault();
        if (active && active.url.trim() !== "") void send(active, vars);
      } else if (e.key === "n") {
        e.preventDefault();
        addRequest();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, vars, send, addRequest]);

  return (
    <div className="app">
      <header className="app-header">
        <button
          className="btn-ghost sidebar-toggle"
          title="Toggle sidebar"
          onClick={toggleSidebar}
        >
          ☰
        </button>
        <span className="brand">Mándalo</span>
        <div className="header-spacer" />
        <EnvBar />
      </header>
      <div className="app-body">
        {!sidebarCollapsed && <Sidebar />}
        <main className="main">
          {active ? (
            <>
              <Workbench draft={active} />
              <ResponsePane response={response} />
            </>
          ) : (
            <div className="pane-empty app-empty">
              <p className="empty-line">
                No request open. Press ⌘N or click “+ New” to create one.
              </p>
            </div>
          )}
        </main>
      </div>
    </div>
  );
}
