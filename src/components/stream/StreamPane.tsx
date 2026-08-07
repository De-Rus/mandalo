import { useEffect, useState } from "react";
import type { RequestDraft } from "../../lib/draft";
import type { Outgoing, SavedMessage, StreamKind } from "../../lib/stream";
import { STREAM_LABELS } from "../../lib/stream";
import { formatUptime } from "../../lib/streamLog";
import { useCollection } from "../../store/collection";
import { useStreams, type Phase, type StreamSession } from "../../store/stream";
import { Broadcast, Warn } from "../Icons";
import { Composer } from "./Composer";
import { MessageLog } from "./MessageLog";

const PHASE_LABEL: Record<Phase, string> = {
  closed: "Not connected",
  opening: "Opening",
  connecting: "Connecting",
  connected: "Connected",
  reconnecting: "Reconnecting",
};

const PHASE_TONE: Record<Phase, string> = {
  closed: "tone-muted",
  opening: "tone-info",
  connecting: "tone-info",
  connected: "tone-success",
  reconnecting: "tone-warn",
};

function useTick(active: boolean): number {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const timer = setInterval(() => setTick((t) => t + 1), 1000);
    return () => clearInterval(timer);
  }, [active]);
  return tick;
}

function Uptime({ since }: { since: number }) {
  useTick(true);
  return <span className="stream-uptime mono">{formatUptime(Date.now() - since)}</span>;
}

function ConnectionBar({
  kind,
  session,
  onCancel,
}: {
  kind: StreamKind;
  session: StreamSession | undefined;
  onCancel: () => void;
}) {
  const phase: Phase = session?.phase ?? "closed";
  const info = session?.info ?? null;
  return (
    <>
      <div className="response-head stream-head">
        <span className="stream-kind">
          <Broadcast size={13} />
          {STREAM_LABELS[kind]}
        </span>
        <span className={`chip ${PHASE_TONE[phase]}`}>
          <span className={`stream-dot stream-dot-${phase}`} />
          {PHASE_LABEL[phase]}
        </span>
        {phase === "connected" && session?.connectedAt !== null && session && (
          <span className="response-meta">
            <span>
              <span className="response-meta-label">up </span>
              <Uptime since={session.connectedAt as number} />
            </span>
          </span>
        )}
        <span className="header-spacer" />
        {info?.protocol && (
          <span className="stream-fact">
            <span className="response-meta-label">subprotocol </span>
            {info.protocol}
          </span>
        )}
        {info?.sessionPresent !== undefined && (
          <span className="stream-fact">
            {info.sessionPresent ? "session resumed" : "clean session"}
          </span>
        )}
        {info?.status !== undefined && (
          <span className="stream-fact">HTTP {info.status}</span>
        )}
      </div>
      {phase === "reconnecting" && session && (
        <div className="notice notice-wrap stream-notice">
          <span className="spinner" />
          <span className="notice-text">
            Reconnecting — attempt {session.attempt} in {session.delayMs} ms ·{" "}
            {session.reason}
          </span>
          <button className="btn-ghost" onClick={onCancel}>
            Stop trying
          </button>
        </div>
      )}
      {phase === "closed" && session?.lastError && (
        <div className="notice notice-error notice-wrap stream-notice">
          <Warn size={13} />
          <span className="notice-text">{session.lastError}</span>
        </div>
      )}
    </>
  );
}

interface Props {
  draft: RequestDraft;
  kind: StreamKind;
}

export function StreamPane({ draft, kind }: Props) {
  const session = useStreams((s) => s.sessions[draft.id]);
  const send = useStreams((s) => s.send);
  const disconnect = useStreams((s) => s.disconnect);
  const clearLog = useStreams((s) => s.clearLog);
  const updateActive = useCollection((s) => s.updateActive);

  const connected = session?.phase === "connected";

  const saveMessage = (message: SavedMessage) =>
    updateActive({
      stream: { ...draft.stream, messages: [...draft.stream.messages, message] },
    });

  const forgetMessage = (id: string) =>
    updateActive({
      stream: {
        ...draft.stream,
        messages: draft.stream.messages.filter((m) => m.id !== id),
      },
    });

  const onSend = (message: Outgoing) => void send(draft.id, message);

  return (
    <section className="response stream-pane">
      <ConnectionBar
        kind={kind}
        session={session}
        onCancel={() => void disconnect(draft.id)}
      />
      <MessageLog
        rows={session?.rows ?? []}
        overflow={session?.overflow ?? 0}
        onClear={() => clearLog(draft.id)}
      />
      <Composer
        kind={kind}
        connected={connected}
        sending={session?.sending ?? false}
        subscriptions={session?.subscriptions ?? []}
        saved={draft.stream.messages}
        onSend={onSend}
        onSaveMessage={saveMessage}
        onForgetMessage={forgetMessage}
      />
    </section>
  );
}
