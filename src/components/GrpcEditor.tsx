import { useState } from "react";
import { errorMessage, listGrpcMethods, type GrpcMethodInfo } from "../lib/api";
import type { GrpcDraft } from "../lib/draft";
import { parseProtoPaths } from "../lib/spec";
import { BodyEditor } from "./BodyEditor";
import { KeyValueEditor } from "./KeyValueEditor";

interface GrpcEditorProps {
  tab: "Proto" | "Message" | "Metadata";
  grpc: GrpcDraft;
  onChange: (grpc: GrpcDraft) => void;
}

export function GrpcEditor({ tab, grpc, onChange }: GrpcEditorProps) {
  const [methods, setMethods] = useState<GrpcMethodInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const patch = (p: Partial<GrpcDraft>) => onChange({ ...grpc, ...p });

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      const found = await listGrpcMethods(parseProtoPaths(grpc.protoPaths));
      setMethods(found);
      if (found.length === 0) setError("No methods found in the given proto files.");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  };

  if (tab === "Message") {
    return (
      <BodyEditor
        value={grpc.message}
        onChange={(message) => patch({ message })}
        placeholder={'{\n  "name": "world"\n}'}
      />
    );
  }

  if (tab === "Metadata") {
    return (
      <KeyValueEditor
        rows={grpc.metadata}
        onChange={(metadata) => patch({ metadata })}
        keyPlaceholder="metadata-key"
      />
    );
  }

  const selectedValue =
    grpc.service && grpc.method ? `${grpc.service}/${grpc.method}` : "";

  return (
    <div className="grpc-proto">
      <label className="field">
        <span className="field-label">Proto files (one path per line)</span>
        <textarea
          className="textarea mono proto-paths"
          value={grpc.protoPaths}
          placeholder="/path/to/service.proto"
          spellCheck={false}
          onChange={(e) => patch({ protoPaths: e.target.value })}
        />
      </label>
      <div className="grpc-load-row">
        <button
          className="btn"
          disabled={loading || parseProtoPaths(grpc.protoPaths).length === 0}
          onClick={load}
        >
          {loading ? "Loading…" : "Load methods"}
        </button>
        <select
          className="select mono grpc-method-select"
          value={selectedValue}
          disabled={methods.length === 0}
          onChange={(e) => {
            const [service, method] = e.target.value.split("/");
            patch({ service, method });
          }}
        >
          <option value="" disabled>
            {methods.length === 0 ? "Load methods first" : "Select a method"}
          </option>
          {selectedValue !== "" &&
            !methods.some(
              (m) => `${m.service}/${m.method}` === selectedValue,
            ) && <option value={selectedValue}>{selectedValue}</option>}
          {methods.map((m) => {
            const streaming = m.clientStreaming || m.serverStreaming;
            const value = `${m.service}/${m.method}`;
            return (
              <option key={value} value={value} disabled={streaming}>
                {value}
                {streaming ? "  (streaming soon)" : ""}
              </option>
            );
          })}
        </select>
      </div>
      {error && <p className="inline-error">{error}</p>}
      {methods.length === 0 && !error && (
        <p className="empty-line">
          Add .proto paths above, then load methods to pick a service and method.
        </p>
      )}
    </div>
  );
}
