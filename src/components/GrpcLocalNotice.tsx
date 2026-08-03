import type { RequestDraft } from "../lib/draft";
import { hasNativeGrpc, isLoopback } from "../lib/host";
import { previewResolve } from "../lib/vars";
import { Warn } from "./Icons";

interface GrpcLocalNoticeProps {
  draft: RequestDraft;
  vars: Record<string, string>;
}

export function GrpcLocalNotice({ draft, vars }: GrpcLocalNoticeProps) {
  if (draft.kind !== "grpc" || hasNativeGrpc()) return null;
  if (!isLoopback(previewResolve(draft.url, vars))) return null;
  return (
    <div className="notice notice-wrap grpc-local-notice" role="note">
      <Warn size={13} />
      <span className="notice-text">
        This request points at your own machine: the hosted mock is a Cloudflare
        Worker and cannot serve gRPC, which needs HTTP/2 trailers. Run{" "}
        <code>make mock-api</code> for the local mock on port 50051, or open the
        request in the desktop app.
      </span>
    </div>
  );
}
