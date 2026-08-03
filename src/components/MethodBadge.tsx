import type { RequestDraft } from "../lib/draft";

const CLASS: Record<string, string> = {
  GET: "badge-get",
  POST: "badge-post",
  PUT: "badge-put",
  PATCH: "badge-patch",
  DELETE: "badge-delete",
  HEAD: "badge-head",
  OPTIONS: "badge-head",
  GQL: "badge-gql",
  gRPC: "badge-grpc",
};

export function badgeLabel(draft: Pick<RequestDraft, "kind" | "method">): string {
  if (draft.kind === "graphql") return "GQL";
  if (draft.kind === "grpc") return "gRPC";
  return draft.method;
}

export function MethodBadge({ draft }: { draft: Pick<RequestDraft, "kind" | "method"> }) {
  const label = badgeLabel(draft);
  return <span className={`badge ${CLASS[label] ?? "badge-head"}`}>{label}</span>;
}
