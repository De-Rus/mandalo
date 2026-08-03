import { useState } from "react";
import type { Kind } from "../lib/api";
import type { RequestDraft } from "../lib/draft";
import { useCollection } from "../store/collection";
import { useActiveVars } from "../store/env";
import { useResponse, useSession } from "../store/session";
import { AuthEditor } from "./AuthEditor";
import { BodyEditor } from "./BodyEditor";
import { GraphqlEditor } from "./GraphqlEditor";
import { GrpcEditor } from "./GrpcEditor";
import { KeyValueEditor } from "./KeyValueEditor";
import { Tabs } from "./Tabs";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

const TABS: Record<Kind, string[]> = {
  http: ["Params", "Headers", "Body", "Auth"],
  graphql: ["Query", "Variables", "Headers", "Auth"],
  grpc: ["Proto", "Message", "Metadata"],
};

const KIND_LABELS: [Kind, string][] = [
  ["http", "HTTP"],
  ["graphql", "GraphQL"],
  ["grpc", "gRPC"],
];

function varsTooltip(vars: Record<string, string>): string {
  const entries = Object.entries(vars);
  if (entries.length === 0) return "No environment variables active";
  return (
    "Active vars — use {{name}}:\n" +
    entries.map(([k, v]) => `{{${k}}} = ${v}`).join("\n")
  );
}

export function Workbench({ draft }: { draft: RequestDraft }) {
  const updateActive = useCollection((s) => s.updateActive);
  const send = useSession((s) => s.send);
  const vars = useActiveVars();
  const response = useResponse(draft.id);
  const [tabsByKind, setTabsByKind] = useState<Partial<Record<Kind, string>>>({});

  const tabs = TABS[draft.kind];
  const activeTab = tabsByKind[draft.kind] ?? tabs[0];
  const sending = response.phase === "loading";

  const setTab = (tab: string) =>
    setTabsByKind((s) => ({ ...s, [draft.kind]: tab }));

  const urlPlaceholder =
    draft.kind === "grpc" ? "http://localhost:50051" : "https://api.example.com/{{path}}";

  return (
    <section className="workbench">
      <div className="workbench-top">
        <div className="segmented">
          {KIND_LABELS.map(([kind, label]) => (
            <button
              key={kind}
              className={`segment ${draft.kind === kind ? "segment-active" : ""}`}
              onClick={() => updateActive({ kind })}
            >
              {label}
            </button>
          ))}
        </div>
        {draft.kind === "http" && (
          <select
            className="select method-select"
            value={draft.method}
            onChange={(e) => updateActive({ method: e.target.value })}
          >
            {METHODS.map((m) => (
              <option key={m}>{m}</option>
            ))}
          </select>
        )}
        <input
          className="input mono url-input"
          value={draft.url}
          placeholder={urlPlaceholder}
          title={varsTooltip(vars)}
          spellCheck={false}
          onChange={(e) => updateActive({ url: e.target.value })}
        />
        <button
          className="btn btn-primary"
          disabled={sending || draft.url.trim() === ""}
          title="Send (⌘⏎)"
          onClick={() => void send(draft, vars)}
        >
          {sending ? "Sending…" : "Send"}
        </button>
      </div>
      <Tabs tabs={tabs} active={activeTab} onSelect={setTab} />
      <div className="workbench-panel">
        {activeTab === "Params" && (
          <KeyValueEditor
            rows={draft.params}
            onChange={(params) => updateActive({ params })}
            keyPlaceholder="param"
          />
        )}
        {activeTab === "Headers" && (
          <KeyValueEditor
            rows={draft.headers}
            onChange={(headers) => updateActive({ headers })}
            keyPlaceholder="Header"
          />
        )}
        {activeTab === "Body" && (
          <BodyEditor
            value={draft.body}
            onChange={(body) => updateActive({ body })}
          />
        )}
        {activeTab === "Auth" && (
          <AuthEditor
            auth={draft.auth}
            onChange={(auth) => updateActive({ auth })}
          />
        )}
        {(activeTab === "Query" || activeTab === "Variables") && (
          <GraphqlEditor
            tab={activeTab}
            query={draft.graphqlQuery}
            variables={draft.graphqlVariables}
            onQueryChange={(graphqlQuery) => updateActive({ graphqlQuery })}
            onVariablesChange={(graphqlVariables) =>
              updateActive({ graphqlVariables })
            }
          />
        )}
        {(activeTab === "Proto" ||
          activeTab === "Message" ||
          activeTab === "Metadata") && (
          <GrpcEditor
            tab={activeTab}
            grpc={draft.grpc}
            onChange={(grpc) => updateActive({ grpc })}
          />
        )}
      </div>
    </section>
  );
}
