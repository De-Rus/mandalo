import type { Auth } from "../api";
import type {
  ConnectionInfo,
  MessageMeta,
  Outgoing,
  Payload,
  StreamEvent,
  StreamKind,
  StreamSpec,
  StreamStatus,
} from "../stream";
import {
  connackError,
  encodeConnect,
  encodeDisconnect,
  encodePingreq,
  encodePuback,
  encodePublish,
  encodeSubscribe,
  encodeUnsubscribe,
  MQTT_SUBPROTOCOL,
  PacketReader,
  randomClientId,
} from "./mqttws";
import { SseParser, SseTooBig } from "./sseParser";
import { base64Utf8, interpolate } from "./send";

export const E_STREAM_CONNECT = "E_STREAM_CONNECT";
export const E_STREAM_PROTOCOL = "E_STREAM_PROTOCOL";
export const E_STREAM_AUTH = "E_STREAM_AUTH";
export const E_STREAM_LIMIT = "E_STREAM_LIMIT";
export const E_STREAM_IDLE = "E_STREAM_IDLE";
/** Something the engine can do and a web page provably cannot. */
export const E_BROWSER_LIMIT = "E_BROWSER_LIMIT";

export const NO_WS_HEADERS =
  "A browser cannot put headers on a WebSocket handshake — the WebSocket API has no place for them, so this connection would silently go out without them. Move the credential into a query parameter (API key · Query), or open this request in the Mándalo desktop app, which owns a real socket.";

export const NO_WS_PING =
  "A browser cannot send WebSocket ping frames — the API exposes no control frames. The desktop app can; here, send an application-level keepalive message instead.";

export const NO_RAW_MQTT =
  "A browser has no TCP socket, so mqtt:// and mqtts:// cannot be reached from a web page. Point this request at the broker's WebSocket listener (ws:// or wss://), or open it in the Mándalo desktop app.";

export const NO_MQTT5 =
  "MQTT 5 is not implemented over the browser WebSocket transport. Use protocol version 3.1.1 here, or open this request in the Mándalo desktop app.";

export const NO_QOS2 =
  "QoS 2 is not implemented over the browser WebSocket transport — only QoS 0 and 1 are. Use the Mándalo desktop app for exactly-once delivery.";

const DEFAULTS = {
  maxMessageBytes: 1024 * 1024,
  maxReconnectAttempts: 5,
  connectTimeoutMs: 15_000,
  idleTimeoutMs: 300_000,
  backoffBaseMs: 500,
  backoffMaxMs: 30_000,
};

/** A connection that stayed up this long makes the reconnect budget start over. */
const HEALTHY_SESSION_MS = 30_000;

type Emit = (event: StreamEvent) => void;

interface Runner {
  id: string;
  kind: StreamKind;
  url: string;
  open: boolean;
  send: (message: Outgoing) => Promise<void>;
  close: () => Promise<void>;
}

const runners = new Map<string, Runner>();

function now(): number {
  return Date.now();
}

function newId(): string {
  return `web-${Math.random().toString(36).slice(2, 10)}${Date.now().toString(36)}`;
}

export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export function base64ToBytes(base64: string): Uint8Array {
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
  return out;
}

/** Bytes stay bytes; text is produced only when the payload really is UTF-8. */
export function payloadOf(bytes: Uint8Array): Payload {
  try {
    return { kind: "text", text: new TextDecoder("utf-8", { fatal: true }).decode(bytes) };
  } catch {
    return { kind: "binary", base64: bytesToBase64(bytes), bytes: bytes.length };
  }
}

interface Resolved {
  url: string;
  headers: [string, string][];
  query: [string, string][];
  limits: typeof DEFAULTS;
}

function applyAuth(
  auth: Auth | undefined,
  vars: Record<string, string>,
): { headers: [string, string][]; query: [string, string][] } {
  const headers: [string, string][] = [];
  const query: [string, string][] = [];
  if (!auth || auth.type === "none") return { headers, query };
  switch (auth.type) {
    case "bearer":
      headers.push(["Authorization", `Bearer ${interpolate(auth.token, vars)}`]);
      break;
    case "basic":
      headers.push([
        "Authorization",
        `Basic ${base64Utf8(
          `${interpolate(auth.username, vars)}:${interpolate(auth.password, vars)}`,
        )}`,
      ]);
      break;
    case "apikey": {
      const key = interpolate(auth.key, vars);
      const value = interpolate(auth.value, vars);
      if (auth.placement === "header") headers.push([key, value]);
      else if (auth.placement === "query") query.push([key, value]);
      else throw new Error(`unsupported apikey placement: ${auth.placement}`);
      break;
    }
  }
  return { headers, query };
}

