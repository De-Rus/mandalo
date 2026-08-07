import { useEffect, useRef, useState } from "react";
import { open as pickDirectory } from "@tauri-apps/plugin-dialog";
import {
  adoptRemote,
  errorMessage,
  reviewRemote,
  type RemoteReview,
} from "../lib/api";
import { currentHost } from "../lib/host";
import { formatBytes } from "../lib/format";
import { useCollection } from "../store/collection";
import { useRemote } from "../store/remote";
import { toast } from "../store/toast";
import { useWorkspaces } from "../store/workspace";
import { useModalGuard } from "../store/ui";
import { Check, Close, Layers, Warn } from "./Icons";

function Line({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="report-section">
      <span className="field-label">{label}</span>
      <div className="report-list">{children}</div>
    </div>
  );
}

function Review({ review }: { review: RemoteReview }) {
  return (
    <>
      <p className="report-counts">
        {review.requests} requests · {review.collections} collections ·{" "}
        {review.environments.length} environments · {review.files} files ·{" "}
        {formatBytes(review.bytes)}
      </p>

      <Line label="Hosts it will contact">
        {review.hosts.length === 0 && review.templatedHosts.length === 0 ? (
          <p>None it names outright.</p>
        ) : (
          <ul>
            {review.hosts.map((host) => (
              <li key={host} className="mono">
                {host}
              </li>
            ))}
            {review.templatedHosts.map((template) => (
              <li key={template} className="mono">
                {template} — whatever you set that to
              </li>
            ))}
          </ul>
        )}
      </Line>

      <Line label="Environments">
        {review.environments.length === 0 ? (
          <p>None.</p>
        ) : (
          <ul>
            {review.environments.map((env) => (
              <li key={env.name}>
                {env.name}: {env.declared.length} variables declared,{" "}
                {env.sharedValues} with a value in the file, {env.awaitingValues}{" "}
                you would have to supply yourself
              </li>
            ))}
          </ul>
        )}
      </Line>

      <Line label="Scripts">
        {review.scripts.length === 0 ? (
          <p>None. Nothing in this collection can run.</p>
        ) : (
          <>
            <p>
              None of these run on opening. A script runs when you send the
              request it belongs to, and not before.
            </p>
            <ul>
              {review.scripts.map((script, i) => (
                <li key={i}>
                  {script.collection} · {script.request} · {script.hook} ·{" "}
                  {script.lines} lines
                </li>
              ))}
            </ul>
          </>
        )}
      </Line>

      {review.findings.length > 0 && (
        <Line label="Credential scanner">
          <ul className="report-warnings">
            {review.findings.map((finding, i) => (
              <li key={i}>
                {finding.path}:{finding.line} — {finding.rule} · {finding.excerpt}
              </li>
            ))}
          </ul>
          <p>
            A credential-looking literal in somebody else's collection is either
            their mistake or your bait. Read it before you send anything.
          </p>
        </Line>
      )}

      {review.skipped.length > 0 && (
        <Line label="Not taken">
          <ul>
            {review.skipped.map((line, i) => (
              <li key={i}>{line}</li>
            ))}
          </ul>
        </Line>
      )}
    </>
  );
}

export function RemoteDialog() {
  useModalGuard();
  const host = currentHost();
  const close = useRemote((s) => s.close);
  const prefill = useRemote((s) => s.prefill);
  const switchWorkspace = useCollection((s) => s.switchWorkspace);
  const loadWorkspaces = useWorkspaces((s) => s.load);
  const refreshOrigin = useRemote((s) => s.refresh);

  const [source, setSource] = useState(prefill);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [review, setReview] = useState<RemoteReview | null>(null);
  const started = useRef(false);

  const look = async (raw: string) => {
    if (raw.trim() === "") return;
    setBusy(`Reading ${raw.trim()}…`);
    setError(null);
    setReview(null);
    try {
      setReview(await reviewRemote(raw.trim()));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    if (prefill === "" || started.current) return;
    started.current = true;
    void look(prefill);
  }, [prefill]);

  const adopt = async () => {
    if (review === null) return;
    setBusy("Opening…");
    setError(null);
    try {
      let dest = review.origin.label.replace(/[^A-Za-z0-9._-]+/g, "-");
      if (host === "desktop") {
        const picked = await pickDirectory({ directory: true, multiple: false });
        if (typeof picked !== "string") {
          setBusy(null);
          return;
        }
        dest = `${picked}/${review.origin.label.split("/").pop() ?? "collection"}`;
      }
      const info = await adoptRemote(review.token, dest);
      await switchWorkspace(info.path);
      await loadWorkspaces();
      await refreshOrigin(info.path);
      toast("success", `${info.name} opened read-only`);
      close();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="modal-backdrop" onClick={close}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{review === null ? "Browse shared collection" : "Before it opens"}</h2>
          <button
            className="btn-ghost btn-icon"
            onClick={close}
            aria-label="Close"
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          <label className="field">
            <span className="field-label">Public repository or bundle URL</span>
            <div className="import-url-row">
              <input
                className="input mono"
                placeholder="owner/name"
                value={source}
                onChange={(e) => setSource(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  void look(source);
                }}
              />
              <button
                className="btn"
                disabled={source.trim() === "" || busy !== null}
                onClick={() => void look(source)}
              >
                Look
              </button>
            </div>
            <p className="import-hint">
              A collection is a git repository of <code>.http</code> and{" "}
              <code>.grpc</code> files, so any public repository can be one:{" "}
              <code>owner/name</code>, <code>owner/name/sub/dir#branch</code>, or
              the URL from the address bar.
              {host === "browser"
                ? " The page reads it straight from raw.githubusercontent.com — nothing of ours in between. Private repositories need the desktop app, which is where a GitHub sign-in can be kept safely."
                : " Mándalo fetches it itself, through the same network checks as any request you send."}
            </p>
          </label>

          {busy && <p className="import-status">{busy}</p>}
          {error && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error}</span>
            </div>
          )}

          {review && (
            <>
              <div className="notice notice-wrap">
                <Layers size={13} />
                <span className="notice-text">
                  {review.origin.label}
                  {review.origin.commit
                    ? ` at ${review.origin.commit.slice(0, 8)}`
                    : ""}{" "}
                  — it opens read-only, and nothing in it has run or will run
                  until you send a request yourself.
                </span>
              </div>
              <Review review={review} />
            </>
          )}

          <div className="modal-actions">
            <button className="btn-ghost" onClick={close}>
              Cancel
            </button>
            <button
              className="btn btn-primary"
              disabled={review === null || busy !== null}
              onClick={() => void adopt()}
            >
              <Check size={13} />
              Open read-only
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
