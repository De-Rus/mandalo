import type { Kind } from "../lib/api";

const CLASS: Record<string, string> = {
  GET: "badge-get",
  POST: "badge-post",
  PUT: "badge-put",
  PATCH: "badge-patch",
  DELETE: "badge-delete",
  HEAD: "badge-head",
  OPTIONS: "badge-options",
  GQL: "badge-gql",
  gRPC: "badge-grpc",
  WS: "badge-ws",
  SSE: "badge-sse",
  MQTT: "badge-mqtt",
};

interface Labelled {
  kind: Kind;
  method: string;
}

/**
 * The tree gives the method the width of a twisty plus an icon, so a folder and
 * a request line their names up. The long verbs do not fit that span, and a
 * clipped word is worse than a short one everybody already reads.
 */
const SHORT: Record<string, string> = {
  DELETE: "DEL",
  OPTIONS: "OPT",
  PATCH: "PATCH",
};

export function badgeLabel(item: Labelled): string {
  if (item.kind === "graphql") return "GQL";
  if (item.kind === "grpc") return "gRPC";
  if (item.kind === "websocket") return "WS";
  if (item.kind === "sse") return "SSE";
  if (item.kind === "mqtt") return "MQTT";
  return item.method;
}

export function methodClass(item: Labelled): string {
  return CLASS[badgeLabel(item)] ?? "badge-head";
}

export function MethodBadge({ item, tree }: { item: Labelled; tree?: boolean }) {
  const label = badgeLabel(item);
  const shown = tree ? (SHORT[label] ?? label) : label;
  return (
    <span
      className={`badge ${methodClass(item)}${tree ? " badge-tree" : ""}`}
      title={tree && shown !== label ? label : undefined}
    >
      {shown}
    </span>
  );
}