export function resolveSpec(spec: StreamSpec): Resolved {
  const vars = spec.vars ?? {};
  let headers: [string, string][] = (spec.headers ?? []).map(
    ([k, v]) => [interpolate(k, vars), interpolate(v, vars)] as [string, string],
  );
  const auth = applyAuth(spec.auth, vars);
  for (const [name] of auth.headers)
    headers = headers.filter(([k]) => k.toLowerCase() !== name.toLowerCase());
  headers = [...headers, ...auth.headers];

  let url = interpolate(spec.url, vars);
  if (auth.query.length > 0) {
    const parsed = new URL(url);
    for (const [k, v] of auth.query) parsed.searchParams.append(k, v);
    url = parsed.toString();
  }
  return {
    url,
    headers,
    query: auth.query,
    limits: { ...DEFAULTS, ...(spec.limits ?? {}) } as typeof DEFAULTS,
  };
}

function schemeOf(url: string): string {
  return new URL(url).protocol.replace(":", "").toLowerCase();
}

function backoff(limits: typeof DEFAULTS, attempt: number): number {
  const shift = Math.min(Math.max(attempt - 1, 0), 16);
  return Math.max(1, Math.min(limits.backoffBaseMs * 2 ** shift, limits.backoffMaxMs));
}

class Lifecycle {
  attempt = 0;
  closedByUser = false;
  private timer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    readonly emit: Emit,
    readonly limits: typeof DEFAULTS,
    readonly autoReconnect: boolean,
  ) {}

  /** `true` when a retry was scheduled, `false` when the stream is finished. */
  retry(
    reason: string,
    upSince: number,
    again: () => void,
    delay?: (attempt: number) => number,
  ): boolean {
    if (this.closedByUser) return false;
    if (now() - upSince >= HEALTHY_SESSION_MS) this.attempt = 0;
    if (!this.autoReconnect || this.attempt >= this.limits.maxReconnectAttempts) {
      const detail = this.autoReconnect
        ? `${reason} — gave up after ${this.limits.maxReconnectAttempts} attempts`
        : reason;
      this.emit({ type: "disconnected", at: now(), reason: detail });
      return false;
    }
    this.attempt += 1;
    const delayMs = (delay ?? ((a: number) => backoff(this.limits, a)))(this.attempt);
    this.emit({
      type: "reconnecting",
      at: now(),
      attempt: this.attempt,
      delayMs,
      reason,
    });
    this.timer = setTimeout(() => {
      this.timer = null;
      if (!this.closedByUser) again();
    }, delayMs);
    return true;
  }

  stop(): void {
    this.closedByUser = true;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = null;
  }
}

