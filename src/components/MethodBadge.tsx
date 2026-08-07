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

export function MethodBadge({ item }: { item: Labelled }) {
  return <span className={`badge ${methodClass(item)}`}>{badgeLabel(item)}</span>;
}
