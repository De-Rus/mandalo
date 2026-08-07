import { useCallback, useEffect, useRef, useState } from "react";
import { BrandMark } from "./components/BrandMark";
import { CommandPalette } from "./components/CommandPalette";
import { ContextPanel } from "./components/ContextPanel";
import { DonateButton } from "./components/DonateButton";
import { DropTarget } from "./components/DropTarget";
import { EnvBar } from "./components/EnvBar";
import { GitBar } from "./components/GitBar";
import { ImportDialog } from "./components/ImportDialog";
import { RemoteBar } from "./components/RemoteBar";
import { RemoteDialog } from "./components/RemoteDialog";
import { Doc, Layers } from "./components/Icons";
import { ResponsePane } from "./components/ResponsePane";
import { StreamPane } from "./components/stream/StreamPane";
import { Sidebar } from "./components/Sidebar";
import { Splitter } from "./components/Splitter";
import { TabStrip } from "./components/TabStrip";
import { Toasts } from "./components/Toasts";
import { Workbench } from "./components/Workbench";
import { WorkspaceActions } from "./components/WorkspaceActions";
import { WorkspaceSwitcher } from "./components/WorkspaceSwitcher";
import { useActiveRequest, useCollection } from "./store/collection";
import { useActiveVars, useEnv } from "./store/env";
import { CONTEXT_WIDTH, useLayout } from "./store/layout";
import { useResponse, useSession } from "./store/session";
import { useTabs } from "./store/tabs";
import { isStreamKind } from "./lib/stream";
import { useStreams } from "./store/stream";
import { useRemote } from "./store/remote";
import { useTransfer } from "./store/transfer";
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
  const toggleStream = useStreams((s) => s.toggle);
  const forgetStream = useStreams((s) => s.forget);
  const streamPhase = useStreams((s) =>
    activeId ? (s.sessions[activeId]?.phase ?? null) : null,
  );

  const sidebarWidth = useLayout((s) => s.sidebarWidth);
  const sidebarCollapsed = useLayout((s) => s.sidebarCollapsed);
  const responseRatio = useLayout((s) => s.responseRatio);
  const setSidebarWidth = useLayout((s) => s.setSidebarWidth);
  const setResponseRatio = useLayout((s) => s.setResponseRatio);
  const streamRatio = useLayout((s) => s.streamRatio);
  const setStreamRatio = useLayout((s) => s.setStreamRatio);
  const setViewportWidth = useLayout((s) => s.setViewportWidth);
  const contextOpen = useLayout((s) => s.contextOpen);
  const toggleContext = useLayout((s) => s.toggleContext);

  const remoteOpen = useRemote((s) => s.dialogOpen);
  const refreshRemote = useRemote((s) => s.refresh);
  const importOpen = useTransfer((s) => s.importOpen);
  const openImport = useTransfer((s) => s.openImport);
  const dropped = useTransfer((s) => s.dropped);
  const dropSeq = useTransfer((s) => s.dropSeq);
  const closeImport = useTransfer((s) => s.closeImport);

  const topPaneRef = useRef<HTMLDivElement>(null);
  const bottomPaneRef = useRef<HTMLDivElement>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [findSignal, setFindSignal] = useState(0);

  useEffect(() => {
    const onResize = () => setViewportWidth(window.innerWidth);
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [setViewportWidth]);

  useEffect(() => {
    void initCollection();
  }, [initCollection]);

  useEffect(() => {
    if (workspace) void initEnv(workspace);
  }, [workspace, initEnv]);

  useEffect(() => {
    void refreshRemote(workspace);
  }, [workspace, refreshRemote]);

  const doSend = useCallback(() => {
    if (!active || active.url.trim() === "") return;
    if (isStreamKind(active.kind)) void toggleStream(active, vars);
    else void send(active);
  }, [active, send, toggleStream, vars]);

  // A tab that is gone must not leave a socket running behind it.
  useEffect(() => {
    const open = new Set(openIds);
    for (const id of Object.keys(useStreams.getState().sessions))
      if (!open.has(id)) void forgetStream(id);
  }, [openIds, forgetStream]);

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
      if (key === "e") {
        e.preventDefault();
        toggleContext();
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
  }, [
    doSend,
    saveActiveNow,
    addRequest,
    activeId,
    closeTab,
    openRequest,
    toggleContext,
  ]);

  const onSidebarDrag = useCallback(
    (x: number) => setSidebarWidth(x),
    [setSidebarWidth],
  );

  const streaming = active !== null && isStreamKind(active.kind);
  const ratio = streaming ? streamRatio : responseRatio;
  const setRatio = streaming ? setStreamRatio : setResponseRatio;

  const onResponseDrag = useCallback(
    (y: number) => {
      const top = topPaneRef.current?.getBoundingClientRect();
      const bottom = bottomPaneRef.current?.getBoundingClientRect();
      if (!top || !bottom) return;
      const span = bottom.bottom - top.top;
      if (span <= 0) return;
      setRatio((bottom.bottom - y) / span);
    },
    [setRatio],
  );

  const sending = response.phase === "loading";
  const dirty = activeId !== null && dirtyIds.includes(activeId);

  return (
    <div className="app">
      <header className="app-header">
        <div
          className={`header-rail header-rail-left${sidebarCollapsed ? " header-rail-compact" : ""}`}
          style={sidebarCollapsed ? undefined : { width: sidebarWidth }}
        >
          <span className="brand-btn">
            <BrandMark size={24} />
          </span>
          <div className="ws-cluster">
            <WorkspaceSwitcher />
            <WorkspaceActions />
          </div>
        </div>
        <div className="header-main" />
        <div
          className={`header-rail header-rail-right${contextOpen ? "" : " header-rail-compact"}`}
          style={contextOpen ? { width: CONTEXT_WIDTH } : undefined}
        >
          <EnvBar />
          <button
            className={`btn-ghost btn-icon ctx-toggle ${contextOpen ? "ctx-toggle-on" : ""}`}
            aria-label="Environment panel"
            aria-expanded={contextOpen}
            title="Environment panel (⌘E)"
            onClick={toggleContext}
          >
            <Layers size={15} />
          </button>
        </div>
      </header>
      <RemoteBar />
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
        <main className="center">
          <TabStrip />
          {active ? (
            <>
              <div
                className="pane"
                ref={topPaneRef}
                style={{ flex: `1 1 ${(1 - ratio) * 100}%` }}
              >
                <Workbench
                  draft={active}
                  vars={vars}
                  sending={sending}
                  dirty={dirty}
                  streamPhase={streamPhase}
                  onPatch={updateActive}
                  onSend={doSend}
                  onSave={() => void saveActiveNow()}
                />
              </div>
              <Splitter
                orientation="horizontal"
                label="Resize response pane"
                onDrag={onResponseDrag}
                onNudge={(d) => setRatio(ratio - d / 600)}
              />
              <div
                className="pane"
                ref={bottomPaneRef}
                style={{ flex: `1 1 ${ratio * 100}%` }}
              >
                {isStreamKind(active.kind) ? (
                  <StreamPane draft={active} kind={active.kind} />
                ) : (
                  <ResponsePane response={response} findSignal={findSignal} />
                )}
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
                Choose one from the sidebar, start a new request, or import an
                OpenAPI / Postman file.
              </p>
              <div className="empty-actions">
                <button
                  className="btn btn-primary"
                  onClick={() => addRequest("http")}
                >
                  New HTTP Request
                </button>
                <button className="btn" onClick={() => openImport()}>
                  Import OpenAPI / Postman
                </button>
                <button
                  className="btn-ghost"
                  onClick={() => setPaletteOpen(true)}
                >
                  Browse…
                </button>
              </div>
              <div className="empty-kbd">
                <kbd>⌘N</kbd> new request
                <span>·</span>
                <kbd>⌘K</kbd> jump to a request
              </div>
            </div>
          )}
        </main>
        {contextOpen && <ContextPanel />}
      </div>
      <footer className="app-footer">
        <div className="app-footer-git">
          <GitBar />
        </div>
        <DonateButton />
      </footer>
      {paletteOpen && <CommandPalette onClose={() => setPaletteOpen(false)} />}
      {remoteOpen && <RemoteDialog />}
      {importOpen && (
        <ImportDialog key={dropSeq} dropped={dropped} onClose={closeImport} />
      )}
      <DropTarget />
      <Toasts />
    </div>
  );
}