function openWebSocket(
  id: string,
  spec: StreamSpec,
  resolved: Resolved,
  emit: Emit,
): Runner {
  if (resolved.headers.length > 0) throw new Error(NO_WS_HEADERS);
  const scheme = schemeOf(resolved.url);
  if (scheme !== "ws" && scheme !== "wss")
    throw new Error(
      `a websocket url must start with ws:// or wss:// — ${scheme}:// is not one`,
    );
  if ((spec.ws?.pingIntervalMs ?? 0) > 0) throw new Error(NO_WS_PING);

  const subprotocols = spec.ws?.subprotocols ?? [];
  const life = new Lifecycle(emit, resolved.limits, spec.ws?.autoReconnect ?? false);
  let socket: WebSocket | null = null;

  const connect = () => {
    emit({ type: "connecting", at: now(), url: resolved.url });
    const upSince = now();
    let settled = false;
    const ws =
      subprotocols.length > 0
        ? new WebSocket(resolved.url, subprotocols)
        : new WebSocket(resolved.url);
    ws.binaryType = "arraybuffer";
    socket = ws;

    ws.onopen = () => {
      settled = true;
      const info: ConnectionInfo = { url: resolved.url };
      if (ws.protocol !== "") info.protocol = ws.protocol;
      emit({ type: "connected", at: now(), info });
    };
    ws.onmessage = (event) => {
      const bytes =
        typeof event.data === "string"
          ? new TextEncoder().encode(event.data)
          : new Uint8Array(event.data as ArrayBuffer);
      if (bytes.length > resolved.limits.maxMessageBytes) {
        emit({
          type: "error",
          at: now(),
          message: `the message is ${bytes.length} bytes, over the ${resolved.limits.maxMessageBytes} byte limit for this stream`,
          code: E_STREAM_LIMIT,
        });
        return;
      }
      emit({
        type: "message",
        at: now(),
        direction: "incoming",
        payload: payloadOf(bytes),
        meta: { frame: typeof event.data === "string" ? "text" : "binary" },
      });
    };
    ws.onerror = () => {
      if (settled) return;
      emit({
        type: "error",
        at: now(),
        message: `cannot reach ${resolved.url} — check the host, the port, that the server is running, and that it allows this origin`,
        code: E_STREAM_CONNECT,
      });
    };
    ws.onclose = (event) => {
      socket = null;
      if (life.closedByUser) {
        emit({ type: "disconnected", at: now(), code: event.code, reason: "closed by you" });
        return;
      }
      const reason =
        event.reason !== ""
          ? event.reason
          : settled
            ? "the server closed the connection"
            : "the connection was never established";
      // A close frame is the server saying it is done, so it ends the stream
      // whatever the reconnect policy is — only a connection that broke without
      // one is worth retrying. The engine draws the same line.
      if (event.wasClean && settled) {
        emit({ type: "disconnected", at: now(), code: event.code, reason });
        return;
      }
      if (!life.retry(reason, upSince, connect) && !life.autoReconnect)
        emit({ type: "disconnected", at: now(), code: event.code, reason });
    };
  };

  connect();

  return {
    id,
    kind: "websocket",
    url: resolved.url,
    open: true,
    send: async (message) => {
      if (!socket || socket.readyState !== WebSocket.OPEN)
        throw new Error("the websocket is not open — wait for the connected event");
      let payload: Payload;
      let meta: MessageMeta;
      switch (message.kind) {
        case "text":
          socket.send(message.text);
          payload = { kind: "text", text: message.text };
          meta = { frame: "text" };
          break;
        case "binary": {
          const bytes = base64ToBytes(message.base64);
          socket.send(bytes);
          payload = { kind: "binary", base64: message.base64, bytes: bytes.length };
          meta = { frame: "binary" };
          break;
        }
        case "ping":
          throw new Error(NO_WS_PING);
        default:
          throw new Error(
            `a websocket cannot send ${message.kind} — that is an mqtt operation`,
          );
      }
      emit({ type: "message", at: now(), direction: "outgoing", payload, meta });
    },
    close: async () => {
      life.stop();
      if (socket && socket.readyState <= WebSocket.OPEN) socket.close(1000, "closed by the client");
      else emit({ type: "disconnected", at: now(), reason: "closed by you" });
    },
  };
}

