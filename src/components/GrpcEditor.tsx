import { useRef, useState } from "react";
import {
  describeMessage,
  errorMessage,
  importProto,
  listGrpcMethods,
  type GrpcMethodInfo,
  type MessageShape,
} from "../lib/api";
import { useCollection } from "../store/collection";
import type { GrpcDraft } from "../lib/draft";
import { replaceable, skeletonFor } from "../lib/skeleton";
import { parseProtoPaths } from "../lib/spec";
import { RawBodyEditor } from "./BodyEditor";
import { Close } from "./Icons";
import { KeyValueEditor } from "./KeyValueEditor";

interface GrpcEditorProps {
  tab: "Proto" | "Message" | "Metadata";
  grpc: GrpcDraft;
  onChange: (grpc: GrpcDraft) => void;
}

interface Offer {
  method: string;
  skeleton: string;
}

export function GrpcEditor({ tab, grpc, onChange }: GrpcEditorProps) {
  const [methods, setMethods] = useState<GrpcMethodInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [offer, setOffer] = useState<Offer | null>(null);
  const workspace = useCollection((s) => s.workspace);
  const picker = useRef<HTMLInputElement>(null);

  /**
   * A request names its proto by a workspace-relative path, so the file has to
   * live in the workspace first. Picking one copies it into `protos/` and adds
   * the path — which is the only way a browser tab can supply a proto at all,
   * and on the desktop is what keeps the request portable.
   */
  const addFiles = async (files: FileList) => {
    if (!workspace) {
      setError("Open a workspace before adding proto files.");
      return;
    }
    setError(null);
    const added: string[] = [];
    for (const file of Array.from(files)) {
      try {
        added.push(await importProto(workspace, file.name, await file.text()));
      } catch (e) {
        setError(errorMessage(e));
        break;
      }
    }
    if (added.length === 0) return;
    const lines = parseProtoPaths(grpc.protoPaths);
    const fresh = added.filter((p) => !lines.includes(p));
    patch({ protoPaths: [...lines, ...fresh].join("\n") });
  };

  const patch = (p: Partial<GrpcDraft>) => onChange({ ...grpc, ...p });

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      if (!workspace) throw new Error("Open a workspace before loading methods.");
      const found = await listGrpcMethods(
        workspace,
        parseProtoPaths(grpc.protoPaths),
      );
      setMethods(found);
      if (found.length === 0) setError("No methods found in the given proto files.");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoading(false);
    }
  };

  const shapeOf = async (
    method: GrpcMethodInfo | undefined,
  ): Promise<MessageShape | null> => {
    if (!method) return null;
    try {
      if (!workspace) return null;
      return await describeMessage(
        workspace,
        parseProtoPaths(grpc.protoPaths),
        method.input,
      );
    } catch {
      return null;
    }
  };

  const selectMethod = async (service: string, method: string) => {
    const previous = methods.find(
      (m) => m.service === grpc.service && m.method === grpc.method,
    );
    const chosen = methods.find(
      (m) => m.service === service && m.method === method,
    );
    patch({ service, method });
    setOffer(null);
    const shape = await shapeOf(chosen);
    if (!shape) return;
    const skeleton = skeletonFor(shape);
    const previousShape = previous === chosen ? null : await shapeOf(previous);
    if (replaceable(grpc.message, previousShape && skeletonFor(previousShape)))
      onChange({ ...grpc, service, method, message: skeleton });
    else setOffer({ method, skeleton });
  };

  const offerBar = offer && (
    <div className="grpc-offer">
      <span className="grpc-offer-text">
        The message still holds the fields of the previous method.
      </span>
      <button
        className="btn btn-sm"
        onClick={() => {
          patch({ message: offer.skeleton });
          setOffer(null);
        }}
      >
        Insert example message for {offer.method}
      </button>
    </div>
  );

  if (tab === "Message") {
    return (
      <div className="grpc-message">
        {offerBar}
        <RawBodyEditor
          value={grpc.message}
          onChange={(message) => patch({ message })}
          placeholder={'{\n  "name": "world"\n}'}
        />
      </div>
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

  // The file is what the user picked; the path underneath is bookkeeping the
  // editor no longer shows, because showing it invited absolute paths that a
  // request file cannot hold.
  const files = parseProtoPaths(grpc.protoPaths);

  const selectedValue =
    grpc.service && grpc.method ? `${grpc.service}/${grpc.method}` : "";

  return (
    <div className="grpc-proto">
      <div className="field">
        <div className="grpc-proto-head">
          <span className="field-label">Proto files</span>
          <button
            className="btn btn-sm"
            type="button"
            onClick={() => picker.current?.click()}
          >
            Add .proto…
          </button>
          <input
            ref={picker}
            type="file"
            accept=".proto"
            multiple
            hidden
            onChange={(e) => {
              const files = e.target.files;
              if (files && files.length > 0) void addFiles(files);
              e.target.value = "";
            }}
          />
        </div>
        {files.length === 0 ? (
          <p className="empty-line proto-empty">
            No proto files yet. Add one and its services show up below.
          </p>
        ) : (
          <ul className="proto-list">
            {files.map((file) => (
              <li key={file} className="proto-item">
                <span className="proto-name mono" title={file}>
                  {file.split("/").pop()}
                </span>
                <button
                  className="btn-ghost btn-icon btn-icon-sm"
                  type="button"
                  aria-label={`Remove ${file}`}
                  onClick={() =>
                    patch({
                      protoPaths: files.filter((f) => f !== file).join("\n"),
                    })
                  }
                >
                  <Close size={12} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
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
            void selectMethod(service, method);
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
      {offerBar}
      {error && <p className="inline-error">{error}</p>}
      {methods.length === 0 && !error && (
        <p className="empty-line">
          Add .proto paths above, then load methods to pick a service and method.
        </p>
      )}
    </div>
  );
}
