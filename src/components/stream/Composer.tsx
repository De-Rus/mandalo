import { useState } from "react";
import { uid } from "../../lib/draft";
import { prettyJson } from "../../lib/format";
import type { Outgoing, SavedMessage, StreamKind } from "../../lib/stream";
import { Close, Plus, Send } from "../Icons";
import { PromptModal } from "../PromptModal";

const QOS = [0, 1, 2];

function summarize(message: Outgoing): string {
  switch (message.kind) {
    case "text":
      return message.text;
    case "binary":
      return `${message.base64.length} base64 chars`;
    case "publish":
      return `${message.topic} · ${message.payload}`;
    case "subscribe":
      return `subscribe ${message.topic}`;
    case "unsubscribe":
      return `unsubscribe ${message.topic}`;
    case "ping":
      return "ping";
  }
}

interface Props {
  kind: StreamKind;
  connected: boolean;
  sending: boolean;
  subscriptions: string[];
  saved: SavedMessage[];
  onSend: (message: Outgoing) => void;
  onSaveMessage: (message: SavedMessage) => void;
  onForgetMessage: (id: string) => void;
}

function SavedRow({
  saved,
  connected,
  onSend,
  onForget,
}: {
  saved: SavedMessage[];
  connected: boolean;
  onSend: (message: Outgoing) => void;
  onForget: (id: string) => void;
}) {
  if (saved.length === 0) return null;
  return (
    <div className="composer-saved">
      <span className="composer-label">Saved</span>
      {saved.map((message) => (
        <span className="saved-chip" key={message.id}>
          <button
            className="saved-chip-send"
            disabled={!connected}
            title={summarize(message.message)}
            onClick={() => onSend(message.message)}
          >
            {message.name}
          </button>
          <button
            className="saved-chip-drop"
            aria-label={`Forget ${message.name}`}
            onClick={() => onForget(message.id)}
          >
            <Close size={9} />
          </button>
        </span>
      ))}
    </div>
  );
}

export function Composer({
  kind,
  connected,
  sending,
  subscriptions,
  saved,
  onSend,
  onSaveMessage,
  onForgetMessage,
}: Props) {
  const [text, setText] = useState("");
  const [topic, setTopic] = useState("");
  const [qos, setQos] = useState(0);
  const [retain, setRetain] = useState(false);
  const [subTopic, setSubTopic] = useState("");
  const [subQos, setSubQos] = useState(0);
  const [naming, setNaming] = useState<Outgoing | null>(null);

  if (kind === "sse")
    return (
      <div className="composer composer-readonly">
        <SavedRow
          saved={saved}
          connected={connected}
          onSend={onSend}
          onForget={onForgetMessage}
        />
        <p className="composer-note">
          Server-sent events only travel from the server to the client — there is
          nothing to send on this stream.
        </p>
      </div>
    );

  const current: Outgoing | null =
    kind === "mqtt"
      ? topic.trim() === ""
        ? null
        : { kind: "publish", topic: topic.trim(), payload: text, qos, retain }
      : text === ""
        ? null
        : { kind: "text", text };

  const save = () => {
    if (current) setNaming(current);
  };

  const send = () => {
    if (!current || !connected) return;
    onSend(current);
  };

  return (
    <div className="composer">
      {naming && (
        <PromptModal
          title="Save this message"
          label="Name"
          initial={summarize(naming).slice(0, 40)}
          confirmLabel="Save"
          onSubmit={(name) => onSaveMessage({ id: uid(), name, message: naming })}
          onClose={() => setNaming(null)}
        />
      )}
      <SavedRow
        saved={saved}
        connected={connected}
        onSend={onSend}
        onForget={onForgetMessage}
      />

      {kind === "mqtt" && (
        <div className="composer-line">
          <input
            className="input mono composer-topic"
            placeholder="Topic to publish to"
            aria-label="Publish topic"
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
          />
          <span className="composer-field">
            <span className="composer-label">QoS</span>
            <select
              className="select"
              aria-label="QoS"
              value={qos}
              onChange={(e) => setQos(Number(e.target.value))}
            >
              {QOS.map((q) => (
                <option key={q} value={q}>
                  {q}
                </option>
              ))}
            </select>
          </span>
          <label className="composer-check">
            <input
              type="checkbox"
              checked={retain}
              aria-label="Retain"
              onChange={(e) => setRetain(e.target.checked)}
            />
            Retain
          </label>
        </div>
      )}

      <div className="composer-line composer-line-grow">
        <textarea
          className="textarea mono composer-body"
          placeholder={
            kind === "mqtt"
              ? "Payload to publish"
              : "Message to send — ⌘⏎ sends, ⇧⏎ makes a new line"
          }
          aria-label="Message body"
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              send();
            }
          }}
        />
        <div className="composer-actions">
          <button
            className="btn btn-primary"
            disabled={!connected || current === null || sending}
            title={connected ? "Send (⌘⏎)" : "Connect first"}
            onClick={send}
          >
            <Send size={13} />
            {kind === "mqtt" ? "Publish" : "Send"}
          </button>
          <button
            className="btn btn-sm"
            disabled={current === null}
            title="Save this message with the request"
            onClick={save}
          >
            <Plus size={11} />
            Save
          </button>
          {kind === "websocket" && (
            <button
              className="btn btn-sm"
              disabled={text.trim() === ""}
              title="Pretty-print the JSON in the box"
              onClick={() => setText((t) => prettyJson(t) ?? t)}
            >
              Format
            </button>
          )}
        </div>
      </div>

      {kind === "mqtt" && (
        <div className="composer-line composer-subs">
          <input
            className="input mono composer-topic"
            placeholder="Topic filter to subscribe to — sensors/#"
            aria-label="Subscribe topic"
            value={subTopic}
            onChange={(e) => setSubTopic(e.target.value)}
          />
          <span className="composer-field">
            <span className="composer-label">QoS</span>
            <select
              className="select"
              aria-label="Subscribe QoS"
              value={subQos}
              onChange={(e) => setSubQos(Number(e.target.value))}
            >
              {QOS.map((q) => (
                <option key={q} value={q}>
                  {q}
                </option>
              ))}
            </select>
          </span>
          <button
            className="btn btn-sm"
            disabled={!connected || subTopic.trim() === ""}
            onClick={() =>
              onSend({ kind: "subscribe", topic: subTopic.trim(), qos: subQos })
            }
          >
            Subscribe
          </button>
          <div className="sub-chips">
            {subscriptions.map((sub) => (
              <span className="saved-chip" key={sub}>
                <span className="saved-chip-send mono">{sub}</span>
                <button
                  className="saved-chip-drop"
                  aria-label={`Unsubscribe from ${sub}`}
                  disabled={!connected}
                  onClick={() => onSend({ kind: "unsubscribe", topic: sub })}
                >
                  <Close size={9} />
                </button>
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