function openSse(id: string, spec: StreamSpec, resolved: Resolved, emit: Emit): Runner {
  const scheme = schemeOf(resolved.url);
  if (scheme !== "http" && scheme !== "https")
    throw new Error(
      `server-sent events are plain HTTP — the url must start with http:// or https://, not ${scheme}://`,
    );

  const life = new Lifecycle(emit, resolved.limits, spec.sse?.autoReconnect ?? true);
  let lastEventId = spec.sse?.lastEventId ?? null;
  let retryMs: number | null = null;
  let abort: AbortController | null = null;

  const delayFor = (attempt: number) =>
    retryMs === null
      ? backoff(resolved.limits, attempt)
      : Math.min(
          Math.max(retryMs, resolved.limits.backoffBaseMs),
          resolved.limits.backoffMaxMs,
        );

  const connect = () => {
    emit({ type: "connecting", at: now(), url: resolved.url });
    const upSince = now();
    const controller = new AbortController();
    abort = controller;
    const headers = new Headers(resolved.headers);
    headers.set("Accept", "text/event-stream");
    headers.set("Cache-Control", "no-store");
    if (lastEventId !== null) headers.set("Last-Event-ID", lastEventId);

    void (async () => {
      let response: Response;
      try {
        response = await fetch(resolved.url, {
          headers,
          signal: controller.signal,
          cache: "no-store",
        });
      } catch (e) {
        if (controller.signal.aborted) return;
        const message = `cannot reach ${resolved.url}: ${
          e instanceof Error ? e.message : String(e)
        } — check the host, that the server is running, and that it allows this origin (a page is bound by CORS; the desktop app is not)`;
        emit({ type: "error", at: now(), message, code: E_STREAM_CONNECT });
        life.retry(message, upSince, connect, delayFor);
        return;
      }

      if (!response.ok) {
        const code =
          response.status === 401 || response.status === 403
            ? E_STREAM_AUTH
            : E_STREAM_CONNECT;
        const message =
          code === E_STREAM_AUTH
            ? `${resolved.url} rejected the credentials with HTTP ${response.status}`
            : `${resolved.url} answered HTTP ${response.status}, not an event stream`;
        emit({ type: "error", at: now(), message, code });
        if (response.status >= 500) life.retry(message, upSince, connect, delayFor);
        else emit({ type: "disconnected", at: now(), reason: message });
        return;
      }

      const contentType = (response.headers.get("content-type") ?? "")
        .split(";")[0]
        .trim()
        .toLowerCase();
      if (contentType !== "text/event-stream") {
        const message = `${resolved.url} answered with ${
          contentType === "" ? "no content type" : `"${contentType}"`
        }, not text/event-stream — this endpoint is not an event stream`;
        emit({ type: "error", at: now(), message, code: E_STREAM_PROTOCOL });
        emit({ type: "disconnected", at: now(), reason: message });
        return;
      }

      const headerPairs: [string, string][] = [];
      response.headers.forEach((value, key) => headerPairs.push([key, value]));
      emit({
        type: "connected",
        at: now(),
        info: {
          url: response.url === "" ? resolved.url : response.url,
          status: response.status,
          headers: headerPairs,
        },
      });

      const body = response.body;
      if (!body) {
        const message = "the browser gave no readable body for this event stream";
        emit({ type: "error", at: now(), message, code: E_STREAM_PROTOCOL });
        emit({ type: "disconnected", at: now(), reason: message });
        return;
      }

      const parser = new SseParser();
      const reader = body.getReader();
      for (;;) {
        let chunk: ReadableStreamReadResult<Uint8Array>;
        try {
          chunk = await reader.read();
        } catch (e) {
          if (controller.signal.aborted) return;
          const message = `the event stream from ${resolved.url} failed: ${
            e instanceof Error ? e.message : String(e)
          }`;
          emit({ type: "error", at: now(), message, code: E_STREAM_PROTOCOL });
          life.retry(message, upSince, connect, delayFor);
          return;
        }
        if (chunk.done) {
          if (controller.signal.aborted) return;
          life.retry("the server ended the event stream", upSince, connect, delayFor);
          return;
        }
        let frames;
        try {
          frames = parser.feed(chunk.value, resolved.limits.maxMessageBytes);
        } catch (e) {
          const message = e instanceof SseTooBig ? e.message : String(e);
          emit({ type: "error", at: now(), message, code: E_STREAM_LIMIT });
          emit({
            type: "disconnected",
            at: now(),
            reason: "an event was too big for this stream",
          });
          return;
        }
        if (parser.retryMs() !== null) retryMs = parser.retryMs();
        if (parser.lastId() !== null) lastEventId = parser.lastId();
        for (const frame of frames)
          emit({
            type: "message",
            at: now(),
            direction: "incoming",
            payload: { kind: "text", text: frame.data },
            meta: {
              frame: "event",
              ...(frame.event === null ? {} : { event: frame.event }),
              ...(frame.id === null ? {} : { id: frame.id }),
            },
          });
      }
    })();
  };

  connect();

  return {
    id,
    kind: "sse",
    url: resolved.url,
    open: true,
    send: async () => {
      throw new Error(
        "server-sent events only travel from the server to the client — there is nothing to send on this stream",
      );
    },
    close: async () => {
      life.stop();
      abort?.abort();
      emit({ type: "disconnected", at: now(), reason: "closed by you" });
    },
  };
}

