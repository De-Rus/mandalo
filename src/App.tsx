import { useCallback, useEffect, useRef, useState } from "react";
import { BrandMark } from "./components/BrandMark";
import { CommandPalette } from "./components/CommandPalette";
import { EnvBar } from "./components/EnvBar";
import { Doc } from "./components/Icons";
import { ResponsePane } from "./components/ResponsePane";
import { Sidebar } from "./components/Sidebar";
import { Splitter } from "./components/Splitter";
import { TabStrip } from "./components/TabStrip";
import { Toasts } from "./components/Toasts";
import { Workbench } from "./components/Workbench";
import { WorkspaceSwitcher } from "./components/WorkspaceSwitcher";
import { useActiveRequest, useCollection } from "./store/collection";
import { useActiveVars, useEnv } from "./store/env";
import { useLayout } from "./store/layout";
import { useResponse, useSession } from "./store/session";
import { useTabs } from "./store/tabs";
import { anyModalOpen } from "./store/ui";

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

export default function App() {
  const active = useActiveRequest();
  const activeId = useCollection((s) => s.activeId);
  const workspace = useCollection((s) => s.workspace);
  const loadingId = useCollection((s) => s.loadingId);
  const initCollection = useCollection((s) => s.init);
  const addRequest = useCollection((s) => s.addRequest);
  const updateActive = useCollection((s) => s.updateActive);
  const saveActiveNow = useCollection((s) => s.saveActiveNow);
  const openIds = useTabs((s) => s.openIds);
  const dirtyIds = useTabs((s) => s.dirtyIds);
  const closeTab = useTabs((s) => s.close);
  const openRequest = useCollection((s) => s.openRequest);
  const initEnv = useEnv((s) => s.init);
  const vars = useActiveVars();
  const send = useSession((s) => s.send);
  const response = useResponse(activeId);

  const sidebarWidth = useLayout((s) => s.sidebarWidth);
  const sidebarCollapsed = useLayout((s) => s.sidebarCollapsed);
  const responseRatio = useLayout((s) => s.responseRatio);
  const setSidebarWidth = useLayout((s) => s.setSidebarWidth);
  const setResponseRatio = useLayout((s) => s.setResponseRatio);

  const centerRef = useRef<HTMLDivElement>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [findSignal, setFindSignal] = useState(0);

  useEffect(() => {
    void initCollection();
  }, [initCollection]);

  useEffect(() => {
    if (workspace) void initEnv(workspace);
  }, [workspace, initEnv]);

  const doSend = useCallback(() => {
    if (active && active.url.trim() !== "") void send(active);
  }, [active, send]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        setPaletteOpen((v) => !v);
        return;
      }
      if (anyModalOpen()) return;
      if (e.key === "Enter") {
        e.preventDefault();
        doSend();
        return;
      }
      if (key === "s") {
        e.preventDefault();
        void saveActiveNow();
        return;
      }
      if (key === "f") {
        e.preventDefault();
        setFindSignal((n) => n + 1);
        return;
      }
      if (key === "w") {
        if (e.shiftKey) return;
        e.preventDefault();
        if (!activeId) return;
        const next = closeTab(activeId, activeId);
        if (next) openRequest(next);
        else useCollection.setState({ activeId: null });
        return;
      }
      if (isTyping(e.target)) return;
      if (key === "n") {
        e.preventDefault();
        addRequest(e.shiftKey ? "graphql" : "http");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [doSend, saveActiveNow, addRequest, activeId, closeTab, openRequest]);

  const onSidebarDrag = useCallback(
    (x: number) => setSidebarWidth(x),
    [setSidebarWidth],
  );

  const onResponseDrag = useCallback(
    (y: number) => {
      const box = centerRef.current?.getBoundingClientRect();
      if (!box) return;
      setResponseRatio(1 - (y - box.top) / box.height);
    },
    [setResponseRatio],
  );

  const sending = response.phase === "loading";
  const dirty = activeId !== null && dirtyIds.includes(activeId);

  return (
    <div className="app">
      <header className="app-header">
        <span className="brand-btn">
          <BrandMark size={26} />
        </span>
        <span className="header-sep" />
        <WorkspaceSwitcher />
        <div className="header-spacer" />
        <EnvBar />
      </header>
      <div className="app-body">
        {!sidebarCollapsed && (
          <>
            <Sidebar width={sidebarWidth} />
            <Splitter
              orientation="vertical"
              label="Resize sidebar"
              onDrag={onSidebarDrag}
              onNudge={(d) => setSidebarWidth(sidebarWidth + d)}
            />
          </>
        )}
        <main className="center" ref={centerRef}>
          <TabStrip />
          {active ? (
            <>
              <div
                className="pane"
                style={{ flex: `1 1 ${(1 - responseRatio) * 100}%` }}
              >
                <Workbench
                  draft={active}
                  vars={vars}
                  sending={sending}
                  dirty={dirty}
                  onPatch={updateActive}
                  onSend={doSend}
                  onSave={() => void saveActiveNow()}
                />
              </div>
              <Splitter
                orientation="horizontal"
                label="Resize response pane"
                onDrag={onResponseDrag}
                onNudge={(d) => setResponseRatio(responseRatio - d / 600)}
              />
              <div
                className="pane"
                style={{ flex: `1 1 ${responseRatio * 100}%` }}
              >
                <ResponsePane response={response} findSignal={findSignal} />
              </div>
            </>
          ) : loadingId ? (
            <div className="skeleton-stack">
              <div className="skeleton" style={{ width: "40%" }} />
              <div className="skeleton" style={{ width: "72%" }} />
              <div className="skeleton" style={{ width: "56%" }} />
            </div>
          ) : (
            <div className="empty">
              <span className="empty-icon">
                <Doc size={30} />
              </span>
              <span className="empty-title">
                {openIds.length === 0 ? "No request open" : "Pick a request"}
              </span>
              <p className="empty-line">
                Choose one from the sidebar, or create a new one.
              </p>
              <div className="empty-kbd">
                <kbd>⌘N</kbd> new request
                <span>·</span>
                <kbd>⌘K</kbd> jump to a request
              </div>
            </div>
          )}
        </main>
      </div>
      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}
      <Toasts />
    </div>
  );
}
