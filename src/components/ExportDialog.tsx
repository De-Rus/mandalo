import { useEffect, useMemo, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import {
  defaultWorkspaceDir,
  errorMessage,
  planExport,
  runExport,
  workspaceShare,
  type CollectionNode,
  type ExportPlan,
  type ExportSelection,
  type FolderNode,
  type ShareFormat,
} from "../lib/api";
import { formatBytes } from "../lib/format";
import { useCollection } from "../store/collection";
import { useEnv } from "../store/env";
import { toast } from "../store/toast";
import { useModalGuard } from "../store/ui";
import { Close, Export, Warn } from "./Icons";

const JSON_FILTER = [{ name: "JSON", extensions: ["json"] }];

function workspaceDir(): Promise<string> {
  const active = useCollection.getState().workspace;
  return active ? Promise.resolve(active) : defaultWorkspaceDir();
}

interface Picked {
  collections: string[];
  environments: string[];
}

function selectionOf(picked: Picked, format: ShareFormat): ExportSelection | undefined {
  if (format === "postman") {
    if (picked.collections.length === 1 && picked.environments.length === 0) {
      return {
        collections: [{ slug: picked.collections[0]! }],
        environments: [],
      };
    }
    if (picked.environments.length === 1 && picked.collections.length === 0) {
      return {
        collections: [],
        environments: [picked.environments[0]!],
      };
    }
    return undefined;
  }
  return {
    collections: picked.collections.map((slug) => ({ slug })),
    environments: picked.environments,
  };
}

function postmanReady(picked: Picked): boolean {
  const oneCollection =
    picked.collections.length === 1 && picked.environments.length === 0;
  const oneEnv =
    picked.environments.length === 1 && picked.collections.length === 0;
  return oneCollection || oneEnv;
}

function FormatPicker({
  format,
  onChange,
}: {
  format: ShareFormat;
  onChange: (next: ShareFormat) => void;
}) {
  return (
    <div className="report-section">
      <span className="field-label">Format</span>
      <div className="export-format-row">
        <button
          type="button"
          role="radio"
          aria-checked={format === "native"}
          className={`sync-output-card ${format === "native" ? "sync-output-card-on" : ""}`}
          onClick={() => onChange("native")}
        >
          <strong>Mándalo bundle</strong>
          <span className="sync-output-desc">
            Whole workspace — as many collections and environments as you tick.
          </span>
        </button>
        <button
          type="button"
          role="radio"
          aria-checked={format === "postman"}
          className={`sync-output-card ${format === "postman" ? "sync-output-card-on" : ""}`}
          onClick={() => onChange("postman")}
        >
          <strong>Postman</strong>
          <span className="sync-output-desc">
            One file — pick a single collection, or a single environment.
          </span>
        </button>
      </div>
    </div>
  );
}

function BundleIncluded({
  plan,
  picked,
  onToggle,
}: {
  plan: ExportPlan;
  picked: Picked;
  onToggle: (next: Picked) => void;
}) {
  const { collections, environments, requestCount } = plan.included;
  const flip = (list: string[], value: string): string[] =>
    list.includes(value) ? list.filter((v) => v !== value) : [...list, value];

  return (
    <div className="report-section">
      <span className="field-label">Included</span>
      <p className="report-counts">
        {requestCount} requests · {collections.length} collections ·{" "}
        {environments.length} environments · {formatBytes(plan.bytes)} · Mándalo
        bundle
      </p>
      <ul className="report-list export-pick">
        {collections.map((c) => (
          <li key={c.slug}>
            <label className="export-pick-row">
              <input
                type="checkbox"
                className="checkbox"
                checked={picked.collections.includes(c.slug)}
                onChange={() =>
                  onToggle({
                    ...picked,
                    collections: flip(picked.collections, c.slug),
                  })
                }
              />
              <span>
                {c.name} — {c.requests.length} requests
              </span>
            </label>
          </li>
        ))}
        {environments.map((name) => (
          <li key={`env-${name}`}>
            <label className="export-pick-row">
              <input
                type="checkbox"
                className="checkbox"
                checked={picked.environments.includes(name)}
                onChange={() =>
                  onToggle({
                    ...picked,
                    environments: flip(picked.environments, name),
                  })
                }
              />
              <span>{name} (environment)</span>
            </label>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PostmanPick({
  collections,
  environments,
  picked,
  onToggle,
}: {
  collections: { slug: string; name: string; requests: number }[];
  environments: string[];
  picked: Picked;
  onToggle: (next: Picked) => void;
}) {
  const selectedCollection =
    picked.collections.length === 1 && picked.environments.length === 0
      ? picked.collections[0]!
      : null;
  const selectedEnv =
    picked.environments.length === 1 && picked.collections.length === 0
      ? picked.environments[0]!
      : null;

  return (
    <div className="report-section">
      <span className="field-label">Export</span>
      <p className="report-counts">
        Postman writes one JSON file. Choose one collection, or one environment.
      </p>
      <ul className="report-list export-pick">
        {collections.map((c) => (
          <li key={c.slug}>
            <label className="export-pick-row">
              <input
                type="radio"
                className="checkbox"
                name="postman-export-target"
                checked={selectedCollection === c.slug}
                onChange={() =>
                  onToggle({ collections: [c.slug], environments: [] })
                }
              />
              <span>
                {c.name} — {c.requests} requests
              </span>
            </label>
          </li>
        ))}
        {environments.map((name) => (
          <li key={`env-${name}`}>
            <label className="export-pick-row">
              <input
                type="radio"
                className="checkbox"
                name="postman-export-target"
                checked={selectedEnv === name}
                onChange={() =>
                  onToggle({ collections: [], environments: [name] })
                }
              />
              <span>{name} (environment)</span>
            </label>
          </li>
        ))}
      </ul>
    </div>
  );
}

function Excluded({ plan }: { plan: ExportPlan }) {
  const lines: string[] = [];
  if (plan.excluded.secretValues > 0) {
    lines.push(
      `${plan.excluded.secretValues} secret value(s) stay on this machine`,
    );
  }
  if (plan.excluded.localValues > 0) {
    lines.push(
      `${plan.excluded.localValues} local value(s) stay on this machine`,
    );
  }
  if (plan.excluded.collections.length > 0) {
    lines.push(`Left out: ${plan.excluded.collections.join(", ")}`);
  }
  if (plan.excluded.requests > 0) {
    lines.push(`${plan.excluded.requests} request(s) left out of the pick`);
  }
  if (plan.excluded.environments.length > 0) {
    lines.push(
      `Environments left out: ${plan.excluded.environments.join(", ")}`,
    );
  }
  if (lines.length === 0) return null;
  return (
    <div className="report-section">
      <span className="field-label">Not included</span>
      <ul className="report-list">
        {lines.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ul>
    </div>
  );
}

function countRequests(node: CollectionNode | FolderNode): number {
  let n = node.requests.length;
  for (const folder of node.folders) n += countRequests(folder);
  return n;
}

export function ExportDialog({ onClose }: { onClose: () => void }) {
  useModalGuard();
  const tree = useCollection((s) => s.tree);
  const envs = useEnv((s) => s.envs);
  const envNames = useMemo(() => envs.map((e) => e.name), [envs]);
  const [plan, setPlan] = useState<ExportPlan | null>(null);
  const [picked, setPicked] = useState<Picked | null>(null);
  const [format, setFormat] = useState<ShareFormat>("native");
  const [formatReady, setFormatReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const [writing, setWriting] = useState(false);

  const catalog = tree.collections.map((c) => ({
    slug: c.slug,
    name: c.name,
    requests: countRequests(c),
  }));

  useEffect(() => {
    let live = true;
    void workspaceDir()
      .then((dir) => workspaceShare(dir))
      .then((share) => {
        if (!live) return;
        const next = share?.format === "postman" ? "postman" : "native";
        setFormat(next);
        if (next === "postman") {
          const first = useCollection.getState().tree.collections[0];
          setPicked(
            first
              ? { collections: [first.slug], environments: [] }
              : { collections: [], environments: [] },
          );
        }
        setFormatReady(true);
      })
      .catch(() => {
        if (!live) return;
        setFormatReady(true);
      });
    return () => {
      live = false;
    };
  }, []);

  const selection = useMemo(
    () => (picked === null ? undefined : selectionOf(picked, format)),
    [picked, format],
  );
  const canPlan =
    formatReady &&
    (format === "native" || (picked !== null && postmanReady(picked)));

  useEffect(() => {
    if (!canPlan) return;
    let live = true;
    setError(null);
    setPlan(null);
    void workspaceDir()
      .then((dir) => planExport(dir, selection, format))
      .then(
        (next) => {
          if (!live) return;
          setPlan(next);
          if (format === "native") {
            setPicked((current) =>
              current ?? {
                collections: next.included.collections.map((c) => c.slug),
                environments: [...next.included.environments],
              },
            );
          }
        },
        (e) => live && setError(errorMessage(e)),
      );
    return () => {
      live = false;
    };
  }, [canPlan, format, selection]);

  const chooseFormat = (next: ShareFormat) => {
    setFormat(next);
    setPlan(null);
    setError(null);
    setAcknowledged(false);
    if (next === "postman") {
      const first = catalog[0];
      setPicked(
        first
          ? { collections: [first.slug], environments: [] }
          : { collections: [], environments: [] },
      );
    } else {
      setPicked(null);
    }
  };

  const write = async () => {
    if (!plan) return;
    setWriting(true);
    setError(null);
    try {
      const path = await save({
        defaultPath:
          format === "postman" ? "collection.json" : "mandalo-bundle.json",
        filters: JSON_FILTER,
      });
      if (!path) return;
      const receipt = await runExport(
        await workspaceDir(),
        plan.token,
        path,
        plan.blocked,
        selection,
        format,
      );
      toast(
        "success",
        `${receipt.requests} requests written to ${receipt.path}`,
      );
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWriting(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal modal-wide" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Export</h2>
          <button
            className="btn-ghost btn-icon"
            onClick={onClose}
            aria-label="Close"
          >
            <Close size={13} />
          </button>
        </div>
        <div className="modal-body">
          {formatReady && (
            <FormatPicker format={format} onChange={chooseFormat} />
          )}

          {error && (
            <div className="notice notice-error notice-wrap">
              <Warn size={13} />
              <span className="notice-text">{error}</span>
            </div>
          )}

          {format === "postman" && picked && (
            <PostmanPick
              collections={catalog}
              environments={envNames}
              picked={picked}
              onToggle={(next) => setPicked(next)}
            />
          )}

          {format === "postman" && catalog.length === 0 && envNames.length === 0 && (
            <p className="empty-line">
              Nothing to export yet — add a collection or an environment first.
            </p>
          )}

          {!plan && !error && canPlan && (
            <p className="import-status">Reading workspace…</p>
          )}

          {plan && format === "native" && picked && (
            <BundleIncluded
              plan={plan}
              picked={picked}
              onToggle={(next) => setPicked(next)}
            />
          )}

          {plan && (
            <>
              {format === "postman" && (
                <p className="report-counts">
                  {plan.included.requestCount} requests · {formatBytes(plan.bytes)}{" "}
                  · Postman
                </p>
              )}
              <Excluded plan={plan} />
              {plan.warnings.length > 0 && (
                <div className="report-section">
                  <span className="field-label">Warnings</span>
                  <ul className="report-list">
                    {plan.warnings.map((w, i) => (
                      <li key={i}>{w}</li>
                    ))}
                  </ul>
                </div>
              )}
              {plan.blocked && (
                <>
                  <div className="notice notice-error notice-wrap">
                    <Warn size={13} />
                    <span className="notice-text">
                      {plan.findings.length} value(s) look like credentials.
                      Anything you export leaves this machine.
                    </span>
                  </div>
                  <ul className="report-list report-findings">
                    {plan.findings.map((finding, i) => (
                      <li key={i}>
                        <span className="finding-rule">{finding.rule}</span>
                        <span className="finding-path mono">
                          {finding.path}:{finding.line}
                        </span>
                        <span className="finding-excerpt mono">
                          {finding.excerpt}
                        </span>
                      </li>
                    ))}
                  </ul>
                  <label className="import-ack">
                    <input
                      className="checkbox"
                      type="checkbox"
                      aria-label="Write the findings to the file anyway"
                      checked={acknowledged}
                      onChange={(e) => setAcknowledged(e.target.checked)}
                    />
                    <span>
                      I have read every finding and want to write them to the
                      file anyway.
                    </span>
                  </label>
                </>
              )}
              <div className="modal-actions">
                <button className="btn-ghost" onClick={onClose}>
                  Cancel
                </button>
                <button
                  className="btn btn-primary"
                  disabled={writing || (plan.blocked && !acknowledged)}
                  onClick={() => void write()}
                >
                  <Export size={13} />
                  {plan.blocked ? "Export anyway…" : "Choose file and export…"}
                </button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