function openMqtt(id: string, spec: StreamSpec, resolved: Resolved, emit: Emit): Runner {
  const scheme = schemeOf(resolved.url);
  if (scheme === "mqtt" || scheme === "mqtts") throw new Error(NO_RAW_MQTT);
  if (scheme !== "ws" && scheme !== "wss")
    throw new Error(
      `an mqtt url must start with ws:// or wss:// in the browser — ${scheme}:// is not one`,
    );
  const options = spec.mqtt ?? {};
  if ((options.protocolVersion ?? "3.1.1") !== "3.1.1") throw new Error(NO_MQTT5);
  for (const sub of options.subscriptions ?? [])
    if ((sub.qos ?? 0) > 1) throw new Error(NO_QOS2);

  const clientId = options.clientId ?? randomClientId();
  const keepAliveSecs = options.keepAliveSecs ?? 60;
  const life = new Lifecycle(emit, resolved.limits, true);
  let socket: WebSocket | null = null;
  let connected = false;
  let packetId = 0;
  let ping: ReturnType<typeof setInterval> | null = null;

  const nextPacketId = () => {
    packetId = (packetId % 65535) + 1;
    return packetId;
  };

  const connect = () => {
    emit({ type: "connecting", at: now(), url: resolved.url });
    const upSince = now();
    connected = false;
    const ws = new WebSocket(resolved.url, MQTT_SUBPROTOCOL);
    ws.binaryType = "arraybuffer";
    socket = ws;
    const reader = new PacketReader();

    ws.onopen = () => {
      ws.send(
        encodeConnect({
          clientId,
          cleanSession: options.cleanSession ?? true,
          keepAliveSecs,
          ...(options.username === undefined ? {} : { username: options.username }),
          ...(options.password === undefined ? {} : { password: options.password }),
        }),
      );
    };

    ws.onmessage = (event) => {
      const packets = reader.feed(new Uint8Array(event.data as ArrayBuffer));
      for (const packet of packets) {
        if (packet.type === "connack") {
          const refusal = connackError(packet.code);
          if (refusal !== null) {
            emit({ type: "error", at: now(), message: refusal, code: E_STREAM_AUTH });
            life.stop();
            ws.close();
            emit({ type: "disconnected", at: now(), reason: refusal });
            return;
          }
          connected = true;
          life.attempt = 0;
          emit({
            type: "connected",
            at: now(),
            info: {
              url: resolved.url,
              protocol: ws.protocol === "" ? undefined : ws.protocol,
              sessionPresent: packet.sessionPresent,
            },
          });
          const subscriptions = (options.subscriptions ?? []).filter(
            (s) => s.topic.trim() !== "",
          );
          if (subscriptions.length > 0)
            ws.send(
              encodeSubscribe(
                subscriptions.map((s) => ({ topic: s.topic, qos: s.qos ?? 0 })),
                nextPacketId(),
              ),
            );
          if (keepAliveSecs > 0)
            ping = setInterval(() => {
              if (ws.readyState === WebSocket.OPEN) ws.send(encodePingreq());
            }, keepAliveSecs * 1000);
        }
        if (packet.type === "publish") {
          if (packet.payload.length > resolved.limits.maxMessageBytes) {
            emit({
              type: "error",
              at: now(),
              message: `the message is ${packet.payload.length} bytes, over the ${resolved.limits.maxMessageBytes} byte limit for this stream`,
              code: E_STREAM_LIMIT,
            });
            continue;
          }
          if (packet.qos === 1 && packet.packetId !== null)
            ws.send(encodePuback(packet.packetId));
          emit({
            type: "message",
            at: now(),
            direction: "incoming",
            payload: payloadOf(packet.payload),
            meta: {
              topic: packet.topic,
              qos: packet.qos,
              retain: packet.retain,
              frame: "publish",
            },
          });
        }
      }
    };

    ws.onerror = () => {
      if (connected) return;
      emit({
        type: "error",
        at: now(),
        message: `cannot reach the broker at ${resolved.url} — check the host, the port, that its WebSocket listener is enabled, and that it allows this origin`,
        code: E_STREAM_CONNECT,
      });
    };

    ws.onclose = (event) => {
      socket = null;
      if (ping !== null) clearInterval(ping);
      ping = null;
      if (life.closedByUser) {
        emit({ type: "disconnected", at: now(), reason: "closed by you" });
        return;
      }
      const reason =
        event.reason !== ""
          ? event.reason
          : connected
            ? "the broker closed the connection"
            : "the broker was never reached";
      life.retry(reason, upSince, connect);
    };
  };

  connect();

  return {
    id,
    kind: "mqtt",
    url: resolved.url,
    open: true,
    send: async (message) => {
      if (!socket || !connected)
        throw new Error("the mqtt connection is not up yet — wait for the connected event");
      switch (message.kind) {
        case "publish": {
          const qos = message.qos ?? 0;
          if (qos > 1) throw new Error(NO_QOS2);
          socket.send(
            encodePublish(
              message.topic,
              message.payload,
              qos,
              message.retain ?? false,
              nextPacketId(),
            ),
          );
          emit({
            type: "message",
            at: now(),
            direction: "outgoing",
            payload: { kind: "text", text: message.payload },
            meta: {
              topic: message.topic,
              qos,
              retain: message.retain ?? false,
              frame: "publish",
            },
          });
          break;
        }
        case "subscribe": {
          const qos = message.qos ?? 0;
          if (qos > 1) throw new Error(NO_QOS2);
          socket.send(encodeSubscribe([{ topic: message.topic, qos }], nextPacketId()));
          emit({
            type: "message",
            at: now(),
            direction: "outgoing",
            payload: { kind: "text", text: `subscribe ${message.topic}` },
            meta: { topic: message.topic, qos, frame: "subscribe" },
          });
          break;
        }
        case "unsubscribe":
          socket.send(encodeUnsubscribe([message.topic], nextPacketId()));
          emit({
            type: "message",
            at: now(),
            direction: "outgoing",
            payload: { kind: "text", text: `unsubscribe ${message.topic}` },
            meta: { topic: message.topic, frame: "unsubscribe" },
          });
          break;
        case "ping":
          socket.send(encodePingreq());
          emit({
            type: "message",
            at: now(),
            direction: "outgoing",
            payload: { kind: "text", text: "ping" },
            meta: { frame: "pingreq" },
          });
          break;
        default:
          throw new Error(
            `mqtt cannot send a ${message.kind} message — publish to a topic instead`,
          );
      }
    },
    close: async () => {
      life.stop();
      if (ping !== null) clearInterval(ping);
      if (socket && socket.readyState === WebSocket.OPEN) {
        socket.send(encodeDisconnect());
        socket.close();
      } else emit({ type: "disconnected", at: now(), reason: "closed by you" });
    },
  };
}

