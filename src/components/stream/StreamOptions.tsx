import {
  emptySubscription,
  type StreamDraft,
  type SubscriptionRow,
} from "../../lib/draft";
import type { MqttVersion, StreamKind } from "../../lib/stream";
import { Close, Plus } from "../Icons";

interface Props {
  kind: StreamKind;
  stream: StreamDraft;
  onChange: (stream: StreamDraft) => void;
}

function Row({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="settings-row stream-option">
      <span className="settings-row-head">{label}</span>
      <div className="stream-option-body">
        {children}
        {hint && <p className="settings-hint">{hint}</p>}
      </div>
    </div>
  );
}

function Number_({
  value,
  placeholder,
  onChange,
  label,
}: {
  value: string;
  placeholder: string;
  label: string;
  onChange: (value: string) => void;
}) {
  return (
    <input
      className="input mono stream-number"
      inputMode="numeric"
      aria-label={label}
      placeholder={placeholder}
      value={value}
      onChange={(e) => onChange(e.target.value.replace(/[^0-9]/g, ""))}
    />
  );
}

function Subscriptions({
  rows,
  onChange,
}: {
  rows: SubscriptionRow[];
  onChange: (rows: SubscriptionRow[]) => void;
}) {
  const patch = (id: string, part: Partial<SubscriptionRow>) =>
    onChange(rows.map((row) => (row.id === id ? { ...row, ...part } : row)));
  return (
    <div className="sub-editor">
      {rows.map((row) => (
        <div className="sub-editor-row" key={row.id}>
          <input
            className="input mono"
            placeholder="sensors/#"
            aria-label="Topic filter"
            value={row.topic}
            onChange={(e) => patch(row.id, { topic: e.target.value })}
          />
          <select
            className="select"
            aria-label="Subscription QoS"
            value={row.qos}
            onChange={(e) => patch(row.id, { qos: Number(e.target.value) })}
          >
            {[0, 1, 2].map((q) => (
              <option key={q} value={q}>
                QoS {q}
              </option>
            ))}
          </select>
          <button
            className="btn-ghost btn-icon"
            aria-label="Remove subscription"
            disabled={rows.length === 1}
            onClick={() => onChange(rows.filter((r) => r.id !== row.id))}
          >
            <Close size={11} />
          </button>
        </div>
      ))}
      <button
        className="btn btn-sm"
        onClick={() => onChange([...rows, emptySubscription()])}
      >
        <Plus size={11} />
        Add subscription
      </button>
    </div>
  );
}

/**
 * Everything the connection itself is configured by. These are the fields that
 * decide what happens when the socket drops, which is the whole difference
 * between a request and a stream.
 */
export function StreamOptions({ kind, stream, onChange }: Props) {
  const patch = (part: Partial<StreamDraft>) => onChange({ ...stream, ...part });

  return (
    <div className="settings-list">
      {kind === "websocket" && (
        <>
          <Row
            label="Subprotocols"
            hint="One per line. The server picks one; the negotiated name shows next to the connection state."
          >
            <textarea
              className="textarea mono stream-subprotocols"
              aria-label="Subprotocols"
              placeholder="v2&#10;chat"
              value={stream.subprotocols}
              onChange={(e) => patch({ subprotocols: e.target.value })}
            />
          </Row>
          <Row label="Ping interval" hint="Milliseconds between client pings. Empty means none.">
            <Number_
              label="Ping interval in milliseconds"
              placeholder="30000"
              value={stream.pingIntervalMs}
              onChange={(pingIntervalMs) => patch({ pingIntervalMs })}
            />
          </Row>
        </>
      )}

      {kind === "sse" && (
        <Row
          label="Last-Event-ID"
          hint="Where to resume from on the first connect. After that the stream tracks the server's own ids."
        >
          <input
            className="input mono"
            aria-label="Last event id"
            placeholder="42"
            value={stream.lastEventId}
            onChange={(e) => patch({ lastEventId: e.target.value })}
          />
        </Row>
      )}

      {kind === "mqtt" && (
        <>
          <Row label="Client ID" hint="Empty means the broker or the client picks one.">
            <input
              className="input mono"
              aria-label="Client id"
              placeholder="mandalo-1"
              value={stream.clientId}
              onChange={(e) => patch({ clientId: e.target.value })}
            />
          </Row>
          <Row label="Credentials">
            <div className="stream-pair">
              <input
                className="input"
                aria-label="MQTT user name"
                placeholder="User name"
                value={stream.username}
                onChange={(e) => patch({ username: e.target.value })}
              />
              <input
                className="input"
                type="password"
                aria-label="MQTT password"
                placeholder="Password"
                value={stream.password}
                onChange={(e) => patch({ password: e.target.value })}
              />
            </div>
          </Row>
          <Row label="Session">
            <div className="stream-pair">
              <label className="composer-check">
                <input
                  type="checkbox"
                  aria-label="Clean session"
                  checked={stream.cleanSession}
                  onChange={(e) => patch({ cleanSession: e.target.checked })}
                />
                Clean session
              </label>
              <select
                className="select"
                aria-label="Protocol version"
                value={stream.protocolVersion}
                onChange={(e) =>
                  patch({ protocolVersion: e.target.value as MqttVersion })
                }
              >
                <option value="3.1.1">MQTT 3.1.1</option>
                <option value="5">MQTT 5</option>
              </select>
              <Number_
                label="Keep alive in seconds"
                placeholder="60"
                value={stream.keepAliveSecs}
                onChange={(keepAliveSecs) => patch({ keepAliveSecs })}
              />
            </div>
          </Row>
          <Row
            label="Subscriptions"
            hint="Subscribed on connect, and again on every reconnect."
          >
            <Subscriptions
              rows={stream.subscriptions}
              onChange={(subscriptions) => patch({ subscriptions })}
            />
          </Row>
        </>
      )}

      <Row
        label="Reconnect"
        hint="A dropped connection is retried with a doubling backoff until the attempts run out."
      >
        <div className="stream-pair">
          <label className="composer-check">
            <input
              type="checkbox"
              aria-label="Reconnect automatically"
              checked={stream.autoReconnect}
              onChange={(e) => patch({ autoReconnect: e.target.checked })}
            />
            Reconnect automatically
          </label>
          <Number_
            label="Maximum reconnect attempts"
            placeholder="5"
            value={stream.maxReconnectAttempts}
            onChange={(maxReconnectAttempts) => patch({ maxReconnectAttempts })}
          />
        </div>
      </Row>

      <Row
        label="Limits"
        hint="Biggest message, how many events may wait to be drawn, and how long silence is allowed to last. Empty means the engine default."
      >
        <div className="stream-pair">
          <Number_
            label="Maximum message bytes"
            placeholder="1048576"
            value={stream.maxMessageBytes}
            onChange={(maxMessageBytes) => patch({ maxMessageBytes })}
          />
          <Number_
            label="Maximum buffered events"
            placeholder="1024"
            value={stream.maxBufferedEvents}
            onChange={(maxBufferedEvents) => patch({ maxBufferedEvents })}
          />
          <Number_
            label="Idle timeout in milliseconds"
            placeholder="300000"
            value={stream.idleTimeoutMs}
            onChange={(idleTimeoutMs) => patch({ idleTimeoutMs })}
          />
        </div>
      </Row>
    </div>
  );
}
