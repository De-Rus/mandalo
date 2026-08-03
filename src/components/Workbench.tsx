import { useState } from "react";
import type { Kind } from "../lib/api";
import type { KVRow, RequestDraft } from "../lib/draft";
import { PRE_SNIPPETS, TEST_SNIPPETS } from "../lib/snippets";
import { AuthEditor } from "./AuthEditor";
import { BodyEditor } from "./BodyEditor";
import { GraphqlEditor } from "./GraphqlEditor";
import { GrpcEditor } from "./GrpcEditor";
import { GrpcLocalNotice } from "./GrpcLocalNotice";
import { KeyValueEditor } from "./KeyValueEditor";
import { ScriptEditor } from "./ScriptEditor";
import { Tabs, type TabItem } from "./Tabs";
import { UrlBar } from "./UrlBar";

const TAB_IDS: Record<Kind, string[]> = {
  http: ["params", "auth", "headers", "body", "pre", "tests", "settings"],
  graphql: ["query", "variables", "auth", "headers", "pre", "tests", "settings"],
  grpc: ["proto", "message", "metadata", "auth", "pre", "tests", "settings"],
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
  pre: "Pre-request Script",
  tests: "Post-response Script",
  settings: "Settings",
};

const TEST_PLACEHOLDER =
  '// Runs after the response arrives — read it, set variables, write pm.test(...)\npm.test("Status code is 200", function () {\n  pm.response.to.have.status(200);\n});';

function activeCount(rows: KVRow[]): number {
  return rows.filter((r) => r.enabled && r.key.trim() !== "").length;
}

interface WorkbenchProps {
  draft: RequestDraft;
  vars: Record<string, string>;
  sending: boolean;
  dirty: boolean;
  onPatch: (patch: Partial<RequestDraft>) => void;
  onSend: () => void;
  onSave: () => void;
}

export function Workbench({
  draft,
  vars,
  sending,
  dirty,
  onPatch,
  onSend,
  onSave,
}: WorkbenchProps) {
  const [tabsByKind, setTabsByKind] = useState<Partial<Record<Kind, string>>>({});

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
    body: draft.kind === "http" && draft.body.trim() !== "",
    auth: draft.auth.type !== "none",
    query: draft.graphqlQuery.trim() !== "",
    variables: draft.graphqlVariables.trim() !== "",
    message: draft.grpc.message.trim() !== "",
    proto: draft.grpc.service !== "",
    pre: draft.preScript.trim() !== "",
    tests: draft.testScript.trim() !== "",
    settings: draft.description.trim() !== "",
  };

  const items: TabItem[] = ids.map((id) => ({
    id,
    label: LABELS[id],
    count: counts[id],
    dot: dots[id],
  }));

  const flush =
    active === "params" || active === "headers" || active === "metadata";

  return (
    <section className="workbench">
      <UrlBar
        draft={draft}
        vars={vars}
        sending={sending}
        dirty={dirty}
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
        {active === "auth" && (
          <AuthEditor auth={draft.auth} onChange={(auth) => onPatch({ auth })} />
        )}
        {active === "body" && (
          <BodyEditor value={draft.body} onChange={(body) => onPatch({ body })} />
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
            value={draft.preScript}
            onChange={(preScript) => onPatch({ preScript })}
            snippets={PRE_SNIPPETS}
            placeholder="// Runs before the request is sent"
          />
        )}
        {active === "tests" && (
          <ScriptEditor
            label="Post-response script"
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
                placeholder="What this request is for, expected inputs, gotchas…"
                onChange={(e) => onPatch({ description: e.target.value })}
              />
            </label>
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
                Saved as <code>{`requests/${draft.id}.toml`}</code> in the active
                workspace. Edits autosave; ⌘S saves immediately.
              </p>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