export function webStreamOpen(
  spec: StreamSpec,
  channel: { onmessage?: (event: StreamEvent) => void },
): { streamId: string } {
  const id = newId();
  const emit: Emit = (event) => {
    channel.onmessage?.(event);
    if (event.type === "disconnected") {
      const runner = runners.get(id);
      if (runner) runner.open = false;
      runners.delete(id);
    }
  };
  const resolved = resolveSpec(spec);
  const runner =
    spec.kind === "websocket"
      ? openWebSocket(id, spec, resolved, emit)
      : spec.kind === "sse"
        ? openSse(id, spec, resolved, emit)
        : openMqtt(id, spec, resolved, emit);
  runners.set(id, runner);
  return { streamId: id };
}

export function webStreamSend(streamId: string, payload: Outgoing): Promise<void> {
  const runner = runners.get(streamId);
  if (!runner) return Promise.reject(new Error(`there is no open stream ${streamId}`));
  return runner.send(payload);
}

export async function webStreamClose(streamId: string): Promise<void> {
  const runner = runners.get(streamId);
  if (!runner) throw new Error(`there is no open stream ${streamId}`);
  runners.delete(streamId);
  await runner.close();
}

export function webStreamStatus(streamId: string): StreamStatus {
  const runner = runners.get(streamId);
  if (!runner) throw new Error(`there is no open stream ${streamId}`);
  return { id: runner.id, kind: runner.kind, url: runner.url, open: runner.open };
}

export function webStreamList(): StreamStatus[] {
  return [...runners.values()].map((r) => ({
    id: r.id,
    kind: r.kind,
    url: r.url,
    open: r.open,
  }));
}
