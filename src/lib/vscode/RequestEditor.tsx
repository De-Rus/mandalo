import { useCallback, useEffect, useRef, useState } from "react";
import { ResponsePane } from "../../components/ResponsePane";
import { Splitter } from "../../components/Splitter";
import { Toasts } from "../../components/Toasts";
import { Workbench } from "../../components/Workbench";
import type { Environment, SavedRequest } from "../api";
import { errorMessage } from "../api";
import { fromSaved, toSaved } from "../collection";
import type { RequestDraft } from "../draft";
import type { ResponseState } from "../../store/session";
import { toast } from "../../store/toast";
import { bridge } from "./bridge";
import { invoke } from "./invoke";
import type { DocumentPayload, EditorContext, EnvironmentPayload } from "./protocol";
import { runRequest } from "./session";

const PATCH_DEBOUNCE_MS = 120;

const IDLE: ResponseState = { phase: "idle" };

function varsOf(payload: EnvironmentPayload | null): Record<string, string> {
  if (!payload?.selected) return {};
  return payload.items.find((env: Environment) => env.name === payload.selected)?.vars ?? {};
}

export function RequestEditor() {
  const [draft, setDraft] = useState<RequestDraft | null>(null);
  const [context, setContext] = useState<EditorContext | null>(null);
  const [parseError, setParseError] = useState<string | null>(null);
  const [environment, setEnvironment] = useState<EnvironmentPayload | null>(null);
  const [response, setResponse] = useState<ResponseState>(IDLE);
  const [dirty, setDirty] = useState(false);
  const [ratio, setRatio] = useState(0.45);
  const [findSignal, setFindSignal] = useState(0);

  const centerRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<RequestDraft | null>(null);
  const timerRef = useRef<number | undefined>(undefined);
  draftRef.current = draft;

  const adopt = useCallback((payload: DocumentPayload) => {
    setContext(payload.context);
    setParseError(payload.error);
    if (!payload.request) return;
    const current = draftRef.current;
    const incoming = JSON.stringify(payload.request);
    if (current && JSON.stringify(toSaved(current)) === incoming) return;
    try {
      setDraft(fromSaved(payload.request, payload.context.collection, payload.context.requestPath));
      setDirty(false);
    } catch (e) {
      setParseError(errorMessage(e));
    }
  }, []);

  const push = useCallback((request: SavedRequest) => {
    void invoke("patch_document", { request }).catch((e) => toast("error", errorMessage(e)));
  }, []);

  const flush = useCallback(() => {
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
    const current = draftRef.current;
    if (current) push(toSaved(current));
  }, [push]);

  useEffect(() => {
    const off = bridge().on((message) => {
      if (message.event === "document") adopt(message.payload as DocumentPayload);
      if (message.event === "environment") setEnvironment(message.payload as EnvironmentPayload);
    });
    void invoke<DocumentPayload>("load_document").then(adopt, (e) => setParseError(errorMessage(e)));
    void invoke<EnvironmentPayload>("list_environments").then(setEnvironment, () => undefined);
    return off;
  }, [adopt]);

  const onPatch = useCallback(
    (patch: Partial<RequestDraft>) => {
      setDraft((current) => {
        if (!current) return current;
        const next = { ...current, ...patch };
        draftRef.current = next;
        return next;
      });
      setDirty(true);
      if (timerRef.current !== undefined) window.clearTimeout(timerRef.current);
      timerRef.current = window.setTimeout(() => {
        timerRef.current = undefined;
        const current = draftRef.current;
        if (current) push(toSaved(current));
      }, PATCH_DEBOUNCE_MS);
    },
    [push],
  );

  const onSave = useCallback(() => {
    const current = draftRef.current;
    if (!current) return;
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
    void invoke("save_document", { request: toSaved(current) })
      .then(() => setDirty(false))
      .catch((e) => toast("error", errorMessage(e)));
  }, []);

  const onSend = useCallback(() => {
    const current = draftRef.current;
    if (!current || current.url.trim() === "") return;
    if (timerRef.current !== undefined) {
      window.clearTimeout(timerRef.current);
      timerRef.current = undefined;
    }
    setResponse({ phase: "loading" });
    runRequest(toSaved(current), environment?.selected ?? null).then(
      (state) => {
        setDirty(false);
        setResponse(state);
      },
      (e) => setResponse({ phase: "error", message: errorMessage(e), logs: [] }),
    );
  }, [environment]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const key = e.key.toLowerCase();
      if (e.key === "Enter") {
        e.preventDefault();
        onSend();
        return;
      }
      if (key === "s") {
        e.preventDefault();
        onSave();
        return;
      }
      if (key === "f") {
        e.preventDefault();
        setFindSignal((n) => n + 1);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("blur", flush);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("blur", flush);
    };
  }, [onSend, onSave, flush]);

  const onResponseDrag = useCallback((y: number) => {
    const box = centerRef.current?.getBoundingClientRect();
    if (!box) return;
    setRatio(Math.min(0.8, Math.max(0.15, 1 - (y - box.top) / box.height)));
  }, []);

  if (!draft) {
    return (
      <div className="vsc-editor">
        <div className="empty">
          <span className="empty-title">
            {parseError ? "This request could not be read" : "Opening request…"}
          </span>
          {parseError && <p className="empty-line">{parseError}</p>}
          {parseError && (
            <p className="empty-line">Fix it in the text editor — “Reopen Editor With… → Text Editor”.</p>
          )}
        </div>
      </div>
    );
  }

  const envName = environment?.selected ?? null;

  return (
    <div className="vsc-editor">
      <div className="vsc-crumbs">
        <span className="vsc-crumb-path truncate">
          {context?.collection ? `${context.collection} · ` : ""}
          {context?.requestPath ?? ""}
        </span>
        <span className="header-spacer" />
        <button
          className="vsc-env"
          title="Choose the environment Mándalo resolves {{vars}} from"
          onClick={() => void invoke("select_environment").catch(() => undefined)}
        >
          {envName ?? "No environment"}
        </button>
      </div>
      {parseError && <div className="vsc-banner">{parseError}</div>}
      <main className="center" ref={centerRef}>
        <div className="pane" style={{ flex: `1 1 ${(1 - ratio) * 100}%` }}>
          <Workbench
            draft={draft}
            vars={varsOf(environment)}
            sending={response.phase === "loading"}
            dirty={dirty}
            streamPhase={null}
            onPatch={onPatch}
            onSend={onSend}
            onSave={onSave}
          />
        </div>
        <Splitter
          orientation="horizontal"
          label="Resize response pane"
          onDrag={onResponseDrag}
          onNudge={(d) => setRatio((r) => Math.min(0.8, Math.max(0.15, r - d / 600)))}
        />
        <div className="pane" style={{ flex: `1 1 ${ratio * 100}%` }}>
          <ResponsePane response={response} findSignal={findSignal} />
        </div>
      </main>
      <Toasts />
    </div>
  );
}
