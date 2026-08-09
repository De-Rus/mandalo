import { useState } from "react";
import type { Kind } from "../lib/api";
import { bodyHasContent, type KVRow, type RequestDraft } from "../lib/draft";
import { useCollection } from "../store/collection";
import { PRE_SNIPPETS, TEST_SNIPPETS } from "../lib/snippets";
import { AuthEditor } from "./AuthEditor";
import { BodyEditor } from "./BodyEditor";
import { GraphqlEditor } from "./GraphqlEditor";
import { GrpcEditor } from "./GrpcEditor";
import { GrpcLocalNotice } from "./GrpcLocalNotice";
import { KeyValueEditor } from "./KeyValueEditor";
import { ScriptEditor } from "./ScriptEditor";
import { isStreamKind } from "../lib/stream";
import type { Phase } from "../store/stream";
import { StreamOptions } from "./stream/StreamOptions";
import { Tabs, type TabItem } from "./Tabs";
import { UrlBar } from "./UrlBar";

const TAB_IDS: Record<Kind, string[]> = {
  http: [
    "params",
    "auth",
    "headers",
    "body",
    "pre",
    "tests",
    "settings",
  ],
  graphql: [
    "query",
    "variables",
    "auth",
    "headers",
    "pre",
    "tests",
    "settings",
  ],
  grpc: [
    "proto",
    "message",
    "metadata",
    "auth",
    "pre",
    "tests",
    "settings",
  ],
  websocket: ["connection", "auth", "headers", "settings"],
  sse: ["connection", "auth", "headers", "settings"],
  mqtt: ["connection", "auth", "settings"],
};

const LABELS: Record<string, string> = {
  params: "Params",
  auth: "Authorization",
  headers: "Headers",
  body: "Body",
  query: "Query",
  variables: "Variables",
  proto: "Proto",
  message: "Message",
  metadata: "Metadata",
  connection: "Connection",
  pre: "Pre-request Script",
  tests: "Post-response Script",
  settings: "Settings",
};

const TEST_PLACEHOLDER =
  '// Runs after the response arrives — read it, set variables, write pm.test(...)\npm.test("Status code is 200", function () {\n  pm.response.to.have.status(200);\n});';

function activeCount(rows: KVRow[]): number {
  return rows.filter((r) => r.enabled && r.key.trim() !== "").length;
}

/** A saved path is `<file>#<index>`; only the file half is worth showing. */
function fileOf(path: string): string {
  const hash = path.lastIndexOf("#");
  return hash === -1 ? path : path.slice(0, hash);
}

interface WorkbenchProps {
  draft: RequestDraft;
  vars: Record<string, string>;
  sending: boolean;
  dirty: boolean;
  streamPhase: Phase | null;
  onPatch: (patch: Partial<RequestDraft>) => void;
  onSend: () => void;
  onSave: () => void;
}

export function Workbench({
  draft,
  vars,
  sending,
  dirty,
  streamPhase,
  onPatch,
  onSend,
  onSave,
}: WorkbenchProps) {
  const [tabsByKind, setTabsByKind] = useState<Partial<Record<Kind, string>>>({});
  const workspaceRoot = useCollection((s) => s.workspace);

  const ids = TAB_IDS[draft.kind];
  const active = ids.includes(tabsByKind[draft.kind] ?? "")
    ? (tabsByKind[draft.kind] as string)
    : ids[0];

  const counts: Record<string, number | undefined> = {
    params: activeCount(draft.params),
    headers: activeCount(draft.headers),
    metadata: activeCount(draft.grpc.metadata),
  };

  const dots: Record<string, boolean> = {
    body: draft.kind === "http" && bodyHasContent(draft),
    auth: draft.auth.type !== "none",
    query: draft.graphqlQuery.trim() !== "",
    variables: draft.graphqlVariables.trim() !== "",
    message: draft.grpc.message.trim() !== "",
    proto: draft.grpc.service !== "",
    pre: draft.preScript.trim() !== "",
    tests: draft.testScript.trim() !== "",
    settings: draft.description.trim() !== "",
    connection: isStreamKind(draft.kind) && draft.stream.messages.length > 0,
  };

  const items: TabItem[] = ids.map((id) => ({
    id,
    label: LABELS[id],
    count: counts[id],
    dot: dots[id],
  }));

  const flush =
    active === "params" || active === "headers" || active === "metadata";

  /**
   * WHY: every kind is stored in a text file (.http/.grpc/.ws/.mqtt), and those
   * formats keep a description in the `#` comment above the request. The engine
   * rejects a description field on every save into one, so an editable field
   * here would break autosave for good — it is only a field until the file exists.
   */
  const file = draft.path === null ? null : fileOf(draft.path);
  const inFile = file !== null;

  return (
    <section className="workbench">
      <UrlBar
        draft={draft}
        vars={vars}
        sending={sending}
        dirty={dirty}
        streamPhase={streamPhase}
        onPatch={onPatch}
        onSend={onSend}
        onSave={onSave}
      />
      <GrpcLocalNotice draft={draft} vars={vars} />
      <Tabs
        items={items}
        active={active}
        onSelect={(id) => setTabsByKind((s) => ({ ...s, [draft.kind]: id }))}
      />
      <div className={`panel ${flush ? "panel-flush" : ""}`}>
        {active === "params" && (
          <KeyValueEditor
            rows={draft.params}
            onChange={(params) => onPatch({ params })}
            keyPlaceholder="Key"
          />
        )}
        {active === "headers" && (
          <KeyValueEditor
            rows={draft.headers}
            onChange={(headers) => onPatch({ headers })}
            keyPlaceholder="Key"
          />
        )}
        {active === "connection" && isStreamKind(draft.kind) && (
          <StreamOptions
            kind={draft.kind}
            stream={draft.stream}
            onChange={(stream) => onPatch({ stream })}
          />
        )}
        {active === "auth" && (
          <AuthEditor auth={draft.auth} onChange={(auth) => onPatch({ auth })} />
        )}
        {active === "body" && (
          <BodyEditor draft={draft} workspaceRoot={workspaceRoot} onChange={onPatch} />
        )}
        {(active === "query" || active === "variables") && (
          <GraphqlEditor
            tab={active === "query" ? "Query" : "Variables"}
            query={draft.graphqlQuery}
            variables={draft.graphqlVariables}
            onQueryChange={(graphqlQuery) => onPatch({ graphqlQuery })}
            onVariablesChange={(graphqlVariables) => onPatch({ graphqlVariables })}
          />
        )}
        {(active === "proto" || active === "message" || active === "metadata") && (
          <GrpcEditor
            tab={
              active === "proto"
                ? "Proto"
                : active === "message"
                  ? "Message"
                  : "Metadata"
            }
            grpc={draft.grpc}
            onChange={(grpc) => onPatch({ grpc })}
          />
        )}
        {active === "pre" && (
          <ScriptEditor
            label="Pre-request script"
            kind="pre"
            value={draft.preScript}
            onChange={(preScript) => onPatch({ preScript })}
            snippets={PRE_SNIPPETS}
            placeholder="// Runs before the request is sent"
          />
        )}
        {active === "tests" && (
          <ScriptEditor
            label="Post-response script"
            kind="post"
            value={draft.testScript}
            onChange={(testScript) => onPatch({ testScript })}
            snippets={TEST_SNIPPETS}
            placeholder={TEST_PLACEHOLDER}
          />
        )}
                        {active === "settings" && (
          <div className="settings-list">
            <label className="field">
              <span className="field-label">Description</span>
              <textarea
                className="textarea"
                value={draft.description}
                readOnly={inFile}
                aria-readonly={inFile}
                placeholder={
                  inFile
                    ? "Written as the # comment above the request"
                    : "What this request is for, expected inputs, gotchas…"
                }
                onChange={
                  inFile
                    ? undefined
                    : (e) => onPatch({ description: e.target.value })
                }
              />
            </label>
            {inFile && (
              <p className="settings-hint">
                Every request is a text file, and a description lives in the{" "}
                <code>#</code> comment lines above the request — not in a field.
                Edit the comment in <code>{file}</code> to change it.
              </p>
            )}
            <div className="settings-row">
              <span className="settings-row-head">Request name</span>
              <input
                className="input"
                value={draft.name}
                onChange={(e) => onPatch({ name: e.target.value })}
              />
            </div>
            <div className="settings-row">
              <span className="settings-row-head">Storage</span>
              <p className="settings-hint">
                {inFile ? (
                  <>
                    Saved as <code>{file}</code> in{" "}
                    <code>{draft.collection}</code>. Edits autosave; ⌘S saves
                    immediately.
                  </>
                ) : (
                  <>
                    Not written to the workspace yet. The first save picks a text
                    file from the request kind. Edits autosave; ⌘S saves
                    immediately.
                  </>
                )}
              </p>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
